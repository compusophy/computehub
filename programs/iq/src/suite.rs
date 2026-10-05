//! The files: the suite (`evals/suites/iq.jsonl`), a [`Task`] a line, and a model's answers, an
//! [`Answer`] a line. Read by `coder::json`'s strict reader; written here, members in one order
//! and strings escaped one way (`\n`, `\"`, `\\`, `\t`, `\r`, other controls as `\u00XX`, the
//! rest as it is), so a suite read and written again is the same, byte for byte:
//!
//! ```text
//! {"id":"counter","tier":1,"family":"counter","ask":"...","check":"...","ref":"...",
//!  "by":{"teacher":"hand","prompt":"","verifier":"<16 hex>","day":"2026-10-05"}}
//! ```

use coder::json::Json;

use crate::{By, Coded, Refusal, Task, codes};

/// The suite's file, from the repository's root.
pub const SUITE: &str = "evals/suites/iq.jsonl";

/// A model's answer to a task: the task's id, the model's name and its reply (whose program is
/// taken as the coder takes it: [`crate::grade_reply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub task: String,
    pub model: String,
    pub reply: String,
}

/// Appends `s` to `out` as a JSON string, escaped as the module says.
fn put(out: &mut String, s: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let n = c as usize;
                out.extend(['\\', 'u', '0', '0', char::from(HEX[n >> 4]), char::from(HEX[n & 15])]);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `t` as its line of the suite, without the line's end.
pub fn write_task(t: &Task) -> String {
    let mut o = String::from("{\"id\":");
    put(&mut o, &t.id);
    o += ",\"tier\":";
    o += &t.tier.to_string();
    for (k, v) in
        [("family", &t.family), ("ask", &t.ask), ("check", &t.check), ("ref", &t.reference)]
    {
        o += ",\"";
        o += k;
        o += "\":";
        put(&mut o, v);
    }
    o += ",\"by\":{\"teacher\":";
    put(&mut o, &t.by.teacher);
    o += ",\"prompt\":";
    put(&mut o, &t.by.prompt);
    o += ",\"verifier\":";
    put(&mut o, &t.by.verifier);
    o += ",\"day\":";
    put(&mut o, &t.by.day);
    o += "}}";
    o
}

/// The suite `tasks` as its file: a line each, each ended.
pub fn write_suite(tasks: &[Task]) -> String {
    tasks.iter().map(|t| write_task(t) + "\n").collect()
}

/// The members of the object `j` exactly as `keys` names them, each once, in any order.
fn members<'a, const N: usize>(
    j: &'a Json,
    keys: [&str; N],
    what: &str,
) -> Result<[&'a Json; N], Refusal> {
    let Json::Obj(m) = j else {
        return Err(Coded::new(codes::NOT_JSON, format!("{what} is not a JSON object")));
    };
    let bad = |why: String| Err(Coded::new(codes::MEMBER, why));
    if let Some((k, _)) = m.iter().find(|(k, _)| !keys.contains(&k.as_str())) {
        return bad(format!("{what} has `{k}`, which is none of {keys:?}"));
    }
    static NULL: Json = Json::Null;
    let mut out = [&NULL; N];
    for (i, key) in keys.iter().enumerate() {
        let mut found = m.iter().filter(|(k, _)| k == key);
        match (found.next(), found.next()) {
            (Some((_, v)), None) => out[i] = v,
            (None, _) => return bad(format!("{what} has no `{key}`")),
            (Some(_), Some(_)) => return bad(format!("{what} has `{key}` twice")),
        }
    }
    Ok(out)
}

/// The string `j`, the member `key` of `what`.
fn string(j: &Json, key: &str, what: &str) -> Result<String, Refusal> {
    match j {
        Json::Str(s) => Ok(s.clone()),
        _ => Err(Coded::new(codes::MEMBER, format!("{what}'s `{key}` is not a string"))),
    }
}

