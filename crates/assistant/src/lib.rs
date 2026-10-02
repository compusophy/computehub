//! The Assistant: talk with a model, and have it build applang apps. A wasm32-wasip1 GUI program
//! (`dist/bin/assistant.wasm`) on the [`uiwire`] protocol. It writes OpenAI-compatible chat
//! requests ([`Request::Ai`]) for the model the desktop names ([`Event::Config`]); the desktop
//! sends them to compusophy's free AI, which needs no key, and streams the body back as
//! [`Event::AiData`] until [`Event::AiEnd`].
//!
//! A reply holding a fenced block whose info string is `app` is compiled with [`applang`]: a
//! program that compiles is saved to `~/apps/<slug>.app` (or the first free `<slug>-<n>.app`: it
//! never replaces a file), opened in Studio, and appended to the fine-tuning corpus
//! ([`ai::CORPUS`]); one that does not compile or faults when it first renders goes back to the
//! model with its problem (its line, a caret, the rule broken), and one cut off where the reply ran
//! out of room asks for the same app shorter, at most [`RETRIES`] times (then the problem, or
//! E0907). Files go through a [`Disk`]: [`Fs`] in the program.
//! A prompt from the desktop's everything bar ([`Event::Ask`]) is sent as if typed (the draft
//! stays), after the request in flight if there is one. A request carries the newest history
//! that fits in [`ai::MAX_BODY`]. [`ai`] and [`json`] are what Studio shares with it.

#![forbid(unsafe_code)]

pub mod ai;
pub mod json;
#[cfg(test)]
mod tests;

use std::io::{self, ErrorKind, Read, Write};

use ai::{CORPUS, MAX_BODY, MAX_REPLY, ROOM, SHORTER, app_block, clip, corpus_line, failure};
pub use ai::{DEFAULT_MODEL, RETRIES};
use ai::{fault, free_path, slug};
use json::Stream;
use uiwire::client::Client;
use uiwire::{Event, Frame, Node, Request, Style, Variant};

/// The most messages (after the system prompt) a request carries.
const HISTORY: usize = 12;
/// Caps in bytes: a prompt, and the transcript a frame shows (in one 1 MiB frame).
const MAX_PROMPT: usize = 16 * 1024;
const MAX_TEXT: usize = 256 * 1024;
/// The most turns kept, and the most Text nodes one reply becomes.
const MAX_TURNS: usize = 64;
const MAX_BLOCKS: usize = 32;

const SEND: u32 = 1;
const STOP: u32 = 2;
/// The prompt Input is this plus the prompts sent: a fresh id starts it empty.
const INPUT: u32 = 100;

/// The system prompt ([`ai::system`]): applang's reference card goes between the parts.
const SYSTEM: [&str; 2] = [
    "You are the Assistant of compusophyOS, a computer that runs in one browser tab: a \
     desktop of floating windows, written in Rust and compiled to WebAssembly. Answer briefly \
     and plainly.\n\nYou can build small apps in applang, the OS's app language:\n\n",
    "\n\nWhen the user asks for an app, reply with one short sentence and one fenced block \
     whose info string is app, holding a complete program. The OS compiles it, saves it and \
     opens it in Studio; if it does not compile, you get the diagnostics back. ",
];

/// Where the program's files go.
pub trait Disk {
    /// Writes `text` to the file at `path`, replacing it or (`append`) after
    /// it, making its directory first.
    fn put(&mut self, path: &str, text: &str, append: bool) -> io::Result<()>;
    /// Whether anything is at `path`.
    fn exists(&mut self, path: &str) -> bool;
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

