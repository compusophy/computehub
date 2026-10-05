//! A task's trial end to end: the coder's make ([`coder::Make`], as Studio runs it) driven over
//! a [`Wire`] (the live free AI, or a [`crate::replay::Replay`]), each exchange kept as an
//! [`Exchange`], the program it installs graded ([`grade`]) and its [`Record`].
//!
//! Time is the make's own clock, which only the AI moves: each request starts when the one
//! before it was over and lasts what its wire says (pauses for the free AI's limits, and the
//! checks between requests, take none of it). So a replay gives the make the very times it had.
//! The make stops a request once its program is in, as Studio does; the wire then reads the rest
//! of the stream for its receipt (what it cost), but feeds the make nothing more.

use coder::ai::MAX_REPLY;
use coder::json::Stream;
use coder::{Done, Knobs, Make, Out, Outcome};

use crate::record::{Meta, Record};
use crate::replay::Exchange;
use makes::Task;

/// Which request a wire sends: its task, trial, place in the make (from 1) and body's hash.
#[derive(Clone, Copy, Debug)]
pub struct At<'a> {
    pub task: &'a str,
    pub trial: u32,
    pub n: u32,
    pub hash: u64,
}

/// What the make wants of the rest of a response: more of it; none, but read on for the receipt
/// (its program is in); or none at all (stop the request).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    More,
    Drain,
    Stop,
}

/// How a request ended: its HTTP status (0: none), the wire's error, when (ms after it was
/// sent) and the receipt its stream ended with.
#[derive(Clone, Debug, Default)]
pub struct Answer {
    pub status: u16,
    pub error: String,
    pub ms: u64,
    pub receipt: Option<[u32; 3]>,
}

/// How a make's requests reach the AI.
pub trait Wire {
    /// POSTs `body` (the request `at`): each chunk of its response goes to `on` with the ms
    /// since it was sent, while `on` says [`Feed::More`]; then how it ended.
    fn post(&mut self, at: &At, body: &str, on: &mut dyn FnMut(&[u8], u64) -> Feed) -> Answer;
}

/// What a response did to the make, kept as it streams: the make's own reading of it, and the
/// lines it keeps aside and the partial line, as [`Stream`] keeps them.
#[derive(Default)]
struct Mirror {
    stream: Stream,
    text: String,
    rest: Vec<u8>,
    other: String,
    fed: Option<u64>,
}

impl Mirror {
    fn feed(&mut self, chunk: &[u8], ms: u64) {
        self.stream.feed(chunk, &mut self.text, MAX_REPLY);
        self.fed = Some(ms);
        self.rest.extend_from_slice(chunk);
        let Some(n) = self.rest.iter().rposition(|&b| b == b'\n') else { return };
        let lines: Vec<u8> = self.rest.drain(..=n).collect();
        for line in String::from_utf8_lossy(&lines).lines() {
            let line = line.trim_end_matches('\r');
            if !line.starts_with("data:")
                && !line.starts_with(':')
                && self.other.len() + line.len() <= 16 * 1024
            {
                self.other += line;
            }
        }
    }

    fn exchange(self, at: &At, a: &Answer, cut: bool) -> Exchange {
        Exchange {
            task: at.task.into(),
            trial: at.trial,
            n: at.n,
            hash: at.hash,
            fed: self.fed,
            end: (!cut).then_some(a.ms),
            status: a.status,
            error: a.error.clone(),
            text: self.text,
            thought: self.stream.thought,
            finish: self.stream.finish,
            said: self.stream.error,
            usage: self.stream.usage,
            other: self.other,
            rest: String::from_utf8_lossy(&self.rest).into_owned(),
            receipt: a.receipt,
        }
    }
}

