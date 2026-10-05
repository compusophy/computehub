//! The Messages API as teach speaks it: a request's body (live and in a Message Batch), and a
//! reply read as the API docs say to read one: its `stop_reason` first (a refusal's content is
//! never read), then its blocks by type (thinking blocks come first and are skipped; the text
//! blocks are the reply), and what each attempt cost by the model that ran it.
//!
//! Requests to Claude Opus 5.5: thinking adaptive (it cannot be turned off there), effort always
//! sent (its default is medium), the system prompt cached for an hour (it is the same across a
//! run, whose requests may start more than 5 minutes apart: a long turn, a batch's spread; the
//! dearer write is paid once an hour). Live requests stream (long turns would outlast an idle
//! connection) and opt into server-side refusal fallbacks in their `"default"` form, which route
//! a declined request by its refusal category; batches take neither (the API rejects
//! `fallbacks` there).

use coder::json::{Json, put};

use crate::{Fail, codes};

/// The beta header that turns on `fallbacks: "default"` (the array form takes another; mixing
/// them is a 400).
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// One request: a system prompt and one user message, as the coder sends every turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Req {
    pub system: String,
    pub user: String,
    /// Room for the thinking and the reply: thinking counts toward it.
    pub max_tokens: u32,
    /// `output_config.effort`: `low`, `medium`, `high`, `xhigh` or `max`.
    pub effort: String,
}

/// The JSON body of `req` to `model`, `live` (streamed, with fallbacks) or for a batch.
pub fn body(req: &Req, model: &str, live: bool) -> String {
    let mut o = String::from("{\"model\":");
    put(&mut o, model);
    o += ",\"max_tokens\":";
    o += &req.max_tokens.to_string();
    o += ",\"thinking\":{\"type\":\"adaptive\"},\"output_config\":{\"effort\":";
    put(&mut o, &req.effort);
    o += if live { "},\"stream\":true,\"fallbacks\":\"default\"" } else { "}" };
    o += ",\"system\":[{\"type\":\"text\",\"text\":";
    put(&mut o, &req.system);
    o += ",\"cache_control\":{\"type\":\"ephemeral\",\"ttl\":\"1h\"}}]";
    o += ",\"messages\":[{\"role\":\"user\",\"content\":";
    put(&mut o, &req.user);
    o += "}]}";
    o
}

/// A Message Batch's body: each request under its `custom_id` (1 to 64 of `A-Za-z0-9_-`).
pub fn batch_body(reqs: &[(String, Req)], model: &str) -> String {
    let mut o = String::from("{\"requests\":[");
    for (i, (id, req)) in reqs.iter().enumerate() {
        o += if i == 0 { "{\"custom_id\":" } else { ",{\"custom_id\":" };
        put(&mut o, id);
        o += ",\"params\":";
        o += &body(req, model, false);
        o.push('}');
    }
    o += "]}";
    o
}

/// The tokens of one attempt: in (uncached), out (thinking included), read from the cache, and
/// written to it for 5 minutes and for an hour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub write_5m: u64,
    pub write_1h: u64,
}

/// A reply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Msg {
    pub id: String,
    /// The model that wrote it: a fallback's, when one served.
    pub model: String,
    /// `end_turn`, `max_tokens` (cut: what came is partial), `refusal`, ...
    pub stop: String,
    /// A refusal's `stop_details.category` (`cyber`, `bio`, ...; may be empty even then).
    pub category: String,
    /// Its text blocks, joined; empty on a refusal.
    pub text: String,
    /// What each attempt cost, by the model that ran it: `usage.iterations` when the reply has
    /// them (a fallback ran: the top-level usage is the serving attempt's alone), else one.
    pub parts: Vec<(String, Usage)>,
}

