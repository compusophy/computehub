//! The OS in another page, never in an iframe: [`mount`] boots an [`App`] into an
//! `OffscreenCanvas`, no element of that page (its host shows the frames as it likes: a texture
//! on a monitor in a 3D room, the OS's screen on a desk), with its files under a base path and
//! its storage keys (and its IndexedDB) under a namespace, and no listener on the page. The host
//! tells it its size ([`resize_to`]) and its input ([`inject`]: pointer, wheel, keys, typed
//! text, whether it shows), and copies its surface ([`surface`]) when [`frames`] moved, or reads
//! the last frame as RGBA ([`pixels`]). The OS schedules its own frames and timers in the page
//! as [`crate::run`] does. One mount a page (the OS's state is the page's), and none where
//! [`crate::run`] runs. Its programs run in workers that share memory, so the page must be
//! cross-origin isolated (COOP `same-origin`, COEP `require-corp`) and its files served beside it.
//! Its `/api/*` calls (the free AI, feedback, the mesh's signaling) go to the OS's own site
//! ([`HOME`], whose functions answer the pages it is mounted in: `api/*.mjs`'s friends).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsValue;
use web_sys::{AddEventListenerOptions, OffscreenCanvas};

use crate::render::Surface;
use crate::{
    App, Event, Renderer, SHARED, Shared, dispatch, frame, handler, resize, restore, sane_dpr,
    shared, tick, window,
};

/// Where and how big: `w` x `h` CSS px at pixel ratio `dpr`, the OS's files under `base` (a path
/// ending in `/`, relative to the page or absolute: `cpu/worker.js`, the fonts and `bin/` are
/// found there), its storage keys (localStorage, sessionStorage, IndexedDB) beginning `ns`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mount {
    pub w: f32,
    pub h: f32,
    pub dpr: f32,
    pub base: String,
    pub ns: String,
}

/// The OS's own site, whose `/api/*` functions a mounted OS calls.
pub const HOME: &str = "https://compusophy.com";

thread_local! {
    /// A mount's base path and storage namespace (a page's: none).
    static PLACE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// The mount's files are under `base`, its storage keys begin `ns` (none: the page's own).
pub(crate) fn place(at: Option<(String, String)>) {
    PLACE.with(|p| *p.borrow_mut() = at);
}

/// The mount's base and namespace, `f` of them (a page's: both empty).
fn with_place<T>(f: impl FnOnce(&str, &str) -> T) -> T {
    PLACE.with(|p| match &*p.borrow() {
        Some((base, ns)) => f(base, ns),
        None => f("", ""),
    })
}

/// `url` (relative) under the mount's base.
pub(crate) fn based(url: &str) -> String {
    with_place(|base, _| [base, url].concat())
}

/// Storage key `key` under the mount's namespace.
pub(crate) fn keyed(key: &str) -> String {
    with_place(|_, ns| [ns, key].concat())
}

/// `url` as the network takes it: a mounted OS's `/api/*` at [`HOME`] (the page it is mounted in
/// has none); a page's, and anything else, as it is.
pub(crate) fn api(url: &str) -> String {
    let mounted = PLACE.with(|p| p.borrow().is_some());
    match mounted && url.starts_with("/api/") {
        true => [HOME, url].concat(),
        false => url.to_string(),
    }
}

/// Whether the page holds `<canvas id="os">`, where [`crate::run`] runs; a page without one may
/// [`mount`].
pub fn page_canvas() -> bool {
    let doc = window().and_then(|w| w.document());
    doc.and_then(|d| d.get_element_by_id("os")).is_some()
}

/// Boots `app` into an `OffscreenCanvas` as `m` says: sends the first Resize and Tick and draws
/// the first frame (see the module docs).
///
/// # Errors
///
/// A mount or a run already in the page, no window, no WebGL2, or a shader failure.
pub fn mount<A: App + 'static>(app: A, m: Mount) -> Result<(), JsValue> {
    if SHARED.with(Cell::get).is_some() {
        return Err("the OS already runs in this page".into());
    }
    let window = window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let (w, h, dpr) = (m.w.max(1.0), m.h.max(1.0), sane_dpr(m.dpr.into()));
    let off = OffscreenCanvas::new((w * dpr).round() as u32, (h * dpr).round() as u32)?;
    place(Some((m.base, m.ns)));
    let canvas = Surface::Off(off);
    let renderer = Renderer::new(&canvas).map_err(|e| JsValue::from_str(&e))?;
    let app = Box::new(app);
    let s = shared(window, document, canvas, None, app, renderer, Some((w, h, dpr)), false);
    resize(&s);
    tick(&s, true);
    s.frame_pending.set(false);
    frame(&s);
    listen(&s)?;
    let _ = SHARED.try_with(|c| c.set(Some(Box::leak(Box::new(s)))));
    Ok(())
}

