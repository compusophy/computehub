//! The compusophyOS browser boundary: the only crate that calls browser APIs.
//!
//! [`run`] takes an [`App`], finds `<canvas id="os">`, and from then on feeds
//! the app [`Event`]s and asks it for frames. The app draws by handing a
//! [`gfx::DrawList`] and the glyph [`gfx::Atlas`] to [`Renderer::draw`],
//! which uploads the instance bytes (and the atlas rows that changed) and
//! issues one instanced WebGL2 draw call. Both calls get a [`Ctl`], the
//! app's handle on the rest of the page: text input, WebSockets, fetch, the
//! location hash and the clock. The crate compiles for the host so the
//! workspace can test natively, but only in the browser does [`run`] do
//! anything.
//!
//! # Frames are on demand
//!
//! Nothing runs while nothing happens: there is no render loop. When
//! [`App::event`] returns [`Handled::redraw`] (or the canvas is resized) and
//! no frame is pending, exactly one `requestAnimationFrame` is requested, and
//! its callback calls [`App::frame`] once. The only timer is the one behind
//! [`Event::Tick`], which fires once a minute; idle CPU is otherwise zero.
//!
//! # Events
//!
//! - keydown and keyup on `window` become [`Event::Key`] (`code` is
//!   `KeyboardEvent.code`, `key` is `KeyboardEvent.key`). Keys pressed while
//!   an IME composes are not delivered: the composition owns them.
//!   Browsers keep a few shortcuts for themselves in a tab (Ctrl+W, Ctrl+T,
//!   Ctrl+N; Cmd+W and Cmd+Q on macOS): those never arrive, or arrive and
//!   cannot be prevented, so Ctrl+W closes the tab even when the app wants
//!   it (readline's delete-word). Chromium passes them to a page only in an
//!   installed app's own window, or in fullscreen with the Keyboard Lock
//!   API. [`Ctl::guard_unload`] makes closing the tab ask first.
//! - Text comes only from the hidden `<textarea>` that [`Ctl::set_text_input`]
//!   focuses, never from keydown: see [`Event::Text`].
//! - Pointer events on the canvas, in CSS pixels relative to the canvas
//!   (whole pixels: `offsetX` / `offsetY`). Only primary pointers are heard,
//!   and during a press only the pressing one. A pointer down captures the
//!   pointer; pointercancel arrives as [`Event::PointerUp`]; a button number
//!   outside `0..=255` (such as the -1 of pointercancel) arrives as 0.
//! - wheel on the canvas becomes [`Event::Wheel`] (a non-passive listener,
//!   so the app can prevent scrolling and pinch zoom).
//! - The canvas context menu is always suppressed.
//! - [`Event::Resize`] carries the canvas CSS size (`getBoundingClientRect`)
//!   and `devicePixelRatio`; it is sent at start, on window resize, and when
//!   the pixel ratio changes (zoom, or a move to another screen).
//! - [`Event::Tick`] is sent at start and at each local minute boundary.
//! - hashchange on `window` becomes [`Event::HashChange`].
//! - [`Event::Ws`] and [`Event::Fetched`] report what [`Ctl`] started.
//!
//! [`Handled::prevent_default`] calls `preventDefault` on the DOM event that
//! produced the [`Event`], with one exception: while text input is active,
//! Ctrl+V, Cmd+V and Shift+Insert are never prevented, so the paste reaches
//! the textarea (and arrives as [`Event::Text`]). The V is the key whose
//! `key` is `v` when `key` is an ASCII letter (Dvorak's V is not on `KeyV`),
//! else the key at `KeyV` (a Cyrillic layout's).
//!
//! # Ctl effects
//!
//! What an app asks of its [`Ctl`] is applied after [`App::event`] or
//! [`App::frame`] returns, so no browser call re-enters the app; events
//! those calls cause synchronously (a blur ends a composition) are
//! dispatched then, and asynchronous results arrive as later events.
//!
//! # Timing marks
//!
//! `performance.mark("first-frame")` runs once the first frame is drawn,
//! synchronously inside [`run`]. If the page's query string contains
//! `debug`, `performance.mark("frame")` runs after every frame.
//!
//! # Lost contexts
//!
//! When the WebGL context is lost, frames are skipped. When it is restored,
//! the renderer is rebuilt (the next draw uploads the whole atlas again) and
//! a frame is requested.

