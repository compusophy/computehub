//! The compusophyOS browser boundary: the only crate that calls browser APIs.
//!
//! [`run`] takes an [`App`], finds `<canvas id="os">`, and from then on feeds
//! the app [`Event`]s and asks it for frames. The app draws by handing a
//! [`gfx::DrawList`] to [`Renderer::draw`], which uploads the instance bytes
//! and issues one instanced WebGL2 draw call. The crate compiles for the
//! host so the workspace can test natively, but only in the browser does
//! [`run`] do anything.
//!
//! # Frames are on demand
//!
//! Nothing runs while nothing happens: there is no render loop. When
//! [`App::event`] returns [`Handled::redraw`] (or the canvas is resized) and
//! no frame is pending, exactly one `requestAnimationFrame` is requested, and
//! its callback calls [`App::frame`] once. With no input, no frames run and
//! idle CPU is zero.
//!
//! # Events
//!
//! - keydown and keyup on `window` become [`Event::Key`] (`code` is
//!   `KeyboardEvent.code`).
//! - Pointer events on the canvas, in CSS pixels relative to the canvas
//!   (whole pixels: `offsetX` / `offsetY`). Only primary pointers are heard,
//!   and during a press only the pressing one. A pointer down captures the
//!   pointer; pointercancel arrives as [`Event::PointerUp`]; a button number
//!   outside `0..=255` (such as the -1 of pointercancel) arrives as 0.
//! - The canvas context menu is always suppressed.
//! - [`Event::Resize`] carries the canvas CSS size (`getBoundingClientRect`)
//!   and `devicePixelRatio`; it is sent at start, on window resize, and when
//!   the pixel ratio changes (zoom, or a move to another screen).
//!
//! [`Handled::prevent_default`] calls `preventDefault` on the DOM event that
//! produced the [`Event`].
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
//! the renderer is rebuilt and a frame is requested.

#![forbid(unsafe_code)]

mod render;

pub use render::Renderer;

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    AddEventListenerOptions, Event as DomEvent, EventTarget, HtmlCanvasElement, KeyboardEvent,
    MediaQueryList, PointerEvent, Window,
};

/// Input and environment changes delivered to [`App::event`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A key went down (or auto-repeated) or up.
    Key {
        /// `KeyboardEvent.code`: the physical key, such as `"KeyA"`.
        code: String,
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
    /// The canvas has a new size or pixel ratio.
    Resize {
        /// Canvas width in CSS pixels.
        w: f32,
        /// Canvas height in CSS pixels.
        h: f32,
        /// `window.devicePixelRatio` (1 if the browser reports nonsense).
        dpr: f32,
    },
}

/// What an [`App`] did with an [`Event`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    /// Request a frame: [`App::frame`] runs on the next animation frame.
    pub redraw: bool,
    /// Call `preventDefault` on the DOM event, so the browser does not also
    /// act on it (scroll, find-in-page, and so on).
    pub prevent_default: bool,
}

/// A program driven by [`run`].
pub trait App {
    /// Handles one event. Never called during [`App::frame`].
    fn event(&mut self, ev: Event) -> Handled;
    /// Draws one frame, normally with one [`Renderer::draw`] call.
    fn frame(&mut self, r: &mut Renderer);
}

/// Starts `app` on `<canvas id="os">`: creates the renderer, delivers the
/// first [`Event::Resize`], draws the first frame, marks `first-frame`, then
/// installs the event listeners and returns. The listeners keep the app
/// alive for the life of the page.
///
/// # Errors
///
/// When there is no window or document, no `<canvas id="os">`, no WebGL2, or
/// the shaders fail to compile or link (the message includes the info log).
pub fn run<A: App + 'static>(app: A) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("os")
        .ok_or("no <canvas id=\"os\">")?
        .dyn_into()
        .map_err(|_| "#os is not a <canvas>")?;
    let renderer = Renderer::new(&canvas).map_err(|e| JsValue::from_str(&e))?;
    let debug = window.location().search().is_ok_and(|q| is_debug(&q));
    let s = Rc::new(Shared {
        window,
        canvas,
        app: RefCell::new(Box::new(app)),
        renderer: RefCell::new(Some(renderer)),
        frame_pending: Cell::new(false),
        pressed: Cell::new(None),
        raf: OnceCell::new(),
        dpr_watch: RefCell::new(None),
        debug,
    });

    // `raf` is still unset, so the Resize cannot schedule a second frame.
    resize(&s);
    frame(&s);
    mark(&s.window, "first-frame");

    let s2 = s.clone();
    let raf = Closure::<dyn FnMut()>::new(move || {
        s2.frame_pending.set(false);
        frame(&s2);
    });
    let _ = s.raf.set(raf.into_js_value().unchecked_into());
    install(&s)?;
    watch_dpr(&s);
    Ok(())
}

