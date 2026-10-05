//! A run's records: one JSON line per task and trial, appended to `evals/results/<suite>.jsonl`.
//! Each line stands alone: which run (its id, date, commit, model), what was run (the suite and
//! its content hash, the coder's prompt hash and knobs hash), and how the task went (pass or
//! fail, the stage that failed and the checker's reason, the make's outcome, requests, tokens in
//! and out, micro-dollars, how many requests' costs were estimated, the make's milliseconds and
//! the program's lines). Nothing in it is read from a clock: a replay writes it again byte for
//! byte.

use coder::ai::put_num;
use coder::json::{Json, put};

/// Who ran what: the run's id, its date (YYYY-MM-DD), the commit it ran at and the model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub run: String,
    pub date: String,
    pub commit: String,
    pub model: String,
}

/// One task's trial in a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub meta: Meta,
    pub suite: String,
    pub suite_hash: u64,
    pub prompt: u64,
    pub knobs: u64,
    pub task: String,
    pub size: String,
    pub trial: u32,
    pub pass: bool,
    /// `ok`, or the first stage that failed: `ai` (the AI failed: not the model's), `make` (no
    /// program installed), `compile`, `smoke`, `icon`, `check`.
    pub stage: String,
    pub reason: String,
    pub outcome: String,
    pub tries: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub usd_micros: u64,
    pub est: u32,
    pub ms: u64,
    pub lines: u32,
}

/// `h` as 16 hex digits.
pub fn hex(h: u64) -> String {
    (0..16).rev().map(|i| char::from(b"0123456789abcdef"[(h >> (i * 4) & 15) as usize])).collect()
}

/// 16 hex digits as a number.
fn unhex(s: &str) -> Option<u64> {
    (s.len() == 16).then(|| u64::from_str_radix(s, 16).ok()).flatten()
}

/// Appends `,"key":` and `value` as a string.
fn s(o: &mut String, key: &str, value: &str) {
    *o += ",\"";
    *o += key;
    *o += "\":";
    put(o, value);
}

/// Appends `,"key":` and `n`.
fn n(o: &mut String, key: &str, n: u64) {
    *o += ",\"";
    *o += key;
    *o += "\":";
    put_num(o, n);
}

impl Record {
    /// Its JSON line, newline included.
    pub fn line(&self) -> String {
        let mut o = String::from("{\"v\":1");
        s(&mut o, "suite", &self.suite);
        s(&mut o, "suite_hash", &hex(self.suite_hash));
        s(&mut o, "run", &self.meta.run);
        s(&mut o, "date", &self.meta.date);
        s(&mut o, "commit", &self.meta.commit);
        s(&mut o, "model", &self.meta.model);
        s(&mut o, "prompt", &hex(self.prompt));
        s(&mut o, "knobs", &hex(self.knobs));
        s(&mut o, "task", &self.task);
        s(&mut o, "size", &self.size);
        n(&mut o, "trial", self.trial.into());
        o += if self.pass { ",\"pass\":true" } else { ",\"pass\":false" };
        s(&mut o, "stage", &self.stage);
        s(&mut o, "reason", &self.reason);
        s(&mut o, "outcome", &self.outcome);
        n(&mut o, "tries", self.tries.into());
        n(&mut o, "in", self.tokens_in);
        n(&mut o, "out", self.tokens_out);
        n(&mut o, "usd_micros", self.usd_micros);
        n(&mut o, "est", self.est.into());
        n(&mut o, "ms", self.ms);
        n(&mut o, "lines", self.lines.into());
        o += "}\n";
        o
    }

    /// A record from its JSON line; `None` if it is not one.
    pub fn parse(line: &str) -> Option<Record> {
        let j = Json::parse(line)?;
        let s = |k: &str| j.get(k).and_then(Json::text).map(str::to_string);
        let n = |k: &str| j.get(k).and_then(Json::text).and_then(|t| t.parse::<u64>().ok());
        let meta =
            Meta { run: s("run")?, date: s("date")?, commit: s("commit")?, model: s("model")? };
        Some(Record {
            meta,
            suite: s("suite")?,
            suite_hash: unhex(&s("suite_hash")?)?,
            prompt: unhex(&s("prompt")?)?,
            knobs: unhex(&s("knobs")?)?,
            task: s("task")?,
            size: s("size")?,
            trial: n("trial")? as u32,
            pass: j.get("pass")? == &Json::Bool(true),
            stage: s("stage")?,
            reason: s("reason")?,
            outcome: s("outcome")?,
            tries: n("tries")? as u32,
            tokens_in: n("in")?,
            tokens_out: n("out")?,
            usd_micros: n("usd_micros")?,
            est: n("est")? as u32,
            ms: n("ms")?,
            lines: n("lines")? as u32,
        })
    }
}

/// The records in `text` (a results file), the last of each run's task and trial where one was
/// run again (a resumed run), in the order they first came; lines that are not records are
/// skipped.
pub fn load(text: &str) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::new();
    for r in text.lines().filter_map(Record::parse) {
        let same = |o: &Record| (&o.meta.run, &o.task, o.trial) == (&r.meta.run, &r.task, r.trial);
        match out.iter().position(same) {
            Some(i) => out[i] = r,
            None => out.push(r),
        }
    }
    out
}
