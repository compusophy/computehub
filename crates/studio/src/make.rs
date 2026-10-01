//! Making with the AI: the prompt (with the program it changes) goes out as a chat request (at
//! most [`MAX_BODY`] bytes), the reply streams in, then is checked. A program that does not
//! compile or faults when it first renders goes back in a fresh request, with what was asked, its
//! problem (its line, a caret, the rule broken) and the program; a reply that ran out of room asks
//! for the same app shorter, and one without a program asks again: at most [`RETRIES`] times
//! (then E0906, or E0907 out of room). Never with an empty or cut-off reply as context, and never
//! asking for reasoning (asked for, it took the whole room). One that runs is saved, run and added
//! to the corpus, unless the code was edited meanwhile; what its first comment says is said for it.

use crate::{Disk, Studio};
use applang::Class;
use assistant::ai::{self, CORPUS, MAX_BODY, MAX_REPLY, RETRIES, ROOM, SHORTER};
use assistant::ai::{app_block, clip, corpus_line, failure, fault, free_path, slug};
use assistant::json::Stream;
use uiwire::{Request, Style};

/// What the example chips make.
pub(crate) const EXAMPLES: [&str; 4] =
    ["a tip calculator", "a pomodoro timer", "a habit tracker", "a dice roller"];

/// The system prompt ([`ai::system`]): applang's reference card goes between the parts.
const SYSTEM: [&str; 2] = [
    "You write apps for Studio, the app maker of compusophyOS, a desktop that runs in a browser \
     tab. Apps are written in applang:\n\n",
    "\n\napplang has no clock and no randomness: for chance, keep a seed in state and step it \
     (seed = (seed * 1103515245 + 12345) % 2147483648). Keep programs under 150 lines. Decide \
     quickly what applang can make, then write it: your reply has room for the program, not for \
     long deliberation.\n\n\
     Reply with the complete program in one fenced block whose info string is app, and nothing \
     else; after its first comment, a label naming the app. ",
];
/// Room for a whole program, steady output, and no reasoning field: the free AI then asks for
/// thinking off (asked for at a low effort, it took all 8,192 tokens and left none for the
/// program). GLM 5.3 may still think unseen, so a reply out of room asks for less.
const OPTIONS: &str = ",\"max_tokens\":8192,\"temperature\":0.3";
/// A fix's last line, and the retry after a reply without a program.
const AGAIN: &str = "Reply with the corrected complete program in one app block.";
pub(crate) const NO_BLOCK: &str =
    "Your reply held no app block. Reply with the complete program in one app block.";

/// A make in flight: its request, what was asked, the program it changes ("" for a first
/// make), the first request's message and this attempt's, which attempt it is (from 1) and the
/// reply as read so far.
#[derive(Debug)]
pub(crate) struct Make {
    pub(crate) id: u32,
    prompt: String,
    base: String,
    first: String,
    asked: String,
    attempt: u32,
    stream: Stream,
    reply: String,
    /// The reply is all in: [`Studio::verify`] checks it next.
    done: bool,
}

impl Studio {
    /// Asks the AI for the app the prompt describes, or for the one open changed as it says.
    pub(crate) fn make(&mut self) {
        let ask = self.prompt.trim().to_string();
        if ask.is_empty() || self.make.is_some() {
            return;
        }
        if let Some(why) = self.locked() {
            self.status = (Style::Error, ["Not made: ", why].concat());
            return;
        }
        let base = if self.text.trim().is_empty() { String::new() } else { self.text.clone() };
        let first = match base.is_empty() {
            true => ask.clone(),
            false => format!(
                "The program now:\n```app\n{}\n```\nChange it: {ask}\nReply with the complete \
                 new program in one app block.",
                base.trim_end()
            ),
        };
        let (asked, stream, reply) = (first.clone(), Stream::default(), String::new());
        let m =
            Make { id: 0, prompt: ask, base, first, asked, attempt: 1, stream, reply, done: false };
        self.make = Some(m);
        self.ask();
    }

    /// Sends the make's message as a new request, alone after the system prompt; one too big to
    /// send ends the make.
    fn ask(&mut self) {
        self.last_id = self.last_id.wrapping_add(1).max(1);
        let system = ai::system(SYSTEM[0], SYSTEM[1]);
        let model = self.model().to_string();
        let Some(m) = &mut self.make else { return };
        let body = ai::chat(&model, OPTIONS, &system, &[("user", m.asked.clone())]);
        if body.len() > MAX_BODY {
            self.make = None;
            let why =
                format!("Not made: too big to send to the AI ({} KiB at most)", MAX_BODY >> 10);
            self.status = (Style::Error, why);
            return;
        }
        (m.id, m.stream, m.reply, m.done) = (self.last_id, Stream::default(), String::new(), false);
        self.requests.push(Request::Ai { id: m.id, body });
        self.status = (Style::Dim, self.progress());
    }

    pub(crate) fn stop(&mut self) {
        if let Some(m) = self.make.take() {
            self.requests.push(Request::AiCancel { id: m.id });
            self.status = (Style::Dim, "Stopped.".into());
        }
    }

