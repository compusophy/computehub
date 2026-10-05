//! # teach: Claude Opus 5.5 as the teacher of applang
//!
//! A dev tool, never shipped. The data engine of the fine-tuning loop: a teacher model writes
//! tasks (an ask, a checker, a reference program) and solves them; a verifier keeps what passes;
//! every kept line says who made it, under which prompt, verified by what, on which day.
//!
//! - [`claude`]: the Messages and Message Batches APIs' bodies and replies, read by block type
//!   after the stop reason; [`wire`]: the system's `curl`, the key on its stdin and never on its
//!   command line, retries with backoff, batches created, polled and read by `custom_id`, and an
//!   OpenAI-compatible chat client for the models being taught.
//! - [`cost`]: every reply's usage priced and appended to a ledger, and a budget that stops a
//!   run before it would pass it.
//! - [`seam`]: what teach asks of a verifier ([`seam::Judge`]); `iq` implements it, and
//!   [`seam::Smoke`] (compile and the coder's smoke test) stands in until it lands.
//! - [`prompts`]: the task writer's, and the solver's and fixer's, which are the coder's own,
//!   byte for byte, so a verified reply is training data in the format it is asked in.
//! - [`run`]: the commands (`teach tasks | solve | export | ask | cost`, see `main.rs`).
//!
//! Every failure is coded, E0980 to E0994 ([`codes`]).

#![forbid(unsafe_code)]

pub mod claude;
pub mod cost;
pub mod prompts;
pub mod run;
pub mod seam;
#[cfg(test)]
mod tests;
pub mod wire;

/// The teacher, unless a run names another.
pub const TEACHER: &str = "claude-opus-5-5";

/// teach's failure codes.
pub mod codes {
    /// `ANTHROPIC_API_KEY` is unset, empty, or holds what no key does.
    pub const NO_KEY: u16 = 980;
    /// `curl` could not run, or reached nothing.
    pub const CURL: u16 = 981;
    /// The API refused the request (a 4xx but 429).
    pub const HTTP: u16 = 982;
    /// The API stayed busy or failing (429, 5xx, overloaded) through every retry.
    pub const BUSY: u16 = 983;
    /// A reply that does not read as one.
    pub const UNREADABLE: u16 = 984;
    /// The model, or a safety classifier, declined (`stop_reason` refusal).
    pub const REFUSED: u16 = 985;
    /// The reply ran out of `max_tokens` before it ended.
    pub const CUT: u16 = 986;
    /// The run's budget has no room for this request.
    pub const BUDGET: u16 = 987;
    /// A batch request errored, expired or was canceled.
    pub const BATCH: u16 = 988;
    /// An input file or line is not what it should be.
    pub const INPUT: u16 = 989;
    /// A command or flag not known, or a flag's value that does not read.
    pub const USAGE: u16 = 990;
    /// A reply with no program in it, or edits that did not apply.
    pub const NO_PROGRAM: u16 = 991;
    /// A file could not be read or written.
    pub const FILE: u16 = 992;
    /// A record made under another system prompt than the coder's now.
    pub const DRIFT: u16 = 993;
    /// The verifier refused a task.
    pub const TASK_REFUSED: u16 = 994;
}

/// A coded failure: its code and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail {
    pub code: u16,
    pub why: String,
}

impl Fail {
    pub fn new(code: u16, why: impl Into<String>) -> Fail {
        Fail { code, why: why.into() }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", coder::ai::ecode(self.code), self.why)
    }
}

/// FNV-1a 64 of `b`, as the coder's test and the evals hash its prompt.
pub fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// `h` as 16 lowercase hex digits.
pub fn hex16(h: u64) -> String {
    format!("{h:016x}")
}

/// The day (UTC, `YYYY-MM-DD`) that `secs` since the epoch fall on.
pub fn day(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

/// Today (UTC).
pub fn today() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    day(now.map_or(0, |d| d.as_secs()))
}

/// Appends `text` to the file at `path`, made if missing.
pub fn append(path: &str, text: &str) -> Result<(), Fail> {
    use std::io::Write as _;
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path);
    let done = file.and_then(|mut f| f.write_all(text.as_bytes()));
    done.map_err(|e| Fail::new(codes::FILE, format!("{path}: {e}")))
}

/// The text of the file at `path`.
pub fn read(path: &str) -> Result<String, Fail> {
    std::fs::read_to_string(path).map_err(|e| Fail::new(codes::FILE, format!("{path}: {e}")))
}
