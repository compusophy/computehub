//! The compusophyOS browser boundary: the only crate that calls browser APIs.
//!
//! [`run`] finds `<canvas id="os">` and drives an [`App`]: it feeds it [`Event`]s and asks it for
//! frames, which [`Renderer::draw`] draws in one instanced WebGL2 call. Both calls get a [`Ctl`]
//! for text input, fetch, frames, the cursor, `localStorage` and the clocks; what the app asks of
//! it is applied after the app returns, so no browser call re-enters the app. Natively the crate
//! only compiles, for tests: [`run`] needs a browser.
//!
//! Frames are on demand, with no render loop: a redraw or [`Ctl::request_frame`] requests one
//! `requestAnimationFrame` unless one is pending. The flag clears before [`App::frame`] runs, so an
//! animation asks on every frame and the first that does not ask is the last. The timers are the
//! minute tick behind [`Event::Tick`] and the one-shot [`Ctl::wake_in`]. While the WebGL context is
//! lost frames are skipped; on restore the renderer is rebuilt. Program workers ([`Ctl::spawn`])
//! and streams ([`Ctl::stream`]) are heard like DOM events.

#![forbid(unsafe_code)]

mod ctl;
mod io;
mod nav;
mod proc;
mod render;
#[cfg(test)]
mod tests;

pub use ctl::{Ctl, Effect, Load, LocalTime};
pub use nav::{Device, beacon, device};
pub use render::Renderer;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use js_sys::Function;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    AddEventListenerOptions, Document, Event as DomEvent, EventTarget, HtmlCanvasElement,
    HtmlTextAreaElement, KeyboardEvent, MediaQueryList, PointerEvent, WheelEvent, Window,
};

/// Input and environment changes delivered to [`App::event`]. Positions are
/// whole CSS pixels relative to the canvas.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A key went down (or repeated) or up, never while an IME composes.
    /// `code` and `key` are `KeyboardEvent`'s, `altgr` its
    /// `getModifierState("AltGraph")`. Typed text comes as [`Event::Text`].
    Key {
        code: String,
        key: String,
        down: bool,
        repeat: bool,
        shift: bool,
        ctrl: bool,
        alt: bool,
        meta: bool,
        altgr: bool,
    },
    /// Text typed, pasted or composed while text input is on: the data of an
    /// `insert*` input event or a committed composition. Never empty.
    Text(String),
    /// The pointer moved.
    PointerMove { x: f32, y: f32 },
    /// A button (0 primary, 1 middle, 2 secondary) went down; the canvas captures the pointer. Only
    /// a primary pointer is heard, and during a press only the pressing one.
    PointerDown { x: f32, y: f32, button: u8 },
    /// A button went up or the pointer was cancelled (button 0 when the DOM's
    /// is outside `0..=255`).
    PointerUp { x: f32, y: f32, button: u8 },
    /// The pointer left the canvas.
    PointerLeave,
    /// `dy` in CSS pixels, positive down: a line is 16 px, a page the canvas.
    Wheel { x: f32, y: f32, dy: f32 },
    /// The canvas CSS size or `devicePixelRatio` (1 if nonsense) changed; also sent at start.
    Resize { w: f32, h: f32, dpr: f32 },
    /// The local time: at start, each minute, and when a hidden page that
    /// missed a minute is shown again.
    Tick { time: LocalTime },
    /// Fetch `id` ([`Ctl::fetch`]) finished: the body, or why there is none.
    Fetched { id: u32, result: Result<Vec<u8>, String> },
    /// More of stream `id`'s response body ([`Ctl::stream`]).
    Chunk { id: u32, data: Vec<u8> },
    /// Stream `id` ended: its HTTP status (0 if no response came), `"network"` if it failed.
    StreamEnd { id: u32, status: u16, error: String },
    /// The worker of `pid` posted `msg`, after its ring's output as a CONS_WRITE.
    Proc { pid: u32, msg: Vec<u8> },
    /// The worker of `pid` failed to load or threw.
    ProcError { pid: u32 },
    /// The one-shot timer of [`Ctl::wake_in`] fired.
    Wake,
    /// The page was hidden.
    Hidden,
}

/// What an [`App`] did with an [`Event`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    /// Request a frame.
    pub redraw: bool,
    /// `preventDefault` the DOM event; never done to a paste shortcut while
    /// text input is on, so the paste reaches the textarea.
    pub prevent_default: bool,
}

