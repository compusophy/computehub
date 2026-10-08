//! A model a tab shares with the pool: an OpenAI-compatible server on its own device (as
//! llama-server, `http://localhost:8080`), which the person points it at and which linked tabs may
//! then ask. Sharing starts with a short question of its own ([`CHECK`]): the answer names the
//! model and says its speed, and the tab says both in its [`Msg::Hello`](crate::msg::Msg). A
//! question from a linked tab ([`Msg::Ask`](crate::msg::Msg)) goes to the server as a streamed
//! chat; what it writes goes back as it comes ([`Msg::Words`](crate::msg::Msg)), then its end
//! ([`Msg::Answered`](crate::msg::Msg)): its speed, or why it failed. One answer at a time;
//! one begun is finished even if sharing stops (only new questions are refused), and an asker
//! that hears nothing from the device answering for [`QUIET`] ms gives up, keeping what came.
//!
//! The request is `text/plain` (a CORS simple request: no preflight), and only a server on this
//! device is ever asked: a linked tab sends a question, never a URL.

use coder::json::{Json, put};
use uiwire::pool;

use crate::msg::Msg;
use crate::pool::{Act, Pool};

/// The most tokens an answer runs to, a question's most bytes, an answer's most bytes kept.
pub const MAX_TOKENS: u32 = 256;
pub const QUESTION: usize = 2000;
pub const ANSWER: usize = 16 << 10;
/// What a tab asks its own server first: a short answer names the model and times it.
pub const CHECK: &str = "Say hello in five words.";
/// Where an OpenAI-compatible server answers chats, after its URL.
pub const CHAT: &str = "/v1/chat/completions";
/// The server a tab shares unless told another.
pub const DEFAULT: &str = "http://localhost:8080";
/// ms an asker waits hearing nothing from the device answering (it says something each second).
pub const QUIET: u64 = 8000;

/// Whether `url` is a server on this device: `http://` or `https://`, then `localhost`,
/// `127.0.0.1` or `[::1]`, then a port, a path or nothing.
pub fn local(url: &str) -> bool {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"));
    let host = ["localhost", "127.0.0.1", "[::1]"];
    let after = host.iter().find_map(|h| rest?.strip_prefix(h));
    after.is_some_and(|a| a.is_empty() || a.starts_with([':', '/']))
        && url.bytes().all(|b| b.is_ascii_graphic())
}

/// A chat request for `question`, streamed, at most `max` tokens.
pub fn body(question: &str, max: u32) -> String {
    let mut out = String::from("{\"messages\":[{\"role\":\"user\",\"content\":");
    put(&mut out, question);
    out.push_str("}],\"stream\":true,\"max_tokens\":");
    crate::pool::dec(&mut out, max.into());
    out.push('}');
    out
}

/// A model's name as a server gives it: its file's, without the folder or `.gguf`, at most 64
/// bytes.
pub fn named(model: &str) -> String {
    let file = model.rsplit(['/', '\\']).next().unwrap_or(model);
    let name = file.strip_suffix(".gguf").unwrap_or(file);
    let mut end = name.len().min(64);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name.get(..end).unwrap_or("").into()
}

/// A JSON number's tenths: `9.53` is 95.
fn tenths(n: &str) -> u32 {
    let (whole, frac) = n.split_once('.').unwrap_or((n, "0"));
    let digit = frac.bytes().next().filter(u8::is_ascii_digit).map_or(0, |d| u32::from(d - b'0'));
    whole.parse::<u32>().map_or(0, |w| w.saturating_mul(10).saturating_add(digit))
}

/// An answer read from a server's stream (server-sent events) in chunks of any size: the bytes
/// after the last whole line, the text so far, the model's name and speed (tenths of a token a
/// second) once a chunk said them, the error the server gave, and lines that are not events (an
/// error's body), kept short.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    rest: Vec<u8>,
    pub text: String,
    pub model: String,
    pub tok: u32,
    pub error: String,
    other: String,
}

