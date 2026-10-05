//! AI exchanges as they were, so a run replays offline, deterministically and for free. An
//! [`Exchange`] keeps a request's FNV-1a hash and what its response did to the make: not the
//! half-megabyte stream, but what the coder reads of it (the reply's text, its thinking's length,
//! how it finished, its error and usage, the lines that were not events, the last partial line),
//! when the make last heard from it, and when it ended (or that the make stopped it there, its
//! program in), with its receipt. Fed back as one chunk at that time ([`Exchange::sse`]), it
//! leaves the make exactly as the stream did: the make decides only after each chunk, from what
//! the chunks so far hold and the time, so every chunk before the last that it heard changes
//! nothing it decides. `evals/replays/<suite>/<run>.jsonl` holds a run's, a line each, in order.
//!
//! So a replay is exact for the coder that reads streams as this one does, and only for it: the
//! thinking's text is gone (its length is kept), and so are the chunks' times and what came
//! after the program. A harness that reads either, or decides inside a stream differently (its
//! time limits, its runaway guard), is approximated by a `loose` replay, never measured by it;
//! an agent that calls tools will need its exchanges kept whole.

use coder::ai::put_num;
use coder::json::{Json, Usage, put};

use crate::record::hex;
use crate::run::{Answer, At, Feed, Wire};

/// One request and what its response did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exchange {
    pub task: String,
    pub trial: u32,
    /// The request's place in its make, from 1, and the FNV-1a 64 of its body.
    pub n: u32,
    pub hash: u64,
    /// When (ms after it was sent) the make last heard from it, if ever; and when it ended,
    /// unless the make stopped it at `fed`.
    pub fed: Option<u64>,
    pub end: Option<u64>,
    /// The HTTP status (0: none) and the wire's error.
    pub status: u16,
    pub error: String,
    /// What the make read: the reply, the chars of thinking, the finish reason, the AI's own
    /// error and its usage; then the lines that were not events and the last partial line.
    pub text: String,
    pub thought: usize,
    pub finish: String,
    pub said: String,
    pub usage: Option<Usage>,
    pub other: String,
    pub rest: String,
    /// The receipt `api/ai.mjs` ended the stream with: tokens in and out, micro-dollars.
    pub receipt: Option<[u32; 3]>,
}

impl Exchange {
    /// The response as one chunk: an event holding what the make read, then the other lines,
    /// then the partial line, unended.
    pub fn sse(&self) -> Vec<u8> {
        let mut o = String::from("data: {\"choices\":[{\"delta\":{\"reasoning\":");
        put(&mut o, &"x".repeat(self.thought));
        o += ",\"content\":";
        put(&mut o, &self.text);
        o += "}";
        if !self.finish.is_empty() {
            o += ",\"finish_reason\":";
            put(&mut o, &self.finish);
        }
        o += "}]";
        if let Some(u) = self.usage {
            let nums = [u.input, u.output, u.cached, u.reasoning];
            let keys = [
                ",\"usage\":{\"prompt_tokens\":",
                ",\"completion_tokens\":",
                ",\"prompt_tokens_details\":{\"cached_tokens\":",
                "},\"completion_tokens_details\":{\"reasoning_tokens\":",
            ];
            for (k, v) in keys.iter().zip(nums) {
                o += k;
                put_num(&mut o, v.into());
            }
            o += "}}";
        }
        if !self.said.is_empty() {
            o += ",\"error\":{\"message\":";
            put(&mut o, &self.said);
            o += "}";
        }
        o += "}\n";
        if !self.other.is_empty() {
            o += &self.other;
            o.push('\n');
        }
        o += &self.rest;
        o.into_bytes()
    }

