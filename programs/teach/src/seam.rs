//! The seam to the verifier. teach talks to models and grades nothing itself: what a task line
//! must hold to stand, and what a program made for a task earns, are a [`Judge`]'s to say. `iq`
//! (the benchmark: its tasks, its checker language, `grade`, `verify`) implements it, as
//! [`Iq`].
//!
//! A task is one JSON line, its keys in this order:
//! `{"id":"snake-wrap","tier":3,"family":"snake","ask":"...","check":"...","ref":"...",
//! "by":{"teacher":"claude-opus-5-5","prompt":"<16 hex>","verifier":"<16 hex>","day":"YYYY-MM-DD"}}`.
//! Held-out families are the verifier's to choose; teach takes their list (`--held`, one family a
//! line) and never solves one nor exports one for training.

use coder::json::{Json, put};

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

/// Whether `held` holds out the task `id` of `family`: by its family, as `iq::held` splits;
/// only a line that lost its family is judged by its id (`family-...`), so `pong-ai-spin` of
/// the family `pong-ai` is never held for `pong`'s sake.
pub fn is_held(held: &[String], family: &str, id: &str) -> bool {
    let by_id = |h: &str| id.strip_prefix(h).is_some_and(|rest| rest.starts_with('-'));
    held.iter().any(|h| h == family || (family.is_empty() && by_id(h)))
}

/// The families in a held-out list: one a line, blank lines and `#` comments aside.
pub fn held(text: &str) -> Vec<String> {
    let lines = text.lines().map(str::trim);
    lines.filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()
}

/// The benchmark's verifier ([`iq`]): a task stands when [`iq::verify`] keeps it (its reference
/// passes, a null app fails its check, most of the reference's mutants die); a program earns
/// [`iq::grade`] of its task, found by id among `tasks` (a task not there grades `harness`), and
/// teaches only when its icon line draws too (stage `icon`).
pub struct Iq {
    pub tasks: Vec<iq::Task>,
}

impl Judge for Iq {
    fn id(&self) -> String {
        iq::verifier_hash()
    }

    fn verify_task(&self, line: &str) -> Result<String, String> {
        let coded = |r: iq::Refusal| format!("E{:04} {}", r.code, r.message);
        let v = iq::verify(&iq::read_task(line).map_err(coded)?).map_err(coded)?;
        Ok(format!("its check kills {} of {} mutants", v.killed, v.counted))
    }

    fn grade(&self, task_id: &str, _ask: &str, program: &str) -> Grade {
        let Some(t) = self.tasks.iter().find(|t| t.id == task_id) else {
            let why = format!("no task {task_id} in the suite");
            return Grade { pass: false, stage: "harness".into(), code: 0, why };
        };
        let g = iq::grade(t, program);
        // What it teaches must also draw its icon, as Studio installs a make; iq grades apps without.
        if let Some(f) = coder::ai::fault(program, "", 3).filter(|_| g.pass()) {
            return Grade {
                pass: false,
                stage: "icon".into(),
                code: f.diag.code.unwrap_or(0),
                why: f.said,
            };
        }
        let stage = if g.pass() { "ok" } else { g.stage.name() };
        Grade { pass: g.pass(), stage: stage.into(), code: g.code, why: g.message }
    }
}