#![forbid(unsafe_code)]

mod ctl;
mod io;
mod render;
#[cfg(test)]
mod tests;

pub use ctl::{Ctl, Effect};
pub use render::Renderer;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use js_sys::Function;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    AddEventListenerOptions, Document, Event as DomEvent, EventTarget, HtmlCanvasElement,
    HtmlTextAreaElement, KeyboardEvent, MediaQueryList, PointerEvent, WheelEvent, Window,
};

/// Input and environment changes delivered to [`App::event`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A key went down (or auto-repeated) or up.
    Key {
        /// `KeyboardEvent.code`: the physical key, such as `"KeyA"`.
        code: String,
        /// `KeyboardEvent.key`: what the key means with the current layout
        /// and modifiers, such as `"a"`, `"A"`, `"Enter"`; `"Unidentified"`
        /// or `"Process"` from phone keyboards and IMEs. For shortcuts and
        /// named keys only: typed text arrives as [`Event::Text`].
        key: String,
        /// Whether the key went down.
        down: bool,
        /// Whether this is an auto-repeat.
        repeat: bool,
        /// Shift held.
        shift: bool,
        /// Control held.
        ctrl: bool,
        /// Alt (Option) held.
        alt: bool,
        /// Meta (Command, Windows) held.
        meta: bool,
        /// `getModifierState("AltGraph")`. Chrome and Edge on Windows report
        /// AltGr as `ctrl` and `alt` too; Firefox on macOS sets it for Option.
        altgr: bool,
    },
    /// Text typed, pasted or composed while text input is active
    /// ([`Ctl::set_text_input`]): the data of each `input` event of an
    /// `insert*` type (a printable key the app did not prevent, a paste, a
    /// phone keyboard, `"\n"` for an unprevented Enter), and the committed
    /// string of each IME composition. Never empty.
    Text(String),
    /// The pointer moved to (`x`, `y`), in CSS pixels relative to the canvas.
    PointerMove { x: f32, y: f32 },
    /// A pointer button went down at (`x`, `y`); the canvas captures the
    /// pointer.
    PointerDown {
        x: f32,
        y: f32,
        /// `PointerEvent.button`: 0 primary, 1 middle, 2 secondary.
        button: u8,
    },
    /// A pointer button went up at (`x`, `y`), or the pointer was cancelled.
    PointerUp {
        x: f32,
        y: f32,
        /// `PointerEvent.button`: 0 primary, 1 middle, 2 secondary.
        button: u8,
    },
    /// The pointer left the canvas.
    PointerLeave,
    /// The wheel turned (or a touchpad scrolled) over (`x`, `y`), in CSS
    /// pixels relative to the canvas.
    Wheel {
        x: f32,
        y: f32,
        /// `WheelEvent.deltaY` in CSS pixels, positive to scroll down: line
        /// deltas count 16 px, page deltas the canvas height.
        dy: f32,
    },
    /// The canvas has a new size or pixel ratio.
    Resize {
        /// Canvas width in CSS pixels.
        w: f32,
        /// Canvas height in CSS pixels.
        h: f32,
        /// `window.devicePixelRatio` (1 if the browser reports nonsense).
        dpr: f32,
    },
    /// The local time of day, at start and whenever the minute changes.
    Tick {
        /// Minutes since local midnight, `0..1440`.
        minutes: u32,
    },
    /// The URL fragment changed (`hashchange`: the user edited it, opened a
    /// link that differs only after the `#`, or went back or forward): the
    /// new `location.hash` without its leading `#`, as the URL has it
    /// (percent-encoded), and `""` when it is gone.
    /// [`Ctl::clear_location_hash`] causes none. It cannot be cancelled, so
    /// [`Handled::prevent_default`] does nothing.
    HashChange(String),
    /// Something happened on WebSocket `id` ([`Ctl::ws_open`]).
    Ws { id: u32, ev: WsEvent },
    /// Fetch `id` finished ([`Ctl::fetch`]): the response body, or why
    /// there is none.
    Fetched { id: u32, result: Result<Vec<u8>, String> },
}

