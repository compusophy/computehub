//! Making with the AI: the prompt (with the program it changes) goes out as a chat request (at
//! most [`MAX_BODY`] bytes) under applang's card and its example programs, the reply streams in,
//! then is checked. A program that does not compile or faults when it runs ([`applang::smoke`]:
//! drawn, clicked, ticked, keyed, tapped, typed into; then drawn and clicked from the states the
//! app keeps, as it will really start) goes back in a fresh request, with what was asked (and the
//! program a change changes), its problem (its line, a caret, the rule broken, what led to it or
//! the states it started from) and the program; a reply cut off by the token limit before its
//! program ended asks for the same app shorter (what it holds is never run), or for the same
//! again if thinking took the room, and one without a program asks again: at most [`RETRIES`]
//! times (then E0906, saying what was tried, or E0907 out of room). Never with an empty or
//! cut-off reply as context, and never asking for reasoning (the free AI bounds it to 1,024
//! tokens, though a provider may think on). One that runs is saved, run and added to the corpus,
//! unless the code was edited meanwhile; what its first comment says is said for it.

use crate::{Disk, Studio};
use applang::Class;
use assistant::ai::{self, CORPUS, MAX_BODY, MAX_REPLY, RETRIES, ROOM, SHORTER, THOUGHT};
use assistant::ai::{clip, corpus_line, failure, fault, fenced, free_path, slug, state_path};
use assistant::json::Stream;
use uiwire::{Request, Style};

/// What the example chips make.
pub(crate) const EXAMPLES: [&str; 4] =
    ["a tip calculator", "a pomodoro timer", "a habit tracker", "a dice roller"];

/// The system prompt ([`ai::system`]): applang's reference card goes between the parts.
const SYSTEM: [&str; 2] = [
    "You write apps for Studio, the app maker of compusophyOS, a desktop that runs in a browser \
     tab. Apps are written in applang:\n\n",
    "\n\nA game remembers its world in states and lists of its own (what has landed, where \
     things are), and draws the list a grid shows anew from them; it moves with every, is \
     steered with on key and with buttons too (a phone has no arrow keys), and shows Start until \
     it runs. Saved states come back as they were kept, also after a change: a saved list keeps \
     its old length (one a change adds starts as declared), so check a list's length before \
     indexing it. Write small functions instead of repeating code; keep programs under 200 \
     lines. Decide quickly what applang can make, then \
     write it: your reply has room for the program, not for long deliberation.\n\n\
     Reply with the complete program in one fenced block whose info string is app, and nothing \
     else; after its first comment, a label naming the app. ",
];
/// Room for a whole program, steady output, and no reasoning field: the free AI then gives the
/// model a thinking budget of 1,024 tokens (asked for at a low effort, or turned off, GLM 5.3
/// took all 8,192 and left none for the program).
const OPTIONS: &str = ",\"max_tokens\":8192,\"temperature\":0.3";
/// Past this many reasoning tokens (half that room), a reply cut off ran out of room thinking:
/// some providers think on past the free AI's budget.
const THINKING: u64 = 4096;
/// A fix's last line, and the retry after a reply without a program.
const AGAIN: &str = "Reply with the corrected complete program in one app block.";
/// A change's first request ends with this line, after what was asked.
const NEW: &str = "\nReply with the complete new program in one app block.";
pub(crate) const NO_BLOCK: &str =
    "Your reply held no app block. Reply with the complete program in one app block.";

/// What a retry asks for: a fix, the same app shorter (the reply ran out of room before its
/// program ended), or a program at all (the reply held none, or thinking took its room: then
/// the same request again).
#[derive(Clone, Copy, Debug)]
enum Retry {
    Fix,
    Shorter,
    Again,
}

/// A make in flight: its request, what was asked, the program it changes ("" for a first
/// make), what was asked as the first request put it (a change's with its program), this
/// attempt's message, how many retries of each kind (by [`Retry`]) and the reply as read so far.
#[derive(Debug)]
pub(crate) struct Make {
    pub(crate) id: u32,
    prompt: String,
    base: String,
    first: String,
    asked: String,
    tried: [u32; 3],
    stream: Stream,
    reply: String,
    /// The reply is all in: [`Studio::verify`] checks it next.
    done: bool,
}

impl Make {
    /// Which attempt this is, from 1.
    fn attempt(&self) -> u32 {
        self.tried.iter().sum::<u32>() + 1
    }

