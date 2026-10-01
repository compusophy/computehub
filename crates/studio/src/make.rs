//! Making with the AI: the prompt (with the program it changes) goes out as a chat request (at
//! most [`MAX_BODY`] bytes), the reply streams in, then is checked; a program that does not
//! compile or faults when it first renders, or a reply without one, goes back with why at most
//! [`RETRIES`] times (then E0906), and one that runs is saved, run and added to the corpus, unless
//! the code was edited meanwhile.

use crate::{Disk, Studio};
use applang::{App, Limits};
use assistant::ai::{self, CORPUS, EXAMPLE, MAX_BODY, MAX_REPLY, RETRIES};
use assistant::ai::{app_block, corpus_line, failure, free_path, problem, slug};
use assistant::json::Stream;
use uiwire::{Request, Style};

/// What the example chips make.
pub(crate) const EXAMPLES: [&str; 4] =
    ["a tip calculator", "a pomodoro timer", "a habit tracker", "a dice roller"];

/// The system prompt, with applang's reference card between the parts and an example after.
const SYSTEM: [&str; 2] = [
    "You write apps for Studio, the app maker of compusophyOS, a desktop that runs in a browser \
     tab. Apps are written in applang:\n\n",
    "\n\napplang has no clock and no randomness: for chance, keep a seed in state and step it \
     (seed = (seed * 1103515245 + 12345) % 2147483648); for time, count with buttons.\n\n\
     Reply with the complete program in one fenced block whose info string is app, and nothing \
     else. Start it with a label naming the app. For example:\n",
];
/// Room to think, a little reasoning (it writes better code), steady output.
const OPTIONS: &str = ",\"max_tokens\":8192,\"temperature\":0.3,\"reasoning\":{\"effort\":\"low\"}";

/// A make in flight: its request, what was asked, the program it changes ("" for a first
/// make), the conversation so far, which attempt it is (from 1) and the reply as read so far.
#[derive(Debug)]
pub(crate) struct Make {
    pub(crate) id: u32,
    prompt: String,
    base: String,
    messages: Vec<(&'static str, String)>,
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
        let user = match base.is_empty() {
            true => ask.clone(),
            false => format!(
                "The program now:\n```app\n{}\n```\nChange it: {ask}\nReply with the complete \
                 new program in one app block.",
                base.trim_end()
            ),
        };
        let (messages, stream, reply) = (vec![("user", user)], Stream::default(), String::new());
        let m = Make { id: 0, prompt: ask, base, messages, attempt: 1, stream, reply, done: false };
        self.make = Some(m);
        self.ask();
    }

    /// Sends the make's conversation as a new request: a fix too big to send goes without the
    /// program it changes (the reply holds the new one); still too big, the make ends.
    fn ask(&mut self) {
        self.last_id = self.last_id.wrapping_add(1).max(1);
        let system = [&SYSTEM.join(applang::REFERENCE), EXAMPLE].concat();
        let model = self.model().to_string();
        let Some(m) = &mut self.make else { return };
        let mut body = ai::chat(&model, OPTIONS, &system, &m.messages);
        if body.len() > MAX_BODY && m.messages.len() > 1 {
            m.messages[0].1.clone_from(&m.prompt);
            body = ai::chat(&model, OPTIONS, &system, &m.messages);
        }
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

    /// Checks a reply that is all in: a program that compiles and renders is the app now; one
    /// that does not, or none, goes back to the AI with why. Whether there was a reply to check.
    pub(crate) fn verify(&mut self, disk: &mut dyn Disk) -> bool {
        let Some(mut m) = self.make.take_if(|m| m.done) else { return false };
        let why = match app_block(&m.reply).map(|src| (src, fault(src))) {
            Some((src, None)) => {
                self.made(src.to_string(), &m, disk);
                return true;
            }
            Some((_, why)) => why,
            None => None,
        };
        if m.attempt > RETRIES {
            let why = why.map_or_else(
                || "the AI replied without a program".into(),
                |([_, still], p)| format!("still {still} after {RETRIES} fixes: {p}"),
            );
            self.status = (Style::Error, ["E0906 ", &why].concat());
            return true;
        }
        let fix = match why {
            Some(([what, _], p)) => format!(
                "The program {what}: {p}. Reply with the corrected complete program in one app \
                 block."
            ),
            None => "Your reply held no app block. Reply with the complete program in one app \
                     block."
                .into(),
        };
        let reply = std::mem::take(&mut m.reply);
        m.messages.truncate(1);
        m.messages.extend([("assistant", reply), ("user", fix)]);
        m.attempt += 1;
        self.make = Some(m);
        self.ask();
        true
    }

    /// `src`, which runs, is the app now: saved (a first make names the file), run, its prompt
    /// kept (and cleared from the prompt, unless another was typed) and the pair added to the
    /// corpus. Unless the code was edited since the make began: then the edits stay.
    fn made(&mut self, src: String, m: &Make, disk: &mut dyn Disk) {
        let blank = m.base.is_empty() && self.text.trim().is_empty();
        if !self.path.is_empty() && self.text != m.base && !blank {
            let kept = "Not applied: the code was edited while it was made; Make again to apply it";
            self.status = (Style::Small, kept.into());
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
        self.replace(src);
        self.made.push(m.prompt.clone());
        self.made.drain(..self.made.len().saturating_sub(5));
        if self.prompt.trim() == m.prompt {
            self.set_prompt("");
        }
        let (style, mut said) = self.save(disk, "Ready \u{2713} \u{2014} saved ");
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
            (0, 0) => (["Asking ", self.model()].concat(), 0),
            (0, t) => ("Thinking".into(), t),
            (c, _) => ("Writing".into(), c),
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

/// What is wrong with `src`, if anything: that it does not compile, or faults when it first
/// renders (as a fix asks and as E0906 says it), and its problem.
fn fault(src: &str) -> Option<([&'static str; 2], String)> {
    match applang::compile(src).map(|p| App::new(p, Limits::default()).render()) {
        Ok(Ok(_)) => None,
        Ok(Err(d)) => Some((["faults when it first renders", "faulting"], problem(&d, src))),
        Err(d) => Some((["did not compile", "not compiling"], problem(&d, src))),
    }
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