/// What happened on a WebSocket, in [`Event::Ws`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsEvent {
    /// The connection is open; queued sends have gone out.
    Open,
    /// One message: the bytes of a binary one, the UTF-8 of a text one.
    Data(Vec<u8>),
    /// The socket closed; nothing more arrives for it. `code` 1006 means it
    /// closed abnormally (it never connected, or the connection dropped).
    Closed { code: u16, reason: String },
    /// The browser reported an error; `Closed` follows.
    Error,
}

/// What an [`App`] did with an [`Event`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    /// Request a frame: [`App::frame`] runs on the next animation frame.
    pub redraw: bool,
    /// Call `preventDefault` on the DOM event, so the browser does not also
    /// act on it (scroll, find-in-page, typing into the textarea, and so on).
    pub prevent_default: bool,
}

/// A program driven by [`run`].
pub trait App {
    /// Handles one event. Never called during [`App::frame`] or while
    /// another call is running.
    fn event(&mut self, ev: Event, ctl: &mut Ctl) -> Handled;
    /// Draws one frame, normally with one [`Renderer::draw`] call.
    fn frame(&mut self, r: &mut Renderer, ctl: &mut Ctl);
}

/// Starts `app` on `<canvas id="os">`: creates the renderer and the hidden
/// text-input `<textarea>`, delivers the first [`Event::Resize`] and
/// [`Event::Tick`], draws the first frame, marks `first-frame`, then installs
/// the event listeners and returns. The app lives as long as the page.
///
/// # Errors
///
/// When there is no window, document or body, no `<canvas id="os">`, no
/// WebGL2, or the shaders fail to compile or link (the message includes the
/// info log).
pub fn run<A: App + 'static>(app: A) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("os")
        .ok_or("no <canvas id=\"os\">")?
        .dyn_into()
        .map_err(|_| "#os is not a <canvas>")?;
    let renderer = Renderer::new(&canvas).map_err(|e| JsValue::from_str(&e))?;
    let sink = io::text_sink(&document)?;
    let debug = window.location().search().is_ok_and(|q| is_debug(&q));
    let s = Rc::new_cyclic(|me: &Weak<Shared>| Shared {
        window,
        document,
        canvas,
        sink,
        app: RefCell::new(Box::new(app)),
        renderer: RefCell::new(Some(renderer)),
        // Until the first frame is drawn, below, no other can be requested.
        frame_pending: Cell::new(true),
        pressed: Cell::new(None),
        css_h: Cell::new(0.0),
        typing: Cell::new(false),
        raf: handler(me, |s, _| {
            s.frame_pending.set(false);
            frame(s);
        }),
        tick_fn: handler(me, |s, _| tick(s, false)),
        tick_timer: Cell::new(None),
        minutes: Cell::new(None),
        ws_fn: handler(me, io::on_ws),
        sockets: RefCell::new(Vec::new()),
        later: RefCell::new(Vec::new()),
        later_fn: handler(me, io::flush_later),
        dpr_watch: RefCell::new(None),
        unload_fn: handler(me, |_, e| e.prevent_default()),
        guarded: Cell::new(false),
        debug,
    });

    resize(&s);
    tick(&s, true);
    frame(&s);
    mark(&s.window, "first-frame");
    s.frame_pending.set(false);
    install(&s)?;
    watch_dpr(&s);
    // Every callback holds only a `Weak`: this reference keeps the state
    // alive for the life of the page.
    core::mem::forget(s);
    Ok(())
}

/// Reads `location.hash` without its `#` and, if it is not empty, removes it
/// from the address bar at once with `history.replaceState` (as
/// [`Ctl::clear_location_hash`] does; no `hashchange` fires). Call it first,
/// before [`run`] or anything else that can fail, so a secret in the
/// fragment never stays in the address bar, even when the page cannot start
/// (no WebGL2, say). Outside the browser it returns `""`.
pub fn take_location_hash() -> String {
    let wasm = cfg!(target_arch = "wasm32");
    let Some(window) = wasm.then(web_sys::window).flatten() else {
        return String::new();
    };
    let hash = ctl::hash_of(&window);
    if !hash.is_empty() {
        io::clear_hash(&window);
    }
    hash
}