    /// The message asking for `src`, which has the problem `account` tells, fixed: what was
    /// asked, a change's with the program it changes (a reply may have left parts of it out).
    fn fix(&self, account: &str, src: &str) -> String {
        let lead = if self.base.is_empty() { "You were asked: " } else { "" };
        let mine = "\n\nYour program:\n```app\n";
        [lead, &self.first, "\n\n", account, mine, src.trim_end(), "\n```\n", AGAIN].concat()
    }
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
            false => {
                format!("The program now:\n```app\n{}\n```\nChange it: {ask}", base.trim_end())
            }
        };
        let asked = [first.as_str(), if base.is_empty() { "" } else { NEW }].concat();
        let (stream, reply) = (Stream::default(), String::new());
        let (tried, done) = ([0; 3], false);
        let m = Make { id: 0, prompt: ask, base, first, asked, tried, stream, reply, done };
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

    /// More of the reply. Once its program's block closes the rest is never used (a model may
    /// go on drafting until it runs out of room), so the request stops and the check comes next.
    pub(crate) fn data(&mut self, data: &[u8]) {
        if let Some(m) = self.make.as_mut().filter(|m| !m.done) {
            m.stream.feed(data, &mut m.reply, MAX_REPLY);
            if fenced(&m.reply).is_some_and(|(_, closed)| closed) {
                m.done = true;
                self.requests.push(Request::AiCancel { id: m.id });
                self.status = (Style::Dim, "Checking\u{2026}".into());
                return;
            }
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

    /// Checks a reply that is all in: a program that compiles and runs, from the states the app
    /// keeps too, is the app now; else the next request asks for it fixed, shorter (the reply ran
    /// out of room before its program ended: what it holds is part of a program, even if that
    /// part runs), the same again (thinking took the room: most of the reply, or over half of the
    /// room by the usage) or at all. Whether there was a reply to check.
    pub(crate) fn verify(&mut self, disk: &mut dyn Disk) -> bool {
        let Some(mut m) = self.make.take_if(|m| m.done) else { return false };
        let block = fenced(&m.reply);
        let cut = m.stream.finish == "length" && !block.is_some_and(|(_, closed)| closed);
        let thought = cut && (m.stream.thought > m.reply.len() || m.stream.reasoning > THINKING);
        let src = block.filter(|_| !cut).map(|(src, _)| src);
        // What it will really start from: the states the app at this path keeps.
        let kept = match self.path.is_empty() {
            true => String::new(),
            false => disk.read(&state_path(&self.path)).unwrap_or_default(),
        };
        let why = match src.map(|src| (src, fault(src, &kept))) {
            Some((src, None)) => {
                self.made(src.to_string(), &m, disk);
                return true;
            }
            Some((_, why)) => why,
            None => None,
        };
        if m.attempt() > RETRIES {
            let why = match why {
                _ if thought => THOUGHT.into(),
                _ if cut => ROOM.into(),
                Some(([_, still], p, _)) => gave_up(still, &p, m.tried),
                None => "E0906 the AI replied without a program".into(),
            };
            self.status = (Style::Error, why);
            return true;
        }
        let (retry, asked) = match (why, src) {
            _ if thought => (Retry::Again, m.asked.clone()),
            _ if cut => (Retry::Shorter, [&m.first, "\n\n", SHORTER].concat()),
            (Some((_, _, account)), Some(src)) => (Retry::Fix, m.fix(&account, src)),
            _ => (Retry::Again, [&m.first, "\n\n", NO_BLOCK].concat()),
        };
        (m.tried[retry as usize], m.asked) = (m.tried[retry as usize] + 1, asked);
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
        self.replace(src, disk);
        self.made.push(m.prompt.clone());
        self.made.drain(..self.made.len().saturating_sub(5));
        if self.prompt.trim() == m.prompt {
            self.set_prompt("");
            // A game that takes keys has the keyboard at once, if it shows; never over a prompt
            // typed meanwhile.
            if !self.code && self.live.as_mut().is_some_and(|live| live.play().1) {
                self.requests.push(Request::Focus { id: 0 });
            }
        }
        let note = if note.is_empty() { "Ready \u{2713}" } else { note.as_str() };
        let (style, mut said) = self.save(disk, &[note, " \u{2014} saved "].concat());
        let line = corpus_line(&m.prompt, &self.text, &m.base, m.attempt(), self.model());
        if let Err(e) = disk.append(CORPUS, &line) {
            said.push_str(&format!(" (not added to the corpus: {e})"));
        }
        self.status = (style, said);
    }

    /// How the make is going: asking, thinking or writing (with the chars so far), and which
    /// retry it is.
    pub(crate) fn progress(&self) -> String {
        let Some(m) = &self.make else { return String::new() };
        let (word, n) = match (m.reply.chars().count(), m.stream.thought) {
            (0, 0) => ("Asking", 0),
            (0, t) => ("Thinking", t),
            (c, _) => ("Writing", c),
        };
        let n = if n == 0 { String::new() } else { [" ", &count(n), " chars"].concat() };
        match m.attempt() {
            1 => format!("{word}\u{2026}{n}"),
            a => {
                let word = word.to_ascii_lowercase();
                format!("Retrying ({} of {RETRIES}) \u{2014} {word}\u{2026}{n}", a - 1)
            }
        }
    }
}

/// How a make ends whose program is `still` not compiling or faulting after the retries
/// `tried` (by [`Retry`]): E0906, saying what each retry asked for.
fn gave_up(still: &str, problem: &str, [fixes, shorter, again]: [u32; 3]) -> String {
    format!(
        "E0906 still {still} after {RETRIES} retries ({fixes} for a fix, {shorter} shorter, \
         {again} for a program): {problem}"
    )
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
