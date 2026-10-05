//! The wire: the system's `curl`, as `tools/eval` uses it. Headers (the API key among them) go
//! to curl on its stdin as a config file (`--config -`), never on its command line, where any
//! process on the machine can read them, and never to disk; a request's body (no secret in it)
//! goes through a file in the temp directory, removed once curl is done. Busy answers (429,
//! 5xx, overloaded, nothing reached) are tried again, up to 6 times, waiting what `retry-after`
//! says or 15 s doubling to 5 minutes; any other refusal is final.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use coder::json::{Json, Stream};

use crate::claude::{self, Msg, Replies, Req, Teacher};
use crate::{Fail, codes, fnv, hex16};

/// The Anthropic API.
pub const API: &str = "https://api.anthropic.com/v1";
/// Where the key is read from: the environment, and nothing else.
pub const KEY_VAR: &str = "ANTHROPIC_API_KEY";
/// Tries after the first.
const RETRIES: u32 = 6;

/// The API key from the environment's value `v`: refused (E0980) when missing, empty, or holding
/// what no key does (so it can never break out of its config line). Never printed.
pub fn key_from(v: Option<String>) -> Result<String, Fail> {
    let k = v.unwrap_or_default();
    let ok =
        !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    match ok {
        true => Ok(k),
        false => Err(Fail::new(
            codes::NO_KEY,
            ["set ", KEY_VAR, " (read from the environment only)"].concat(),
        )),
    }
}

/// A curl config of `url` and `headers`: what curl reads on its stdin.
pub fn config(url: &str, headers: &[String]) -> String {
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut out = ["url = \"", &q(url), "\"\n"].concat();
    for h in headers {
        out += &["header = \"", &q(h), "\"\n"].concat();
    }
    out
}

/// curl's command line for `method`, at most `secs` seconds, the body from `file` if any: the
/// URL and headers come on its stdin.
pub fn args(method: &str, secs: u32, file: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> =
        ["-sS", "--config", "-", "-X", method, "--max-time"].map(String::from).into();
    a.push(secs.to_string());
    a.push("-w".into());
    a.push("%{stderr}\nstatus=%{http_code} retry=%header{retry-after}\n".into());
    if let Some(f) = file {
        a.push("--data-binary".into());
        a.push(["@", f].concat());
    }
    a
}

/// What curl answered: the HTTP status (0: none), the body, `retry-after`'s seconds (0: none).
#[derive(Debug)]
pub struct Http {
    pub status: u16,
    pub body: String,
    pub retry: u64,
}