/// Everything the callbacks share. Borrows of `app` and `renderer` never
/// outlive the statement that takes them, except in [`frame`], which holds
/// both across [`App::frame`]; [`Ctl`] effects run after every borrow ends.
struct Shared {
    window: Window,
    document: Document,
    canvas: HtmlCanvasElement,
    /// The hidden `<textarea>` that text input goes through.
    sink: HtmlTextAreaElement,
    app: RefCell<Box<dyn App>>,
    /// `None` while the WebGL context is lost.
    renderer: RefCell<Option<Renderer>>,
    /// Whether a frame is requested (or the first is not drawn yet).
    frame_pending: Cell<bool>,
    /// The `pointerId` of the press the app is following, if any.
    pressed: Cell<Option<i32>>,
    /// The canvas CSS height, as last measured (for page-sized wheel deltas).
    css_h: Cell<f32>,
    /// Whether the app asked for text input.
    typing: Cell<bool>,
    /// The `requestAnimationFrame` callback.
    raf: Function,
    /// The minute timer's callback, its pending handle, and the minute last
    /// sent as [`Event::Tick`].
    tick_fn: Function,
    tick_timer: Cell<Option<i32>>,
    minutes: Cell<Option<u32>>,
    /// The one handler of every WebSocket event; it finds the socket by the
    /// event's target.
    ws_fn: Function,
    sockets: RefCell<Vec<io::Sock>>,
    /// Events waiting for the microtask that runs `later_fn`.
    later: RefCell<Vec<Event>>,
    later_fn: Function,
    /// The live `(resolution: Xdppx)` query, kept so its listener lives.
    dpr_watch: RefCell<Option<MediaQueryList>>,
    /// The `beforeunload` listener, and whether it is on `window`
    /// ([`Ctl::guard_unload`]).
    unload_fn: Function,
    guarded: Cell<bool>,
    debug: bool,
}

/// Which pointer [`Event`] a DOM pointer event becomes.
#[derive(Clone, Copy)]
enum Ptr {
    Down,
    Move,
    Up,
    Leave,
}

/// A callback of the page: a DOM listener, a timer, a socket handler.
type Handler = fn(&Rc<Shared>, &DomEvent);

/// `f` as a JS function that runs while the state lives. Every callback is
/// this one closure type, so its glue is compiled once.
fn handler(me: &Weak<Shared>, f: Handler) -> Function {
    let me = me.clone();
    let cb = Closure::<dyn FnMut(DomEvent)>::new(move |e: DomEvent| {
        if let Some(s) = me.upgrade() {
            f(&s, &e);
        }
    });
    cb.into_js_value().unchecked_into()
}

fn install(s: &Rc<Shared>) -> Result<(), JsValue> {
    let me = Rc::downgrade(s);
    let (win, doc): (&EventTarget, &EventTarget) = (&s.window, &s.document);
    let (canvas, sink): (&EventTarget, &EventTarget) = (&s.canvas, &s.sink);
    let listeners: [(&EventTarget, &str, Handler); 17] = [
        (win, "keydown", |s, e| on_key(s, e, true)),
        (win, "keyup", |s, e| on_key(s, e, false)),
        (win, "resize", |s, _| resize(s)),
        (win, "hashchange", |s, _| {
            dispatch(s, Event::HashChange(ctl::hash_of(&s.window)));
        }),
        (canvas, "pointerdown", |s, e| on_pointer(s, e, Ptr::Down)),
        (canvas, "pointermove", |s, e| on_pointer(s, e, Ptr::Move)),
        (canvas, "pointerup", |s, e| on_pointer(s, e, Ptr::Up)),
        (canvas, "pointercancel", |s, e| on_pointer(s, e, Ptr::Up)),
        (canvas, "pointerleave", |s, e| on_pointer(s, e, Ptr::Leave)),
        (canvas, "wheel", on_wheel),
        // A press on the canvas would move focus to the body and so end text
        // input: keep the focus where the app put it.
        (canvas, "mousedown", |s, e| {
            if s.typing.get() {
                e.prevent_default();
            }
        }),
        (canvas, "contextmenu", |_, e| e.prevent_default()),
        (canvas, "webglcontextlost", |s, e| {
            e.prevent_default();
            *s.renderer.borrow_mut() = None;
        }),
        (canvas, "webglcontextrestored", |s, _| restore(s)),
        // Hidden pages throttle timers, up to a minute: catch up on return.
        (doc, "visibilitychange", |s, _| {
            if !s.document.hidden() {
                tick(s, false);
            }
        }),
        (sink, "input", io::on_input),
        (sink, "compositionend", io::on_composition_end),
    ];
    // Not passive, so the app can prevent scrolling and pinch zoom on the
    // wheel; that is already the default for every other listener here.
    let opts = AddEventListenerOptions::new();
    opts.set_passive(false);
    for (on, ty, f) in listeners {
        let f = handler(&me, f);
        on.add_event_listener_with_callback_and_add_event_listener_options(ty, &f, &opts)?;
    }
    Ok(())
}

