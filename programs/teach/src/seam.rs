//! The seam to the verifier. teach talks to models and grades nothing itself: what a task line
//! must hold to stand, and what a program made for a task earns, are a [`Judge`]'s to say. `iq`
//! (the benchmark: its tasks, its checker language, `grade`, `verify`) implements it; until it
//! lands, [`Smoke`] stands in, with what the coder itself checks: a program compiles and runs
//! clean through applang's smoke test on seeds 1 to 3, and its icon line draws.
//!
//! A task is one JSON line, its keys in this order:
//! `{"id":"snake-wrap","tier":3,"family":"snake","ask":"...","check":"...","ref":"...",
//! "by":{"teacher":"claude-opus-5-5","prompt":"<16 hex>","verifier":"<16 hex>","day":"YYYY-MM-DD"}}`.
//! Held-out families are the verifier's to choose; teach takes their list (`--held`, one family a
//! line) and never solves one nor exports one for training.

use coder::json::{Json, put};

use crate::{fnv, hex16};

/// What a program earns: whether it passes, the stage it reached (the first that failed:
/// `compile`, `smoke`, `icon`, `check`; `ok` when it passes), the code of what failed it (0:
/// none) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grade {
    pub pass: bool,
    pub stage: String,
    pub code: u16,
    pub why: String,
}

/// What teach asks of a verifier.
pub trait Judge {
    /// Who verifies, as 16 hex digits: what `by.verifier` records.
    fn id(&self) -> String;
    /// A task's line (`by` included): a summary of why it stands, or the refusal saying why not.
    fn verify_task(&self, line: &str) -> Result<String, String>;
    /// The grade of `program`, made for the task `task_id`, which asks `ask`.
    fn grade(&self, task_id: &str, ask: &str, program: &str) -> Grade;
}

/// The stand-in verifier: a task stands when it is well formed and its reference passes
/// [`Judge::grade`]; a program passes when the coder finds nothing wrong with it
/// ([`coder::ai::fault`]: compiles, runs clean on seeds 1 to 3, its icon draws). Its checks are
/// never run: it has no checker language.
pub struct Smoke;

impl Judge for Smoke {
    fn id(&self) -> String {
        hex16(fnv(b"teach::seam::Smoke 1: coder::ai::fault(program, \"\", 3)"))
    }

    fn verify_task(&self, line: &str) -> Result<String, String> {
        let t = Task::parse(line).ok_or("not a task line")?;
        well_formed(&t)?;
        let g = self.grade(&t.id, &t.ask, &t.reference);
        match g.pass {
            true => Ok(format!(
                "its ref runs clean ({} lines); its check is unrun",
                t.reference.lines().count()
            )),
            false => Err(format!("its ref fails at {}: {}", g.stage, g.why)),
        }
    }

    fn grade(&self, _task_id: &str, _ask: &str, program: &str) -> Grade {
        let seeds = coder::Knobs::default().seeds;
        let Some(f) = coder::ai::fault(program, "", seeds) else {
            return Grade { pass: true, stage: "ok".into(), code: 0, why: String::new() };
        };
        let stage = match (f.compiles, f.runs) {
            (false, _) => "compile",
            (true, false) => "smoke",
            (true, true) => "icon",
        };
        Grade { pass: false, stage: stage.into(), code: f.diag.code.unwrap_or(0), why: f.said }
    }
}

/// Whether `t` reads as a task should: an id of 3 to 64 of `a-z0-9-` that begins with its family
/// and `-`, a family of `a-z0-9-`, a tier from 1 to 6, and an ask, a check and a ref.
pub fn well_formed(t: &Task) -> Result<(), String> {
    let name =
        |s: &str| s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let fam = [t.family.as_str(), "-"].concat();
    let why = match () {
        _ if t.family.is_empty() || !name(&t.family) => "a family of a-z, 0-9 and -",
        _ if !(3..=64).contains(&t.id.len()) || !name(&t.id) => "an id of 3 to 64 of a-z, 0-9, -",
        _ if !t.id.starts_with(&fam) || t.id.len() == fam.len() => "an id that begins family-",
        _ if !(1..=6).contains(&t.tier) => "a tier from 1 to 6",
        _ if t.ask.trim().is_empty() || t.check.trim().is_empty() => "an ask and a check",
        _ if t.reference.trim().is_empty() => "a ref",
        _ => return Ok(()),
    };
    Err(["a task needs ", why].concat())
}

/// Who made a line, under which prompt (FNV-1a 64 of its system prompt), verified by whom
/// ([`Judge::id`]), on which day.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct By {
    pub teacher: String,
    pub prompt: String,
    pub verifier: String,
    pub day: String,
}

impl By {
    /// The object `j`'s members (missing ones empty).
    pub fn read(j: Option<&Json>) -> By {
        let s = |k: &str| j.and_then(|j| j.get(k)).and_then(Json::text).unwrap_or("").to_string();
        By { teacher: s("teacher"), prompt: s("prompt"), verifier: s("verifier"), day: s("day") }
    }

    /// Appends it as JSON, its keys in order.
    pub fn put(&self, o: &mut String) {
        let parts = [&self.teacher, &self.prompt, &self.verifier, &self.day];
        for (k, v) in
            ["{\"teacher\":", ",\"prompt\":", ",\"verifier\":", ",\"day\":"].iter().zip(parts)
        {
            *o += k;
            put(o, v);
        }
        o.push('}');
    }
}

/// A task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub tier: u8,
    pub family: String,
    pub ask: String,
    pub check: String,
    pub reference: String,
    pub by: By,
}

impl Task {
    /// A task line: an object with an id, a tier, a family and an ask (a check, a ref and `by`
    /// read when there).
    pub fn parse(line: &str) -> Option<Task> {
        let j = Json::parse(line.trim())?;
        let s = |k: &str| j.get(k).and_then(Json::text).map(String::from);
        Some(Task {
            id: s("id")?,
            tier: s("tier")?.parse().ok()?,
            family: s("family")?,
            ask: s("ask")?,
            check: s("check").unwrap_or_default(),
            reference: s("ref").unwrap_or_default(),
            by: By::read(j.get("by")),
        })
    }

    /// Its line, newline-ended, its keys in the order of the module's docs.
    pub fn line(&self) -> String {
        let mut o = String::from("{\"id\":");
        put(&mut o, &self.id);
        o += &format!(",\"tier\":{},\"family\":", self.tier);
        put(&mut o, &self.family);
        for (k, v) in
            [(",\"ask\":", &self.ask), (",\"check\":", &self.check), (",\"ref\":", &self.reference)]
        {
            o += k;
            put(&mut o, v);
        }
        o += ",\"by\":";
        self.by.put(&mut o);
        o += "}\n";
        o
    }
}

/// Whether `held` holds out the task `id` of `family`: by its family, or by its id's
/// (`family-...`), so a line that lost its family is held all the same.
pub fn is_held(held: &[String], family: &str, id: &str) -> bool {
    let by_id = |h: &str| id.strip_prefix(h).is_some_and(|rest| rest.starts_with('-'));
    held.iter().any(|h| h == family || by_id(h))
}

/// The families in a held-out list: one a line, blank lines and `#` comments aside.
pub fn held(text: &str) -> Vec<String> {
    let lines = text.lines().map(str::trim);
    lines.filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()
}
