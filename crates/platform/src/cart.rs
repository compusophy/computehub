//! Cartridges in the OS: another page's program (a secretspace game, the OS itself) run in a
//! window, never in an iframe. A cartridge is an ES module whose `default` loads its wasm and
//! whose `Cartridge` class draws into an `OffscreenCanvas` of its own and takes its host's input:
//! the protocol `os::cartridge` serves when the OS is the cartridge. [`load`] imports the module
//! under a URL of its own (one cartridge a module instance, so the same game twice, or a game
//! inside itself, gets two), awaits its `default`, then constructs it; the renderer copies its
//! surface into a texture whenever its frame count moved ([`gfx::Kind::Image`] draws it); the
//! app hands it its input through the calls below, which never throw (a call that fails answers
//! nothing). A cartridge that shows compusophyOS on a screen of its own (Battlestation) is handed
//! a module of the OS loaded for it alone (`os`). Importing a module is the one thing Rust cannot
//! say, so [`load`] makes `import(u)` a function.

use std::cell::{Cell, RefCell};

use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// What [`load`] starts: the module at `url`, a surface `w` x `h` CSS px at `dpr`, its files
/// under `base` and its storage keys under `ns` (both as the protocol says), `query` in place of
/// a page's, and for a cartridge that shows compusophyOS the OS's files' URL (`os`, ending `/`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub url: String,
    pub w: f32,
    pub h: f32,
    pub dpr: f32,
    pub base: String,
    pub ns: String,
    pub query: String,
    pub os: Option<String>,
}

/// Where a cartridge is: loading, running, or failed (why).
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Loading,
    Ready,
    Failed(String),
}

struct Cart {
    id: u32,
    state: State,
    obj: JsValue,
}

