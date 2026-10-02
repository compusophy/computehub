//! The worker's side of its SAB (requests, console ring and doorbell, waits;
//! Atomics.wait never refuses in a worker; the meters around each wait), clocks, randomness,
//! guest memory.

use js_sys::{ArrayBuffer, Atomics, Int32Array, SharedArrayBuffer, Uint8Array, WebAssembly};
use kernel::wire::{BELL, BUSY, COLS, CONS_BELL, EFAULT, ERRNO, HEAD, INPUT, LEN, MAX_PAYLOAD};
use kernel::wire::{PAGES, PAYLOAD_AT, RING_AT, RING_BYTES, ROWS, RUN, SINCE, SLEEP, STATE, TAIL};
use wasi::{Host, Mem};
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::{Crypto, Performance};

/// A process's [`Host`]: its SAB as words and bytes, its spawn tty size, clocks; the guest's
/// memory once linked, which its meters count.
pub struct Js {
    words: Int32Array,
    bytes: Uint8Array,
    tty: (u16, u16),
    perf: Performance,
    crypto: Crypto,
    pub mem: Option<WebAssembly::Memory>,
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
        Js { words, bytes, tty: tty.unwrap_or_default(), perf, crypto, mem: None }
    }

    fn load(&self, i: u32) -> i32 {
        Atomics::load(&self.words, i).unwrap_or(0)
    }

    fn store(&self, i: u32, v: u32) {
        let _ = Atomics::store(&self.words, i, v as i32);
    }

    /// `timeOrigin + now()` in ms, wrapping: the clock the page reads the meters by.
    fn ms(&self) -> u32 {
        (self.perf.time_origin() + self.perf.now()) as u64 as u32
    }

    /// The program begins to run (its compile too): RUN 1 from now.
    pub fn run(&self) {
        self.store(SINCE, self.ms());
        self.store(RUN, 1);
    }

    /// PAGES: the memory now, the guest's (once linked) and the worker's own.
    pub fn pages(&self) {
        let guest = self
            .mem
            .as_ref()
            .map_or(0, |m| m.buffer().unchecked_ref::<ArrayBuffer>().byte_length());
        self.store(PAGES, (guest >> 16) + own_pages());
    }

    /// Sleeps while word `i` holds `v`, for at most `ms`; the meters say it waits meanwhile: BUSY
    /// gains the run that ends, PAGES is the memory now, RUN is 0 until it wakes. RUN goes to 0
    /// before BUSY grows, and main reads BUSY before RUN: a look between the two misses the run
    /// that ends (the next look has it), never counts it twice.
    fn sleep(&self, i: u32, v: i32, ms: f64) {
        let ran = self.ms().wrapping_sub(self.load(SINCE) as u32);
        let ran = if self.load(RUN) == 1 { ran } else { 0 };
        self.store(RUN, 0);
        self.store(BUSY, (self.load(BUSY) as u32).wrapping_add(ran));
        self.pages();
        let _ = Atomics::wait_with_timeout(&self.words, i, v, ms);
        self.run();
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

    /// Into the ring, then the doorbell: CONS_BELL unless one is in flight. Full: sleeps on TAIL.
    fn console(&mut self, mut b: &[u8]) {
        while !b.is_empty() {
            let (head, tail) = (self.load(HEAD) as u32, self.load(TAIL));
            let (at, first, n) = span(head, tail as u32, b.len());
            let put = |at: u32, b: &[u8]| self.bytes.subarray(at, at + b.len() as u32).copy_from(b);
            put(RING_AT + at, &b[..first]);
            put(RING_AT, &b[first..n]);
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

/// The worker's own memory in 64 KiB pages (none natively).
fn own_pages() -> u32 {
    #[cfg(target_arch = "wasm32")]
    return core::arch::wasm32::memory_size::<0>() as u32;
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// Guest memory: a fresh view per access (`grow` detaches), checked first.
pub struct GuestMem(pub WebAssembly::Memory);

fn view(m: &WebAssembly::Memory, at: u32, len: u32) -> Result<Uint8Array, u16> {
    let (buf, end) = (m.buffer(), at.checked_add(len).ok_or(EFAULT)?);
    let fits = end <= buf.unchecked_ref::<ArrayBuffer>().byte_length();
    fits.then(|| Uint8Array::new_with_byte_offset_and_length(&buf, at, len)).ok_or(EFAULT)
}

impl Mem for GuestMem {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16> {
        Ok(view(&self.0, at, len)?.to_vec())
    }

    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16> {
        view(&self.0, at, u32::try_from(d.len()).map_err(|_| EFAULT)?).map(|v| v.copy_from(d))
    }
}
