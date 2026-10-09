//! Testing this device (Activity's Pool page, "Test this device"): measured, not reported. The
//! browser's own figures are guesses (Firefox gives no memory, Chrome rounds it and caps it at 32
//! GB), so the pool times real work on workers of its own, `/bin/gauge work`, none of a job's:
//!
//! 1. **CPU.** Every worker warms up on a little work, one at a time (each compiles the program
//!    on its own and tiers it up on background threads, which get no time while every core is
//!    busy: warmed all at once, 16 workers stayed on their slow first code, all of them together
//!    slower than one), then one alone hashes [`CPU_MIB`]
//!    mebibytes (one core's speed), then all of them at once, as much each (all cores'
//!    throughput). The work is deterministic, so every device does the same.
//! 2. **Memory**, once the CPU's figures are told and kept (a tab lost to the memory half keeps
//!    them), after [`REST`] ms. [`STEP`] mebibytes at a time, one step at a time, worker after
//!    worker (a process holds at most a gibibyte; more workers start as needed, up to [`MAX`]),
//!    every byte written with bytes no memory compressor can shrink, so it is really held. Never
//!    past the test's [`cap`], well below the device's memory: a ramp until something refuses took
//!    all of a Linux laptop's memory and swap in Firefox, nothing refused and the device stalled.
//!    It stops at the cap, at the first step a fresh worker is refused (the browser's limit), at a
//!    step slower than [`SLOW`] times the first steps' median (at least [`FLOOR`] ms; a device
//!    short of memory swaps, and swapping shows as time, not refusal): the worker stops writing
//!    there itself, and the step is not counted; at a step not answered by its limit and
//!    [`GRACE`] (the pool's own timer ends the workers, whatever they do), or after [`TIME`] ms in
//!    all. Then every worker ends, and its memory with it. The figure is "usable": the most the
//!    test held, not the device's total.
//!
//! What it measured goes into this tab's Hello, so every linked tab sees it.

use crate::pool::Act;

/// Whether the memory half runs. Off since 2026-10-08 (its ramp to 16 GiB took all of a Linux
/// laptop's memory and swap in Firefox; no worker was refused and no step slowed before the
/// device stalled); made safe since (the cap, the slow-step rule, the timers), and on again once
/// that laptop has run it.
pub const MEMORY: bool = false;
/// The program the workers run, and the most of them.
pub const PROGRAM: &str = "gauge";
pub const MAX: u16 = 32;
/// Mebibytes a worker hashes for the CPU's speed, and for its warm-up.
pub const CPU_MIB: u32 = 16;
const WARM_MIB: u32 = 4;
/// Mebibytes of memory a step takes; the most the test takes (MiB), and on a phone or tablet
/// whose browser does not say its memory.
pub const STEP: u32 = 64;
pub const MOST: u32 = 2048;
pub const SMALL: u32 = 512;
/// ms between the CPU's end and the memory's start (its figures shown and kept first).
pub const REST: u64 = 1000;
/// The steps whose median is the usual step, and each one's limit (ms).
pub const BASE: usize = 4;
pub const FIRST: u64 = 1000;
/// A later step's limit: this many times the usual step, and at least [`FLOOR`] ms.
pub const SLOW: u64 = 2;
pub const FLOOR: u64 = 150;
/// ms past a step's limit before the pool's timer ends the workers; the memory half's most ms;
/// the longest the test waits for any answer.
pub const GRACE: u64 = 1000;
pub const TIME: u64 = 8000;
const PATIENCE: u64 = 60_000;

/// The most memory the test takes on a device whose browser reports `ram_mb` (0: says nothing):
/// a quarter of that, at most [`MOST`]; with no figure, [`MOST`] on a computer and [`SMALL`] on a
/// phone or tablet; in whole steps.
pub fn cap(ram_mb: u32, computer: bool) -> u32 {
    let most = match (ram_mb, computer) {
        (0, true) => MOST,
        (0, false) => SMALL,
        (r, _) => (r / 4).min(MOST),
    };
    (most / STEP * STEP).max(STEP)
}