/// The surface's context lost and restored: all a mount listens to.
fn listen(s: &Rc<Shared>) -> Result<(), JsValue> {
    let me = Rc::downgrade(s);
    let lost = handler(&me, 0, |s, _, e| {
        e.prevent_default();
        *s.renderer.borrow_mut() = None;
    });
    let back = handler(&me, 0, |s, _, _| restore(s));
    let opts = AddEventListenerOptions::new();
    let on = s.canvas.target();
    on.add_event_listener_with_callback_and_add_event_listener_options(
        "webglcontextlost",
        &lost,
        &opts,
    )?;
    on.add_event_listener_with_callback_and_add_event_listener_options(
        "webglcontextrestored",
        &back,
        &opts,
    )?;
    Ok(())
}

/// `f` on the mount's state, if the OS runs in this page.
fn with<T>(f: impl FnOnce(&Rc<Shared>) -> T) -> Option<T> {
    SHARED.with(Cell::get).map(f)
}

/// Hands `ev` to the OS as the page's own input would be (positions in the mount's CSS px);
/// whether it would prevent the event's default (the host's own handling of it). A shown
/// mount is `Tick`ed to catch up; hidden, it hears [`Event::Hidden`].
pub fn inject(ev: Event) -> bool {
    with(|s| dispatch(s, ev).prevent_default).unwrap_or(false)
}

/// Whether the host shows the mount (a monitor in view): shown again, its clock catches up.
pub fn visible(on: bool) {
    with(|s| if on { tick(s, false) } else { _ = dispatch(s, Event::Hidden) });
}

/// The mount is now `w` x `h` CSS px at `dpr`.
pub fn resize_to(w: f32, h: f32, dpr: f32) {
    with(|s| {
        s.mounted.set(Some((w.max(1.0), h.max(1.0), sane_dpr(dpr.into()))));
        resize(s);
    });
}

/// The mount's surface, whose last frame stays until the next.
pub fn surface() -> Option<OffscreenCanvas> {
    with(|s| match &s.canvas {
        Surface::Off(c) => Some(c.clone()),
        Surface::Page(_) => None,
    })
    .flatten()
}

/// How many frames the OS drew: copy the surface when it moved.
pub fn frames() -> u32 {
    with(|s| s.frames.get()).unwrap_or(0)
}

/// The CSS cursor the OS wants over its screen.
pub fn cursor() -> &'static str {
    with(|s| s.cursor.get()).unwrap_or("default")
}

/// Whether the OS takes typed text now (a field has the keys): send it as [`Event::Text`].
pub fn typing() -> bool {
    with(|s| s.typing.get()).unwrap_or(false)
}

/// The last frame's size in device px, and the frame into `out` as RGBA rows, the top first
/// (width x height x 4 bytes); false: none, or another size.
pub fn pixels(out: &mut [u8]) -> bool {
    with(|s| s.renderer.borrow().as_ref().is_some_and(|r| r.pixels(out))).unwrap_or(false)
}

/// The last frame's size in device px.
pub fn backing() -> (u32, u32) {
    with(|s| s.renderer.borrow().as_ref().map(Renderer::backing)).flatten().unwrap_or((0, 0))
}