/// Hands `ev` to the app, requests a frame if it asked for one, then applies
/// what it asked of its [`Ctl`] (the app borrow has ended by then).
fn dispatch(s: &Rc<Shared>, ev: Event) -> Handled {
    let mut ctl = Ctl::new();
    let h = s.app.borrow_mut().event(ev, &mut ctl);
    if h.redraw {
        request_frame(s);
    }
    io::apply(s, ctl.into_effects());
    h
}

fn on_key(s: &Rc<Shared>, e: &DomEvent, down: bool) {
    let Some(k) = e.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if k.is_composing() {
        return;
    }
    let (code, key) = (k.code(), k.key());
    let (shift, ctrl, alt, meta) = (k.shift_key(), k.ctrl_key(), k.alt_key(), k.meta_key());
    let paste = s.typing.get() && is_paste(&code, &key, [shift, ctrl, alt, meta]);
    let ev = Event::Key {
        code,
        key,
        down,
        repeat: k.repeat(),
        shift,
        ctrl,
        alt,
        meta,
        altgr: k.get_modifier_state("AltGraph"),
    };
    if dispatch(s, ev).prevent_default && !paste {
        e.prevent_default();
    }
}

fn on_pointer(s: &Rc<Shared>, e: &DomEvent, kind: Ptr) {
    let Some(p) = e.dyn_ref::<PointerEvent>() else {
        return;
    };
    let Some(pressed) = gate(s.pressed.get(), p.pointer_id(), p.is_primary(), kind) else {
        return;
    };
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
    let Some(w) = e.dyn_ref::<WheelEvent>() else {
        return;
    };
    let (x, y) = (w.offset_x() as f32, w.offset_y() as f32);
    let dy = wheel_px(w.delta_y(), w.delta_mode(), s.css_h.get());
    if dispatch(s, Event::Wheel { x, y, dy }).prevent_default {
        e.prevent_default();
    }
}

/// Re-measures the canvas, updates the renderer and sends [`Event::Resize`].
/// Always requests a frame: the drawing surface itself changed.
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

/// Sends [`Event::Tick`] if the local minute changed since the last one (or
/// `always`), and sets the one timer for just after the next minute starts.
fn tick(s: &Rc<Shared>, always: bool) {
    if let Some(t) = s.tick_timer.take() {
        s.window.clear_timeout_with_handle(t);
    }
    let now = js_sys::Date::new_0();
    let minutes = now.get_hours() * 60 + now.get_minutes();
    let delay = ms_to_next_minute(now.get_seconds(), now.get_milliseconds());
    let timer = s.window.set_timeout_with_callback_and_timeout_and_arguments_0(&s.tick_fn, delay);
    s.tick_timer.set(timer.ok());
    if always || s.minutes.get() != Some(minutes) {
        s.minutes.set(Some(minutes));
        dispatch(s, Event::Tick { minutes });
    }
}

