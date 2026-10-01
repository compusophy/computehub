//! The Assistant: talk with a model, and have it build applang apps. A wasm32-wasip1 GUI program
//! (`dist/bin/assistant.wasm`) on the [`uiwire`] protocol. It writes OpenAI-compatible chat
//! requests ([`Request::Ai`]) for the model the desktop names ([`Event::Config`]); the desktop
//! sends them to compusophy's free AI, which needs no key, and streams the body back as
//! [`Event::AiData`] until [`Event::AiEnd`].
//!
//! A reply holding a fenced block whose info string is `app` is compiled with [`applang`]: a
//! program that compiles is saved to `~/apps/<slug>.app`, opened, and appended to the fine-tuning
//! corpus (`~/.ai/corpus.jsonl`); one that does not goes back to the model with its diagnostics, at
//! most [`RETRIES`] times. Files go through a [`Disk`]: [`Fs`] in the program.

#![forbid(unsafe_code)]

mod json;
#[cfg(test)]
mod tests;

use std::io::{self, ErrorKind, Read, Write};

use applang::Class;
use json::{Stream, quote};
use uiwire::client::Client;
use uiwire::{Event, Frame, Node, Request, Style, Variant};

/// The guest's home, where built apps and the corpus go.
const HOME: &str = concat!("/home/", "guest");
/// The model until the desktop names one.
pub const DEFAULT_MODEL: &str = "zai/glm-5.3";
/// How many times a program that does not compile goes back to the model.
pub const RETRIES: u32 = 2;
/// The most messages (after the system prompt) a request carries.
const HISTORY: usize = 12;
/// Caps in bytes: a prompt, a reply, and both the history a request carries
/// (quoted) and the transcript a frame shows, which share one 1 MiB frame.
const MAX_PROMPT: usize = 16 * 1024;
const MAX_REPLY: usize = 64 * 1024;
const MAX_TEXT: usize = 256 * 1024;
/// The most turns kept, and the most Text nodes one reply becomes.
const MAX_TURNS: usize = 64;
const MAX_BLOCKS: usize = 32;

const SEND: u32 = 1;
const STOP: u32 = 2;
/// The prompt Input is this plus the prompts sent: a fresh id starts it empty.
const INPUT: u32 = 100;

/// The system prompt, with applang's reference card between the halves.
const SYSTEM: [&str; 2] = [
    "You are the Assistant of compusophyOS, a computer that runs in one browser tab: a \
     desktop of floating windows, written in Rust and compiled to WebAssembly. Answer briefly \
     and plainly.\n\nYou can build small apps in applang, the OS's app language:\n\n",
    "\n\nWhen the user asks for an app, reply with one short sentence and one fenced block \
     whose info string is app, holding a complete program. The OS compiles it, saves it and \
     opens it in a window; if it does not compile, you get the diagnostics back. For \
     example:\n```app\nstate count = 0;\nlabel \"Counter\";\nrow {\n  button \"-\" { count = \
     count - 1; }\n  label count;\n  button \"+\" { count = count + 1; }\n}\n```",
];

/// Where the program's files go.
pub trait Disk {
    /// Writes `text` to the file at `path`, replacing it or (`append`) after
    /// it, making its directory first.
    fn put(&mut self, path: &str, text: &str, append: bool) -> io::Result<()>;
}

/// The program's files: `std::fs`, which WASI serves from the desktop's VFS.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fs;

impl Disk for Fs {
    fn put(&mut self, path: &str, text: &str, append: bool) -> io::Result<()> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            // If this fails, the open says why.
            let _ = std::fs::create_dir_all(dir);
        }
        let mut file = std::fs::OpenOptions::new();
        file.create(true).write(true).append(append).truncate(!append);
        file.open(path)?.write_all(text.as_bytes())
    }
}

