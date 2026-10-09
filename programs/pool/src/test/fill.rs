//! "Measure all memory" (compusophy, 2026-10-09: "make the RAM test show real numbers"): the
//! memory this device can give a tab before it starts to swap, the figure that matters for what
//! the pool can hold. A browser never says the device's memory (Firefox nothing, Chrome at most
//! 32 GB of a 128 GB box), and it does not refuse memory either: the system swaps, and swapping
//! shows as time. A step's write time alone missed it on a Linux laptop (the kernel was already
//! swapping while steps stayed flat), so after every round of steps the worker holding the
//! oldest memory re-touches [`PAGES`] random pages of it and times that: pages written long ago
//! are what the kernel swaps out first, and reaching one then takes milliseconds, not the
//! microseconds RAM takes.
//!
//! Workers take [`FILL_STEP`] mebibytes at a time, [`PAR`] at once, every byte incompressible
//! (gauge's `ram`); a worker refused once it holds some is full and the next takes over (a
//! process holds at most a gibibyte; more workers start as needed, up to [`super::MAX`]). It
//! stops, every worker ending and its memory with it:
//! - at the first sign of swapping: the old pages' time past [`TOUCH_X`] times its usual (the
//!   median of the first three) and at least [`TOUCH_MIN`] us, or past [`TOUCH_MAX`] us at all;
//! - at a step slower than one and a half times the usual step (at least [`super::FLOOR`] ms),
//!   or refused on a fresh worker, or not answered by its limit and [`super::GRACE`];
//! - at its own limits, its figure then a floor: [`FILL_MOST`] MiB, [`FILL_TIME`] ms, the most
//!   workers.
//!
//! When the device stopped it, the figure is what was held before the round that showed it.

use super::{BASE, FIRST, FLOOR, GRACE, MAX, PROGRAM, Phase, Test, said};
use crate::pool::Act;

/// Mebibytes a step takes, and how many steps a round gives at once.
pub const FILL_STEP: u32 = 128;
pub const PAR: usize = 4;
/// The most it takes (MiB) and the longest it runs (ms).
pub const FILL_MOST: u32 = 96 * 1024;
pub const FILL_TIME: u64 = 30_000;
/// Pages re-touched after each round; the swap signal: past `TOUCH_X` times the usual and at
/// least `TOUCH_MIN` us, or past `TOUCH_MAX` us; how long a touch may take to answer (ms).
pub const PAGES: u32 = 64;
pub const TOUCH_X: u64 = 10;
pub const TOUCH_MIN: u64 = 500;
pub const TOUCH_MAX: u64 = 2000;
const TOUCH_WAIT: u64 = 2000;

/// The fill under way: the workers with a step out, the memory held when the round began and
/// when it began, whether a touch is out and since when, each touch's microseconds, the usual
/// touch (0 until three), the slowest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    out: Vec<usize>,
    before: u32,
    round: u64,
    touching: Option<u64>,
    touches: Vec<u64>,
    usual: u64,
    peak: u64,
}

/// Microseconds as milliseconds, two places: `0.02 ms`, `3.10 ms`.
fn ms(us: u64) -> String {
    let h = us / 10;
    [&(h / 100).to_string(), ".", &format!("{:02}", h % 100), " ms"].concat()
}

impl Test {
    /// A fill test with a worker a core (at most [`MAX`]), at `now`: no CPU, all the memory.
    pub fn start_fill(cores: u16, now: u64, out: &mut Vec<Act>) -> Test {
        let mut t = Test::start(cores, FILL_MOST, now, out);
        (t.all, t.note) = (true, "Measuring all memory: starting its workers".into());
        t
    }

    /// Its workers ready: the first round.
    pub(super) fn fill_begin(&mut self, now: u64, out: &mut Vec<Act>) {
        (self.phase, self.began) = (Phase::Fill, now);
        self.round(now, out);
    }

    /// The next round: a step on each of up to [`PAR`] workers not full, more workers asked
    /// for when they run short; or the end at the test's own limits.
    pub(super) fn round(&mut self, now: u64, out: &mut Vec<Act>) {
        if self.measured.mem + FILL_STEP > self.cap {
            return self.finish_fill("the test's most memory", true, self.measured.mem, out);
        }
        if now >= self.began + FILL_TIME {
            return self.finish_fill("the test's time", true, self.measured.mem, out);
        }
        let free: Vec<usize> = (0..self.workers.len())
            .filter(|&i| !self.workers[i].full && self.workers[i].ready)
            .collect();
        let total = self.workers.len() + usize::from(self.pending);
        if free.len() < PAR && total < usize::from(MAX) && self.pending == 0 {
            let more = (MAX - total as u16).min(8);
            self.pending += more;
            out.push(Act::Spawn(PROGRAM.into(), more));
        }
        if free.is_empty() {
            if self.pending == 0 {
                return self.finish_fill("the test's most workers", true, self.measured.mem, out);
            }
            self.stalled = true;
            return;
        }
        let limit = if self.steps.len() < BASE {
            FIRST
        } else {
            let mut s = self.steps.clone();
            s.sort_unstable();
            FLOOR.max(s[s.len() / 2] * 3 / 2)
        };
        let chosen: Vec<usize> = free.into_iter().take(PAR).collect();
        let what = ["ram ", &FILL_STEP.to_string(), " ", &limit.to_string()].concat();
        chosen.iter().for_each(|&i| self.feed(i, &what, out));
        self.limit = limit;
        self.fill = Fill {
            out: chosen,
            before: self.measured.mem,
            round: now,
            ..std::mem::take(&mut self.fill)
        };
        self.note = ["Measuring all memory: ", &said(self.measured.mem)].concat();
    }