impl Reading {
    /// More of the body: the text it adds.
    pub fn feed(&mut self, data: &[u8]) -> String {
        self.rest.extend_from_slice(data);
        let Some(n) = self.rest.iter().rposition(|b| *b == b'\n') else { return String::new() };
        let lines: Vec<u8> = self.rest.drain(..=n).collect();
        String::from_utf8_lossy(&lines).lines().map(|l| self.line(l)).collect()
    }

    /// The body ended: its last line, then the error an error's body gave.
    pub fn end(&mut self) -> String {
        let rest = std::mem::take(&mut self.rest);
        let words = self.line(&String::from_utf8_lossy(&rest));
        let body = Json::parse(&self.other);
        let error = body.as_ref().and_then(|b| b.get("error"));
        let said = error.and_then(|e| e.get("message").or(Some(e))).and_then(Json::text);
        if let Some(m) = said.filter(|_| self.error.is_empty()) {
            self.error = m.into();
        }
        words
    }

    /// One line: an event's delta, the model's name, the timings that end a stream, an error.
    fn line(&mut self, line: &str) -> String {
        let line = line.trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            if self.other.len() + line.len() <= 4096 {
                self.other.push_str(line);
            }
            return String::new();
        };
        let Some(j) = Json::parse(data) else { return String::new() };
        if let Some(m) = j.get("model").and_then(Json::text).filter(|_| self.model.is_empty()) {
            self.model = named(m);
        }
        let speed = j.get("timings").and_then(|t| t.get("predicted_per_second"));
        if let Some(t) = speed.and_then(Json::text) {
            self.tok = tenths(t);
        }
        let error = j.get("error").and_then(|e| e.get("message").or(Some(e)));
        if let Some(e) = error.and_then(Json::text) {
            self.error = e.into();
        }
        let delta = j.get("choices").and_then(|c| c.at(0)).and_then(|c| c.get("delta"));
        let words = delta.and_then(|d| d.get("content")).and_then(Json::text).unwrap_or("");
        let room = ANSWER.saturating_sub(self.text.len());
        let mut end = words.len().min(room);
        while !words.is_char_boundary(end) {
            end -= 1;
        }
        let words = words.get(..end).unwrap_or("");
        self.text.push_str(words);
        words.into()
    }
}

/// This tab's model, if it shares one: its server's URL; the model's name and speed once its
/// server answered (empty until then, and after a failure); what sharing says (empty when it is
/// well); the answer being written, for whom (a link, 0 here) and which ask (0: the check); the
/// URL the check under way asks; and whether a check waits for the server to be free.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    pub url: String,
    pub model: String,
    pub tok: u32,
    pub note: String,
    pub writing: Option<(u32, u32, Reading)>,
    pub checking: String,
    pub due: bool,
}

/// Why an answer failed, from how its post ended (`status`, 0: no response) and what the server
/// said; empty if it did not.
pub fn why(status: u32, r: &Reading) -> String {
    match status {
        200 if r.error.is_empty() => String::new(),
        0 => "the server did not answer: is it running, and does it allow this page (CORS)?".into(),
        _ if !r.error.is_empty() => r.error.clone(),
        _ => {
            let mut s = String::from("HTTP ");
            crate::pool::dec(&mut s, status.into());
            s
        }
    }
}

/// A question this tab asked the pool's model: the link its answer comes on (0: this tab's own
/// model), its name, and the answer as Activity shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asking {
    pub link: u32,
    pub ask: u32,
    pub answer: pool::Answer,
}

impl Pool {
    /// Shares the model at `url` (a server on this device), checked first; empty stops. An
    /// answer being written still finishes, as its asker was promised: only new questions are
    /// refused, and a server named anew is checked once that answer ends.
    pub fn serve(&mut self, url: &str) {
        let url = url.trim().trim_end_matches('/');
        let (writing, checking) = (self.shared.writing.take(), self.shared.checking.clone());
        self.shared = Shared { url: url.into(), writing, checking, ..Shared::default() };
        if url.is_empty() {
            return self.said_model();
        }
        if !local(url) {
            self.shared.note = "Only a server on this device: http://localhost and its port".into();
            return self.said_model();
        }
        self.shared.note = ["Reaching ", url].concat();
        self.shared.due = true;
        self.check();
    }