    pub(crate) fn data(&mut self, data: &[u8]) {
        if let Some(m) = &mut self.make {
            m.stream.feed(data, &mut m.reply, MAX_REPLY);
        }
        self.status = (Style::Dim, self.progress());
    }

    /// The request ended: a failure ends the make, else the check comes next.
    pub(crate) fn end(&mut self, status: u16, error: &str) {
        let Some(m) = &mut self.make else { return };
        m.stream.end(&mut m.reply, MAX_REPLY);
        match failure(status, error, &m.stream.error) {
            Some(failed) => (self.make, self.status) = (None, failed),
            None => (m.done, self.status) = (true, (Style::Dim, "Checking\u{2026}".into())),
        }
    }

    /// Checks a reply that is all in: a program that compiles and renders is the app now; else
    /// the next request asks for it fixed, shorter (the reply ran out of room) or at all.
    /// Whether there was a reply to check.
    pub(crate) fn verify(&mut self, disk: &mut dyn Disk) -> bool {
        let Some(mut m) = self.make.take_if(|m| m.done) else { return false };
        let src = app_block(&m.reply);
        let why = match src.map(|src| (src, fault(src))) {
            Some((src, None)) => {
                self.made(src.to_string(), &m, disk);
                return true;
            }
            Some((_, why)) => why,
            None => None,
        };
        let long = m.stream.finish == "length";
        if m.attempt > RETRIES {
            let why = match why {
                _ if long => ROOM.into(),
                Some(([_, still], p, _)) => {
                    format!("E0906 still {still} after {RETRIES} fixes: {p}")
                }
                None => "E0906 the AI replied without a program".into(),
            };
            self.status = (Style::Error, why);
            return true;
        }
        m.asked = match (why, src) {
            _ if long => [&m.first, "\n\n", SHORTER].concat(),
            (Some((_, _, account)), Some(src)) => format!(
                "You were asked: {}\n\n{account}\n\nYour program:\n```app\n{}\n```\n{AGAIN}",
                m.prompt,
                src.trim_end()
            ),
            _ => [&m.first, "\n\n", NO_BLOCK].concat(),
        };
        m.attempt += 1;
        self.make = Some(m);
        self.ask();
        true
    }

    /// `src`, which runs, is the app now: saved (a first make names the file), run, its prompt
    /// kept (and cleared from the prompt, unless another was typed) and the pair added to the
    /// corpus; what its first comment says (what it is, what it leaves out) said for it. Unless
    /// the code was edited since the make began: then the edits stay.
    fn made(&mut self, src: String, m: &Make, disk: &mut dyn Disk) {
        let blank = m.base.is_empty() && self.text.trim().is_empty();
        if !self.path.is_empty() && self.text != m.base && !blank {
            self.status = (Style::Small, "Kept your edits, not the AI's".into());
            return;
        }
        if self.path.is_empty() {
            let Some(path) = free_path(&slug(&src), |p| disk.exists(p)) else {
                self.status =
                    (Style::Error, "Not saved: every name for it in ~/apps is taken".into());
                return;
            };
            self.path = path;
        }
        let note = about(&src);
        self.replace(src);
        self.made.push(m.prompt.clone());
        self.made.drain(..self.made.len().saturating_sub(5));
        if self.prompt.trim() == m.prompt {
            self.set_prompt("");
        }
        let note = if note.is_empty() { "Ready \u{2713}" } else { note.as_str() };
        let (style, mut said) = self.save(disk, &[note, " \u{2014} saved "].concat());
        let line = corpus_line(&m.prompt, &self.text, &m.base, m.attempt, self.model());
        if let Err(e) = disk.append(CORPUS, &line) {
            said.push_str(&format!(" (not added to the corpus: {e})"));
        }
        self.status = (style, said);
    }

    /// How the make is going: asking, thinking or writing (with the chars so far), and which
    /// fix it is.
    pub(crate) fn progress(&self) -> String {
        let Some(m) = &self.make else { return String::new() };
        let (word, n) = match (m.reply.chars().count(), m.stream.thought) {
            (0, 0) => ("Asking", 0),
            (0, t) => ("Thinking", t),
            (c, _) => ("Writing", c),
        };
        let n = if n == 0 { String::new() } else { [" ", &count(n), " chars"].concat() };
        match m.attempt {
            1 => format!("{word}\u{2026}{n}"),
            a => {
                let word = word.to_ascii_lowercase();
                format!("Fixing ({} of {RETRIES}) \u{2014} {word}\u{2026}{n}", a - 1)
            }
        }
    }
}

/// What `src` says it is: its leading comments, markers and spaces trimmed, joined, at most 400
/// bytes (the "Without:" a long one ends with may be cut); "" if none.
fn about(src: &str) -> String {
    let lead = applang::highlight(src).into_iter().take_while(|(_, c)| *c == Class::Comment);
    let trim: &[char] = &['/', '*', ' ', '\t', '\r', '\n'];
    let text: Vec<&str> = lead.map(|(s, _)| src[s.start..s.end].trim_matches(trim)).collect();
    clip(&text.join(" "), 400)
}

/// `n` with its thousands apart: 12,345.
fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
