//! The desktop's meters as Activity hears them: one [`Event::Stats`](crate::Event::Stats)
//! sample, little-endian, a `str` a u32 length and UTF-8; and [`Pace`], when the desktop takes
//! one and whether it posts it.
//!
//! ```text
//! u8 VERSION | u32 at (page ms, wrapping)
//! loud:  u16 n, n u32 (counts by LOUD index) | table | u16 k, k (u32 pid, u16 m, m u32)
//! quiet: u16 q, q u32 (counts by QUIET index) | u16 m, m u32 (the watcher's own meters)
//! table: u16 count, count (u32 pid, u32 window, u8 state, u16 argc, argc str, u16 c, c u32)
//! ```
//!
//! The kernel writes the table (deterministic: pid order, the window 0 the overlay, argv's words
//! while their bytes fit in 160, counts by [`DRAWS`]); the page writes the rest, as measured: a
//! process's meters ([`BUSY_MS`], [`MEM_KB`]; none for one it cannot see) and the watcher's own.
//! Counts and meters are cumulative and wrap, so a reader takes rates from two samples with
//! `wrapping_sub`. Indices, like codes, are only appended: a reader ignores counts it does not
//! know and reads one a sample lacks as unknown.
//!
//! Loud is what a person or another program did: a change there posts a sample. Quiet is
//! carried, never a reason to post: the meter's own costs (sampling, the watcher's frames and
//! time), the clock, timers (the grain's). So a still desktop sends nothing, its grain living or
//! not.

use crate::{Out, Reader};

/// The format's version, a sample's first byte.
pub const VERSION: u8 = 1;
/// Loud counts: frames drawn for the person's input, for something moving and for programs (their
/// frames and timers); 1 while the living grain lives; the bytes of /home as kept, and 1 while
/// files go unkept; AI requests, those that failed, those that ended with no [`receipt`], and
/// what the receipts said: tokens in and out, and µ$ (millionths of a dollar) spent.
pub const INPUT: usize = 0;
pub const MOTION: usize = 1;
pub const PROGRAMS: usize = 2;
pub const GRAIN_ON: usize = 3;
pub const HOME: usize = 4;
pub const UNKEPT: usize = 5;
pub const ASKED: usize = 6;
pub const FAILED: usize = 7;
pub const UNMETERED: usize = 8;
pub const TOKENS_IN: usize = 9;
pub const TOKENS_OUT: usize = 10;
pub const MICROUSD: usize = 11;
/// How many loud counts this version writes.
pub const LOUD: usize = 12;
/// Quiet counts: frames drawn for the watcher's own frames, by a timer (the living grain's, an
/// app's, whose program's frames count as programs), for anything else (the clock, fonts); µs the
/// desktop spent handling events and drawing; KB of its memory.
pub const SELF: usize = 0;
pub const TIMER: usize = 1;
pub const OTHER: usize = 2;
pub const DESKTOP_US: usize = 3;
pub const DESKTOP_KB: usize = 4;
/// How many quiet counts this version writes.
pub const QUIET: usize = 5;
/// A process's meters: ms it ran (its compile too), and KB of its memory as of its last wait.
pub const BUSY_MS: usize = 0;
pub const MEM_KB: usize = 1;
/// A table row's counts: the frames (DRAWs) it drew.
pub const DRAWS: usize = 0;
/// A table row's state: it waits for an event, it runs, it ended (its status not yet taken).
pub const IDLE: u8 = 0;
pub const RUNS: u8 = 1;
pub const ENDED: u8 = 2;
/// The least time between two samples, in ms.
pub const GAP_MS: u64 = 1000;

/// One sample, decoded: when, the counts, the processes, the meters by pid, the watcher's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub at: u32,
    pub loud: Vec<u32>,
    pub procs: Vec<Proc>,
    pub meters: Vec<(u32, Vec<u32>)>,
    pub quiet: Vec<u32>,
    pub own: Vec<u32>,
}

/// A row of the kernel's table: its pid, its window, its state, its argv (as told), its counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub window: u32,
    pub state: u8,
    pub argv: Vec<String>,
    pub counts: Vec<u32>,
}

impl Stats {
    /// Exactly one sample, or `None`: another version, short or trailing bytes, a count past
    /// the bytes left, a state past [`ENDED`], a word not UTF-8.
    pub fn decode(b: &[u8]) -> Option<Stats> {
        let mut r = Reader(b);
        r.u8().filter(|v| *v == VERSION)?;
        let (at, loud) = (r.u32()?, counts(&mut r)?);
        let mut procs = Vec::new();
        for _ in 0..r.u16()? {
            let (pid, window, state) = (r.u32()?, r.u32()?, r.u8().filter(|s| *s <= ENDED)?);
            let argc = usize::from(r.u16()?);
            let argv = (0..argc.min(r.0.len() / 4)).map(|_| r.str()).collect::<Option<Vec<_>>>()?;
            (argv.len() == argc).then_some(())?;
            procs.push(Proc { pid, window, state, argv, counts: counts(&mut r)? });
        }
        let mut meters = Vec::new();
        for _ in 0..r.u16()? {
            meters.push((r.u32()?, counts(&mut r)?));
        }
        let (quiet, own) = (counts(&mut r)?, counts(&mut r)?);
        r.0.is_empty().then_some(Stats { at, loud, procs, meters, quiet, own })
    }