/// The Assistant's window: the transcript, the prompt and the request in flight.
#[derive(Debug, Default)]
pub struct Assistant {
    /// The model, as the desktop's last Config said.
    model: String,
    /// The prompt being typed, and how many were sent.
    input: String,
    sent: u32,
    turns: Vec<Turn>,
    /// The conversation as the model sees it: (role, content).
    history: Vec<(&'static str, String)>,
    run: Option<Run>,
    last_id: u32,
    requests: Vec<Request>,
    framed: bool,
}

/// One exchange as shown: the prompt ("" for an automatic retry), the reply and the notes under it.
#[derive(Debug, Default)]
struct Turn {
    prompt: String,
    reply: String,
    notes: Vec<(Style, String)>,
}

/// The request in flight: its id, the user's prompt it answers, which
/// attempt it is (from 1) and its body as read so far.
#[derive(Debug)]
struct Run {
    id: u32,
    prompt: String,
    attempt: u32,
    stream: Stream,
}

impl Assistant {
    /// Handles one event; whether the window changed.
    pub fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        let live = self.run.as_ref().map(|r| r.id);
        match ev {
            Event::Resize { .. } => return !self.framed,
            Event::Config { model } => self.model = model.clone(),
            Event::Change { id, text, .. } if *id == self.input_id() => {
                self.input = clip(text, MAX_PROMPT);
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Event::Change { .. } => {}
            Event::Submit { .. } | Event::Click { id: SEND } => self.send(),
            Event::Click { id: STOP } => {
                self.requests.extend(live.map(|id| Request::AiCancel { id }));
                self.finish(0, "cancelled", disk);
            }
            Event::AiData { id, data } if live == Some(*id) => {
                if let (Some(run), Some(turn)) = (&mut self.run, self.turns.last_mut()) {
                    run.stream.feed(data, &mut turn.reply, MAX_REPLY);
                }
            }
            Event::AiEnd { id, status, error } if live == Some(*id) => {
                self.finish(*status, error, disk);
            }
            _ => return false,
        }
        true
    }

    fn model(&self) -> &str {
        if self.model.is_empty() { DEFAULT_MODEL } else { &self.model }
    }

    fn input_id(&self) -> u32 {
        INPUT.wrapping_add(self.sent)
    }

    /// Sends the prompt typed, unless it is blank or a request is in flight.
    fn send(&mut self) {
        let text = self.input.trim().to_string();
        if !text.is_empty() && self.run.is_none() {
            (self.input, self.sent) = (String::new(), self.sent.wrapping_add(1));
            self.ask(text.clone(), text, 1);
        }
    }

    /// Asks the model `text` as the user, attempt `attempt` at `prompt`; a
    /// first attempt shows `prompt` as its turn's.
    fn ask(&mut self, text: String, prompt: String, attempt: u32) {
        self.last_id = self.last_id.wrapping_add(1).max(1);
        self.history.push(("user", text));
        self.history.drain(..self.history.len().saturating_sub(HISTORY));
        let shown = if attempt == 1 { prompt.clone() } else { String::new() };
        self.turns.push(Turn { prompt: shown, ..Turn::default() });
        self.turns.drain(..self.turns.len().saturating_sub(MAX_TURNS));
        self.requests.push(Request::Ai { id: self.last_id, body: self.body() });
        self.run = Some(Run { id: self.last_id, prompt, attempt, stream: Stream::default() });
    }

    /// The chat-completions request: the system prompt and the newest
    /// history that fits in [`MAX_TEXT`] bytes.
    fn body(&self) -> String {
        let keep = fit(&self.history, MAX_TEXT, |m| quote(&m.1).len());
        let message = |(role, text): &(&str, String)| {
            format!(",{{\"role\":\"{role}\",\"content\":{}}}", quote(text))
        };
        let history: String =
            self.history[self.history.len() - keep..].iter().map(message).collect();
        let (model, system) = (quote(self.model()), quote(&SYSTEM.join(applang::REFERENCE)));
        format!(
            "{{\"model\":{model},\"stream\":true,\"stream_options\":{{\"include_usage\":true}},\
             \"max_tokens\":4096,\"temperature\":0.3,\"messages\":[{{\"role\":\"system\",\
             \"content\":{system}}}{history}]}}"
        )
    }

    /// Ends the request in flight with `status` and the host's `error`: notes
    /// what went wrong, else builds the app the reply holds, if any.
    fn finish(&mut self, status: u16, error: &str, disk: &mut dyn Disk) {
        let (Some(mut run), Some(turn)) = (self.run.take(), self.turns.last_mut()) else { return };
        run.stream.end(&mut turn.reply, MAX_REPLY);
        if let Some((sent, got)) = run.stream.usage.take() {
            turn.notes.push((Style::Small, format!("{sent} tokens in, {got} out")));
        }
        let reply = turn.reply.clone();
        let failed = failure(status, error, &run.stream.error);
        let empty = || reply.is_empty().then(|| (Style::Dim, "No reply.".to_string()));
        turn.notes.extend(failed.clone().or_else(empty));
        // An unanswered prompt stays in the transcript, not in the history.
        match reply.is_empty() {
            true => _ = self.history.pop(),
            false => self.history.push(("assistant", reply.clone())),
        }
        let Some(src) = app_block(&reply).filter(|_| failed.is_none()) else { return };
        let why = match applang::compile(src) {
            Ok(_) => return self.build(src, &run, disk),
            Err(d) => ["The program did not compile: ", &problem(&d, src)].concat(),
        };
        if run.attempt > RETRIES {
            return self.note((Style::Error, why));
        }
        self.note((Style::Dim, [&why, "; asking for a fix."].concat()));
        let text = [&why, ". Reply with the corrected full program in one app block."].concat();
        self.ask(text, run.prompt, run.attempt + 1);
    }