    /// The check due, if the server is free.
    fn check(&mut self) {
        if self.shared.due && self.shared.writing.is_none() {
            self.shared.due = false;
            self.shared.checking.clone_from(&self.shared.url);
            self.post(0, 0, CHECK, 16);
        }
    }

    /// The model this tab shares, as its Hello says it: told to every link.
    fn said_model(&mut self) {
        (self.me.model, self.me.tok) = (self.shared.model.clone(), self.shared.tok);
        let hello = self.hello();
        self.send_all(&hello);
    }

    /// Asks the server `question` for link `link`'s `ask`.
    fn post(&mut self, link: u32, ask: u32, question: &str, max: u32) {
        self.shared.writing = Some((link, ask, Reading::default()));
        let url = [&self.shared.url, CHAT].concat();
        self.out.push(Act::Post { url, body: body(question, max) });
    }

    /// Asks the pool's model `text`: the fastest a device shares, this tab's own if none other
    /// (once its check is done, if it is being checked).
    pub fn question(&mut self, text: &str) {
        self.last_ask += 1;
        let ask = self.last_ask;
        let shares = self.peers.iter().filter(|p| !p.info.model.is_empty());
        let peer = shares.max_by_key(|p| p.info.tok).map(|p| (p.link, &p.info));
        let checking = self.shared.due || self.shared.writing.as_ref().is_some_and(|w| w.1 == 0);
        let own = Some((0, &self.me)).filter(|_| !self.shared.model.is_empty() || checking);
        let Some((link, by)) = peer.or(own) else {
            let why = "No device in the pool shares a model yet".into();
            let answer =
                pool::Answer { question: text.into(), done: true, why, ..Default::default() };
            return self.asking = Some(Asking { link: 0, ask, answer });
        };
        let answer =
            pool::Answer { question: text.into(), by: by.name.clone(), ..Default::default() };
        self.asking = Some(Asking { link, ask, answer });
        match link {
            0 if checking => {}
            0 => self.write(0, ask, text),
            _ => self.out.push(Act::Send(link, Msg::Ask { ask, text: text.into() })),
        }
    }

    /// Link `link` (0: this tab) asks this tab's model `text`: one answer at a time.
    pub fn write(&mut self, link: u32, ask: u32, text: &str) {
        let why = match &self.shared {
            s if s.model.is_empty() => "That device shares no model now",
            s if s.writing.is_some() => "Its model is busy with another answer: ask again soon",
            _ => {
                let end = (0..=text.len().min(QUESTION)).rev().find(|&i| text.is_char_boundary(i));
                let question = text.get(..end.unwrap_or(0)).unwrap_or("").to_string();
                return self.post(link, ask, &question, MAX_TOKENS);
            }
        };
        self.reply(link, ask, Reply::End(0, why.into()));
    }

    /// What this tab's model wrote for link `link`'s `ask`: to the link, or here.
    fn reply(&mut self, link: u32, ask: u32, r: Reply) {
        match (link, r) {
            (0, Reply::Words(text)) => self.words(0, ask, &text),
            (0, Reply::End(tok, why)) => self.finished(0, ask, tok, why),
            (_, Reply::Words(text)) => self.out.push(Act::Send(link, Msg::Words { ask, text })),
            (_, Reply::End(tok, why)) => {
                self.out.push(Act::Send(link, Msg::Answered { ask, tok, why }))
            }
        }
    }

