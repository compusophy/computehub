//! The worker's side of its SAB (requests, console ring and doorbell, waits;
//! Atomics.wait never refuses in a worker), clocks, randomness, guest memory.

use js_sys::{ArrayBuffer, Atomics, Int32Array, SharedArrayBuffer, Uint8Array, WebAssembly};
use kernel::wire::{BELL, COLS, CONS_BELL, EFAULT, ERRNO, HEAD, INPUT, LEN, MAX_PAYLOAD};
use kernel::wire::{PAYLOAD_AT, RING_AT, RING_BYTES, ROWS, SLEEP, STATE, TAIL};
use wasi::{Host, Mem};
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::{Crypto, Performance};

/// A process's [`Host`]: its SAB as words and bytes, its spawn tty size, clocks.
pub struct Js {
    words: Int32Array,
    bytes: Uint8Array,
    tty: (u16, u16),
    perf: Performance,
    crypto: Crypto,
}

/// Where `len` more bytes go in a ring holding `head - tail`: `(at, first, n)`,
/// the `n` that fit (0 when full), `first` of them at offset `at`, the rest at 0.
pub fn span(head: u32, tail: u32, len: usize) -> (u32, usize, usize) {
    let free = RING_BYTES.saturating_sub(head.wrapping_sub(tail)) as usize;
    let (at, n) = (head % RING_BYTES, free.min(len));
    (at, n.min((RING_BYTES - at) as usize), n)
}

impl Js {
    pub fn new(sab: &SharedArrayBuffer, tty: Option<(u16, u16)>) -> Js {
        let scope = crate::scope();
        let (perf, crypto) = (scope.performance().unwrap_throw(), scope.crypto().unwrap_throw());
        let (words, bytes) = (Int32Array::new(sab), Uint8Array::new(sab));
        Js { words, bytes, tty: tty.unwrap_or_default(), perf, crypto }
    }

    fn load(&self, i: u32) -> i32 {
        Atomics::load(&self.words, i).unwrap_or(0)
    }

    /// Sleeps while word `i` holds `v`, for at most `ms`.
    fn sleep(&self, i: u32, v: i32, ms: f64) {
        let _ = Atomics::wait_with_timeout(&self.words, i, v, ms);
    }
}

impl Host for Js {
    fn call(&mut self, req: &[u8]) -> (u16, Vec<u8>) {
        crate::post(req);
        while self.load(STATE) != 1 {
            self.sleep(STATE, 0, f64::INFINITY);
        }
        let len = (self.load(LEN) as u32).min(MAX_PAYLOAD as u32);
        let data = self.bytes.subarray(PAYLOAD_AT, PAYLOAD_AT + len).to_vec();
        let reply = (self.load(ERRNO) as u16, data);
        let _ = Atomics::store(&self.words, STATE, 0);
        reply
    }

    fn post(&mut self, msg: &[u8]) {
        crate::post(msg);
    }

    /// Into the ring, then the doorbell: CONS_BELL unless one is in flight. Full: sleeps on TAIL.
    fn console(&mut self, mut b: &[u8]) {
        while !b.is_empty() {
            let (head, tail) = (self.load(HEAD) as u32, self.load(TAIL));
            let (at, first, n) = span(head, tail as u32, b.len());
            let put = |at: u32, b: &[u8]| self.bytes.subarray(at, at + b.len() as u32).copy_from(b);
            put(RING_AT + at, &b[..first]);
            if n > first {
                put(RING_AT, &b[first..n]);
            }
            let _ = Atomics::store(&self.words, HEAD, head.wrapping_add(n as u32) as i32);
            if Atomics::compare_exchange(&self.words, BELL, 0, 1) == Ok(0) {
                crate::post(&[CONS_BELL]);
            }
            if n == 0 {
                self.sleep(TAIL, tail, f64::INFINITY);
            }
            b = &b[n..];
        }
    }

    /// Realtime (0) is `timeOrigin + now`; monotonic and CPU time (1 to 3) `now`.
    fn now_ns(&mut self, clock: u32) -> Option<u64> {
        let origin = if clock == 0 { self.perf.time_origin() } else { 0.0 };
        let ns = ((origin + self.perf.now()) * 1e6) as u64;
        (clock < 4).then_some(ns - ns % 100_000)
    }

    /// Proc asks for at most 64 KiB at a time, getRandomValues' limit.
    fn random(&mut self, buf: &mut [u8]) {
        let _ = self.crypto.get_random_values_with_u8_array(buf);
    }

    fn wait(&mut self, timeout_ns: Option<u64>, stdin: bool) -> bool {
        let end = timeout_ns.map_or(f64::INFINITY, |ns| self.perf.now() + ns as f64 / 1e6);
        let ready = || stdin && self.load(INPUT) != 0;
        while !ready() && self.perf.now() < end {
            self.sleep(if stdin { INPUT } else { SLEEP }, 0, end - self.perf.now());
        }
        ready()
    }

    fn winsize(&self) -> (u16, u16) {
        let live = |i, spawned| Some(self.load(i) as u16).filter(|v| *v > 0).unwrap_or(spawned);
        (live(COLS, self.tty.0), live(ROWS, self.tty.1))
    }
}

/// Guest memory: a fresh view per access (`grow` detaches), checked first.
pub struct GuestMem(pub WebAssembly::Memory);

impl GuestMem {
    fn view(&self, at: u32, len: u32) -> Result<Uint8Array, u16> {
        let (buf, end) = (self.0.buffer(), at.checked_add(len).ok_or(EFAULT)?);
        let fits = end <= buf.unchecked_ref::<ArrayBuffer>().byte_length();
        fits.then(|| Uint8Array::new_with_byte_offset_and_length(&buf, at, len)).ok_or(EFAULT)
    }
}

impl Mem for GuestMem {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16> {
        Ok(self.view(at, len)?.to_vec())
    }

    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16> {
        let len = u32::try_from(d.len()).map_err(|_| EFAULT)?;
        self.view(at, len)?.copy_from(d);
        Ok(())
    }
}