    fn note(&mut self, note: (Style, String)) {
        if let Some(t) = self.turns.last_mut() {
            t.notes.push(note);
        }
    }

    /// Saves `src`, which compiles, to `~/apps`, opens it and adds it to the corpus, noting so.
    fn build(&mut self, src: &str, run: &Run, disk: &mut dyn Disk) {
        let name = [&slug(src), ".app"].concat();
        let path = [HOME, "/apps/", &name].concat();
        if let Err(e) = disk.put(&path, src, false) {
            return self.note((Style::Error, format!("Couldn't save {path}: {e}")));
        }
        self.requests.push(Request::Open { name: path });
        let line = format!("{{\"prompt\":{},\"program\":{},", quote(&run.prompt), quote(src));
        let line =
            format!("{line}\"attempts\":{},\"model\":{}}}\n", run.attempt, quote(self.model()));
        let built = format!("Built {name} \u{2713}");
        self.note(match disk.put(&[HOME, "/.ai/corpus.jsonl"].concat(), &line, true) {
            Ok(()) => (Style::Success, built),
            Err(e) => (Style::Success, format!("{built} (not added to the corpus: {e})")),
        });
    }

    /// The window now, with the requests since the last frame.
    pub fn frame(&mut self) -> Frame {
        let text = |style, text: &str| Node::Text { id: 0, style, text: text.into() };
        let button = |id, variant, label: &str| Node::Button { id, variant, label: label.into() };
        let head = vec![text(Style::Title, "Assistant"), text(Style::Dim, self.model())];
        let mut nodes = vec![Node::Row { id: 0, gap: 12, children: head }, Node::Separator];
        let size = |t: &Turn| t.prompt.len() + t.reply.len() + 1024;
        let first = self.turns.len() - fit(&self.turns, MAX_TEXT, size);
        for (i, t) in self.turns.iter().enumerate().skip(first) {
            if !t.prompt.is_empty() {
                nodes.extend([text(Style::Subheading, "You"), text(Style::Body, &t.prompt)]);
            }
            nodes.push(text(Style::Subheading, "Assistant"));
            if t.reply.is_empty() && self.run.is_some() && i + 1 == self.turns.len() {
                nodes.push(text(Style::Dim, "\u{2026}"));
            }
            nodes.extend(blocks(&t.reply));
            nodes.extend(t.notes.iter().map(|(style, note)| text(*style, note)));
        }
        let placeholder = "Ask anything, or describe an app to build".into();
        let input = Node::Input { id: self.input_id(), value: self.input.clone(), placeholder };
        let action = match self.run {
            Some(_) => button(STOP, Variant::Normal, "Stop"),
            None => button(SEND, Variant::Primary, "Send"),
        };
        nodes.push(Node::Row { id: 0, gap: 8, children: vec![input, action] });
        let size = Request::Size { w: 560, h: 640 };
        let size = (!std::mem::replace(&mut self.framed, true)).then_some(size);
        let requests = size.into_iter().chain(std::mem::take(&mut self.requests)).collect();
        Frame { seq: 0, title: "Assistant".into(), requests, nodes }
    }
}

/// Runs the Assistant on `ui`, a frame per event that changes it, until
/// Close or the end of the events; an event that does not decode is skipped.
pub fn serve<R: Read, W: Write>(
    ui: &mut Client<R, W>,
    assistant: &mut Assistant,
    disk: &mut dyn Disk,
) -> io::Result<()> {
    let mut seq = 0u32;
    loop {
        let ev = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => ev,
            Err(e) if e.kind() == ErrorKind::InvalidData => continue,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if assistant.event(&ev, disk) {
            ui.show(&Frame { seq, ..assistant.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

/// How many of the last `items` fit in `room` by `len`; at least one, if any.
fn fit<T>(items: &[T], mut room: usize, len: impl Fn(&T) -> usize) -> usize {
    let fits = |t: &&T| room.checked_sub(len(t)).map(|r| room = r).is_some();
    items.iter().rev().take_while(fits).count().max(1).min(items.len())
}

/// What went wrong with a request that ended with HTTP `status` (0: none
/// made), the host's `error` and the AI service's own message `said`.
fn failure(status: u16, error: &str, said: &str) -> Option<(Style, String)> {
    let (code, what) = match status {
        _ if error == "cancelled" => return Some((Style::Dim, "Stopped.".into())),
        200..=299 if error.is_empty() && said.is_empty() => return None,
        200..=299 if error.is_empty() => (5, "the AI stopped with an error"),
        401 | 403 => (1, "the AI service refused the request"),
        402 => (2, "the free AI is out of credit for now, try again later"),
        429 => (3, "the free AI is busy, try again in a minute"),
        400 | 404 => (4, "the model was not found or refused the request"),
        503 => (5, "the AI is not available right now"),
        _ if !error.is_empty() => (5, error),
        _ => (5, "couldn't reach the AI service"),
    };
    let said = if said.is_empty() { String::new() } else { [": ", &clip(said, 300)].concat() };
    Some((Style::Error, format!("E090{code} {what}{said}")))
}

/// The program in `reply`'s first fenced block whose info string is `app`
/// (to the end of the reply if the block is not closed).
fn app_block(reply: &str) -> Option<&str> {
    let (mut at, mut start) = (0, None);
    for line in reply.split_inclusive('\n') {
        let fence = line.trim().strip_prefix("```");
        match start {
            None if fence.map(str::trim) == Some("app") => start = Some(at + line.len()),
            Some(s) if fence.is_some() => return Some(&reply[s..at]),
            _ => {}
        }
        at += line.len();
    }
    start.map(|s| &reply[s..])
}

/// `reply` as Text nodes: prose as Body, each fenced block (fences dropped)
/// as Mono; past [`MAX_BLOCKS`], the rest is one node as it is.
fn blocks(reply: &str) -> Vec<Node> {
    let (mut out, mut part, mut code) = (Vec::new(), String::new(), false);
    for line in reply.split_inclusive('\n').map(Some).chain([None]) {
        let text = |l: &&str| !l.trim_start().starts_with("```") || out.len() + 1 >= MAX_BLOCKS;
        if let Some(l) = line.filter(text) {
            part.push_str(l);
            continue;
        }
        let text = part.trim_matches(['\n', '\r']);
        if !text.trim().is_empty() {
            let style = if code { Style::Mono } else { Style::Body };
            out.push(Node::Text { id: 0, style, text: text.into() });
        }
        (part, code) = (String::new(), !code);
    }
    out
}

/// A slug for `src`'s file: its first label's text, lowercase ASCII words
/// joined by `-`, at most 32 bytes; "app" if that leaves nothing.
fn slug(src: &str) -> String {
    let toks = applang::highlight(src);
    let tok = |i: usize| toks.get(i).map(|(s, c)| (&src[s.start..s.end], *c));
    let label = (0..toks.len()).find_map(|i| match (tok(i)?, tok(i + 1)?) {
        (("label", Class::Keyword), (text, Class::Str)) => Some(text),
        _ => None,
    });
    let alnum = |b: u8| if b.is_ascii_alphanumeric() { b.to_ascii_lowercase() } else { b' ' };
    let words: String = label.unwrap_or("").bytes().map(|b| char::from(alnum(b))).collect();
    let slug = words.split_whitespace().collect::<Vec<_>>().join("-");
    let slug = slug[..slug.len().min(32)].trim_end_matches('-');
    if slug.is_empty() { "app".into() } else { slug.into() }
}

/// A diagnostic as `E0101 3:5 message`: line and column (in chars) from 1.
fn problem(d: &applang::Diag, src: &str) -> String {
    let code = d.code.map_or_else(|| "error".into(), |c| format!("E{c:04}"));
    let at = |s: applang::Span| {
        let before = src.get(..s.start).unwrap_or(src);
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!(" {}:{col}", before.matches('\n').count() + 1)
    };
    format!("{code}{} {}", d.span.map(at).unwrap_or_default(), clip(&d.message, 512))
}

/// `s` cut to at most `max` bytes, on a char boundary.
fn clip(s: &str, max: usize) -> String {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    s[..end].to_string()
}
