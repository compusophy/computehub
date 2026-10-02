//! Touch: a finger on what scrolls becomes a scroll once it travels, a long press if it stays
//! put, and a fling if it lifts while moving. Times are page-clock ms; speeds are px per ms, and
//! scrolling is positive downward: the content follows the finger, so a finger moving up
//! scrolls down.

/// Travel before a finger on scrollable content scrolls it.
pub const SCROLL_PX: f32 = 8.0;
/// How far a finger may wander, and how long it must stay, to long-press.
pub const HOLD_PX: f32 = 10.0;
pub const HOLD_MS: f64 = 500.0;
/// A gap between checks (frames) longer than this is a stall: input may wait behind it, and a
/// browser may run a frame or two after a long task before that input. A hold that saw a stall
/// fires at a check this long after the first that found it due, so a queued lift is heard.
pub const STALL_MS: f64 = 50.0;
pub const LATE_MS: f64 = 100.0;
/// A fling's time constant, and the speed where it stops.
pub const DECAY_MS: f32 = 325.0;
pub const STOP: f32 = 0.02;
/// A finger that rested this long before it lifted does not fling.
const REST_MS: f64 = 100.0;
/// How much each new move weighs in the speed.
const NEW: f32 = 0.8;

/// A finger down: where and when it went down, whether what it pressed scrolls, whether it
/// scrolls now, whether a long press is out (it wandered, scrolled or already fired), the last
/// check (from the press), whether checks stalled and when one first found it due, where it was
/// last (height, time) and its speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub at: (f32, f32),
    pub t0: f64,
    pub scrolls: bool,
    pub scrolling: bool,
    pub done: bool,
    seen: f64,
    stalled: bool,
    due: Option<f64>,
    last: (f32, f64),
    v: f32,
}

impl Touch {
    pub fn new(at: (f32, f32), now: f64, scrolls: bool) -> Touch {
        let (seen, stalled, due, last, v) = (now, false, None, (at.1, now), 0.0);
        Touch { at, t0: now, scrolls, scrolling: false, done: false, seen, stalled, due, last, v }
    }

    /// The finger moved to `(x, y)`: how far to scroll (down if positive), once scrolling. It
    /// starts scrolling where it passes [`SCROLL_PX`], so the content never jumps.
    pub fn moved(&mut self, (x, y): (f32, f32), now: f64) -> Option<f32> {
        let far = (x - self.at.0).abs().max((y - self.at.1).abs());
        self.done |= far > HOLD_PX;
        if !self.scrolling {
            self.scrolling = self.scrolls && far > SCROLL_PX;
            self.done |= self.scrolling;
            self.last = (y, now);
            return None;
        }
        let (dy, dt) = (self.last.0 - y, (now - self.last.1) as f32);
        if dt > 0.0 {
            self.v = NEW * dy / dt + (1.0 - NEW) * self.v;
        }
        self.last = (y, now);
        Some(dy)
    }

    /// A check (a frame) at `now`: whether the finger, still since it went down, long-presses
    /// (once). On a page keeping up, at the first check past [`HOLD_MS`]; after a stall, at a
    /// check [`LATE_MS`] after that one, never at it: a busy page may see the time pass while a
    /// quick tap's lift waits behind its frames, and heard meanwhile, it is a tap.
    pub fn held(&mut self, now: f64) -> bool {
        self.stalled |= now - std::mem::replace(&mut self.seen, now) > STALL_MS;
        if self.done || now - self.t0 < HOLD_MS {
            return false;
        }
        let fire = !self.stalled || now - *self.due.get_or_insert(now) >= LATE_MS;
        self.done |= fire;
        fire
    }

    /// The finger lifted at `now`: a fling if it scrolled and was moving.
    pub fn lift(&self, now: f64) -> Option<Fling> {
        let v = if now - self.last.1 > REST_MS { 0.0 } else { self.v };
        (self.scrolling && v.abs() >= STOP).then_some(Fling { v, t: now })
    }
}

/// Content moving on after a finger let go, its speed decaying as `v *= exp(-dt / DECAY_MS)`;
/// `t` is its last step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fling {
    pub v: f32,
    pub t: f64,
}

impl Fling {
    /// How far it scrolls from its last step to `now` (the speed's integral).
    pub fn step(&mut self, now: f64) -> f32 {
        let dt = (now - self.t).max(0.0) as f32;
        let k = decay(dt / DECAY_MS);
        let dy = self.v * DECAY_MS * (1.0 - k);
        (self.v, self.t) = (self.v * k, now.max(self.t));
        dy
    }

    pub fn moving(&self) -> bool {
        self.v.abs() >= STOP
    }
}

/// `exp(-x)` for `x >= 0`, near enough for a frame's step (within 1% up to 1): the reciprocal
/// of the first five terms of `exp(x)`'s series, which needs no libm.
pub fn decay(x: f32) -> f32 {
    let x = x.max(0.0);
    1.0 / (1.0 + x * (1.0 + x * (0.5 + x * (1.0 / 6.0 + x / 24.0))))
}
