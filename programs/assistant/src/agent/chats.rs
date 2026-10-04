//! Chats: the Assistant's conversations, each with its own transcript and memory, so a request
//! carries only its own chat's past. At most [`MAX_CHATS`] are kept, the oldest going when
//! another starts. The card lists them just above its prompt, where its view stays (it follows
//! the newest line), in rows as its width holds them: with two or more, a chip each, named by its
//! first prompt cut short (the current one lit; a click switches to it); then New chat while this
//! one holds anything (an empty one switched away from goes), and Compact while its memory holds
//! two tasks or more. Compact asks the model, with no tools, to condense the chat into a note of
//! [`NOTE`] bytes at most, which then stands for the memory as the answer to [`SUM`], first while
//! later tasks come and go, until the next Compact folds it in; a failure (E0901 to E0905,
//! E0907, or E0929 for no note) leaves the memory as it was, and Compact there to ask again.
//! Compacting shows as a task does, the pill with Stop. None of it while a task is in hand.
//!
//! They are kept in [`FILE`] under the person's home, written whole after each change
//! ([`Agent::kept`]) and read at the start ([`Agent::load`]): `compusophy chats 1`, `@ <current>`,
//! then per chat `c <name>`, `m <prompt>\t<answer>` per remembered task, `t <prompt>` per turn and
//! `l <style> <text>` per line of it, each text clipped (`\`, newline, tab and CR escaped as `\\`,
//! `\n`, `\t`, `\r`), a chat's newest turns within 6 KiB. Read defensively: a line unknown or past
//! a bound is skipped, and a file past [`MAX_FILE`] bytes is not read.

use std::mem;

use uiwire::{Event, Node, Request, Style, Variant};

use super::{Agent, MAX_TURNS, MEMORY, RETRY, SEND, STOP, Turn, tokens};
use crate::ai::{ROOM, chat, clip, failure, put_clip};
use crate::calls::Calls;

/// The most chats kept, and the most bytes of a compaction's note.
pub const MAX_CHATS: usize = 8;
pub const NOTE: usize = 600;
/// The prompt a compaction's note answers in the memory.
pub const SUM: &str = "Sum up our conversation so far.";
/// Where the chats are kept, under the person's home, and the most bytes read from it.
pub const FILE: &str = "/.assistant/chats";
pub const MAX_FILE: usize = 256 << 10;
/// The buttons: New chat, Compact, and chat `i` (`CHAT + i`).
pub const NEW: u32 = 4;
pub const COMPACT: u32 = 5;
pub const CHAT: u32 = 10;
/// The first line of the file.
const HEAD: &str = "compusophy chats 1";
/// The bytes of a chat's turns kept; the most lines of a turn kept (its last); the most bytes of
/// a name, of a turn's prompt or line, and of a remembered prompt or answer.
const TURNS_KEPT: usize = 6 << 10;
const LINES: usize = 16;
const NAME: usize = 200;
const LINE: usize = 300;
const ITEM: usize = 1000;
/// What a compaction asks of the model, and the most bytes of each prompt and answer it reads.
const CONDENSE: &str = "You condense a conversation between a user and the assistant that \
operates compusophyOS, a desktop in a browser tab, for them, into the note the assistant keeps in \
its place: what the user wants, what was done and changed, names and choices, and anything left \
open. Plain text, under 500 characters, nothing before or after the note.";
const READ: usize = 1500;
/// A compaction's reply that held no note.
const NONE: &str = "E0929 the AI wrote no note; the memory is as it was";
/// The gap between chips.
const GAP: u32 = 6;

/// A chat: its name (its first prompt), and its transcript and memory while another is the
/// current one (whose are the agent's own).
#[derive(Debug, Default)]
pub(super) struct Chat {
    name: String,
    turns: Vec<Turn>,
    memory: Vec<(String, String)>,
}

impl Agent {
    /// Whether it works or compacts: the desktop shows the pill.
    pub(super) fn working(&self) -> bool {
        let task = self.task.as_ref();
        self.compacting.is_some() || task.is_some_and(|t| !matches!(t.wait, super::Wait::User))
    }

    /// Whether a task is in hand or a compaction asked for: the chats stay as they are.
    fn busy(&self) -> bool {
        self.task.is_some() || self.compacting.is_some()
    }

    /// Whether the current chat holds nothing yet.
    fn blank(&self) -> bool {
        self.turns.is_empty() && self.memory.is_empty()
    }