impl Msg {
    /// The Messages response `j` to a request for `asked`.
    pub fn read(j: &Json, asked: &str) -> Result<Msg, Fail> {
        if j.get("type").and_then(Json::text) != Some("message") {
            let not = || Fail::new(codes::UNREADABLE, "a reply that is not a message");
            return Err(api_error(j).unwrap_or_else(not));
        }
        let s = |k: &str| j.get(k).and_then(Json::text).unwrap_or("").to_string();
        let (model, stop) = (s("model"), s("stop_reason"));
        let category = j.get("stop_details").and_then(|d| d.get("category"));
        let category = category.and_then(Json::text).unwrap_or("").to_string();
        let mut text = String::new();
        if let (false, Some(Json::Arr(blocks))) = (stop == "refusal", j.get("content")) {
            for b in blocks.iter().filter(|b| b.get("type").and_then(Json::text) == Some("text")) {
                text += b.get("text").and_then(Json::text).unwrap_or("");
            }
        }
        let u = j.get("usage");
        let parts = match u.and_then(|u| u.get("iterations")) {
            Some(Json::Arr(its)) if !its.is_empty() => {
                let by = |it: &Json| match it.get("model").and_then(Json::text) {
                    Some(m) => m.to_string(),
                    None if it.get("type").and_then(Json::text) == Some("fallback_message") => {
                        model.clone()
                    }
                    None => asked.to_string(),
                };
                its.iter().map(|it| (by(it), usage(it))).collect()
            }
            _ => {
                let by = if model.is_empty() { asked.to_string() } else { model.clone() };
                vec![(by, u.map(usage).unwrap_or_default())]
            }
        };
        Ok(Msg { id: s("id"), model, stop, category, text, parts })
    }

    /// Why the reply cannot be used whole, if so: a refusal (E0985, with its category) or a reply
    /// cut at `max_tokens` (E0986).
    pub fn short(&self) -> Option<Fail> {
        match self.stop.as_str() {
            "refusal" => Some(Fail::new(codes::REFUSED, ["declined: ", &self.category].concat())),
            "max_tokens" => Some(Fail::new(codes::CUT, "the reply ran out of max_tokens")),
            _ => None,
        }
    }
}

/// The usage object `u` (or an iteration of one). Cache writes with no breakdown by TTL are
/// counted as the hour's, the dearer, so a ledger never reads low.
fn usage(u: &Json) -> Usage {
    let n = |o: Option<&Json>, k: &str| -> u64 {
        o.and_then(|o| o.get(k)).and_then(Json::text).and_then(|t| t.parse().ok()).unwrap_or(0)
    };
    let (c, write) = (u.get("cache_creation"), n(Some(u), "cache_creation_input_tokens"));
    let write_5m = n(c, "ephemeral_5m_input_tokens");
    let write_1h = n(c, "ephemeral_1h_input_tokens").max(write.saturating_sub(write_5m));
    Usage {
        input: n(Some(u), "input_tokens"),
        output: n(Some(u), "output_tokens"),
        cache_read: n(Some(u), "cache_read_input_tokens"),
        write_5m,
        write_1h,
    }
}

/// The member `k` of the object `o`, made (null) if it is missing.
fn slot<'a>(o: &'a mut Json, k: &str) -> Option<&'a mut Json> {
    let Json::Obj(m) = o else { return None };
    let i = match m.iter().position(|(n, _)| n == k) {
        Some(i) => i,
        None => {
            m.push((k.into(), Json::Null));
            m.len() - 1
        }
    };
    Some(&mut m[i].1)
}

