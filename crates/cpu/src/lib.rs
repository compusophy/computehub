//! cpu.wasm, the virtual CPU every program Worker (and homed) runs: `web/worker.js` calls
//! `init()`, the start function posts READY, and main answers with one Start,
//! `[sab | null, start bytes, program]`. The JIT runs the guest behind the loader in `link`, the
//! 46 WASI exports in `sys` (each call goes to a [`wasi::Proc`]) and the SAB and ring in `js`.

#![forbid(unsafe_code)]

mod js;
mod link;
mod sys;
#[cfg(test)]
mod tests;

use core::cell::RefCell;
use js::{GuestMem, Js};
use js_sys::{Array, SharedArrayBuffer, Uint8Array};
use kernel::wire::{self, Msg, Start};
use wasi::{ARITY, Exit, Host, Proc};
use wasm_bindgen::prelude::*;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

/// The process: host, Start, then Proc and memory. Never borrowed across a guest call.
struct Cpu {
    js: Js,
    start: Start,
    run: Option<(Proc, GuestMem)>,
}

thread_local! {
    static CPU: RefCell<Option<Cpu>> = const { RefCell::new(None) };
}

// The entry point. Plain comments: a doc comment on an export ships in cpu.js.
#[wasm_bindgen(start)]
pub fn start() {
    let on = |e: JsValue| message(e.unchecked_into::<MessageEvent>().data());
    let on = Closure::<dyn FnMut(JsValue)>::new(on);
    scope().set_onmessage(Some(on.as_ref().unchecked_ref()));
    on.forget();
    post(&Msg::Ready { version: wire::VERSION }.encode());
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(thread_local_v2, js_name = "self")]
    static SCOPE: DedicatedWorkerGlobalScope;
}

/// The worker's `self` (`js_sys::global` would search four globals for it).
fn scope() -> DedicatedWorkerGlobalScope {
    SCOPE.with(Clone::clone)
}

fn post(msg: &[u8]) {
    let _ = scope().post_message(&Uint8Array::from(msg));
}

/// Runs `f` on the process: `None` before Start or while borrowed (a trap
/// mid-syscall). `try_with` and `try_borrow_mut`: no panic path to ship.
fn with<R>(f: impl FnOnce(&mut Cpu) -> R) -> Option<R> {
    CPU.try_with(|c| c.try_borrow_mut().ok()?.as_mut().map(f)).ok()?
}

/// Writes `text` to the console, when there is one, then posts EXIT.
fn exit(status: i32, text: &str) {
    with(|c| c.js.console(text.as_bytes()));
    post(&Msg::Exit { status }.encode());
}

/// The Start; others are ignored. homed (no SAB) exits 126 until R2 step 4.
fn message(data: JsValue) {
    let Ok(a) = data.dyn_into::<Array>() else { return };
    let start = Start::decode(&Uint8Array::new(&a.get(1)).to_vec());
    let (Some(start), Ok(sab)) = (start, a.get(0).dyn_into::<SharedArrayBuffer>()) else {
        return exit(wire::CANNOT_EXECUTE, "");
    };
    let cpu = Some(Cpu { js: Js::new(&sab, start.tty), start, run: None });
    let _ = CPU.try_with(|c| c.try_borrow_mut().map(|mut c| *c = cpu));
    let program = a.get(2);
    match program.as_string() {
        Some(url) => link::fetch(&url),
        None => link::run(&Uint8Array::new(&program).to_vec()),
    }
}

/// One WASI call, its arguments widened and zero-padded to nine (path_open's): its errno, or
/// EXIT (status `& 0xFF`) then sleep till killed; ENOSYS before Start. Never inlined (46 callers).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn sys(f: usize, a: u64, b: u64, c: u64, d: u64, e: u64, g: u64, h: u64, i: u64, j: u64) -> u32 {
    let all = [a, b, c, d, e, g, h, i, j];
    let a = all.get(..ARITY.get(f).map_or(0, |n| *n)).unwrap_or_default();
    match with(|c| c.run.as_mut().map(|(p, m)| p.call(f, a, m, &mut c.js))).flatten() {
        Some(Ok(errno)) => errno.into(),
        Some(Err(Exit(status))) => {
            exit((status & 0xFF) as i32, "");
            with(|c| c.js.wait(None, false));
            0
        }
        None => wire::ENOSYS.into(),
    }
}