    /// Its JSON line, newline included.
    pub fn line(&self) -> String {
        let mut o = String::from("{\"task\":");
        put(&mut o, &self.task);
        let num = |o: &mut String, k: &str, v: u64| {
            *o += ",\"";
            *o += k;
            *o += "\":";
            put_num(o, v);
        };
        num(&mut o, "trial", self.trial.into());
        num(&mut o, "n", self.n.into());
        o += ",\"hash\":";
        put(&mut o, &hex(self.hash));
        if let Some(t) = self.fed {
            num(&mut o, "fed", t);
        }
        if let Some(t) = self.end {
            num(&mut o, "end", t);
        }
        num(&mut o, "status", self.status.into());
        for (k, v) in [("error", &self.error), ("text", &self.text)] {
            o += ",\"";
            o += k;
            o += "\":";
            put(&mut o, v);
        }
        num(&mut o, "thought", self.thought as u64);
        for (k, v) in [("finish", &self.finish), ("said", &self.said)] {
            o += ",\"";
            o += k;
            o += "\":";
            put(&mut o, v);
        }
        let list = |o: &mut String, k: &str, v: &[u32]| {
            *o += ",\"";
            *o += k;
            *o += "\":[";
            for (i, n) in v.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                put_num(o, (*n).into());
            }
            o.push(']');
        };
        if let Some(u) = self.usage {
            list(&mut o, "usage", &[u.input, u.cached, u.output, u.reasoning]);
        }
        for (k, v) in [("other", &self.other), ("rest", &self.rest)] {
            o += ",\"";
            o += k;
            o += "\":";
            put(&mut o, v);
        }
        if let Some(r) = self.receipt {
            list(&mut o, "receipt", &r);
        }
        o += "}\n";
        o
    }

    /// An exchange from its JSON line; `None` if it is not one.
    pub fn parse(line: &str) -> Option<Exchange> {
        let j = Json::parse(line)?;
        let s = |k: &str| j.get(k).and_then(Json::text).map(str::to_string);
        let n = |k: &str| j.get(k).and_then(Json::text).and_then(|t| t.parse::<u64>().ok());
        let list = |k: &str| -> Option<Vec<u32>> {
            let Json::Arr(a) = j.get(k)? else { return None };
            a.iter().map(|v| v.text()?.parse().ok()).collect()
        };
        let usage = list("usage").and_then(|u| match u[..] {
            [input, cached, output, reasoning] => Some(Usage { input, cached, output, reasoning }),
            _ => None,
        });
        Some(Exchange {
            task: s("task")?,
            trial: n("trial")? as u32,
            n: n("n")? as u32,
            hash: u64::from_str_radix(&s("hash")?, 16).ok()?,
            fed: n("fed"),
            end: n("end"),
            status: n("status")? as u16,
            error: s("error")?,
            text: s("text")?,
            thought: n("thought")? as usize,
            finish: s("finish")?,
            said: s("said")?,
            usage,
            other: s("other")?,
            rest: s("rest")?,
            receipt: list("receipt").and_then(|r| r.try_into().ok()),
        })
    }
}

/// A recorded run served again: each task's trial gets its exchanges in order, each checked
/// against the request's hash (unless `loose`: then served by place alone, to see roughly how a
/// changed harness runs on old answers). The first request that differs, or that has no
/// exchange, is a divergence: it is answered as unreachable, and noted.
#[derive(Debug, Default)]
pub struct Replay {
    all: Vec<Exchange>,
    pub loose: bool,
    pub diverged: Vec<String>,
}

impl Replay {
    /// The exchanges of a replay file's text; a task's trial that starts again (a resumed run)
    /// keeps only its last attempt's.
    pub fn load(text: &str) -> Replay {
        let mut all: Vec<Exchange> = Vec::new();
        for x in text.lines().filter_map(Exchange::parse) {
            if x.n == 1 {
                all.retain(|o| (&o.task, o.trial) != (&x.task, x.trial));
            }
            all.push(x);
        }
        Replay { all, ..Replay::default() }
    }

    /// The exchanges it holds.
    pub fn exchanges(&self) -> &[Exchange] {
        &self.all
    }
}

impl Wire for Replay {
    fn post(&mut self, at: &At, _: &str, on: &mut dyn FnMut(&[u8], u64) -> Feed) -> Answer {
        let found =
            self.all.iter().find(|x| (x.task.as_str(), x.trial, x.n) == (at.task, at.trial, at.n));
        let Some(x) = found.filter(|x| self.loose || x.hash == at.hash) else {
            let why = if found.is_some() { "differs from" } else { "is not in" };
            self.diverged.push(format!(
                "{} trial {} request {} {why} the recording",
                at.task, at.trial, at.n
            ));
            return Answer { status: 0, error: "replay diverged".into(), ms: 0, receipt: None };
        };
        if let Some(t) = x.fed {
            on(&x.sse(), t);
        }
        Answer {
            status: x.status,
            error: x.error.clone(),
            ms: x.end.or(x.fed).unwrap_or(0),
            receipt: x.receipt,
        }
    }
}
