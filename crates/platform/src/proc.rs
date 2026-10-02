//! Program workers: a module Worker per process and its SAB as `kernel::wire`
//! lays it out (repeated: platform does not depend on the kernel). A worker's
//! messages come after its ring, drained as a CONS_WRITE; a bell goes no further. Its meters
//! are read where it keeps them, with no message ([`stats`]).

use std::cell::{Cell, RefMut};
use std::rc::Rc;

use js_sys::{Array, Atomics, Int32Array, Reflect, SharedArrayBuffer, Uint8Array};
use wasm_bindgen::JsValue;
use web_sys::{AddEventListenerOptions, Event as DomEvent, Worker, WorkerOptions, WorkerType};

use crate::ctl::Load;
use crate::{Event, Shared, dispatch, handler, mark_debug};

macro_rules! consts {
    ($t:ty: $($n:ident = $v:expr),+) => { $(pub(crate) const $n: $t = $v;)+ };
}
// The payload and the ring (after 16 words), the word indexes, the ops.
consts!(u32: PAYLOAD_AT = 64, RING_AT = 65_600, RING_BYTES = 65_536);
consts!(u32: STATE = 0, ERRNO = 1, LEN = 2, HEAD = 7, TAIL = 8, BELL = 9);
consts!(u32: RUN = 10, BUSY = 11, SINCE = 12, PAGES = 13);
consts!(u8: CONS_BELL = 0x10, CONS_WRITE = 0x11, HOME_STATE = 0x18, EXIT = 0x1F);

/// A worker; unless homed, its SAB as words and bytes and TAIL (main's alone).
pub(crate) struct Proc {
    pid: u32,
    worker: Worker,
    sab: Option<(Int32Array, Uint8Array, SharedArrayBuffer)>,
    tail: u32,
}

pub(crate) fn spawn(s: &Rc<Shared>, pid: u32, sab: bool) {
    let opts = WorkerOptions::new();
    opts.set_type(WorkerType::Module);
    // Only an isolated page has a SharedArrayBuffer: the kernel refuses to
    // spawn on another, so a `sab` here means isolated (not checked twice).
    // A worker that cannot start hangs its process until a kill, as one that never loads.
    let Ok(worker) = Worker::new_with_options("cpu/worker.js", &opts) else { return };
    let sab = sab.then(|| {
        let b = SharedArrayBuffer::new(RING_AT + RING_BYTES);
        (Int32Array::new(&b), Uint8Array::new(&b), b)
    });
    let (f, o) = (handler(&Rc::downgrade(s), pid, on_message), AddEventListenerOptions::new());
    for ty in ["message", "error"] {
        let _ = worker.add_event_listener_with_callback_and_add_event_listener_options(ty, &f, &o);
    }
    s.procs.borrow_mut().push(Proc { pid, worker, sab, tail: 0 });
}

/// The worker of `pid`, borrowed.
fn find(s: &Shared, pid: u32) -> Option<RefMut<'_, Proc>> {
    RefMut::filter_map(s.procs.borrow_mut(), |v| v.iter_mut().find(|p| p.pid == pid)).ok()
}

/// A worker's message, or its error (which has no data), after its ring.
fn on_message(s: &Rc<Shared>, pid: u32, e: &DomEvent) {
    let data = Reflect::get(e, &"data".into()).unwrap_or_default();
    let msg = crate::bytes(&Uint8Array::new(&data));
    let Some(out) = drain(s, pid) else { return };
    if out.len() > 1 {
        dispatch(s, Event::Proc { pid, msg: out });
    }
    if let [EXIT, a, b, c, d] = *msg {
        mark_debug(s, "exit", &[pid, u32::from_le_bytes([a, b, c, d])]);
    } else if let [HOME_STATE, state, ..] = *msg {
        mark_debug(s, "home", &[state.into()]);
    }
    if msg.first() != Some(&CONS_BELL) {
        let ev = if msg.is_empty() { Event::ProcError { pid } } else { Event::Proc { pid, msg } };
        dispatch(s, ev);
    }
}

/// The console bytes in the ring of `pid` as a CONS_WRITE: BELL cleared,
/// then TAIL moved up to HEAD and the worker told; `None` once it is gone.
fn drain(s: &Shared, pid: u32) -> Option<Vec<u8>> {
    let mut p = find(s, pid)?;
    let p = &mut *p;
    let mut out = vec![CONS_WRITE];
    if let Some((words, bytes, _)) = &p.sab {
        set(words, BELL, 0);
        let head = Atomics::load(words, HEAD).unwrap_or(0) as u32;
        // A copy, not copy_to into a resized Vec: fewer boot bytes.
        for (a, b) in ring_spans(p.tail, head) {
            out.extend_from_slice(&crate::bytes(&bytes.subarray(RING_AT + a, RING_AT + b)));
        }
        p.tail = head;
        set(words, TAIL, head as i32);
    }
    Some(out)
}