    fn exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).exists()
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
    /// Prompts from the everything bar, sent in turn once nothing is in flight.
    asks: Vec<String>,
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
            Event::Ask { text } if !text.trim().is_empty() => {
                self.asks.push(clip(text, MAX_PROMPT))
            }
            _ => return false,
        }
        // Sent as they are: the draft and its Input stay.
        while self.run.is_none() && !self.asks.is_empty() {
            let text = self.asks.remove(0).trim().to_string();
            self.ask(text.clone(), text, 1);
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
            self.requests.push(Request::Focus { id: self.input_id() });
        }
    }

    /// Asks the model `text` as the user, attempt `attempt` at `prompt`; a
    /// first attempt shows `prompt` as its turn's. Text too long to send is noted, not sent.
    fn ask(&mut self, text: String, prompt: String, attempt: u32) {
        self.history.push(("user", text));
        self.history.drain(..self.history.len().saturating_sub(HISTORY));
        let shown = if attempt == 1 { prompt.clone() } else { String::new() };
        self.turns.push(Turn { prompt: shown, ..Turn::default() });
        self.turns.drain(..self.turns.len().saturating_sub(MAX_TURNS));
        let Some(body) = self.body() else {
            self.history.pop();
            let why = format!("Not sent: too long for the AI ({} KiB at most)", MAX_BODY >> 10);
            return self.note((Style::Error, why));
        };
        self.last_id = self.last_id.wrapping_add(1).max(1);
        self.requests.push(Request::Ai { id: self.last_id, body });
        self.run = Some(Run { id: self.last_id, prompt, attempt, stream: Stream::default() });
    }

    /// The chat-completions request: the system prompt and the newest history that fits in
    /// [`MAX_BODY`] bytes; none if the newest message alone does not.
    fn body(&self) -> Option<String> {
        let system = ai::system(SYSTEM[0], SYSTEM[1]);
        let options = ",\"max_tokens\":4096,\"temperature\":0.3";
        let chat = |messages| ai::chat(self.model(), options, &system, messages);
        // A message takes its content quoted, its role and 23 bytes of JSON.
        let room = MAX_BODY.saturating_sub(chat(&[]).len());
        let keep = fit(&self.history, room, |m| json::quote(&m.1).len() + 32);
        let body = chat(&self.history[self.history.len() - keep..]);
        (body.len() <= MAX_BODY).then_some(body)
    }

    /// Ends the request in flight with `status` and the host's `error`: notes what went wrong,
    /// else builds the app the reply holds, if any; a reply cut off where it ran out of room
    /// (`finish_reason` length) says so, and its program is asked for again, shorter.
    fn finish(&mut self, status: u16, error: &str, disk: &mut dyn Disk) {
        let (Some(mut run), Some(turn)) = (self.run.take(), self.turns.last_mut()) else { return };
        run.stream.end(&mut turn.reply, MAX_REPLY);
        if let Some((sent, got)) = run.stream.usage.take() {
            turn.notes.push((Style::Small, format!("{sent} tokens in, {got} out")));
        }
        let reply = turn.reply.clone();
        let failed = failure(status, error, &run.stream.error);
        let long = failed.is_none() && run.stream.finish == "length";
        let empty = || (reply.is_empty() && !long).then(|| (Style::Dim, "No reply.".to_string()));
        turn.notes.extend(failed.clone().or_else(empty));
        // An unanswered prompt stays in the transcript, not in the history.
        match reply.is_empty() {
            true => _ = self.history.pop(),
            false => self.history.push(("assistant", reply.clone())),
        }
        let src = app_block(&reply).filter(|_| failed.is_none());
        let (why, text) = match src.map(|src| (src, fault(src))) {
            None => {
                return if long {
                    self.note((Style::Error, ROOM.into()))
                };
            }
            Some((src, None)) => return self.build(src, &run, disk),
            Some(_) if long => ("The program ran out of room".into(), SHORTER.into()),
            Some((_, Some(([what, _], problem, account)))) => {
                let again = "\nReply with the corrected full program in one app block.";
                (format!("The program {what}: {problem}"), [account.as_str(), again].concat())
            }
        };
        if run.attempt > RETRIES {
            return self.note((Style::Error, if long { ROOM.into() } else { why }));
        }
        let next = if long { "; asking for a shorter one." } else { "; asking for a fix." };
        self.note((Style::Dim, [&why, next].concat()));
        self.ask(text, run.prompt, run.attempt + 1);
    }

    fn note(&mut self, note: (Style, String)) {
        if let Some(t) = self.turns.last_mut() {
            t.notes.push(note);
        }
    }

    /// Saves `src`, which compiles, to a free name in `~/apps`, opens it in Studio and adds it
    /// to the corpus, noting so.
    fn build(&mut self, src: &str, run: &Run, disk: &mut dyn Disk) {
        let slug = slug(src);
        let Some(path) = free_path(&slug, |p| disk.exists(p)) else {
            let why = format!("Couldn't save {slug}.app: every name for it in ~/apps is taken");
            return self.note((Style::Error, why));
        };
        let name = path.rsplit('/').next().unwrap_or_default();
        if let Err(e) = disk.put(&path, src, false) {
            return self.note((Style::Error, format!("Couldn't save {path}: {e}")));
        }
        self.requests.push(Request::Open { name: ["studio:", &path].concat() });
        let line = corpus_line(&run.prompt, src, "", run.attempt, self.model());
        let built = format!("Built {name} \u{2713} \u{2014} open in Studio to change it");
        self.note(match disk.put(CORPUS, &line, true) {
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
        let first = [Request::Size { w: 560, h: 640 }, Request::Focus { id: self.input_id() }];
        let first = (!std::mem::replace(&mut self.framed, true)).then_some(first);
        let requests =
            first.into_iter().flatten().chain(std::mem::take(&mut self.requests)).collect();
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