/// A streamed reply (its server-sent events, whole) to a request for `asked`, assembled into the
/// message it streams and read as [`Msg::read`] reads one. A stream that ends before its
/// `message_stop` is unreadable (E0984): what came is partial, and its usage unknown.
pub fn from_sse(sse: &str, asked: &str) -> Result<Msg, Fail> {
    let (mut msg, mut blocks, mut ended) = (None::<Json>, Vec::<Json>::new(), false);
    for line in sse.lines() {
        let Some(ev) = line.strip_prefix("data:").and_then(|d| Json::parse(d.trim())) else {
            continue;
        };
        match ev.get("type").and_then(Json::text).unwrap_or("") {
            "message_start" => msg = ev.get("message").cloned(),
            "content_block_start" => blocks.extend(ev.get("content_block").cloned()),
            "content_block_delta" => {
                let delta = ev
                    .get("delta")
                    .filter(|d| d.get("type").and_then(Json::text) == Some("text_delta"));
                let more = delta.and_then(|d| d.get("text")).and_then(Json::text);
                let at = ev.get("index").and_then(Json::text).and_then(|i| i.parse().ok());
                if let (Some(more), Some(b)) = (more, at.and_then(|i: usize| blocks.get_mut(i))) {
                    if let Some(Json::Str(t)) = slot(b, "text") {
                        t.push_str(more);
                    }
                }
            }
            "message_delta" => {
                let m = msg.get_or_insert_with(|| Json::Obj(Vec::new()));
                if let Some(Json::Obj(d)) = ev.get("delta") {
                    for (k, v) in d {
                        slot(m, k).into_iter().for_each(|s| *s = v.clone());
                    }
                }
                if let (Some(Json::Obj(us)), Some(mu)) = (ev.get("usage"), slot(m, "usage")) {
                    if !matches!(mu, Json::Obj(_)) {
                        *mu = Json::Obj(Vec::new());
                    }
                    for (k, v) in us {
                        slot(mu, k).into_iter().for_each(|s| *s = v.clone());
                    }
                }
            }
            "message_stop" => ended = true,
            "error" => {
                let odd = || Fail::new(codes::UNREADABLE, "the stream said error");
                return Err(api_error(&ev).unwrap_or_else(odd));
            }
            _ => {}
        }
    }
    let mut m = match (msg, ended) {
        (Some(m), true) => m,
        _ => return Err(Fail::new(codes::UNREADABLE, "the stream ended before its message did")),
    };
    slot(&mut m, "content").into_iter().for_each(|c| *c = Json::Arr(blocks.clone()));
    Msg::read(&m, asked)
}

/// The API's error in `j`, however deep its `error` members nest (a batch's errored result
/// holds the error response): busy (E0983, retried) when it is overloaded, a rate limit or the
/// API's own, else refused (E0982); with its type and message.
pub fn api_error(j: &Json) -> Option<Fail> {
    let mut e = j.get("error")?;
    while let Some(inner) = e.get("error") {
        e = inner;
    }
    let kind = e.get("type").and_then(Json::text).unwrap_or("error");
    let said = e.get("message").and_then(Json::text).unwrap_or("");
    let busy = matches!(kind, "overloaded_error" | "api_error" | "rate_limit_error");
    Some(Fail::new(if busy { codes::BUSY } else { codes::HTTP }, [kind, ": ", said].concat()))
}

/// One line of a batch's results: its `custom_id` and its reply, or why there is none (E0988:
/// errored, canceled or expired).
pub fn batch_line(line: &str, asked: &str) -> Option<(String, Result<Msg, Fail>)> {
    let j = Json::parse(line.trim())?;
    let id = j.get("custom_id").and_then(Json::text)?.to_string();
    let r = j.get("result")?;
    let got = match r.get("type").and_then(Json::text).unwrap_or("") {
        "succeeded" => match r.get("message") {
            Some(m) => Msg::read(m, asked),
            None => Err(Fail::new(codes::UNREADABLE, "a success with no message")),
        },
        "errored" => {
            let why = api_error(r).map_or_else(|| "errored".into(), |f| f.why);
            Err(Fail::new(codes::BATCH, ["the request errored: ", &why].concat()))
        }
        other => Err(Fail::new(codes::BATCH, ["the request was ", other].concat())),
    };
    Some((id, got))
}

/// Each request's reply, or why there is none, by `custom_id`.
pub type Replies = Vec<(String, Result<Msg, Fail>)>;

/// What answers the teacher's requests: [`crate::wire::Anthropic`], or a fake in tests.
pub trait Teacher {
    /// The model asked.
    fn model(&self) -> &str;
    /// One request, now.
    fn call(&mut self, req: &Req) -> Result<Msg, Fail>;
    /// Requests as one Message Batch (half price; slower), its replies in any order.
    fn batch(&mut self, reqs: &[(String, Req)]) -> Result<Replies, Fail>;
}