/// What testing measured: the CPU's speed on one core and on all at once (MiB of SHA-256 a
/// second), the memory a tab could hold (MiB) and when (Unix seconds; 0: never).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub cpu1: u32,
    pub cpun: u32,
    pub mem: u32,
    pub tested: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Starting,
    Warming,
    One,
    All,
    Rest,
    Memory,
}

/// A worker: its pid, whether it reads lines yet, its output short of a line, the memory it
/// holds (MiB).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Bencher {
    pid: u32,
    ready: bool,
    buf: Vec<u8>,
    held: u32,
}

/// A test under way, or over: its phase, its workers and those still starting, the answers this
/// phase waits for, when its work was given (in its rest: when the memory half starts) and when
/// anything was last heard; the memory it may take, when its memory half started, the worker
/// taking memory and whether it waits for one to start, whether a step is out and its limit,
/// each step's ms and the usual step; what it measured, whether that is news to tell, what it
/// says, and whether it is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    phase: Phase,
    workers: Vec<Bencher>,
    pending: u16,
    waiting: usize,
    t0: u64,
    last: u64,
    pub cap: u32,
    began: u64,
    cur: usize,
    stalled: bool,
    stepping: bool,
    limit: u64,
    steps: Vec<u64>,
    usual: u64,
    pub measured: Measured,
    pub fresh: bool,
    pub note: String,
    pub over: bool,
    /// Whether its memory half runs ([`MEMORY`]; tests turn it on).
    pub memory: bool,
}

/// MiB as people say memory: `960 MB`, `9.4 GB`.
pub fn said(mib: u32) -> String {
    match mib {
        0..1024 => [&mib.to_string(), " MB"].concat(),
        m => [&(m / 1024).to_string(), ".", &(m % 1024 * 10 / 1024).to_string(), " GB"].concat(),
    }
}

impl Test {
    /// A test with a worker a core (at most [`MAX`]), taking at most `cap` MiB, at `now`.
    pub fn start(cores: u16, cap: u32, now: u64, out: &mut Vec<Act>) -> Test {
        let n = cores.clamp(1, MAX);
        out.push(Act::Spawn(PROGRAM.into(), n));
        Test {
            phase: Phase::Starting,
            workers: Vec::new(),
            pending: n,
            waiting: 0,
            t0: now,
            last: now,
            cap,
            began: 0,
            cur: 0,
            stalled: false,
            stepping: false,
            limit: FIRST,
            steps: Vec::new(),
            usual: 0,
            measured: Measured::default(),
            fresh: false,
            note: "Testing: starting its workers".into(),
            over: false,
            memory: MEMORY,
        }
    }

    /// Whether worker `pid` is the test's.
    pub fn has(&self, pid: u32) -> bool {
        self.workers.iter().any(|w| w.pid == pid)
    }

    /// Whether workers it asked for are still starting (the next started is the test's).
    pub fn starting(&self) -> bool {
        self.pending > 0
    }

    /// A test that never ran, saying why.
    pub fn refused(note: &str) -> Test {
        let mut t = Test::start(0, 0, 0, &mut Vec::new());
        (t.pending, t.over, t.note) = (0, true, note.into());
        t
    }

    /// The desktop started a worker (`None`: it could not); one that comes after the test is over
    /// ends at once.
    pub fn spawned(&mut self, pid: Option<u32>, out: &mut Vec<Act>) {
        self.pending = self.pending.saturating_sub(1);
        match pid {
            Some(pid) if self.over => out.push(Act::Stop(vec![pid])),
            Some(pid) => self.workers.push(Bencher { pid, ..Bencher::default() }),
            None if self.workers.is_empty() && self.pending == 0 => {
                self.finish("Could not start the test's workers", out)
            }
            None if self.phase == Phase::Memory && self.pending == 0 && self.stalled => {
                self.finish_memory("the most processes a tab may run", out)
            }
            None => {}
        }
    }

    /// Output of its worker `pid`.
    pub fn output(&mut self, pid: u32, bytes: &[u8], now: u64, out: &mut Vec<Act>) {
        let Some(w) = self.workers.iter_mut().find(|w| w.pid == pid) else { return };
        w.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(nl) = w.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = w.buf.drain(..=nl).filter(|b| *b != b'\r' && *b != b'\n').collect();
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
        self.last = now;
        for l in lines {
            if self.over {
                return;
            }
            self.line(pid, &l, now, out);
        }
    }

