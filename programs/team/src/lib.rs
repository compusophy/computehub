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
pub const TEMPERATURE: &str = "0.2";
/// The samplers a pinned request names, Qwen's own (the 3B's GGUF carries them; the 7B's does
/// not, so llama-server's top_k 40 and top_p 0.95 sample it), so helpers of every size sample
/// alike.
pub const PIN: &str = ",\"top_k\":20,\"top_p\":0.8,\"min_p\":0.05";
/// What a fix asks besides when the helper is to reply with whole programs.
pub const WHOLE: &str =
    "\n\nReply with the whole corrected program in one app block, and nothing else.";

/// How a repair asks, past the make loop's fix: its temperature (a decimal, as JSON has it), a
/// seed (each turn's request seeded with it plus the turns taken, so a run replays), the samplers
/// pinned ([`PIN`]), whole programs asked for ([`WHOLE`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opts {
    pub temperature: String,
    pub seed: Option<u32>,
    pub pin: bool,
    pub whole: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { temperature: TEMPERATURE.into(), seed: None, pin: false, whole: false }
    }
}

/// The seed of a repair's `turn` (from 0): its `seed` plus the turn, under 2^31, so it is never
/// llama.cpp's "any seed" (0xFFFFFFFF) and reads the same signed or not.
pub fn turn_seed(seed: u32, turn: usize) -> u32 {
    seed.wrapping_add(turn as u32) & 0x7FFF_FFFF
}

/// What a helper's reply held: a whole program, edit blocks that applied, a unified diff read as
/// edit blocks that applied ([`diff`]), edit blocks (or a diff) that did not (asked again with
/// why), or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    Program,
    Edits,
    Diff,
    /// One-line edit blocks quoting part of one line (most often the icon line without its
    /// `// icon:`), applied in that line.
    Inline,
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

/// `edits` that the make loop could not apply, applied as one-line edits within a line: each
/// block one line to find and one to put, the first found in exactly one line of `src` (not as
/// that whole line, which the make loop would have matched) and replaced there. Of the base 3B's
/// 37 one-line blocks that missed (2026-10-09), 32 quoted part of exactly one line, nearly all
/// an icon line without its `// icon:`.
fn inline(src: &str, edits: &[coder::edits::Edit]) -> Option<String> {
    let mut lines: Vec<String> = src.lines().map(String::from).collect();
    for e in edits {
        let find: Vec<&String> = e.search.iter().filter(|l| !l.trim().is_empty()).collect();
        let put: Vec<&String> = e.replace.iter().filter(|l| !l.trim().is_empty()).collect();
        let ([find], [put]) = (&find[..], &put[..]) else { return None };
        let (find, put) = (find.trim(), put.trim());
        let hits: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].contains(find)).collect();
        let [i] = hits[..] else { return None };
        if lines[i].trim() == find || find.is_empty() {
            return None;
        }
        lines[i] = lines[i].replacen(find, put, 1);
    }
    Some(lines.join("\n") + "\n")
}

/// The rank of `src` as a repair ranks it (0 it does not compile, 1 it faults, 2 only its icon
/// is wrong, 3 it runs clean), run from the saved states `kept` by the make loop's test.
pub fn rank_of(src: &str, kept: &str) -> u8 {
    rank(&ai::fault(src, kept, coder::Knobs::default().seeds))
}

/// How many of `draft`'s distinct lines (trimmed, not blank) `src` still has, and how many it has.
pub fn kept(draft: &str, src: &str) -> (usize, usize) {
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
/// name, how it is asked, the turns it may take and the smoke test's seeds; the best program so
/// far and what is wrong with it; the last message asked and its seed; the turns taken.
#[derive(Debug)]
pub struct Repair {
    ask: String,
    kept: String,
    model: String,
    opts: Opts,
    turns: u8,
    seeds: u8,
    system: String,
    best: String,
    fault: Option<Fault>,
    asked: String,
    seed: Option<u32>,
    steps: Vec<Step>,
}

impl Repair {
    /// Repairs `draft`, written for `ask`, with the helper `model` in at most `turns` turns:
    /// the first request, or the end (it runs clean already, or no turns are allowed).
    pub fn start(ask: &str, kept: &str, draft: &str, model: &str, turns: u8) -> (Repair, Next) {
        Repair::start_at(ask, kept, draft, model, turns, TEMPERATURE)
    }

    /// [`Repair::start`] at another sampling temperature (a decimal, as JSON has it): another
    /// try at the same draft samples another repair.
    pub fn start_at(
        ask: &str,
        kept: &str,
        draft: &str,
        model: &str,
        turns: u8,
        temperature: &str,
    ) -> (Repair, Next) {
        let opts = Opts { temperature: temperature.into(), ..Opts::default() };
        Repair::start_with(ask, kept, draft, model, turns, &opts)
    }

    /// [`Repair::start`] asked as `opts` says: seeded, its samplers pinned, whole programs asked
    /// for. With only a temperature, asked as [`Repair::start_at`] asks.
    pub fn start_with(
        ask: &str,
        kept: &str,
        draft: &str,
        model: &str,
        turns: u8,
        opts: &Opts,
    ) -> (Repair, Next) {
        let seeds = coder::Knobs::default().seeds;
        let fault = ai::fault(draft, kept, seeds);
        let mut r = Repair {
            ask: ask.into(),
            kept: kept.into(),
            model: model.into(),
            opts: opts.clone(),
            turns,
            seeds,
            system: prompt::system(),
            best: draft.into(),
            fault,
            asked: String::new(),
            seed: None,
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
        // A re-ask after a miss repeats the message asked: WHOLE once, if it was asked.
        let msg = match missed {
            Some(why) => prompt::missed(&self.asked, &why),
            None if self.opts.whole => prompt::fix(&self.ask, &self.best, &account) + WHOLE,
            None => prompt::fix(&self.ask, &self.best, &account),
        };
        let mut options = String::from(",\"max_tokens\":");
        ai::put_num(&mut options, ROOM.into());
        options += ",\"temperature\":";
        options += &self.opts.temperature;
        if self.opts.pin {
            options += PIN;
        }
        self.seed = self.opts.seed.map(|s| turn_seed(s, self.steps.len()));
        if let Some(seed) = self.seed {
            options += ",\"seed\":";
            ai::put_num(&mut options, seed.into());
        }
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

    /// The seed the request in flight named, if it is seeded.
    pub fn seed(&self) -> Option<u32> {
        self.seed
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
                Err(why) => match inline(&self.best, &e) {
                    Some(src) => (Held::Inline, Some(src), None),
                    None => (Held::Missed, None, Some(why)),
                },
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
