//! compusophyOS's evals: fixed, versioned suites whose checkers are automatic and
//! deterministic, so that a change to a model, a prompt, the harness or the language shows as a
//! measured gain or loss, never an impression.
//!
//! - **Suites.** [`makes`], Suite 1: apps described precisely, made by Studio's coding agent
//!   ([`coder`]) and graded by what they do when driven headlessly: each compiles, runs clean
//!   through the smoke test on 3 seeds, draws its icon, and its checker's scripted taps, keys and
//!   ticks find what the description promised. A suite's content hash covers its tasks and
//!   checkers.
//! - **Runs.** [`run::task`] drives one make over a [`run::Wire`] and returns its
//!   [`record::Record`] (appended to `evals/results/<suite>.jsonl`) and its AI exchanges
//!   ([`replay::Exchange`], kept in `evals/replays/<suite>/<run>.jsonl`). The date and commit are
//!   passed in; nothing here reads a clock, so a run replayed from its exchanges
//!   ([`replay::Replay`]) writes the same records byte for byte.
//! - **Reading a gain.** [`summary`]: two runs or two models, pass rates with Wilson intervals,
//!   their difference with Newcombe's, tokens and dollars per pass, and the tasks that flipped.
//!
//! The live wire (the system's `curl` against the free AI, paced under its limits) is
//! `tools/eval`, a dev tool that never ships; this library is pure, so it can later run inside the
//! OS, from the Terminal.

#![forbid(unsafe_code)]

pub mod record;
pub mod replay;
pub mod run;
pub mod summary;
#[cfg(test)]
mod tests;

/// FNV-1a 64 of `b`, as the coder hashes its prompt.
pub fn fnv(b: &[u8]) -> u64 {
    uiwire::stat::hash(b)
}

/// The coder's system prompt's hash.
pub fn prompt_hash() -> u64 {
    fnv(coder::system().as_bytes())
}

/// The hash of a make's budgets and settings.
pub fn knobs_hash(k: &coder::Knobs) -> u64 {
    fnv(format!("{k:?}").as_bytes())
}