/// Everything the listeners share. Borrows of `app` and `renderer` never
/// outlive the statement that takes them, except in [`frame`], which holds
/// both across [`App::frame`] (nothing in there reaches back into the page).
struct Shared {
    window: Window,
    canvas: HtmlCanvasElement,
    app: RefCell<Box<dyn App>>,
    /// `None` while the WebGL context is lost.
    renderer: RefCell<Option<Renderer>>,
    frame_pending: Cell<bool>,
    /// The `pointerId` of the press the app is following, if any.
    pressed: Cell<Option<i32>>,
    /// The `requestAnimationFrame` callback; set once the first frame is done.
    raf: OnceCell<js_sys::Function>,
    /// The live `(resolution: Xdppx)` query, kept so its listener lives.
    dpr_watch: RefCell<Option<MediaQueryList>>,
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

fn install(s: &Rc<Shared>) -> Result<(), JsValue> {
    for (ty, down) in [("keydown", true), ("keyup", false)] {
        let s2 = s.clone();
        listen(&s.window, ty, move |e| on_key(&s2, &e, down))?;
    }
    let pointer = [
        ("pointerdown", Ptr::Down),
        ("pointermove", Ptr::Move),
        ("pointerup", Ptr::Up),
        ("pointercancel", Ptr::Up),
        ("pointerleave", Ptr::Leave),
    ];
    for (ty, kind) in pointer {
        let s2 = s.clone();
        listen(&s.canvas, ty, move |e| on_pointer(&s2, &e, kind))?;
    }
    listen(&s.canvas, "contextmenu", |e| e.prevent_default())?;
    let s2 = s.clone();
    listen(&s.window, "resize", move |_| resize(&s2))?;
    let s2 = s.clone();
    listen(&s.canvas, "webglcontextlost", move |e| {
        e.prevent_default();
        *s2.renderer.borrow_mut() = None;
    })?;
    let s2 = s.clone();
    listen(&s.canvas, "webglcontextrestored", move |_| restore(&s2))
}

/// Adds a listener that lives as long as the page.
fn listen(on: &EventTarget, ty: &str, f: impl FnMut(DomEvent) + 'static) -> Result<(), JsValue> {
    let f = Closure::<dyn FnMut(DomEvent)>::new(f).into_js_value();
    on.add_event_listener_with_callback(ty, f.unchecked_ref())
}

/// Hands `ev` to the app, then requests a frame if it asked for one.
fn dispatch(s: &Shared, ev: Event) -> Handled {
    let h = s.app.borrow_mut().event(ev);
    if h.redraw {
        request_frame(s);
    }
    h
}

fn on_key(s: &Shared, e: &DomEvent, down: bool) {
    let Some(k) = e.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    let ev = Event::Key {
        code: k.code(),
        down,
        repeat: k.repeat(),
        shift: k.shift_key(),
        ctrl: k.ctrl_key(),
        alt: k.alt_key(),
        meta: k.meta_key(),
        altgr: k.get_modifier_state("AltGraph"),
    };
    if dispatch(s, ev).prevent_default {
        e.prevent_default();
    }
}

fn on_pointer(s: &Shared, e: &DomEvent, kind: Ptr) {
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

/// Re-measures the canvas, updates the renderer and sends [`Event::Resize`].
/// Always requests a frame: the drawing surface itself changed.
fn resize(s: &Shared) {
    let (w, h, dpr) = measure(s);
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

/// Watches `(resolution: <current dpr>dppx)`; when it stops matching, sends
/// a Resize and watches the new ratio. The listener fires once.
fn watch_dpr(s: &Rc<Shared>) {
    // JS prints it as `${devicePixelRatio}` does: Rust float fmt costs ~10 KB.
    let ratio = js_sys::Number::from(s.window.device_pixel_ratio()).to_string_with_radix(10);
    let query = ratio.map(|n| ["(resolution: ", &String::from(n), "dppx)"].concat());
    let Ok(Some(mql)) = query.and_then(|q| s.window.match_media(&q)) else {
        return;
    };
    let s2 = s.clone();
    let f = Closure::once_into_js(move || {
        resize(&s2);
        watch_dpr(&s2);
    });
    let opts = AddEventListenerOptions::new();
    opts.set_once(true);
    let added = mql.add_event_listener_with_callback_and_add_event_listener_options(
        "change",
        f.unchecked_ref(),
        &opts,
    );
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
    let Some(f) = s.raf.get().filter(|_| !s.frame_pending.get()) else {
        return;
    };
    if s.window.request_animation_frame(f).is_ok() {
        s.frame_pending.set(true);
    }
}

/// Runs [`App::frame`] if there is a renderer.
fn frame(s: &Shared) {
    let mut slot = s.renderer.borrow_mut();
    let Some(r) = slot.as_mut() else {
        return;
    };
    s.app.borrow_mut().frame(r);
    drop(slot);
    if s.debug {
        mark(&s.window, "frame");
    }
}

fn mark(window: &Window, name: &str) {
    if let Some(p) = window.performance() {
        let _ = p.mark(name);
    }
}

/// `devicePixelRatio` as `f32`, or 1 when it is not a positive finite number.
fn sane_dpr(dpr: f64) -> f32 {
    if dpr.is_finite() && dpr > 0.0 {
        dpr as f32
    } else {
        1.0
    }
}

/// Whether `location.search` asks for debug marks.
fn is_debug(search: &str) -> bool {
    search.contains("debug")
}

/// `PointerEvent.button` as `u8`; out-of-range values (such as -1) become 0.
fn button_u8(b: i16) -> u8 {
    u8::try_from(b).unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::render::{ATTRIBS, MIN_CAPACITY, backing_size, clear_rgb, grow_capacity};
    use super::*;
    use gfx::{INSTANCE_BYTES, Rgba};

    #[test]
    fn backing_store_is_rounded_physical_pixels() {
        assert_eq!(backing_size((800.0, 600.0), 1.0), (800, 600));
        assert_eq!(backing_size((800.0, 600.0), 2.0), (1600, 1200));
        assert_eq!(backing_size((333.3, 100.2), 1.5), (500, 150));
        assert_eq!(backing_size((0.0, 10.0), 2.0), (1, 20));
        assert_eq!(backing_size((f32::NAN, -5.0), 1.0), (1, 1));
    }

    #[test]
    fn buffer_grows_by_powers_of_two() {
        assert_eq!(grow_capacity(1), MIN_CAPACITY);
        assert_eq!(grow_capacity(MIN_CAPACITY), MIN_CAPACITY);
        assert_eq!(grow_capacity(MIN_CAPACITY + 1), 32768);
        assert_eq!(grow_capacity(1000 * INSTANCE_BYTES), 65536);
        assert_eq!(grow_capacity(1 << 20), 1 << 20);
        assert_eq!(grow_capacity(usize::MAX), usize::MAX);
    }

    #[test]
    fn attributes_match_the_gfx_layout() {
        let got: Vec<(u32, i32)> = ATTRIBS.iter().map(|a| (a.0, a.3)).collect();
        assert_eq!(got, [(0, 0), (1, 16), (2, 32)]);
        // Two vec4 of f32, then four normalized bytes, fill one instance.
        assert_eq!(ATTRIBS[2].3 as usize + 4, INSTANCE_BYTES);
        assert!(ATTRIBS[2].2 && !ATTRIBS[0].2 && !ATTRIBS[1].2);
        for (loc, name) in [(0, "a_rect"), (1, "a_params"), (2, "a_color")] {
            let decl = format!("layout(location = {loc}) in vec4 {name};");
            assert!(gfx::VERTEX_SHADER.contains(&decl), "missing {decl}");
        }
    }

    #[test]
    fn a_second_pointer_cannot_steal_or_end_a_press() {
        use Ptr::{Down, Leave, Move, Up};
        let mut pressed = None;
        let mut heard = |id, primary, kind| {
            let next = gate(pressed, id, primary, kind);
            pressed = next.unwrap_or(pressed);
            next.is_some()
        };
        // Finger 1 drags; finger 2 (not primary) and mouse 3 cut in.
        assert!(heard(1, true, Down) && heard(1, true, Move));
        assert!(!heard(2, false, Down) && !heard(2, false, Move));
        assert!(!heard(3, true, Down) && !heard(3, true, Move));
        assert!(!heard(2, false, Up) && !heard(2, false, Leave));
        assert!(!heard(3, true, Up) && heard(1, true, Move));
        // Finger 1 lifts; the mouse is heard again and takes its own press.
        assert!(heard(1, true, Up) && heard(1, true, Leave));
        assert!(heard(3, true, Move) && heard(3, true, Down));
        assert!(!heard(1, true, Down) && heard(3, true, Up));
        assert!(!heard(2, false, Move) && heard(3, true, Leave));
    }

    #[test]
    fn small_helpers() {
        let dprs = [2.0, 1.25, 0.0, -1.0, f64::NAN, f64::INFINITY].map(sane_dpr);
        assert_eq!(dprs, [2.0, 1.25, 1.0, 1.0, 1.0, 1.0]);
        // The clear color is opaque sRGB as GL floats.
        assert_eq!(clear_rgb(Rgba::hex(0xff0000)), [1.0, 0.0, 0.0]);
        assert_eq!(clear_rgb(Rgba(0, 51, 255, 0)), [0.0, 0.2, 1.0]);
        assert!(is_debug("?debug") && is_debug("?x=1&debug=1"));
        assert!(!is_debug("") && !is_debug("?dbg=1"));
        assert_eq!([0, 2, -1, 300].map(button_u8), [0, 2, 0, 0]);
    }
}