/// A program driven by [`run`]. Calls never overlap.
pub trait App {
    fn event(&mut self, ev: Event, ctl: &mut Ctl) -> Handled;
    /// Draws one frame, normally with one [`Renderer::draw`].
    fn frame(&mut self, r: &mut Renderer, ctl: &mut Ctl);
}

/// Starts `app` on `<canvas id="os">`: sends the first [`Event::Resize`] and [`Event::Tick`], draws
/// the first frame and marks it (`performance.mark("first-frame")`; with `debug` in the query
/// string, `"frame"` after every frame), then listens. The app lives with the page.
///
/// # Errors
///
/// No window, document, body or canvas, no WebGL2, or a shader failure.
pub fn run<A: App + 'static>(app: A) -> Result<(), JsValue> {
    let window = window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("os")
        .ok_or("no <canvas id=\"os\">")?
        .dyn_into()
        .map_err(|_| "#os is not a <canvas>")?;
    let renderer = Renderer::new(&canvas).map_err(|e| JsValue::from_str(&e))?;
    let sink = io::text_sink(&document)?;
    let debug = window.location().search().is_ok_and(|q| has(&q, "debug"));
    let s = Rc::new_cyclic(|me: &Weak<Shared>| Shared {
        window,
        document,
        canvas,
        sink,
        app: RefCell::new(Box::new(app)),
        renderer: RefCell::new(Some(renderer)),
        // No frame can be requested until the first is drawn, below.
        frame_pending: Cell::new(true),
        pressed: Cell::new(None),
        css_h: Cell::new(0.0),
        typing: Cell::new(false),
        raf: handler(me, 0, |s, _, _| {
            s.frame_pending.set(false);
            frame(s);
        }),
        tick_fn: handler(me, 0, |s, _, _| tick(s, false)),
        tick_timer: Cell::new(None),
        time: Cell::new(None),
        cursor: Cell::new("default"),
        later: RefCell::new(Vec::new()),
        later_fn: handler(me, 0, |s, _, _| io::flush_later(s)),
        dpr_watch: RefCell::new(None),
        debug,
        procs: RefCell::new(Vec::new()),
        wake_fn: handler(me, 0, |s, _, _| _ = dispatch(s, Event::Wake)),
        wake_timer: Cell::new(None),
        streams: RefCell::new(Vec::new()),
    });

    resize(&s);
    tick(&s, true);
    s.frame_pending.set(false);
    frame(&s);
    mark(&s.window, "first-frame");
    install(&s)?;
    watch_dpr(&s);
    // The callbacks hold only `Weak`s: this keeps the state for the page's life.
    core::mem::forget(s);
    Ok(())
}

/// What the callbacks share. Borrows of `app` and `renderer` end within a
/// statement, except across [`App::frame`]; effects apply after them.
struct Shared {
    window: Window,
    document: Document,
    canvas: HtmlCanvasElement,
    /// The hidden `<textarea>` that text input goes through.
    sink: HtmlTextAreaElement,
    app: RefCell<Box<dyn App>>,
    /// `None` while the WebGL context is lost.
    renderer: RefCell<Option<Renderer>>,
    /// Whether a frame is requested; cleared just before [`App::frame`].
    frame_pending: Cell<bool>,
    /// The `pointerId` of the press the app is following.
    pressed: Cell<Option<i32>>,
    /// The canvas CSS height, for page-sized wheel deltas.
    css_h: Cell<f32>,
    /// Whether the app asked for text input.
    typing: Cell<bool>,
    raf: Function,
    /// The minute timer: its callback, its handle, and the time last sent.
    tick_fn: Function,
    tick_timer: Cell<Option<i32>>,
    time: Cell<Option<LocalTime>>,
    /// The CSS cursor last written to the canvas.
    cursor: Cell<&'static str>,
    /// Events for the microtask that runs `later_fn`.
    later: RefCell<Vec<Event>>,
    later_fn: Function,
    /// The live `(resolution: Xdppx)` query, kept so its listener lives.
    dpr_watch: RefCell<Option<MediaQueryList>>,
    debug: bool,
    procs: RefCell<Vec<proc::Proc>>,
    /// The one-shot timer of [`Ctl::wake_in`]: its callback and handle.
    wake_fn: Function,
    wake_timer: Cell<Option<i32>>,
    streams: RefCell<Vec<io::Stream>>,
}