/// Every worker's meters ([`crate::Ctl::proc_stats`]), from its SAB words.
pub(crate) fn stats() -> Vec<(u32, [u32; 3])> {
    let mut out = Vec::new();
    let s = crate::SHARED.try_with(Cell::get).ok().flatten();
    if let Some((s, perf)) = s.and_then(|s| Some((s, s.window.performance()?))) {
        let now = (perf.time_origin() + perf.now()) as u64 as u32;
        for p in s.procs.borrow().iter() {
            if let Some((w, ..)) = &p.sab {
                // BUSY before RUN (the worker clears RUN first): a run is never counted twice.
                let w = |i| Atomics::load(w, i).unwrap_or(0) as u32;
                let busy = w(BUSY);
                out.push((p.pid, meters([w(RUN), busy, w(SINCE), w(PAGES)], now)));
            }
        }
    }
    out
}

/// The meters from words RUN, BUSY, SINCE and PAGES at `now` (ms, wrapping): BUSY plus the run
/// so far (none if `now` reads before SINCE: another thread's clock), PAGES in KB, RUN.
pub(crate) fn meters([run, busy, since, pages]: [u32; 4], now: u32) -> [u32; 3] {
    let ran = if run == 1 { (now.wrapping_sub(since) as i32).max(0) as u32 } else { 0 };
    [busy.wrapping_add(ran), pages.wrapping_mul(64), run]
}

/// `Atomics.store` and `notify` of word `i`.
fn set(words: &Int32Array, i: u32, v: i32) {
    let _ = Atomics::store(words, i, v);
    let _ = Atomics::notify(words, i);
}

/// The ring offsets of console bytes `tail..head` (wrapping counts): two
/// spans, the second empty unless the bytes wrap; at most a ring's worth.
pub(crate) fn ring_spans(tail: u32, head: u32) -> [(u32, u32); 2] {
    let n = head.wrapping_sub(tail).min(RING_BYTES);
    let at = tail % RING_BYTES;
    let first = n.min(RING_BYTES - at);
    [(at, at + first), (0, n - first)]
}

/// Posts `msg` to the worker of `pid`; with a program, as Start's array.
pub(crate) fn post(s: &Shared, pid: u32, msg: &[u8], program: Option<Load>) {
    let Some(p) = find(s, pid) else { return };
    let (mut msg, null): (JsValue, _) = (Uint8Array::from(msg).into(), JsValue::NULL);
    if let Some(program) = program {
        let program = match program {
            Load::None => JsValue::NULL,
            Load::Bytes(b) => Uint8Array::from(&b[..]).into(),
            Load::Url(u) => u.into(),
        };
        let sab = p.sab.as_ref().map_or(&null, |m| m.2.as_ref());
        msg = Array::of3(sab, &msg, &program).into();
    }
    let _ = p.worker.post_message(&msg);
}

/// Copies `data` to the payload of `pid`, then sets `words` below 16.
pub(crate) fn store(s: &Shared, pid: u32, data: &[u8], words: &[(u32, i32)]) {
    if let Some(Proc { sab: Some((w, b, _)), .. }) = find(s, pid).as_deref() {
        b.set(&Uint8Array::from(data), PAYLOAD_AT);
        words.iter().filter(|w| w.0 < PAYLOAD_AT / 4).for_each(|&(i, v)| set(w, i, v));
    }
}

/// The payload (at most 64 KiB), LEN and ERRNO, then STATE = 1.
pub(crate) fn reply(s: &Shared, pid: u32, errno: u16, data: &[u8]) {
    let data = &data[..data.len().min((RING_AT - PAYLOAD_AT) as usize)];
    store(s, pid, data, &[(LEN, data.len() as i32), (ERRNO, errno.into()), (STATE, 1)]);
}

/// Terminates the worker of `pid`; the GC takes its callbacks.
pub(crate) fn kill(s: &Shared, pid: u32) {
    // One borrow, and `remove` (its panic is in the boot already), not `swap_remove`.
    let mut procs = s.procs.borrow_mut();
    if let Some(i) = procs.iter().position(|p| p.pid == pid) {
        procs.remove(i).worker.terminate();
        mark_debug(s, "kill", &[pid]);
    }
}