/// A task from its line of the suite. Its members are those [`write_task`] writes, each once
/// (in any order), `tier` a whole number to 255, the rest strings; its provenance reads (a
/// teacher, a verifier of 16 lowercase hex digits, a day as YYYY-MM-DD). Whether the task is
/// one to keep is [`crate::verify`]'s to say.
pub fn read_task(line: &str) -> Result<Task, Refusal> {
    let j = Json::parse(line).ok_or_else(|| Coded::new(codes::NOT_JSON, "a line is not JSON"))?;
    let keys = ["id", "tier", "family", "ask", "check", "ref", "by"];
    let [id, tier, family, ask, check, reference, by] = members(&j, keys, "a task")?;
    let id = string(id, "id", "a task")?;
    let what = ["task ", &id].concat();
    let tier = match tier {
        Json::Num(n) if !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()) => {
            n.parse::<u8>().ok()
        }
        _ => None,
    };
    let tier =
        tier.ok_or_else(|| Coded::new(codes::MEMBER, format!("{what}'s tier is not 0 to 255")))?;
    let [teacher, prompt, verifier, day] =
        members(by, ["teacher", "prompt", "verifier", "day"], &[&what, "'s by"].concat())?;
    let by = By {
        teacher: string(teacher, "teacher", &what)?,
        prompt: string(prompt, "prompt", &what)?,
        verifier: string(verifier, "verifier", &what)?,
        day: string(day, "day", &what)?,
    };
    let hex = |s: &str| {
        s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    let date = |s: &str| {
        let b = s.as_bytes();
        b.len() == 10
            && b.iter()
                .enumerate()
                .all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
    };
    let bad = |why: &str| Err(Coded::new(codes::PROVENANCE, format!("{what}: {why}")));
    if by.teacher.trim().is_empty() {
        return bad("no teacher (hand, or the model's name)");
    }
    if !hex(&by.verifier) {
        return bad("its verifier is not 16 lowercase hex digits");
    }
    if !date(&by.day) {
        return bad("its day is not YYYY-MM-DD");
    }
    Ok(Task {
        id,
        tier,
        family: string(family, "family", &what)?,
        ask: string(ask, "ask", &what)?,
        check: string(check, "check", &what)?,
        reference: string(reference, "ref", &what)?,
        by,
    })
}

/// Every line of `text`, each ended, none blank, with its number (from 1).
fn lines(text: &str) -> Result<Vec<(usize, &str)>, Refusal> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(Coded::new(codes::BLANK, format!("line {} is blank", i + 1)));
        }
        out.push((i + 1, line));
    }
    Ok(out)
}

/// The suite in `text`: a task a line ([`read_task`]), no blank lines, no id twice.
pub fn read_suite(text: &str) -> Result<Vec<Task>, Refusal> {
    let mut out: Vec<Task> = Vec::new();
    for (no, line) in lines(text)? {
        let t =
            read_task(line).map_err(|e| Coded::new(e.code, format!("line {no}: {}", e.message)))?;
        if out.iter().any(|o| o.id == t.id) {
            let why = format!("line {no}: the id {} is a task's already", t.id);
            return Err(Coded::new(codes::DUPLICATE, why));
        }
        out.push(t);
    }
    Ok(out)
}

/// A model's answers in `text`, one JSON object a line: `{"task":"id","model":"m","reply":"..."}`
/// (other members, a sampler's notes, are let be).
pub fn read_answers(text: &str) -> Result<Vec<Answer>, Refusal> {
    let mut out = Vec::new();
    for (no, line) in lines(text)? {
        let at = |code: u16, why: &str| Coded::new(code, format!("line {no}: {why}"));
        let j = Json::parse(line).ok_or_else(|| at(codes::NOT_JSON, "not JSON"))?;
        let Json::Obj(_) = j else { return Err(at(codes::NOT_JSON, "not a JSON object")) };
        let get = |k: &str| match j.get(k) {
            Some(Json::Str(s)) => Ok(s.clone()),
            _ => Err(at(codes::MEMBER, &format!("`{k}` is missing or not a string"))),
        };
        out.push(Answer { task: get("task")?, model: get("model")?, reply: get("reply")? });
    }
    Ok(out)
}