    /// The current chat's slot (an empty chat, the first, if there is none yet).
    fn current(&mut self) -> &mut Chat {
        if self.chat >= self.chats.len() {
            self.chat = self.chats.len();
            self.chats.push(Chat::default());
        }
        &mut self.chats[self.chat]
    }

    /// The chats' events, before the rest: the card's width, New chat, Compact, a chat picked,
    /// and while compacting its reply, its Stop, and the prompts it holds off. Whether the window
    /// changed; `None` for the rest's.
    pub(super) fn chats_event(&mut self, ev: &Event) -> Option<bool> {
        let compacting = self.compacting.as_ref().map(|c| c.0);
        let chat = |id: u32| id.checked_sub(CHAT).filter(|i| (*i as usize) < MAX_CHATS);
        Some(match ev {
            // The card's, not the pill's: its chips' rows follow it.
            Event::Resize { w, .. } => {
                if self.working() || self.width == *w {
                    return None;
                }
                self.width = *w;
                if !self.framed {
                    return None;
                }
                true
            }
            Event::Click { id: NEW } => self.new_chat(),
            Event::Click { id: COMPACT } => self.compact(),
            Event::Click { id } if chat(*id).is_some() => self.switch((id - CHAT) as usize),
            Event::Click { id: STOP } | Event::Halt if compacting.is_some() => {
                self.requests.push(Request::AiCancel { id: compacting.unwrap_or_default() });
                self.compacted(0, "cancelled")
            }
            Event::AiData { id, data } if Some(*id) == compacting => {
                if let Some(c) = &mut self.compacting {
                    c.1.feed(data);
                }
                false
            }
            Event::AiEnd { id, status, error } if Some(*id) == compacting => {
                self.compacted(*status, error)
            }
            Event::Ask { .. } | Event::Submit { .. } | Event::Click { id: SEND | RETRY }
                if compacting.is_some() =>
            {
                false
            }
            _ => return None,
        })
    }

    /// Starts an empty chat, this one kept (none if this one is empty); the oldest goes past
    /// [`MAX_CHATS`].
    fn new_chat(&mut self) -> bool {
        if self.busy() || self.blank() {
            return false;
        }
        self.stow();
        self.chats.push(Chat::default());
        if self.chats.len() > MAX_CHATS {
            self.chats.remove(0);
        }
        self.chat = self.chats.len() - 1;
        (self.retry, self.unkept) = (None, true);
        self.requests.push(Request::Focus { id: self.input_id() });
        true
    }

    /// Makes chat `i` the current one; this one goes if it is empty.
    fn switch(&mut self, mut i: usize) -> bool {
        if self.busy() || i == self.chat || i >= self.chats.len() {
            return false;
        }
        let empty = self.blank();
        self.stow();
        if empty {
            self.chats.remove(self.chat);
            i -= usize::from(i > self.chat);
        }
        self.chat = i;
        let c = self.current();
        let (turns, memory) = (mem::take(&mut c.turns), mem::take(&mut c.memory));
        (self.turns, self.memory, self.retry, self.unkept) = (turns, memory, None, true);
        true
    }

    /// Puts the current chat's transcript and memory in its slot.
    fn stow(&mut self) {
        let (turns, memory) = (mem::take(&mut self.turns), mem::take(&mut self.memory));
        let c = self.current();
        (c.turns, c.memory) = (turns, memory);
    }

    /// Asks the model for a note of the chat so far, with no tools: it shows as a task does.
    fn compact(&mut self) -> bool {
        if self.busy() || self.memory.len() < 2 {
            return false;
        }
        let mut talk = String::from("The conversation so far:\n");
        for (prompt, answer) in &self.memory {
            talk += "\nUser: ";
            put_clip(&mut talk, prompt, READ);
            talk += "\nAssistant: ";
            put_clip(&mut talk, answer, READ);
            talk += "\n";
        }
        let body = chat(self.model(), ",\"max_tokens\":1024,\"temperature\":0.2", CONDENSE, &talk);
        let id = self.next_id();
        self.retry = None;
        self.turns.push(Turn { prompt: "Compact".into(), lines: Vec::new() });
        self.turns.drain(..self.turns.len().saturating_sub(MAX_TURNS));
        self.compacting = Some((id, Calls::default()));
        self.requests.push(Request::Status { working: true });
        self.requests.push(Request::Ai { id, body });
        true
    }

