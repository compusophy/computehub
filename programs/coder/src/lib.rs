//! The coding agent: makes an applang app with the free AI. The harness drives; the model only
//! writes. A [`Make`] asks for a program (or for one changed), checks what comes back (compiles,
//! then [`applang::smoke`] on seeds 1 to 3, then a run from the states its app keeps), and
//! sends each problem back as a fresh request asking for SEARCH/REPLACE edits ([`edits`]), with
//! the program numbered and the problem's coded account. The same problem twice gets one
//! rewrite; edits that do not apply get one more try, then the rewrite. It keeps the best
//! program so far (compiles, then runs clean, then newer), so a change never makes an app worse,
//! and stops by budget ([`Knobs`]: requests, output tokens, dollars, milliseconds).
//!
//! It is sans-IO: requests go out as [`Out::Ask`] bodies, the response bytes and its end come in
//! ([`Make::data`], [`Make::end`]), and time is an input (`now_ms`), so a make is a pure
//! function of what it was given and a recorded transcript replays it bit for bit. Studio runs
//! it; the Assistant shares its [`json`] and [`ai`] helpers.

#![forbid(unsafe_code)]

pub mod ai;
pub mod edits;
pub mod json;
mod make;
mod prompt;
pub mod receipt;
#[cfg(test)]
mod tests;

use ai::num;
pub use make::Make;
pub use prompt::system;

/// What to make: the ask, the program it changes ("" for a new app), the model, and what the
/// saved states' file of the app it changes holds ("" for none): a program must run from it.
#[derive(Clone, Debug, Default)]
pub struct Task {
    pub ask: String,
    pub base: String,
    pub model: String,
    pub kept: String,
}

/// A make's budgets and request settings; each one an eval flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    /// Requests in all, and rewrites among them.
    pub requests: u8,
    pub rewrites: u8,
    /// Output tokens in all, reasoning included (estimated for a cancelled reply).
    pub out_tokens: u32,
    /// Dollars in all, in micro-dollars.
    pub usd_micros: u32,
    /// Milliseconds in all, checked at every chunk and turn.
    pub ms: u64,
    /// `max_tokens` of a request for a program, and of a fix.
    pub write_tokens: u32,
    pub fix_tokens: u32,
    /// The thinking budget asked for a program and for a fix: 0 for the free AI's (1,024).
    pub write_reasoning: u16,
    pub fix_reasoning: u16,
    /// Past this many reasoning tokens (estimated from their chars, 3.4 a token) with no reply
    /// yet, the model is thinking past its budget: the request stops and is made again, once.
    /// Twice the 1,024 the free AI asks for (a provider that keeps to it stops near 1,037; one
    /// that did not, live, thought 6,144 tokens at 40 a second, 2.2 chars each, and left no time).
    pub runaway: u32,
    /// The smoke test's seeds: 1 to this.
    pub seeds: u8,
}

impl Default for Knobs {
    /// 5 requests, 1 rewrite, 20,000 output tokens, $0.08, 150 s; 6,144 tokens for a program and
    /// 4,096 for a fix, the free AI's thinking budget; a runaway past 2,048 reasoning tokens;
    /// seeds 1 to 3.
    fn default() -> Knobs {
        Knobs {
            requests: 5,
            rewrites: 1,
            out_tokens: 20_000,
            usd_micros: 80_000,
            ms: 150_000,
            write_tokens: 6_144,
            fix_tokens: 4_096,
            write_reasoning: 0,
            fix_reasoning: 0,
            runaway: 2_048,
            seeds: 3,
        }
    }
}

/// What a make wants done next.
#[derive(Debug)]
pub enum Out {
    /// Send this chat-completions body (the request before it, if any, is over).
    Ask(String),
    /// Stop the request in flight: its reply is in. [`Make::check`] comes next.
    Cancel,
    /// The make is over.
    Done(Done),
}

/// What a turn asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// A new app.
    Write,
    /// A change to the program open.
    Change,
    /// A fix of a compile error or a fault.
    Fix,
    /// The same, after edits that did not apply.
    Missed,
    /// The program again, simpler: the same problem twice.
    Rewrite,
    /// The same app, shorter: the reply ran out of room.
    Shorter,
    /// A program or edits at all: the reply held neither.
    Format,
}

/// How a make ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Its program runs clean: installed.
    Ready,
    /// A new app's best program compiles but faults: installed, its fault showing.
    Faulting,
    /// Nothing beat what there was: nothing installed.
    Broken,
    /// The model says applang can make nothing close: nothing installed.
    Cant,
    /// The person stopped it: the best so far installed under the same rules.
    Stopped,
    /// The AI failed (E0901 to E0905).
    Failed,
}

/// One request: what it asked for, its tokens (in, cached, out, reasoning), its price in
/// micro-dollars, its milliseconds and the code of the problem it left (0: none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnLog {
    pub turn: Turn,
    pub input: u32,
    pub cached: u32,
    pub output: u32,
    pub reasoning: u32,
    pub usd_micros: u32,
    pub ms: u32,
    pub code: u16,
}

/// A make's turns, milliseconds and price; `est` when a cancelled reply's usage was estimated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    pub turns: Vec<TurnLog>,
    pub ms: u64,
    pub usd_micros: u32,
    pub est: bool,
}

/// A make's end: how, the program (to install, if `install`; else the last seen, or what a reply
/// held of one, for the code view) and its problem's bytes in it, the code and line of the
/// problem to show (0: none; a code of the make's own, E0906 to E0910, when there was no
/// program), what the program's first comment says, whether it changed a program, and the
/// receipt.
#[derive(Clone, Debug)]
pub struct Done {
    pub outcome: Outcome,
    pub install: bool,
    pub draft: String,
    pub mark: Option<applang::Span>,
    pub code: u16,
    pub line: u32,
    pub why: String,
    pub plan: String,
    pub change: bool,
    pub receipt: Receipt,
}

impl Done {
    /// The status a person reads: `ready · 46 s`, `runs, but faults · E0215 line 41`,
    /// `couldn't · E0302 line 43`, `can't make that`, `AI busy · try in a minute`, ...
    pub fn said(&self) -> String {
        let lead = match self.outcome {
            Outcome::Ready => {
                return ["ready \u{b7} ", &num(self.receipt.ms / 1000), " s"].concat();
            }
            Outcome::Faulting => "runs, but faults",
            Outcome::Broken if self.change => "couldn't change it",
            Outcome::Broken => "couldn't",
            Outcome::Cant => "can't make that",
            Outcome::Stopped => "stopped",
            Outcome::Failed if self.code == 903 => "AI busy \u{b7} try in a minute",
            Outcome::Failed if self.code == 902 => "out of AI for today",
            Outcome::Failed => "AI unreachable",
        };
        let mut out = String::from(lead);
        if matches!(self.outcome, Outcome::Faulting | Outcome::Broken) {
            out += " \u{b7} ";
            ai::put_code(&mut out, self.code);
            if self.line > 0 {
                out += " line ";
                ai::put_num(&mut out, self.line.into());
            }
        }
        out
    }
}

/// The ids Studio's own widgets use, so that another program (the overlay) can find them: Stop,
/// and the prompt Input, whose id is in `PROMPT..PROMPT_END`.
pub mod ids {
    pub const STOP: u32 = 2;
    pub const PROMPT: u32 = 1 << 24;
    pub const PROMPT_END: u32 = 2 << 24;
}
