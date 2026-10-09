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
//! 2. **Memory.** [`STEP`] mebibytes at a time, one step at a time, worker after worker (a process
//!    holds at most a gibibyte; more workers start as needed, up to [`MAX`]), every byte written so
//!    it is really held. It stops at the first step a fresh worker is refused (the browser's or the
//!    device's limit), at one that takes [`SLOW`] times the usual step (the device swapping: the
//!    step is not counted), or at [`CEILING`]. Then every worker ends, and its memory with it.
//!
//! What it measured goes into this tab's Hello, so every linked tab sees it.

use crate::pool::Act;

/// The program the workers run, and the most of them.
pub const PROGRAM: &str = "gauge";
pub const MAX: u16 = 32;
/// Mebibytes a worker hashes for the CPU's speed, and for its warm-up.
pub const CPU_MIB: u32 = 16;
const WARM_MIB: u32 = 4;
/// Mebibytes of memory a step takes, and the most the test takes (it is said as "at least").
pub const STEP: u32 = 64;
pub use uiwire::pool::CEILING;
/// A step this many times the median of those before it (and past [`FLOOR`] ms) is the device
/// swapping; and the longest the test waits for any answer.
pub const SLOW: u64 = 4;
pub const FLOOR: u64 = 500;
const PATIENCE: u64 = 60_000;

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
/// phase waits for, when its work was given and when anything was last heard, the worker taking
/// memory and whether it waits for one to start, each memory step's ms; what it measured, what
/// it says, and whether it is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    phase: Phase,
    workers: Vec<Bencher>,
    pending: u16,
    waiting: usize,
    t0: u64,
    last: u64,
    cur: usize,
    stalled: bool,
    steps: Vec<u64>,
    pub measured: Measured,
    pub note: String,
    pub over: bool,
}

/// MiB as people say memory: `960 MB`, `9.4 GB`.
pub fn said(mib: u32) -> String {
    match mib {
        0..1024 => [&mib.to_string(), " MB"].concat(),
        m => [&(m / 1024).to_string(), ".", &(m % 1024 * 10 / 1024).to_string(), " GB"].concat(),
    }
}

impl Test {
    /// A test with a worker a core (at most [`MAX`]), at `now`.
    pub fn start(cores: u16, now: u64, out: &mut Vec<Act>) -> Test {
        let n = cores.clamp(1, MAX);
        out.push(Act::Spawn(PROGRAM.into(), n));
        Test {
            phase: Phase::Starting,
            workers: Vec::new(),
            pending: n,
            waiting: 0,
            t0: now,
            last: now,
            cur: 0,
            stalled: false,
            steps: Vec::new(),
            measured: Measured::default(),
            note: "Testing: starting its workers".into(),
            over: false,
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
        let mut t = Test::start(0, 0, &mut Vec::new());
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
                if self.waiting == 0 {
                    self.phase = Phase::Memory;
                    self.step(now, out);
                }
            }
            Phase::Memory => {
                let ok = result == "ok";
                let _ = mib;
                self.stepped(ok, now, out);
            }
            Phase::Starting => {}
        }
    }

    /// The next memory step, on the worker taking memory: or the end.
    fn step(&mut self, now: u64, out: &mut Vec<Act>) {
        if self.measured.mem >= CEILING {
            return self.finish_memory("the test's ceiling", out);
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
        self.t0 = now;
        self.feed(self.cur, &["ram ", &STEP.to_string()].concat(), out);
    }

    /// The worker taking memory answered its step (`ok`: it holds the step).
    fn stepped(&mut self, ok: bool, now: u64, out: &mut Vec<Act>) {
        let ms = now.saturating_sub(self.t0);
        if !ok {
            // A fresh process refused memory: the browser's or the device's limit. A process
            // that holds some is full: the next takes over.
            if self.workers[self.cur].held == 0 {
                return self.finish_memory("the browser's limit", out);
            }
            self.cur += 1;
            return self.step(now, out);
        }
        let mut sorted = self.steps.clone();
        sorted.sort_unstable();
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
        if self.steps.len() >= 3 && ms > FLOOR.max(median * SLOW) {
            return self.finish_memory("a slowing step (the device swapping)", out);
        }
        self.steps.push(ms);
        self.workers[self.cur].held += STEP;
        self.measured.mem += STEP;
        self.note = ["Testing memory: ", &said(self.measured.mem)].concat();
        self.step(now, out);
    }

    /// The memory's end, at what stopped it.
    fn finish_memory(&mut self, why: &str, out: &mut Vec<Act>) {
        let mem = said(self.measured.mem);
        let held = if self.measured.mem >= CEILING { [&mem, " or more"].concat() } else { mem };
        let note = ["Tested: memory ", &held, ", stopped by ", why].concat();
        self.finish(&note, out);
    }

    /// Worker `pid` ended under the test: in the memory phase the browser ended it (its limit).
    pub fn ended(&mut self, pid: u32, out: &mut Vec<Act>) {
        let Some(i) = self.workers.iter().position(|w| w.pid == pid) else { return };
        self.workers.remove(i);
        if self.over {
            return;
        }
        match self.phase {
            Phase::Memory => self.finish_memory("the browser ending a worker", out),
            _ => self.finish("A test worker ended: test again", out),
        }
    }

    /// At `now`: a test heard from for too long is over.
    pub fn tick(&mut self, now: u64, out: &mut Vec<Act>) {
        if !self.over && now >= self.last + PATIENCE {
            self.finish("The test took too long: test again", out);
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