    /// Gives worker `i` the line `what`.
    fn feed(&self, i: usize, what: &str, out: &mut Vec<Act>) {
        let line = [&i.to_string(), " ", what].concat();
        out.push(Act::Feed(self.workers[i].pid, line));
    }

    fn line(&mut self, pid: u32, line: &str, now: u64, out: &mut Vec<Act>) {
        let Some(i) = self.workers.iter().position(|w| w.pid == pid) else { return };
        if line == "ready" {
            self.workers[i].ready = true;
            let all = self.pending == 0 && self.workers.iter().all(|w| w.ready);
            if self.phase == Phase::Starting && all {
                // Warm up every worker, one at a time, before any is timed.
                (self.phase, self.waiting) = (Phase::Warming, self.workers.len());
                self.feed(0, &["cpu ", &WARM_MIB.to_string()].concat(), out);
                self.note = "Testing the CPU: warming up".into();
            } else if self.phase == Phase::Memory && self.stalled && self.workers[self.cur].ready {
                self.stalled = false;
                self.step(now, out);
            }
            return;
        }
        let mut words = line.split(' ');
        let (Some(_), Some(mib), Some(_), Some(result)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            return;
        };
        // A CPU answer's own time (ms), else the time it took to come.
        let own = words.next().and_then(|w| w.parse::<u64>().ok());
        let took = own.unwrap_or_else(|| now.saturating_sub(self.t0)).max(1);
        let rate = (u64::from(CPU_MIB) * 1000 / took) as u32;
        let cpu = ["cpu ", &CPU_MIB.to_string()].concat();
        match self.phase {
            Phase::Warming => {
                self.waiting -= 1;
                let next = self.workers.len() - self.waiting;
                if self.waiting > 0 {
                    self.feed(next, &["cpu ", &WARM_MIB.to_string()].concat(), out);
                } else {
                    (self.phase, self.t0, self.waiting) = (Phase::One, now, 1);
                    self.feed(0, &cpu, out);
                    self.note = "Testing the CPU: one core".into();
                }
            }
            Phase::One => {
                self.measured.cpu1 = rate;
                (self.phase, self.t0, self.waiting) = (Phase::All, now, self.workers.len());
                (0..self.workers.len()).for_each(|i| self.feed(i, &cpu, out));
                self.note =
                    ["Testing the CPU: all ", &self.workers.len().to_string(), " cores"].concat();
            }
            Phase::All => {
                // Every worker's rate while all ran at once, summed: the device's throughput.
                self.waiting -= 1;
                self.measured.cpun += rate;
                if self.waiting > 0 {
                    return;
                }
                // The CPU's figures are news now, told and kept before any memory is taken.
                self.fresh = true;
                if !self.memory {
                    return self.finish("Tested the CPU (memory: its test is off for now)", out);
                }
                (self.phase, self.t0) = (Phase::Rest, now + REST);
                self.note = "Testing memory next".into();
            }
            Phase::Memory if self.stepping => {
                let _ = mib;
                let ms = own.unwrap_or_else(|| now.saturating_sub(self.t0));
                self.stepped(result, ms, now, out);
            }
            Phase::Starting | Phase::Rest | Phase::Memory => {}
        }
    }

    /// The next memory step, on the worker taking memory: or the end.
    fn step(&mut self, now: u64, out: &mut Vec<Act>) {
        if self.measured.mem + STEP > self.cap {
            return self.finish_memory("the test's cap (it takes no more)", out);
        }
        if now >= self.began + TIME {
            return self.finish_memory("the test's time", out);
        }
        if self.cur >= self.workers.len() {
            if self.workers.len() + usize::from(self.pending) >= usize::from(MAX) {
                return self.finish_memory("the test's most workers", out);
            }
            let more = (MAX - self.workers.len() as u16).min(8);
            self.pending += more;
            out.push(Act::Spawn(PROGRAM.into(), more));
        }
        if self.cur >= self.workers.len() || !self.workers[self.cur].ready {
            self.stalled = true;
            return;
        }
        // The first steps' limit is fixed; a later one's, a multiple of the usual step.
        let limit = if self.usual == 0 { FIRST } else { FLOOR.max(self.usual * SLOW) };
        (self.t0, self.limit, self.stepping) = (now, limit, true);
        let what = ["ram ", &STEP.to_string(), " ", &limit.to_string()].concat();
        self.feed(self.cur, &what, out);
    }