/// Which pointer [`Event`] a DOM pointer event becomes.
#[derive(Clone, Copy)]
enum Ptr {
    Down,
    Move,
    Up,
    Leave,
}

/// A callback of the page: a DOM listener or a timer (tag 0), or a worker's (tagged with its pid).
type Handler = fn(&Rc<Shared>, u32, &DomEvent);

/// `f` as a JS function that runs while the state lives, told `tag`. Every callback is this one
/// closure type, which fetches share, so its glue exists once.
fn handler(me: &Weak<Shared>, tag: u32, f: Handler) -> Function {
    let me = me.clone();
    let cb = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        if let Some(s) = me.upgrade() {
            f(&s, tag, e.unchecked_ref());
        }
    });
    cb.into_js_value().unchecked_into()
}

fn install(s: &Rc<Shared>) -> Result<(), JsValue> {
    let me = Rc::downgrade(s);
    let (win, doc): (&EventTarget, &EventTarget) = (&s.window, &s.document);
    let (canvas, sink): (&EventTarget, &EventTarget) = (&s.canvas, &s.sink);
    let listeners: [(&EventTarget, &str, Handler); 16] = [
        (win, "keydown", |s, _, e| on_key(s, e, true)),
        (win, "keyup", |s, _, e| on_key(s, e, false)),
        (win, "resize", |s, _, _| resize(s)),
        (canvas, "pointerdown", |s, _, e| on_pointer(s, e, Ptr::Down)),
        (canvas, "pointermove", |s, _, e| on_pointer(s, e, Ptr::Move)),
        (canvas, "pointerup", |s, _, e| on_pointer(s, e, Ptr::Up)),
        (canvas, "pointercancel", |s, _, e| on_pointer(s, e, Ptr::Up)),
        (canvas, "pointerleave", |s, _, e| on_pointer(s, e, Ptr::Leave)),
        (canvas, "wheel", |s, _, e| on_wheel(s, e)),
        // A press would move focus to the body and end text input.
        (canvas, "mousedown", |s, _, e| {
            if s.typing.get() {
                e.prevent_default();
            }
        }),
        (canvas, "contextmenu", |_, _, e| e.prevent_default()),
        (canvas, "webglcontextlost", |s, _, e| {
            e.prevent_default();
            *s.renderer.borrow_mut() = None;
        }),
        (canvas, "webglcontextrestored", |s, _, _| restore(s)),
        // Hidden pages throttle timers, up to a minute: catch up on return.
        (doc, "visibilitychange", |s, _, _| {
            if s.document.hidden() {
                dispatch(s, Event::Hidden);
            } else {
                tick(s, false);
            }
        }),
        (sink, "input", |s, _, e| io::on_input(s, e)),
        (sink, "compositionend", |s, _, e| io::on_composition_end(s, e)),
    ];
    // Default options: none of these is passive (wheel defaults to passive
    // only on window, document and body), so the app can prevent scrolling.
    let opts = AddEventListenerOptions::new();
    for (on, ty, f) in listeners {
        let f = handler(&me, 0, f);
        on.add_event_listener_with_callback_and_add_event_listener_options(ty, &f, &opts)?;
    }
    Ok(())
}

/// Hands `ev` to the app, requests a frame if asked, then applies its [`Ctl`].
fn dispatch(s: &Rc<Shared>, ev: Event) -> Handled {
    let mut ctl = Ctl::default();
    let h = s.app.borrow_mut().event(ev, &mut ctl);
    if h.redraw {
        request_frame(s);
    }
    io::apply(s, ctl.effects);
    h
}

fn on_key(s: &Rc<Shared>, e: &DomEvent, down: bool) {
    let Some(k) = e.dyn_ref::<KeyboardEvent>().filter(|k| !k.is_composing()) else { return };
    let (code, key) = (k.code(), k.key());
    let (shift, ctrl, alt, meta) = (k.shift_key(), k.ctrl_key(), k.alt_key(), k.meta_key());
    let paste = s.typing.get() && is_paste(&code, &key, [shift, ctrl, alt, meta]);
    let repeat = k.repeat();
    let altgr = k.get_modifier_state("AltGraph");
    let ev = Event::Key { code, key, down, repeat, shift, ctrl, alt, meta, altgr };
    if dispatch(s, ev).prevent_default && !paste {
        e.prevent_default();
    }
}