    /// The bytes, as the desktop writes them.
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out(Vec::new());
        list(o.u8(VERSION).u32(self.at), &self.loud).u16(self.procs.len() as u16);
        for p in &self.procs {
            o.u32(p.pid).u32(p.window).u8(p.state).u16(p.argv.len() as u16);
            p.argv.iter().for_each(|a| _ = o.str(a));
            list(&mut o, &p.counts);
        }
        o.u16(self.meters.len() as u16);
        for (pid, m) in &self.meters {
            list(o.u32(*pid), m);
        }
        list(list(&mut o, &self.quiet), &self.own);
        o.0
    }

    /// The meters of process `pid`; empty when the sample has none for it.
    pub fn meters_of(&self, pid: u32) -> &[u32] {
        self.meters.iter().find(|m| m.0 == pid).map_or(&[], |m| &m.1)
    }
}

/// A u16 count of u32s, none past the bytes left.
fn counts(r: &mut Reader<'_>) -> Option<Vec<u32>> {
    let n = usize::from(r.u16()?);
    (n <= r.0.len() / 4).then(|| (0..n).map(|_| r.u32()).collect())?
}

fn list<'a>(o: &'a mut Out, v: &[u32]) -> &'a mut Out {
    v.iter().fold(o.u16(v.len() as u16), |o, &n| o.u32(n))
}

/// The tokens in and out and the µ$ of the last AI receipt in `tail` (the end of a stream from
/// /api/ai, whose last line is `: receipt in=<n> out=<n> microusd=<n>`), if it holds one whole:
/// each number 1 to 9 digits.
pub fn receipt(tail: &[u8]) -> Option<[u32; 3]> {
    const LINE: &[u8] = b"\n: receipt ";
    let at = tail.windows(LINE.len()).rposition(|w| w == LINE)? + LINE.len();
    let line = tail[at..].split(|&c| c == b'\n').next()?;
    let mut words = line.split(|&c| c == b' ');
    let mut n = |key: &[u8]| {
        let digits = words.next()?.strip_prefix(key)?;
        let ok = !digits.is_empty() && digits.len() <= 9 && digits.iter().all(u8::is_ascii_digit);
        ok.then(|| digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    let r = [n(b"in=")?, n(b"out=")?, n(b"microusd=")?];
    words.next().is_none().then_some(r)
}

/// The FNV-1a 64 hash of `b`: what tells two samples' loud parts apart.
pub fn hash(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3))
}

/// When the desktop samples for a watcher, and whether it posts what it took: pure, the page's
/// clock passed in (ms).
///
/// Anything but the quiet stirs it ([`Pace::stir`]): a sample is then due at once, or [`GAP_MS`]
/// after the last ([`Pace::wait`] says when to wake for it). A sample is posted when its loud part
/// changed, and once more after one that did, so the watcher sees what moved come to rest; a new
/// watcher's first at once ([`Pace::fresh`]). While another process ran at the last sample (hot),
/// one is due each [`GAP_MS`] with no stir. Unstirred and cold, it asks for nothing: no timer, no
/// sample, no post.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pace {
    /// Something happened since the last sample; a new watcher waits for its first.
    pub stir: bool,
    pub fresh: bool,
    /// A process ran at the last sample; the last one changed (look once more).
    hot: bool,
    again: bool,
    /// When the next may be taken (page ms), and the last one's loud hash.
    next: u64,
    last: u64,
}

impl Pace {
    /// Whether to take a sample now.
    pub fn due(&self, now: u64) -> bool {
        self.fresh || self.wants() && now >= self.next
    }

    /// A sample was taken at `now` with the loud hash `loud`, another process running (`hot`)
    /// or not: whether to post it.
    pub fn took(&mut self, now: u64, loud: u64, hot: bool) -> bool {
        let changed = self.fresh || loud != self.last;
        let post = changed || self.again;
        let next = now + GAP_MS;
        *self = Pace { stir: false, fresh: false, hot, again: changed, next, last: loud };
        post
    }

    /// In how many ms a sample will be due, if one will be.
    pub fn wait(&self, now: u64) -> Option<u32> {
        self.wants().then(|| self.next.saturating_sub(now).min(GAP_MS) as u32)
    }

    fn wants(&self) -> bool {
        self.stir || self.hot || self.again
    }
}