    /// The worker taking memory answered its step, in `ms` as it timed it: `ok` (it holds the
    /// step), `no` (refused) or `slow` (past its limit, it stopped writing).
    fn stepped(&mut self, result: &str, ms: u64, now: u64, out: &mut Vec<Act>) {
        self.stepping = false;
        match result {
            // A fresh process refused memory: the browser's or the device's limit. A process
            // that holds some is full: the next takes over.
            "no" if self.workers[self.cur].held == 0 => {
                return self.finish_memory("the browser's limit", out);
            }
            "no" => {
                self.cur += 1;
                return self.step(now, out);
            }
            "ok" if ms <= self.limit => {}
            // Not counted: the device short of memory swaps, and swapping shows as time.
            _ => return self.finish_memory("a slow step (the device short of memory)", out),
        }
        self.steps.push(ms);
        if self.steps.len() == BASE {
            let mut sorted = self.steps.clone();
            sorted.sort_unstable();
            self.usual = sorted[BASE / 2].max(1);
        }
        self.workers[self.cur].held += STEP;
        self.measured.mem += STEP;
        self.note = ["Testing memory: ", &said(self.measured.mem)].concat();
        self.step(now, out);
    }

    /// The memory's end, at what stopped it: what it held is news to tell.
    fn finish_memory(&mut self, why: &str, out: &mut Vec<Act>) {
        self.fresh = true;
        let note = match self.measured.mem {
            0 => ["Tested the CPU; memory not measured, stopped by ", why].concat(),
            m => ["Tested: ", &said(m), " of memory usable, stopped by ", why].concat(),
        };
        self.finish(&note, out);
    }

    /// Worker `pid` ended under the test: once the CPU is measured, the browser ended it (its
    /// limit).
    pub fn ended(&mut self, pid: u32, out: &mut Vec<Act>) {
        let Some(i) = self.workers.iter().position(|w| w.pid == pid) else { return };
        self.workers.remove(i);
        if self.over {
            return;
        }
        match self.phase {
            Phase::Rest | Phase::Memory => self.finish_memory("the browser ending a worker", out),
            _ => self.finish("A test worker ended: test again", out),
        }
    }

    /// When the test must next look at the time (`None`: over): its memory half's start, a
    /// step's limit and [`GRACE`], the memory half's [`TIME`], or its patience.
    pub fn due(&self) -> Option<u64> {
        let end = self.began + TIME;
        let at = match self.phase {
            _ if self.over => return None,
            Phase::Rest => self.t0,
            Phase::Memory if self.stepping => (self.t0 + self.limit + GRACE).min(end),
            Phase::Memory => end,
            _ => self.last + PATIENCE,
        };
        Some(at)
    }

    /// At `now`: the memory half starts after its rest; a step unanswered past its limit and
    /// [`GRACE`], or a memory half past [`TIME`], ends it (and its workers, whatever they do);
    /// a test not heard from for too long is over.
    pub fn tick(&mut self, now: u64, out: &mut Vec<Act>) {
        let end = self.began + TIME;
        match self.phase {
            _ if self.over => {}
            Phase::Rest if now >= self.t0 => {
                (self.phase, self.began) = (Phase::Memory, now);
                self.note = "Testing memory".into();
                self.step(now, out);
            }
            Phase::Rest => {}
            Phase::Memory if self.stepping && now >= self.t0 + self.limit + GRACE => {
                self.finish_memory("a step that did not answer in time", out)
            }
            Phase::Memory if now >= end => self.finish_memory("the test's time", out),
            Phase::Memory => {}
            _ if now >= self.last + PATIENCE => {
                self.finish("The test took too long: test again", out)
            }
            _ => {}
        }
    }

    /// The end: every worker ends, its memory freed.
    pub fn finish(&mut self, note: &str, out: &mut Vec<Act>) {
        let pids: Vec<u32> = self.workers.drain(..).map(|w| w.pid).collect();
        if !pids.is_empty() {
            out.push(Act::Stop(pids));
        }
        (self.over, self.note) = (true, note.into());
    }
}