/// `task` made by `model` within `k` (its trial `trial`), over `wire`: how it ended and its
/// exchanges.
pub fn make(
    task: &Task,
    model: &str,
    trial: u32,
    k: Knobs,
    wire: &mut dyn Wire,
) -> (Done, Vec<Exchange>) {
    let t = coder::Task { ask: task.ask.into(), model: model.into(), ..coder::Task::default() };
    let (mut m, mut out) = Make::start(t, k, 0);
    let (mut now, mut log) = (0, Vec::new());
    loop {
        let body = match out {
            Out::Done(done) => return (done, log),
            Out::Ask(body) => body,
            Out::Cancel => unreachable!("a make cancels only while a reply streams"),
        };
        let at =
            At { task: task.id, trial, n: log.len() as u32 + 1, hash: crate::fnv(body.as_bytes()) };
        let (mut x, mut cut) = (Mirror::default(), None);
        let a = wire.post(&at, &body, &mut |chunk, ms| {
            if cut.is_some() {
                return Feed::Drain;
            }
            x.feed(chunk, ms);
            match m.data(chunk, now + ms) {
                None => Feed::More,
                Some(_) => {
                    cut = Some(ms);
                    let program = coder::edits::program(&x.text).is_some_and(|p| p.1);
                    if program { Feed::Drain } else { Feed::Stop }
                }
            }
        });
        let next = match cut {
            Some(ms) => {
                now += ms;
                m.check(now)
            }
            None => {
                now += a.ms;
                m.end(a.status, &a.error, now).or_else(|| m.check(now))
            }
        };
        out = next.unwrap_or_else(|| Out::Done(m.stop(now)));
        log.push(x.exchange(&at, &a, cut.is_some()));
    }
}

/// How `done` fares: whether it passes, the first stage that failed (or `ok`) and why: `ai` when
/// the AI failed (not the model's), `make` when it installed no program, else as
/// [`makes::judge`] grades the program it installed.
pub fn grade(task: &Task, done: &Done) -> (bool, &'static str, String) {
    let fail = |stage, why: String| (false, stage, coder::ai::clip(&why, 300));
    if done.outcome == Outcome::Failed {
        return fail("ai", done.why.clone());
    }
    if !done.install {
        return fail("make", done.said());
    }
    makes::judge(task, &done.draft)
}

/// `task`'s trial `trial` in the run `meta`, over `wire`: its record and exchanges.
pub fn task(meta: &Meta, task: &Task, trial: u32, wire: &mut dyn Wire) -> (Record, Vec<Exchange>) {
    let k = Knobs::default();
    let (done, log) = make(task, &meta.model, trial, k, wire);
    let (pass, stage, reason) = grade(task, &done);
    let mut r = Record {
        meta: meta.clone(),
        suite: makes::ID.into(),
        suite_hash: makes::hash(),
        prompt: crate::prompt_hash(),
        knobs: crate::knobs_hash(&k),
        task: task.id.into(),
        size: task.size.into(),
        trial,
        pass,
        stage: stage.into(),
        reason,
        outcome: match done.outcome {
            Outcome::Ready => "ready",
            Outcome::Faulting => "faulting",
            Outcome::Broken => "broken",
            Outcome::Cant => "cant",
            Outcome::Stopped => "stopped",
            Outcome::Failed => "failed",
        }
        .into(),
        tries: log.len() as u32,
        ms: done.receipt.ms,
        lines: if done.install { done.draft.lines().count() as u32 } else { 0 },
        ..Record::default()
    };
    // What each request cost: its receipt; else, if it reached the AI, the make's reckoning.
    for (i, x) in log.iter().enumerate() {
        let turn = done.receipt.turns.get(i);
        match (x.receipt, turn) {
            (Some([i, o, u]), _) => {
                (r.tokens_in, r.tokens_out) =
                    (r.tokens_in + u64::from(i), r.tokens_out + u64::from(o));
                r.usd_micros += u64::from(u);
            }
            (None, Some(t)) if x.status < 300 && x.fed.is_some() => {
                (r.tokens_in, r.tokens_out) =
                    (r.tokens_in + u64::from(t.input), r.tokens_out + u64::from(t.output));
                r.usd_micros += u64::from(t.usd_micros);
                r.est += 1;
            }
            _ => {}
        }
    }
    (r, log)
}