fn on_pointer(s: &Rc<Shared>, e: &DomEvent, kind: Ptr) {
    let Some(p) = e.dyn_ref::<PointerEvent>() else { return };
    let Some(pressed) = gate(s.pressed.get(), p.pointer_id(), p.is_primary(), kind) else { return };
    s.pressed.set(pressed);
    let (x, y) = (p.offset_x() as f32, p.offset_y() as f32);
    let button = button_u8(p.button());
    let ev = match kind {
        Ptr::Down => {
            let _ = s.canvas.set_pointer_capture(p.pointer_id());
            Event::PointerDown { x, y, button }
        }
        Ptr::Move => Event::PointerMove { x, y },
        Ptr::Up => Event::PointerUp { x, y, button },
        Ptr::Leave => Event::PointerLeave,
    };
    if dispatch(s, ev).prevent_default {
        e.prevent_default();
    }
}

fn on_wheel(s: &Rc<Shared>, e: &DomEvent) {
    let Some(w) = e.dyn_ref::<WheelEvent>() else { return };
    let (x, y) = (w.offset_x() as f32, w.offset_y() as f32);
    let dy = wheel_px(w.delta_y(), w.delta_mode(), s.css_h.get());
    if dispatch(s, Event::Wheel { x, y, dy }).prevent_default {
        e.prevent_default();
    }
}

/// Re-measures the canvas and sends [`Event::Resize`]; always requests a
/// frame, as the drawing surface itself changed.
fn resize(s: &Rc<Shared>) {
    let (w, h, dpr) = measure(s);
    s.css_h.set(h);
    if let Some(r) = s.renderer.borrow_mut().as_mut() {
        r.set_size(w, h, dpr);
    }
    dispatch(s, Event::Resize { w, h, dpr });
    request_frame(s);
}

fn measure(s: &Shared) -> (f32, f32, f32) {
    let rect = s.canvas.get_bounding_client_rect();
    let dpr = sane_dpr(s.window.device_pixel_ratio());
    (rect.width() as f32, rect.height() as f32, dpr)
}

/// Sends [`Event::Tick`] if the local minute changed (or `always`), and sets
/// the one timer for just after the next minute starts.
fn tick(s: &Rc<Shared>, always: bool) {
    let now = js_sys::Date::new_0();
    let time = LocalTime::of(&now);
    arm(s, &s.tick_timer, &s.tick_fn, ms_to_next_minute(now.get_seconds(), now.get_milliseconds()));
    if always || s.time.get() != Some(time) {
        s.time.set(Some(time));
        dispatch(s, Event::Tick { time });
    }
}

/// Replaces the timer in `slot` with one that runs `f` in `ms`.
fn arm(s: &Shared, slot: &Cell<Option<i32>>, f: &Function, ms: u32) {
    if let Some(t) = slot.take() {
        s.window.clear_timeout_with_handle(t);
    }
    let ms = i32::try_from(ms).unwrap_or(i32::MAX);
    slot.set(s.window.set_timeout_with_callback_and_timeout_and_arguments_0(f, ms).ok());
}

/// Watches `(resolution: <dpr>dppx)`; when it stops matching (zoom, another
/// screen), resizes and watches the new ratio.
fn watch_dpr(s: &Rc<Shared>) {
    // JS prints the ratio as `${devicePixelRatio}` would: Rust's float
    // formatting costs ~10 KB.
    let ratio = js_sys::Number::from(s.window.device_pixel_ratio()).to_string_with_radix(10);
    let query = ratio.map(|n| ["(resolution: ", &String::from(n), "dppx)"].concat());
    let Ok(Some(mql)) = query.and_then(|q| s.window.match_media(&q)) else { return };
    let f = handler(&Rc::downgrade(s), 0, |s, _, _| {
        resize(s);
        watch_dpr(s);
    });
    let opts = AddEventListenerOptions::new();
    opts.set_once(true);
    let added =
        mql.add_event_listener_with_callback_and_add_event_listener_options("change", &f, &opts);
    if added.is_ok() {
        *s.dpr_watch.borrow_mut() = Some(mql);
    }
}

fn restore(s: &Shared) {
    match Renderer::new(&s.canvas) {
        Ok(mut r) => {
            let (w, h, dpr) = measure(s);
            r.set_size(w, h, dpr);
            *s.renderer.borrow_mut() = Some(r);
            request_frame(s);
        }
        Err(e) => web_sys::console::error_1(&JsValue::from_str(&e)),
    }
}