    /// The compaction's reply ended with HTTP `status` and the host's `error`: its note stands
    /// for the chat's memory, or the transcript says why not, coded; then the receipt.
    fn compacted(&mut self, status: u16, error: &str) -> bool {
        let Some((_, mut c)) = self.compacting.take() else { return false };
        c.end();
        let ((i, o), note) = (c.usage.unwrap_or_default(), c.text.trim());
        let (style, line, done) = match failure(status, error, &c.error) {
            Some((0, why)) => (Style::Dim, why, ""),
            Some((_, why)) => (Style::Error, why, ""),
            None if note.is_empty() && c.finish == "length" => (Style::Error, ROOM.into(), ""),
            None if note.is_empty() => (Style::Error, NONE.into(), ""),
            None => {
                let note = clip(note, NOTE);
                self.memory = vec![(SUM.into(), note.clone())];
                (Style::Dim, note, "Compacted: ")
            }
        };
        self.note(style, line);
        self.note(Style::Small, format!("{done}{} tokens in, {} out", tokens(i), tokens(o)));
        self.requests.push(Request::Status { working: false });
        self.unkept = true;
        true
    }

    /// Keeps the memory to the last [`MEMORY`] tasks, a note first staying while it is there.
    pub(super) fn forget(&mut self) {
        let note = usize::from(self.memory.first().is_some_and(|m| m.0 == SUM));
        let over = self.memory.len().saturating_sub(MEMORY);
        self.memory.drain(note..(note + over).min(self.memory.len()));
    }

    /// A task ended: the chat is named by its first prompt if it has no name yet, and to be kept.
    pub(super) fn changed(&mut self) {
        let first = self.turns.first().map(|t| clip(&t.prompt, NAME));
        let c = self.current();
        if c.name.is_empty() {
            c.name = first.unwrap_or_default();
        }
        self.unkept = true;
    }

    /// The chats' rows above the prompt (see the module docs), none while a task is in hand.
    pub(super) fn chips(&mut self) -> Vec<Node> {
        if self.task.is_some() {
            return Vec::new();
        }
        let (blank, compact) = (self.blank(), self.memory.len() >= 2);
        self.current();
        // The card's width (the desktop's card until told) less its padding; a phone's names
        // are shorter.
        let w = if self.width == 0 { 560 } else { u32::from(self.width) };
        let (room, long) = (w.saturating_sub(40), if w < 560 { 10 } else { 16 });
        let mut all = Vec::new();
        for (i, c) in self.chats.iter().enumerate().filter(|_| self.chats.len() > 1) {
            let variant = if i == self.chat { Variant::On } else { Variant::Chip };
            let label = if c.name.is_empty() { "New chat".into() } else { short(&c.name, long) };
            all.push(Node::Button { id: CHAT + i as u32, variant, label });
        }
        let quiet =
            |id, label: &str| Node::Button { id, variant: Variant::Quiet, label: label.into() };
        all.extend((!blank).then(|| quiet(NEW, "New chat")));
        all.extend(compact.then(|| quiet(COMPACT, "Compact")));
        // Each one's width estimated a little over Inter's for text (a chip's label 12 px with 12
        // px either side, a quiet button's 14 px with 8 px), so a row stays within the card.
        let mut rows: Vec<(u32, Vec<Node>)> = Vec::new();
        for b in all {
            let wide = match &b {
                Node::Button { variant: Variant::Quiet, label, .. } => {
                    (16 + 9 * chars(label)).max(32)
                }
                Node::Button { label, .. } => 24 + 15 * chars(label) / 2,
                _ => 0,
            };
            match rows.last_mut() {
                Some((used, row)) if *used + GAP + wide <= room => {
                    *used += GAP + wide;
                    row.push(b);
                }
                _ => rows.push((wide, vec![b])),
            }
        }
        rows.into_iter()
            .map(|(_, children)| Node::Row { id: 0, gap: GAP as u8, children })
            .collect()
    }

    /// The chats as the file keeps them, if they changed since last asked.
    pub fn kept(&mut self) -> Option<String> {
        mem::take(&mut self.unkept).then(|| self.encode())
    }