thread_local! {
    static CARTS: RefCell<Vec<Cart>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

/// Starts loading `spec`'s cartridge; its number, for the calls below. Natively it fails at once.
pub fn load(spec: Spec) -> u32 {
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    CARTS.with(|c| c.borrow_mut().push(Cart { id, state: State::Loading, obj: JsValue::NULL }));
    if !cfg!(target_arch = "wasm32") {
        set(id, State::Failed("cartridges need a browser".into()), JsValue::NULL);
        return id;
    }
    // `import(u)`: the module system's own function, which Rust has no word for.
    let import = Function::new_with_args("u", "return import(u)");
    let get = |u: &str| import.call1(&JsValue::NULL, &own_url(u).into()).ok();
    let module = get(&spec.url).unwrap_or_else(|| reject("import failed").into());
    let os = spec.os.as_deref().map(|o| [o, "os.js"].concat());
    let os = os.and_then(|u| get(&u)).unwrap_or(JsValue::UNDEFINED);
    let both = Promise::all(&Array::of2(&module, &os));
    then(id, &both, move |both| start(id, spec, &both));
    id
}

/// When `p` settles: `ok` with what it resolved to, or cartridge `id` failed with what it
/// rejected with. The callbacks live with the page; each runs at most once.
fn then(id: u32, p: &Promise, ok: impl FnOnce(JsValue) + 'static) {
    let mut ok = Some(ok);
    let ok = Closure::new(move |v: JsValue| {
        if let Some(f) = ok.take() {
            f(v);
        }
    });
    let err = Closure::new(move |e: JsValue| {
        set(id, State::Failed(crate::js_text(&e)), JsValue::NULL);
    });
    let _ = p.then2(&ok, &err);
    ok.forget();
    err.forget();
}

/// The modules are in (the cartridge's and, for one that shows the OS, the OS's): runs the
/// cartridge's `default`, then constructs it.
fn start(id: u32, spec: Spec, both: &JsValue) {
    let both = Array::from(both);
    let (module, os) = (both.get(0), both.get(1));
    let init = Reflect::get(&module, &"default".into()).ok().and_then(|f| f.dyn_into().ok());
    let ready = match init.map(|f: Function| f.call0(&module)) {
        Some(Ok(v)) => Promise::resolve(&v),
        Some(Err(e)) => return set(id, State::Failed(crate::js_text(&e)), JsValue::NULL),
        None => return set(id, State::Failed("the module has no default".into()), JsValue::NULL),
    };
    then(id, &ready, move |_| construct(id, &spec, &module, &os));
}

/// `new Cartridge(w, h, dpr, base, ns, opts)`, the protocol's constructor.
fn construct(id: u32, spec: &Spec, module: &JsValue, os: &JsValue) {
    let class = Reflect::get(module, &"Cartridge".into()).ok().and_then(|c| c.dyn_into().ok());
    let Some(class) = class.map(|c: Function| c) else {
        return set(id, State::Failed("the module has no Cartridge".into()), JsValue::NULL);
    };
    let opts = Object::new();
    let put = |k: &str, v: &JsValue| _ = Reflect::set(&opts, &k.into(), v);
    put("v", &1.into());
    put("query", &spec.query.as_str().into());
    if let Some(at) = &spec.os {
        put("os", &at.as_str().into());
        put("osModule", os);
    }
    let args = Array::new();
    for v in [spec.w.into(), spec.h.into(), spec.dpr.into()] {
        args.push(&v);
    }
    for v in [spec.base.as_str(), spec.ns.as_str()] {
        args.push(&v.into());
    }
    args.push(&opts);
    match Reflect::construct(&class, &args) {
        Ok(obj) => set(id, State::Ready, obj),
        Err(e) => set(id, State::Failed(crate::js_text(&e)), JsValue::NULL),
    }
}

/// `url` as a module of its own: a page imports a URL once, so each cartridge gets another query.
fn own_url(url: &str) -> String {
    let n = NEXT.with(|n| n.replace(n.get() + 1));
    let salt = (js_sys::Math::random() * 1e9) as u32;
    let sep = if url.contains('?') { "&" } else { "?" };
    [url, sep, "i=", &n.to_string(), "-", &salt.to_string()].concat()
}

fn reject(why: &str) -> Promise {
    Promise::reject(&why.into())
}

fn set(id: u32, state: State, obj: JsValue) {
    CARTS.with(|c| {
        if let Some(cart) = c.borrow_mut().iter_mut().find(|c| c.id == id) {
            (cart.state, cart.obj) = (state, obj);
        }
    });
}

/// Where cartridge `id` is; `None` once ended.
pub fn state(id: u32) -> Option<State> {
    CARTS.with(|c| c.borrow().iter().find(|c| c.id == id).map(|c| c.state.clone()))
}

/// Method `name` of cartridge `id` called with `args()`; none if it is not running or the call
/// fails (a cartridge's error never reaches the OS). The arguments are made only for a running
/// one (natively there is none, and no JS value).
fn call(id: u32, name: &str, args: impl FnOnce() -> Vec<JsValue>) -> Option<JsValue> {
    let obj = CARTS.with(|c| {
        let c = c.borrow();
        c.iter().find(|c| c.id == id && c.state == State::Ready).map(|c| c.obj.clone())
    })?;
    let f = Reflect::get(&obj, &name.into()).ok()?.dyn_into::<Function>().ok()?;
    let args: Array = args().iter().collect();
    f.apply(&obj, &args).ok()
}

fn yes(v: Option<JsValue>) -> bool {
    v.and_then(|v| v.as_bool()).unwrap_or(false)
}

fn none() -> Vec<JsValue> {
    Vec::new()
}

/// Its frame count (asking it is the poll a cartridge draws for) and surface, to copy.
pub(crate) fn source(id: u32) -> Option<(u32, JsValue)> {
    let frames = call(id, "frames", none)?.as_f64()?;
    let surface = call(id, "surface", none)?;
    (!surface.is_null() && !surface.is_undefined()).then_some((frames as u32, surface))
}

/// Whether cartridge `id` is still known (the renderer keeps textures only for those).
pub(crate) fn alive(id: u32) -> bool {
    CARTS.with(|c| c.borrow().iter().any(|c| c.id == id))
}

/// The pointer at `(x, y)` CSS px in its surface: `kind` 0 down, 1 moved, 2 up, 3 left.
pub fn pointer(id: u32, kind: u8, x: f32, y: f32, button: u8) -> bool {
    yes(call(id, "pointer", || vec![kind.into(), x.into(), y.into(), button.into()]))
}

pub fn wheel(id: u32, x: f32, y: f32, dy: f32) -> bool {
    yes(call(id, "wheel", || vec![x.into(), y.into(), dy.into()]))
}

/// A key down or up: `KeyboardEvent`'s `code` and `key`, `mods` Shift 1, Ctrl 2, Alt 4, Meta 8,
/// AltGr 16.
pub fn key(id: u32, down: bool, code: &str, key: &str, mods: u8, repeat: bool) -> bool {
    yes(call(id, "key", || vec![down.into(), code.into(), key.into(), mods.into(), repeat.into()]))
}

pub fn text(id: u32, text: &str) {
    call(id, "text", || vec![text.into()]);
}

/// Whether a text field of it has the keys.
pub fn typing(id: u32) -> bool {
    yes(call(id, "typing", none))
}

pub fn resize(id: u32, w: f32, h: f32, dpr: f32) {
    call(id, "resize", || vec![w.into(), h.into(), dpr.into()]);
}

pub fn visible(id: u32, on: bool) {
    call(id, "visible", || vec![on.into()]);
}

/// Whether it wants the pointer locked.
pub fn capture(id: u32) -> bool {
    yes(call(id, "capture", none))
}

/// The pointer was locked to it, or let go.
pub fn locked(id: u32, on: bool) {
    call(id, "locked", || vec![on.into()]);
}

/// The locked pointer moved by `(dx, dy)` CSS px.
pub fn motion(id: u32, dx: f32, dy: f32) {
    call(id, "motion", || vec![dx.into(), dy.into()]);
}

/// Whether it asked to be closed.
pub fn closed(id: u32) -> bool {
    yes(call(id, "closed", none))
}

/// Why it stopped ("" while fine).
pub fn error(id: u32) -> String {
    call(id, "error", none).and_then(|v| v.as_string()).unwrap_or_default()
}

/// The CSS cursor it wants over its surface ("" the default).
pub fn cursor(id: u32) -> String {
    call(id, "cursor", none).and_then(|v| v.as_string()).unwrap_or_default()
}

/// Lets cartridge `id` go: it ends, and is forgotten.
pub fn end(id: u32) {
    call(id, "end", none);
    CARTS.with(|c| c.borrow_mut().retain(|c| c.id != id));
}