/// Watches `(resolution: <current dpr>dppx)`; when it stops matching, sends
/// a Resize and watches the new ratio. The listener fires once.
fn watch_dpr(s: &Rc<Shared>) {
    // JS prints it as `${devicePixelRatio}` does: Rust float fmt costs ~10 KB.
    let ratio = js_sys::Number::from(s.window.device_pixel_ratio()).to_string_with_radix(10);
    let query = ratio.map(|n| ["(resolution: ", &String::from(n), "dppx)"].concat());
    let Ok(Some(mql)) = query.and_then(|q| s.window.match_media(&q)) else {
        return;
    };
    let f = handler(&Rc::downgrade(s), |s, _| {
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

/// Requests one animation frame unless one is pending (or the first frame
/// has not been drawn yet).
fn request_frame(s: &Shared) {
    if !s.frame_pending.get() && s.window.request_animation_frame(&s.raf).is_ok() {
        s.frame_pending.set(true);
    }
}

/// Runs [`App::frame`] if there is a renderer, then applies its [`Ctl`].
fn frame(s: &Rc<Shared>) {
    let mut ctl = Ctl::new();
    {
        let mut slot = s.renderer.borrow_mut();
        let Some(r) = slot.as_mut() else {
            return;
        };
        s.app.borrow_mut().frame(r, &mut ctl);
    }
    if s.debug {
        mark(&s.window, "frame");
    }
    io::apply(s, ctl.into_effects());
}

fn mark(window: &Window, name: &str) {
    if let Some(p) = window.performance() {
        let _ = p.mark(name);
    }
}

/// A thrown or rejected JS value as text: an `Error`'s message, a string
/// itself, else a generic note.
pub(crate) fn js_text(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => err.message().into(),
        None => e.as_string().unwrap_or_else(|| "unknown error".to_owned()),
    }
}

/// `devicePixelRatio` as `f32`, or 1 when it is not a positive finite number.
fn sane_dpr(dpr: f64) -> f32 {
    if dpr.is_finite() && dpr > 0.0 { dpr as f32 } else { 1.0 }
}

/// Whether `location.search` asks for debug marks.
fn is_debug(search: &str) -> bool {
    has(search, "debug")
}

/// Whether `hay` contains `needle` (not empty): a byte scan, which is all
/// these short strings need and far smaller than `str::contains`.
pub(crate) fn has(hay: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    hay.as_bytes().windows(needle.len()).any(|w| w == needle)
}

/// `PointerEvent.button` as `u8`; out-of-range values (such as -1) become 0.
fn button_u8(b: i16) -> u8 {
    u8::try_from(b).unwrap_or(0)
}

/// Whether a key press is a paste shortcut, given `[shift, ctrl, alt, meta]`:
/// Ctrl+V or Cmd+V, or Shift+Insert. The V goes by meaning when `key` is an
/// ASCII letter (Dvorak's `v` on `Period`; its `k` on `KeyV` is no V), else
/// by position (`KeyV`, for layouts like Cyrillic): the rule `os` maps
/// shortcut letters by, so the shell sees the same key.
fn is_paste(code: &str, key: &str, [shift, ctrl, alt, meta]: [bool; 4]) -> bool {
    let v = match key.as_bytes() {
        [b] if b.is_ascii_alphabetic() => b.eq_ignore_ascii_case(&b'v'),
        _ => code == "KeyV",
    };
    ((ctrl || meta) && !alt && v) || (shift && !ctrl && !alt && !meta && code == "Insert")
}

/// `WheelEvent.deltaY` in CSS pixels: `DOM_DELTA_LINE` (1) counts 16 px,
/// `DOM_DELTA_PAGE` (2) counts `page` px; a non-finite result is 0.
fn wheel_px(delta: f64, mode: u32, page: f32) -> f32 {
    let scale = match mode {
        1 => 16.0,
        2 => f64::from(page),
        _ => 1.0,
    };
    let px = (delta * scale) as f32;
    if px.is_finite() { px } else { 0.0 }
}

/// Milliseconds from `sec`:`ms` past a minute until just after the next
/// minute starts (10 ms late, so a timer that fires a hair early still sees
/// the new minute).
fn ms_to_next_minute(sec: u32, ms: u32) -> i32 {
    let into = sec.min(59) * 1000 + ms.min(999);
    (60_000 - into + 10) as i32
}

/// `Some(the pointer pressed after it)` if the app hears a pointer event, else
/// `None`. Only primary pointers count, and during a press only the pressing
/// one: a second finger or a touch mid-drag can neither steal nor end it.
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
