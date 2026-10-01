//! The loader: fetch, [`kernel::module::cap_memory`], compile, link, then `Proc::new` and
//! `_start`. A preview 1 function in [`NAMES`] links to cpu's own raw export of that name
//! (`sys`), wasm to wasm at any arity; any other function to a stub that says what was called
//! and returns ENOSYS; another import kind refuses the program. The guest never sees cpu's exports.

use crate::{GuestMem, Proc, exit, with};
use js_sys::{Function, Object, Promise, Reflect, Uint8Array, WebAssembly};
use kernel::wire::{CANNOT_EXECUTE, ENOSYS, MEM_PAGES, NOT_FOUND, TRAPPED};
use wasi::{Host, NAMES};
use wasm_bindgen::prelude::*;
use web_sys::Response;

/// The WASI function id the function import `module.name` links to, if any.
pub fn wasi(module: &str, name: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == name).filter(|_| module == "wasi_snapshot_preview1")
}

/// Says `<argv0>: <why>` and exits with `status`, before the guest runs.
fn fail(status: i32, why: &str) {
    let argv0 = with(|c| c.start.argv.first().cloned()).flatten().unwrap_or_default();
    exit(status, &[&argv0, ": ", why, "\n"].concat());
}

/// The message of a thrown value, less the expression JavaScriptCore appends.
fn text(e: &JsValue) -> String {
    let message = Reflect::get(e, &"message".into()).ok().and_then(|m| m.as_string());
    trim(message.or_else(|| e.as_string()).unwrap_or_default())
}

/// `m` up to ` (evaluating '`, found by bytes (a `str` pattern ships 2 KB).
pub fn trim(mut m: String) -> String {
    m.truncate(m.as_bytes().windows(14).position(|w| w == b" (evaluating '").unwrap_or(m.len()));
    m
}

/// Fetches the page-relative `url` (the worker lives in `cpu/`), then runs it.
pub fn fetch(url: &str) {
    then(&crate::scope().fetch_with_str(&["../", url].concat()), |r| {
        let r = r.unchecked_into::<Response>();
        match r.array_buffer() {
            Ok(body) if r.ok() => then(&body, |b| run(&Uint8Array::new(&b).to_vec())),
            _ => fail(NOT_FOUND, &["not found (HTTP ", &r.status().to_string(), ")"].concat()),
        }
    });
}

/// Calls `f` with what `p` resolves to; a rejection is "not found".
fn then(p: &Promise, f: impl FnMut(JsValue) + 'static) {
    let no = |e: JsValue| fail(NOT_FOUND, &["not found (", &text(&e), ")"].concat());
    let (yes, no) = (Closure::<dyn FnMut(JsValue)>::new(f), Closure::new(no));
    let _ = p.then2(&yes, &no);
    yes.forget();
    no.forget();
}

/// Runs the program `bytes`; EXIT 0 when `_start` returns, 134 on a trap.
pub fn run(bytes: &[u8]) {
    let (main, mem) = match load(bytes) {
        Ok(linked) => linked,
        Err(why) => return fail(CANNOT_EXECUTE, &["cannot execute: ", &why].concat()),
    };
    match with(|c| Proc::new(&c.start, &mut c.js).map(|p| c.run = Some((p, GuestMem(mem))))) {
        Some(Ok(())) => match main.call0(&JsValue::UNDEFINED) {
            Ok(_) => exit(0, ""),
            Err(e) => exit(TRAPPED, &["trap: ", &text(&e), "\n"].concat()),
        },
        Some(Err(e)) => fail(CANNOT_EXECUTE, &["cannot start: errno ", &e.to_string()].concat()),
        None => {}
    }
}

/// Caps, compiles and links the program `bytes`: its `_start` and memory.
fn load(bytes: &[u8]) -> Result<(Function, WebAssembly::Memory), String> {
    let wasm = kernel::module::cap_memory(bytes, MEM_PAGES)?;
    let m = WebAssembly::Module::new(&Uint8Array::from(&wasm[..])).map_err(|e| text(&e))?;
    let i = WebAssembly::Instance::new(&m, &imports(&m)?).map_err(|e| text(&e))?;
    let get = |k: &str| Reflect::get(&i.exports(), &k.into()).unwrap_or_default();
    let main = get("_start").dyn_into().map_err(|_| "no _start")?;
    Ok((main, get("memory").dyn_into().map_err(|_| "no memory export")?))
}

/// The import object for `m`, or why it cannot be linked.
fn imports(m: &WebAssembly::Module) -> Result<Object, String> {
    let (all, exports) = (bare(), wasm_bindgen::exports());
    for d in WebAssembly::Module::imports(m).iter() {
        let s = |k: &str| Reflect::get(&d, &k.into()).ok().and_then(|v| v.as_string());
        let [module, name, kind] = ["module", "name", "kind"].map(|k| s(k).unwrap_or_default());
        if kind != "function" {
            return Err([&kind, " import ", &module, ".", &name].concat());
        }
        let f = match wasi(&module, &name) {
            Some(_) => Reflect::get(&exports, &name.as_str().into()).unwrap_or_default(),
            None => stub([&module, ".", &name, ": not supported\n"].concat()),
        };
        let key = JsValue::from(module);
        let mut ns = Reflect::get(&all, &key).unwrap_or_default();
        if ns.is_undefined() {
            ns = bare().into();
            let _ = Reflect::set(&all, &key, &ns);
        }
        let _ = Reflect::set(&ns, &name.into(), &f);
    }
    Ok(all)
}

/// An object with no prototype, so no import name reaches Object.prototype.
fn bare() -> Object {
    Object::create(JsValue::NULL.unchecked_ref())
}

/// Says `text` and returns ENOSYS, a number (an i64 import traps instead).
fn stub(text: String) -> JsValue {
    let f = move || {
        with(|c| c.js.console(text.as_bytes()));
        u32::from(ENOSYS)
    };
    Closure::<dyn FnMut() -> u32>::new(f).into_js_value()
}