/// Requests one animation frame unless one is pending.
fn request_frame(s: &Shared) {
    if !s.frame_pending.get() && s.window.request_animation_frame(&s.raf).is_ok() {
        s.frame_pending.set(true);
    }
}

/// Runs [`App::frame`] if there is a renderer, then applies its [`Ctl`].
fn frame(s: &Rc<Shared>) {
    let mut ctl = Ctl::default();
    {
        let mut slot = s.renderer.borrow_mut();
        let Some(r) = slot.as_mut() else { return };
        s.app.borrow_mut().frame(r, &mut ctl);
    }
    mark_debug(s, "frame", &[]);
    io::apply(s, ctl.effects);
}

fn mark(window: &Window, name: &str) {
    if let Some(p) = window.performance() {
        let _ = p.mark(name);
    }
}

/// With `?debug`, `performance.mark` of `name` and `nums`, `:`-joined.
pub(crate) fn mark_debug(s: &Shared, name: &str, nums: &[u32]) {
    if s.debug {
        let mut name = String::from(name);
        for &n in nums {
            name.push(':');
            io::push_num(&mut name, n);
        }
        mark(&s.window, &name);
    }
}

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(thread_local_v2, js_name = window)]
    static WINDOW: Option<Window>;
}

/// `window`, or `None` outside a page: `web_sys::window` without its search
/// through the other global objects.
fn window() -> Option<Window> {
    WINDOW.with(Clone::clone)
}

/// A thrown or rejected value as text: an `Error`'s message, a string, or a generic note.
pub(crate) fn js_text(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => err.message().into(),
        None => e.as_string().unwrap_or_else(|| "unknown error".to_owned()),
    }
}

/// `devicePixelRatio`, or 1 when it is not a positive finite number.
fn sane_dpr(dpr: f64) -> f32 {
    if dpr.is_finite() && dpr > 0.0 { dpr as f32 } else { 1.0 }
}

/// Whether `hay` contains `needle`: a byte scan, far smaller than `str::contains`.
pub(crate) fn has(hay: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    hay.as_bytes().windows(needle.len()).any(|w| w == needle)
}

/// `PointerEvent.button` as `u8`; out-of-range values (such as -1) are 0.
fn button_u8(b: i16) -> u8 {
    u8::try_from(b).unwrap_or(0)
}

/// Whether a key is a paste shortcut, given `[shift, ctrl, alt, meta]`: Ctrl/Cmd+V or Shift+Insert.
/// The V goes by meaning when `key` is an ASCII letter (Dvorak), else by position (`KeyV`;
/// Cyrillic), as `os` maps shortcut letters.
fn is_paste(code: &str, key: &str, [shift, ctrl, alt, meta]: [bool; 4]) -> bool {
    let v = match key.as_bytes() {
        [b] if b.is_ascii_alphabetic() => b.eq_ignore_ascii_case(&b'v'),
        _ => code == "KeyV",
    };
    ((ctrl || meta) && !alt && v) || (shift && !ctrl && !alt && !meta && code == "Insert")
}

/// `WheelEvent.deltaY` in CSS pixels: mode 1 (lines) counts 16 px, mode 2
/// (pages) `page` px; a non-finite result is 0.
fn wheel_px(delta: f64, mode: u32, page: f32) -> f32 {
    let scale = [1.0, 16.0, f64::from(page)].get(mode as usize).copied().unwrap_or(1.0);
    let px = (delta * scale) as f32;
    if px.is_finite() { px } else { 0.0 }
}

/// Milliseconds from `sec`:`ms` past a minute until 10 ms after the next
/// starts, so a timer that fires a hair early still sees the new minute.
fn ms_to_next_minute(sec: u32, ms: u32) -> u32 {
    60_000 - (sec.min(59) * 1000 + ms.min(999)) + 10
}

/// `Some(the press after it)` if the app hears a pointer event, else `None`:
/// only primary pointers count, and during a press only the pressing one.
fn gate(pressed: Option<i32>, id: i32, primary: bool, kind: Ptr) -> Option<Option<i32>> {
    if !primary || pressed.is_some_and(|p| p != id) {
        return None;
    }
    Some(match kind {
        Ptr::Down => Some(id),
        Ptr::Up => None,
        Ptr::Move | Ptr::Leave => pressed,
    })
}