/// Runs curl: `method`, the config `cfg` on its stdin, the body `data` if any, `secs` at most.
pub fn curl(method: &str, cfg: &str, data: Option<&str>, secs: u32) -> Result<Http, Fail> {
    let path = std::env::temp_dir().join(format!("compusophy-teach-{}.json", std::process::id()));
    let file = path.to_string_lossy().into_owned();
    if let Some(d) = data {
        std::fs::write(&path, d).map_err(|e| Fail::new(codes::FILE, format!("{file}: {e}")))?;
    }
    let mut cmd = Command::new("curl");
    cmd.args(args(method, secs, data.map(|_| file.as_str())));
    let child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let out = child.and_then(|mut c| {
        if let Some(mut stdin) = c.stdin.take() {
            stdin.write_all(cfg.as_bytes())?;
        }
        c.wait_with_output()
    });
    if data.is_some() {
        let _ = std::fs::remove_file(&path);
    }
    let out = out.map_err(|e| Fail::new(codes::CURL, format!("curl: {e}")))?;
    let err = String::from_utf8_lossy(&out.stderr);
    let tail = err.rsplit("status=").next().unwrap_or("");
    let status = tail.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let retry = tail.split("retry=").nth(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let mut body = String::from_utf8_lossy(&out.stdout).into_owned();
    if status == 0 {
        let said = err.lines().filter(|l| !l.starts_with("status=") && !l.trim().is_empty());
        body = said.collect::<Vec<_>>().join(" ");
    }
    Ok(Http { status, body, retry })
}

/// What an error body says: the API's error, else its start.
fn said(body: &str) -> String {
    let api = Json::parse(body).and_then(|j| claude::api_error(&j)).map(|f| f.why);
    api.unwrap_or_else(|| coder::ai::clip(body.trim(), 300))
}

/// Sends until an answer that is not busy, and reads it with `read` (which may say busy too).
fn send<T>(
    method: &str,
    cfg: &str,
    data: Option<&str>,
    secs: u32,
    read: impl Fn(&str) -> Result<T, Fail>,
) -> Result<T, Fail> {
    let (mut last, mut retry) = (Fail::new(codes::BUSY, ""), 0);
    for attempt in 0..=RETRIES {
        if attempt > 0 {
            let wait = if retry > 0 { retry.min(600) } else { (15 << (attempt - 1)).min(300) };
            eprintln!("  {last}; again in {wait} s");
            std::thread::sleep(Duration::from_secs(wait));
        }
        let h = curl(method, cfg, data, secs)?;
        let got = match h.status {
            200..=299 => read(&h.body),
            0 | 429 | 500..=599 => {
                Err(Fail::new(codes::BUSY, format!("HTTP {}: {}", h.status, said(&h.body))))
            }
            s => Err(Fail::new(codes::HTTP, format!("HTTP {s}: {}", said(&h.body)))),
        };
        match got {
            Err(f) if f.code == codes::BUSY => (last, retry) = (f, h.retry),
            done => return done,
        }
    }
    Err(Fail::new(codes::BUSY, format!("{} tries: {}", RETRIES + 1, last.why)))
}

/// The Anthropic API as a [`Teacher`]. It holds the key, and has no `Debug`: it never prints it.
pub struct Anthropic {
    key: String,
    model: String,
    /// Where a batch's id is kept while it runs: a run stopped mid-batch and started again
    /// finds it there and waits for it, rather than paying for the same batch twice.
    pub state: String,
    /// Seconds between polls of a batch.
    pub poll: u64,
}

impl Anthropic {
    /// The API for `model`, the key from the environment (E0980 without one).
    pub fn new(model: &str, state: &str) -> Result<Anthropic, Fail> {
        let key = key_from(std::env::var(KEY_VAR).ok())?;
        Ok(Anthropic { key, model: model.into(), state: state.into(), poll: 60 })
    }

    /// The config for `path` of the API, with the `beta` header if any.
    fn cfg(&self, path: &str, beta: &str) -> String {
        let mut h = vec![["x-api-key: ", &self.key].concat()];
        h.push("anthropic-version: 2023-06-01".into());
        h.push("content-type: application/json".into());
        if !beta.is_empty() {
            h.push(["anthropic-beta: ", beta].concat());
        }
        let url = if path.starts_with("https://") { path.into() } else { [API, path].concat() };
        config(&url, &h)
    }
}

impl Teacher for Anthropic {
    fn model(&self) -> &str {
        &self.model
    }

    fn call(&mut self, req: &Req) -> Result<Msg, Fail> {
        let cfg = self.cfg("/messages", claude::FALLBACK_BETA);
        let body = claude::body(req, &self.model, true);
        send("POST", &cfg, Some(&body), 1800, |b| claude::from_sse(b, &self.model))
    }

    /// Creates the batch (or finds it running, kept under the hash of its body in
    /// [`Anthropic::state`]), polls it until it ends, and reads its results.
    fn batch(&mut self, reqs: &[(String, Req)]) -> Result<Replies, Fail> {
        let body = claude::batch_body(reqs, &self.model);
        let key = hex16(fnv(body.as_bytes()));
        let kept = std::fs::read_to_string(&self.state).unwrap_or_default();
        let id = match kept.trim().strip_prefix(&key).map(str::trim) {
            Some(id) if !id.is_empty() => {
                eprintln!("  the batch {id} ({}) is this one: waiting for it", self.state);
                id.to_string()
            }
            _ => {
                let id = |b: &str| {
                    let j = Json::parse(b).and_then(|j| j.get("id")?.text().map(String::from));
                    j.ok_or_else(|| Fail::new(codes::UNREADABLE, "a batch with no id"))
                };
                let id = send("POST", &self.cfg("/messages/batches", ""), Some(&body), 600, id)?;
                let kept = [key.as_str(), " ", &id, "\n"].concat();
                std::fs::write(&self.state, kept)
                    .map_err(|e| Fail::new(codes::FILE, format!("{}: {e}", self.state)))?;
                eprintln!("  batch {id}: {} requests", reqs.len());
                id
            }
        };
        let json = |b: &str| Json::parse(b).ok_or_else(|| Fail::new(codes::UNREADABLE, "batch"));
        let url = loop {
            let j =
                send("GET", &self.cfg(&["/messages/batches/", &id].concat(), ""), None, 120, json)?;
            let s = |k: &str| j.get(k).and_then(Json::text).unwrap_or("").to_string();
            if s("processing_status") == "ended" {
                break s("results_url");
            }
            let n =
                |k: &str| j.get("request_counts").and_then(|c| c.get(k)?.text().map(String::from));
            let (left, done) = (n("processing"), n("succeeded"));
            eprintln!(
                "  batch {id}: {} processing, {} succeeded",
                left.unwrap_or_default(),
                done.unwrap_or_default()
            );
            std::thread::sleep(Duration::from_secs(self.poll));
        };
        let url =
            if url.is_empty() { ["/messages/batches/", &id, "/results"].concat() } else { url };
        let text = send("GET", &self.cfg(&url, ""), None, 1200, |b| Ok(b.to_string()))?;
        let _ = std::fs::remove_file(&self.state);
        Ok(text.lines().filter_map(|l| claude::batch_line(l, &self.model)).collect())
    }
}

/// What answers a chat-completions request: its reply's text, or why there is none.
pub trait Chat {
    fn chat(&mut self, body: &str) -> Result<String, Fail>;
}

/// An OpenAI-compatible endpoint (llama-server's `/v1/chat/completions`, the free AI), at
/// least `gap` ms between requests, with the `Origin` of its own URL (the free AI asks the
/// page's).
pub struct OpenAi {
    pub url: String,
    pub gap: u64,
    pub sent: bool,
}

impl Chat for OpenAi {
    fn chat(&mut self, body: &str) -> Result<String, Fail> {
        if std::mem::replace(&mut self.sent, true) {
            std::thread::sleep(Duration::from_millis(self.gap));
        }
        let at = self.url.find("://").map_or(0, |i| i + 3);
        let end = self.url[at..].find('/').map_or(self.url.len(), |i| at + i);
        let origin = ["Origin: ", &self.url[..end]].concat();
        let cfg = config(&self.url, &["content-type: application/json".into(), origin]);
        send("POST", &cfg, Some(body), 900, chat_reply)
    }
}

/// The reply in a chat-completions response: streamed (server-sent events, read as the coder
/// reads them) or whole (`choices[0].message.content`).
pub fn chat_reply(body: &str) -> Result<String, Fail> {
    let (mut s, mut text, max) = (Stream::default(), String::new(), coder::ai::MAX_REPLY);
    s.feed(body.as_bytes(), &mut text, max);
    s.end(&mut text, max);
    if !s.error.is_empty() {
        return Err(Fail::new(codes::HTTP, s.error));
    }
    let whole = || {
        let j = Json::parse(body)?;
        j.get("choices")?.at(0)?.get("message")?.get("content")?.text().map(String::from)
    };
    match (text.is_empty(), s.finish.is_empty()) {
        (false, _) | (true, false) => Ok(text),
        (true, true) => {
            whole().ok_or_else(|| Fail::new(codes::UNREADABLE, "no reply in the answer"))
        }
    }
}
