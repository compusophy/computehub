//! The team: models working as one on a make (DESIGN.md, "The team: many models, one mind"). Its
//! first move, [`Repair`]: a helper repairs the lead's draft. The lead (a cloud model) wrote a
//! program; when it does not run clean (it does not compile, it faults on the smoke test, its
//! icon does not draw), a helper (a small model on a device) is asked to fix it as Studio asks a
//! fix (`coder::prompt::fix`, with the fault's account), a few turns, every answer judged by the
//! same test the make loop runs (`coder::ai::fault`), so no model's word is taken. A fix that
//! drops more than a quarter of the program's lines is refused, as the make loop refuses one.
//!
//! Sans-IO, as `coder` is: [`Repair::start`] and [`Repair::reply`] say what to send next
//! ([`Next::Ask`], a chat-completions body) or how the repair ended ([`Next::Done`]); the host
//! carries the requests, to a model wherever it runs. `coder` itself is untouched: its files are
//! in the IQ verifier's hash.

#![forbid(unsafe_code)]

pub mod diff;

use coder::ai::{self, Fault};
use coder::edits::{self, Reply};
use coder::prompt;

/// A fix's room in output tokens and its temperature, as the make loop asks a fix.
pub const ROOM: u32 = 4_096;
const TEMPERATURE: &str = "0.2";

/// What a helper's reply held: a whole program, edit blocks that applied, a unified diff read as
/// edit blocks that applied ([`diff`]), edit blocks (or a diff) that did not (asked again with
/// why), or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    Program,
    Edits,
    Diff,
    Missed,
    Nothing,
}

/// One helper turn: what its reply held, and the best program's problem before and after it (its
/// code; 0: it runs clean).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub held: Held,
    pub before: u16,
    pub after: u16,
}

/// How a repair ended: the best program, whether it runs clean, and each turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixed {
    pub src: String,
    pub clean: bool,
    pub steps: Vec<Step>,
}

/// What a repair wants done next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Send this chat-completions body to the helper; its whole reply comes to [`Repair::reply`].
    Ask(String),
    /// The repair is over.
    Done(Fixed),
}

/// A program's rank: 0 it does not compile, 1 it compiles but faults, 2 it runs (only its icon is
/// wrong), 3 it runs clean.
fn rank(f: &Option<Fault>) -> u8 {
    match f {
        None => 3,
        Some(f) if f.runs => 2,
        Some(f) if f.compiles => 1,
        Some(_) => 0,
    }
}

/// How many of `draft`'s distinct lines (trimmed, not blank) `src` still has, and how many it has.
fn kept(draft: &str, src: &str) -> (usize, usize) {
    let lines = |s: &str| {
        let mut v: Vec<String> =
            s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (d, s) = (lines(draft), lines(src));
    (d.iter().filter(|l| s.binary_search(l).is_ok()).count(), d.len())
}

/// The problem's code (0: none).
fn code(f: &Option<Fault>) -> u16 {
    f.as_ref().map_or(0, |f| f.diag.code.unwrap_or(1))
}

/// A helper repairing a draft: the ask, the saved states it must run from, the helper's model
/// name, the turns it may take and the smoke test's seeds; the best program so far and what is
/// wrong with it; the last message asked; the turns taken.
#[derive(Debug)]
pub struct Repair {
    ask: String,
    kept: String,
    model: String,
    turns: u8,
    seeds: u8,
    system: String,
    best: String,
    fault: Option<Fault>,
    asked: String,
    steps: Vec<Step>,
}

impl Repair {
    /// Repairs `draft`, written for `ask`, with the helper `model` in at most `turns` turns:
    /// the first request, or the end (it runs clean already, or no turns are allowed).
    pub fn start(ask: &str, kept: &str, draft: &str, model: &str, turns: u8) -> (Repair, Next) {
        let seeds = coder::Knobs::default().seeds;
        let fault = ai::fault(draft, kept, seeds);
        let mut r = Repair {
            ask: ask.into(),
            kept: kept.into(),
            model: model.into(),
            turns,
            seeds,
            system: prompt::system(),
            best: draft.into(),
            fault,
            asked: String::new(),
            steps: Vec::new(),
        };
        let next = r.next(None);
        (r, next)
    }

    /// The next request (a fix of the best program, or the last one again with why its edits did
    /// not apply), or the end.
    fn next(&mut self, missed: Option<String>) -> Next {
        let account = match &self.fault {
            Some(f) if self.steps.len() < usize::from(self.turns) => f.account.clone(),
            _ => {
                let (src, clean) = (self.best.clone(), self.fault.is_none());
                return Next::Done(Fixed { src, clean, steps: self.steps.clone() });
            }
        };
        let msg = match missed {
            Some(why) => prompt::missed(&self.asked, &why),
            None => prompt::fix(&self.ask, &self.best, &account),
        };
        let mut options = String::from(",\"max_tokens\":");
        ai::put_num(&mut options, ROOM.into());
        options += ",\"temperature\":";
        options += TEMPERATURE;
        let body = ai::chat(&self.model, &options, &self.system, &msg);
        self.asked = msg;
        Next::Ask(body)
    }

    /// The last turn, once one is taken.
    pub fn last(&self) -> Option<&Step> {
        self.steps.last()
    }

    /// The message the request in flight asked (a trace's prompt).
    pub fn asked(&self) -> &str {
        &self.asked
    }

    /// The helper's whole reply (`cut`: it ran out of room): judged, kept if it is no worse than
    /// the best (the newer wins a tie, as in the make loop), then the next request or the end.
    pub fn reply(&mut self, text: &str, cut: bool) -> Next {
        let before = code(&self.fault);
        // A reply with nothing the make loop reads, but a unified diff: its hunks as edit blocks.
        let read = match edits::read(text, cut) {
            Reply::Nothing | Reply::Unclosed(_) => match diff::blocks(text) {
                Some(blocks) => (edits::read(&blocks, false), Held::Diff),
                None => (Reply::Nothing, Held::Nothing),
            },
            r => (r, Held::Edits),
        };
        let (held, candidate, missed) = match read {
            (Reply::Program(src), _) => (Held::Program, Some(src), None),
            (Reply::Edits(e), applied) => match edits::apply(&self.best, &e) {
                Ok(src) => (applied, Some(src), None),
                Err(why) => (Held::Missed, None, Some(why)),
            },
            _ => (Held::Nothing, None, None),
        };
        // A fix that drops more than a quarter of the lines is not a fix; nor is a whole program
        // that is another program (tonight a 0.5B answered five of GLM's passing drafts, faulted
        // only by their icons, with an example app from the prompt, which ran clean): it keeps at
        // least half the draft's lines.
        let whole = |src: &String| edits::lines(src) * 4 >= edits::lines(&self.best) * 3;
        let same = |src: &String| {
            let (k, n) = kept(&self.best, src);
            held != Held::Program || k * 2 >= n
        };
        if let Some(src) = candidate.filter(|s| whole(s) && same(s)) {
            let fault = ai::fault(&src, &self.kept, self.seeds);
            if rank(&fault) >= rank(&self.fault) {
                (self.best, self.fault) = (src, fault);
            }
        }
        self.steps.push(Step { held, before, after: code(&self.fault) });
        self.next(missed)
    }
}

#[cfg(test)]
mod tests;