    /// More of the server's answer, as it comes.
    pub fn part(&mut self, data: &[u8]) {
        let Some((link, ask, r)) = &mut self.shared.writing else { return };
        let (link, ask, words) = (*link, *ask, r.feed(data));
        if ask != 0 && !words.is_empty() {
            self.reply(link, ask, Reply::Words(words));
        }
    }

    /// The server's answer ended with `status` (0: none came).
    pub fn posted(&mut self, status: u32) {
        let Some((link, ask, mut r)) = self.shared.writing.take() else { return };
        let words = r.end();
        let why = why(status, &r);
        if ask == 0 && self.shared.checking != self.shared.url {
            // A check of a server shared no more: the one shared now is checked instead.
            return self.check();
        }
        if ask == 0 {
            // The check: the model is shared once its server answered it.
            self.shared.due = false;
            self.shared.note = match why.is_empty() {
                true => String::new(),
                false => ["Could not share ", &self.shared.url, ": ", &why].concat(),
            };
            (self.shared.model, self.shared.tok) = match why.is_empty() {
                true if r.model.is_empty() => ("a model".into(), r.tok),
                true => (r.model, r.tok),
                false => (String::new(), 0),
            };
            self.said_model();
            // A question asked here while it was checked: asked now, or told why not.
            let waiting = self.asking.as_ref().filter(|a| a.link == 0 && !a.answer.done);
            let Some((ask, question)) = waiting.map(|a| (a.ask, a.answer.question.clone())) else {
                return;
            };
            return match why.is_empty() {
                true => self.write(0, ask, &question),
                false => self.finished(0, ask, 0, why),
            };
        }
        if !words.is_empty() {
            self.reply(link, ask, Reply::Words(words));
        }
        if r.tok > 0 && why.is_empty() && !self.shared.model.is_empty() {
            self.shared.tok = r.tok;
            self.said_model();
        }
        self.reply(link, ask, Reply::End(r.tok, why));
        self.check();
    }

    /// More of the answer to this tab's `ask`, from link `link`.
    pub fn words(&mut self, link: u32, ask: u32, text: &str) {
        let Some(a) = self.asking.as_mut().filter(|a| a.link == link && a.ask == ask) else {
            return;
        };
        if !a.answer.done && a.answer.text.len() + text.len() <= ANSWER {
            a.answer.text.push_str(text);
        }
    }

    /// The answer to this tab's `ask` is whole, from link `link`.
    pub fn finished(&mut self, link: u32, ask: u32, tok: u32, why: String) {
        let this = |a: &&mut Asking| a.link == link && a.ask == ask && !a.answer.done;
        if let Some(a) = self.asking.as_mut().filter(this) {
            (a.answer.done, a.answer.tok, a.answer.why) = (true, tok, why);
        }
    }

    /// At `now`: an answer whose device said nothing for [`QUIET`] ms is given up (its tab
    /// closed or slept), keeping what came.
    pub fn quiet(&mut self, now: u64) {
        let Some(a) = self.asking.as_ref().filter(|a| a.link != 0 && !a.answer.done) else {
            return;
        };
        let (link, ask) = (a.link, a.ask);
        let last = self.peers.iter().find(|p| p.link == link).map(|p| p.last);
        if last.is_some_and(|t| now >= t + QUIET) {
            let why = "The device stopped answering: its tab closed, or slept".into();
            self.finished(link, ask, 0, why);
        }
    }

    /// Link `link` is gone: an answer coming on it never will.
    pub fn lost(&mut self, link: u32) {
        let why = "The device left before its model finished".to_string();
        if let Some(a) = self.asking.as_ref().filter(|a| a.link == link && !a.answer.done) {
            let ask = a.ask;
            self.finished(link, ask, 0, why);
        }
    }

    /// Whether an answer is being written here: snapshots go more often.
    pub fn answering(&self) -> bool {
        self.asking.as_ref().is_some_and(|a| !a.answer.done)
    }
}

/// What this tab's model wrote: more words, or its end (its speed, why it failed).
enum Reply {
    Words(String),
    End(u32, String),
}