    /// Worker `i` answered: a step (`ok`, `no`, `slow`, in `ms` as it timed it) or a touch.
    pub(super) fn filled(&mut self, i: usize, result: &str, n: u64, now: u64, out: &mut Vec<Act>) {
        if result == "touch" {
            return self.touched(n, now, out);
        }
        let Some(k) = self.fill.out.iter().position(|&w| w == i) else { return };
        self.fill.out.remove(k);
        match result {
            "no" if self.workers[i].held == 0 => {
                return self.finish_fill("the browser's limit", false, self.measured.mem, out);
            }
            "no" => self.workers[i].full = true,
            "ok" if n <= self.limit => {
                self.workers[i].held += FILL_STEP;
                self.measured.mem += FILL_STEP;
                self.steps.push(n);
            }
            _ => {
                let before = self.fill.before;
                return self.finish_fill(
                    "a slow step (the device short of memory)",
                    false,
                    before,
                    out,
                );
            }
        }
        if !self.fill.out.is_empty() {
            return;
        }
        // The round is in: the oldest memory's pages, timed.
        match (0..self.workers.len()).find(|&w| self.workers[w].held > 0) {
            Some(w) => {
                self.feed(w, &["touch ", &PAGES.to_string()].concat(), out);
                self.fill.touching = Some(now);
            }
            None => self.round(now, out),
        }
    }

    /// The old pages took `us` microseconds: the first sign of swapping ends it, else the next
    /// round.
    fn touched(&mut self, us: u64, now: u64, out: &mut Vec<Act>) {
        if self.fill.touching.take().is_none() {
            return;
        }
        let f = &mut self.fill;
        f.touches.push(us);
        f.peak = f.peak.max(us);
        if f.touches.len() == 3 {
            let mut s = f.touches.clone();
            s.sort_unstable();
            f.usual = s[1].max(1);
        }
        let limit = match f.usual {
            0 => TOUCH_MAX,
            u => TOUCH_MAX.min(TOUCH_MIN.max(u * TOUCH_X)),
        };
        if us > limit {
            let before = f.before;
            return self.finish_fill(
                "old pages slowing (the device starting to swap)",
                false,
                before,
                out,
            );
        }
        self.round(now, out);
    }

    /// What was held when the round under way began.
    pub(super) fn fill_before(&self) -> u32 {
        self.fill.before
    }

    /// When the fill must next look at the time.
    pub(super) fn fill_due(&self) -> u64 {
        let end = self.began + FILL_TIME + GRACE;
        match self.fill.touching {
            Some(t) => (t + TOUCH_WAIT + GRACE).min(end),
            None if !self.fill.out.is_empty() => (self.fill.round + self.limit + GRACE).min(end),
            None => end,
        }
    }

    /// At `now`: a round or a touch unanswered past its limit ends it (the device stalled: what
    /// was held before the round), as does the test's time.
    pub(super) fn fill_tick(&mut self, now: u64, out: &mut Vec<Act>) {
        let before = self.fill.before;
        match self.fill.touching {
            Some(t) if now >= t + TOUCH_WAIT + GRACE => self.finish_fill(
                "old pages not answering (the device swapping)",
                false,
                before,
                out,
            ),
            None if !self.fill.out.is_empty() && now >= self.fill.round + self.limit + GRACE => {
                self.finish_fill("a step that did not answer in time", false, before, out)
            }
            _ if now >= self.began + FILL_TIME + GRACE => {
                self.finish_fill("the test's time", true, self.measured.mem, out)
            }
            _ => {}
        }
    }

    /// The fill's end: `mem` held, stopped by `why` (`floor`: the test's own limit, so at least
    /// that); its figures are news to tell.
    pub(super) fn finish_fill(&mut self, why: &str, floor: bool, mem: u32, out: &mut Vec<Act>) {
        (self.measured.mem, self.measured.floor, self.fresh) = (mem, floor && mem > 0, true);
        let f = &self.fill;
        let pages = match (f.usual, f.peak) {
            (_, 0) => String::new(),
            (0, p) => [" (old pages up to ", &ms(p), ")"].concat(),
            (u, p) => [" (old pages ", &ms(u), " usually, ", &ms(p), " at most)"].concat(),
        };
        let note = match (mem, floor) {
            (0, _) => ["Measured no memory: stopped by ", why].concat(),
            (m, true) => {
                ["Measured: at least ", &said(m), " usable, stopped by ", why, &pages].concat()
            }
            (m, false) => ["Measured: ", &said(m), " usable, stopped by ", why, &pages].concat(),
        };
        self.finish(&note, out);
    }
}