    /// The chats as the file keeps them: see the module docs.
    pub(super) fn encode(&self) -> String {
        let mut out = [HEAD, "\n@ ", &self.chat.to_string(), "\n"].concat();
        let none = Chat::default();
        for i in 0..self.chats.len().max(1) {
            let c = self.chats.get(i).unwrap_or(&none);
            let (turns, memory) = match i == self.chat {
                true => (&self.turns, &self.memory),
                false => (&c.turns, &c.memory),
            };
            out += "c ";
            esc(&mut out, &clip(&c.name, NAME));
            out.push('\n');
            for (prompt, answer) in memory {
                out += "m ";
                esc(&mut out, &clip(prompt, ITEM));
                out.push('\t');
                esc(&mut out, &clip(answer, ITEM));
                out.push('\n');
            }
            // The newest turns that fit, oldest first.
            let (mut kept, mut size) = (Vec::new(), 0);
            for t in turns.iter().rev() {
                let mut s = String::from("t ");
                esc(&mut s, &clip(&t.prompt, LINE));
                let last = t.lines.get(t.lines.len().saturating_sub(LINES)..).unwrap_or_default();
                for (style, line) in last {
                    s += "\nl ";
                    s += &(*style as u8).to_string();
                    s.push(' ');
                    esc(&mut s, &clip(line, LINE));
                }
                s.push('\n');
                size += s.len();
                if size > TURNS_KEPT {
                    break;
                }
                kept.push(s);
            }
            kept.iter().rev().for_each(|s| out += s);
        }
        out
    }

    /// Puts back the chats `text` keeps (see the module docs), if it holds one; never while a
    /// task is in hand.
    pub fn load(&mut self, text: &str) {
        let mut lines = text.lines();
        if self.busy() || text.len() > MAX_FILE || lines.next() != Some(HEAD) {
            return;
        }
        let (mut chats, mut at, mut skip) = (Vec::<Chat>::new(), 0, false);
        for line in lines {
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            if kind == "c" && chats.len() == MAX_CHATS {
                break;
            }
            if kind == "c" {
                chats.push(Chat { name: clip(&unesc(rest), NAME), ..Chat::default() });
                skip = false;
                continue;
            }
            match (kind, chats.last_mut()) {
                ("@", _) => at = rest.parse().unwrap_or(0),
                ("m", Some(c)) if c.memory.len() < MEMORY => {
                    if let Some((prompt, answer)) = rest.split_once('\t') {
                        c.memory.push((unesc(prompt), unesc(answer)));
                    }
                }
                ("t", Some(c)) => {
                    skip = c.turns.len() == MAX_TURNS;
                    if !skip {
                        c.turns.push(Turn { prompt: unesc(rest), lines: Vec::new() });
                    }
                }
                ("l", Some(c)) => {
                    let (code, s) = rest.split_once(' ').unwrap_or((rest, ""));
                    let style = code.parse().ok().and_then(Style::from_u8);
                    if let (Some(t), Some(style), false) = (c.turns.last_mut(), style, skip) {
                        if t.lines.len() < LINES {
                            t.lines.push((style, unesc(s)));
                        }
                    }
                }
                _ => {}
            }
        }
        if chats.is_empty() {
            return;
        }
        let at = at.min(chats.len() - 1);
        (self.chats, self.chat) = (chats, at);
        let c = self.current();
        let (turns, memory) = (mem::take(&mut c.turns), mem::take(&mut c.memory));
        (self.turns, self.memory) = (turns, memory);
    }
}

/// Puts `text` in the file at `path`, whole: written beside it, then moved over it, its folder
/// made if missing; a write that fails leaves what was there.
pub fn keep(path: &str, text: &str) {
    let Some((dir, name)) = path.rsplit_once('/') else { return };
    _ = std::fs::create_dir(dir);
    let part = [dir, "/.", name, ".saving"].concat();
    if std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path)).is_err() {
        _ = std::fs::remove_file(&part);
    }
}

/// `s`'s first line, `n` chars at most (cut short with an ellipsis).
fn short(s: &str, n: usize) -> String {
    let s = s.lines().next().unwrap_or("").trim();
    match s.char_indices().nth(n) {
        Some((at, _)) => [s.get(..at).unwrap_or(s).trim_end(), "\u{2026}"].concat(),
        None => s.to_string(),
    }
}

/// The chars of `s`, as a width counts them.
fn chars(s: &str) -> u32 {
    s.chars().count() as u32
}

/// Appends `s` to `out` escaped: `\`, newline, tab and CR as `\\`, `\n`, `\t`, `\r`.
fn esc(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => *out += "\\\\",
            '\n' => *out += "\\n",
            '\t' => *out += "\\t",
            '\r' => *out += "\\r",
            c => out.push(c),
        }
    }
}

/// `s` unescaped (see [`esc`]); a `\` before anything else is that thing.
fn unesc(s: &str) -> String {
    let (mut out, mut it) = (String::with_capacity(s.len()), s.chars());
    while let Some(c) = it.next() {
        out.push(match (c, if c == '\\' { it.next() } else { None }) {
            (_, Some('n')) => '\n',
            (_, Some('t')) => '\t',
            (_, Some('r')) => '\r',
            (_, Some(next)) => next,
            (c, None) => c,
        });
    }
    out
}
