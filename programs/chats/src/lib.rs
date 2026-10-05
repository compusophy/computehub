//! The Assistant's chats: its conversations, each with its own transcript and memory, so a
//! request carries only its own chat's past. Pure: the Assistant asks the model and keeps the
//! file.
//!
//! - **Order.** At most [`MAX`], the current one first, then the others as they were last
//!   current. Its transcript and memory are the Assistant's own while it is current. A new chat
//!   past [`MAX`] lets the one left longest ago go, never the one just left; an empty one
//!   switched away from goes.
//! - **Memory.** A chat's last tasks as (prompt, answer), each text clipped as the file keeps it
//!   ([`remember`]), so a chat's memory never outgrows a request. A compaction's note has no
//!   prompt (no task's is empty): it stays first, one of them, until the next folds it in
//!   ([`forget`]).
//! - **Shown.** One row over the card's prompt, within its width ([`Chats::row`]): the current
//!   chat's chip, lit, then the others' that fit, "N more" for the rest, New chat while the
//!   current one holds anything, and Compact while its memory is past [`LEAN`] bytes, more than
//!   a note takes. A chip switches to its chat; the lit one, or "N more", shows them all as a
//!   list over the row (the current one last, nearest it) with Delete chat, asked again
//!   ("Delete for good").
//! - **Kept.** [`Chats::encode`] and [`Chats::load`]: `compusophy chats 1`, then per chat, the
//!   current one first, `c <name>`, `m <prompt>\t<answer>` per remembered task, `t <prompt>` per
//!   turn and `l <style> <text>` per line of it; each text clipped (an ellipsis where cut), and
//!   `\`, newline, tab and CR escaped as `\\`, `\n`, `\t`, `\r`; a chat's newest turns within
//!   6 KiB (the newest one always, with its last lines that fit). Read defensively: each text
//!   clipped as it was kept, a line unknown or past a bound skipped, and a file past
//!   [`MAX_FILE`] bytes not read.
//! - **Answers.** Whether the person's answer to a question that waits on their yes is one
//!   ([`yes`]): yes words alone, never a question, never "I'm good". And the model's as the card
//!   shows them, plain text ([`plain`]): the card prints markdown as it is. Whether a prompt asks
//!   to be told something ([`asks`]), so its answer is the news the card keeps showing.

#![forbid(unsafe_code)]

use std::mem;

use uiwire::{Node, Style, Variant};

/// The most chats kept, and the most bytes of the file read.
pub const MAX: usize = 8;
pub const MAX_FILE: usize = 256 << 10;
/// Past this many bytes a chat's memory is worth compacting: more than a note takes.
pub const LEAN: usize = 1 << 10;
/// The buttons: New chat, Compact, "N more", Delete chat; chat `n`'s chip, and its item in the
/// list.
pub const NEW: u32 = 4;
pub const COMPACT: u32 = 5;
pub const MORE: u32 = 6;
pub const DELETE: u32 = 7;
pub const CHAT: u32 = 1 << 20;
pub const ITEM: u32 = 2 << 20;
/// The first line of the file.
const HEAD: &str = "compusophy chats 1";
/// The bytes of a chat's turns kept; the most lines of a turn kept (its last); the most bytes of
/// a name, of a turn's prompt or line, and of a remembered prompt or answer.
const TURNS_KEPT: usize = 6 << 10;
const LINES: usize = 16;
const NAME: usize = 200;
const LINE: usize = 300;
const TASK: usize = 1000;
/// The gap between the row's buttons.
const GAP: u32 = 6;

/// A task as the transcript shows it: its prompt, then what it did, asked and answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    pub prompt: String,
    pub lines: Vec<(Style, String)>,
}

/// A chat's memory: its last tasks as (prompt, answer).
pub type Memory = Vec<(String, String)>;

/// What a click did: the list shown or hidden, or Delete asked; a new chat started; or another
/// chat made the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clicked {
    Shown,
    New,
    Switched,
}

/// A chat: the number its buttons carry, its name (its first prompt), and its transcript and
/// memory while another is the current one.
#[derive(Debug, Default)]
struct Chat {
    n: u32,
    name: String,
    turns: Vec<Turn>,
    memory: Memory,
}

/// The chats (see the crate docs), the current one first; the next one's number; whether they
/// show as a list, and whether Delete chat was asked.
#[derive(Debug)]
pub struct Chats {
    list: Vec<Chat>,
    next: u32,
    listing: bool,
    asked: bool,
}

impl Default for Chats {
    /// One chat, empty.
    fn default() -> Chats {
        Chats { list: vec![Chat::default()], next: 1, listing: false, asked: false }
    }
}

impl Chats {
    /// Names the current chat `prompt` (its first) unless it has a name.
    pub fn name(&mut self, prompt: &str) {
        if let Some(c) = self.list.first_mut().filter(|c| c.name.is_empty()) {
            c.name = clip(prompt, NAME).to_string();
        }
    }

    /// Hides the list.
    pub fn close(&mut self) {
        (self.listing, self.asked) = (false, false);
    }

    /// A click on `id`, `turns` and `memory` being the current chat's (the Assistant's own): what
    /// it did, if it was the chats'. They are then those of the chat current now.
    pub fn click(
        &mut self,
        id: u32,
        turns: &mut Vec<Turn>,
        memory: &mut Memory,
    ) -> Option<Clicked> {
        let (blank, asked) = (turns.is_empty() && memory.is_empty(), mem::take(&mut self.asked));
        let mut i = match id {
            NEW if !blank => {
                self.stow(turns, memory);
                self.list.insert(0, Chat { n: self.next, ..Chat::default() });
                self.list.truncate(MAX);
                self.next = self.next.wrapping_add(1);
                self.close();
                return Some(Clicked::New);
            }
            MORE => {
                self.listing = !self.listing;
                return Some(Clicked::Shown);
            }
            DELETE if self.listing && !blank && !asked => {
                self.asked = true;
                return Some(Clicked::Shown);
            }
            // The current one goes: the one left last is current, or a new one.
            DELETE if self.listing && !blank => {
                if !self.list.is_empty() {
                    self.list.remove(0);
                }
                if self.list.is_empty() {
                    self.list.push(Chat { n: self.next, ..Chat::default() });
                    self.next = self.next.wrapping_add(1);
                }
                self.take(turns, memory);
                self.close();
                return Some(Clicked::Switched);
            }
            _ => {
                let at = |c: &Chat| [CHAT, ITEM].map(|b| b.wrapping_add(c.n)).contains(&id);
                self.list.iter().position(at)?
            }
        };
        // The current one's chip or item shows the list, or hides it.
        if i == 0 {
            self.listing = !self.listing;
            return Some(Clicked::Shown);
        }
        self.stow(turns, memory);
        if blank {
            self.list.remove(0);
            i -= 1;
        }
        if let Some(head) = self.list.get_mut(..=i) {
            head.rotate_right(1);
        }
        self.take(turns, memory);
        self.close();
        Some(Clicked::Switched)
    }

    /// Puts the current chat's `turns` and `memory` in its place.
    fn stow(&mut self, turns: &mut Vec<Turn>, memory: &mut Memory) {
        if let Some(c) = self.list.first_mut() {
            (c.turns, c.memory) = (mem::take(turns), mem::take(memory));
        }
    }

    /// Takes the current chat's transcript and memory from its place into `turns` and `memory`.
    fn take(&mut self, turns: &mut Vec<Turn>, memory: &mut Memory) {
        if let Some(c) = self.list.first_mut() {
            (*turns, *memory) = (mem::take(&mut c.turns), mem::take(&mut c.memory));
        }
    }

    /// The chats over the card's prompt, `turns` and `memory` being the current one's and `w`
    /// the card's width (0: not told yet, the desktop's card's): the list while it shows, then
    /// the row (see the crate docs); nothing while the one chat there is holds nothing.
    pub fn row(&self, w: u16, turns: &[Turn], memory: &Memory) -> Vec<Node> {
        let blank = turns.is_empty() && memory.is_empty();
        let Some(current) = self.list.first().filter(|_| !blank || self.list.len() > 1) else {
            return Vec::new();
        };
        let w = if w == 0 { 560 } else { u32::from(w) };
        // The card's padding off, and a phone's names shorter.
        let (room, long) = (w.saturating_sub(40) + GAP, if w < 560 { 10 } else { 16 });
        let button = |id, variant, label: String| Node::Button { id, variant, label };
        let chip = |c: &Chat, variant| button(CHAT.wrapping_add(c.n), variant, named(c, long));
        let (mut out, mut row, mut end) = (Vec::new(), vec![chip(current, Variant::On)], vec![]);
        if self.listing {
            for (i, c) in self.list.iter().enumerate().rev() {
                let n = if i == 0 { turns.len() } else { c.turns.len() };
                let detail = if n == 1 { "1 task".into() } else { format!("{n} tasks") };
                let (id, text) = (ITEM.wrapping_add(c.n), named(c, 60));
                out.push(Node::Item { id, text, detail, selected: i == 0 });
            }
            match self.asked {
                _ if blank => {}
                false => end.push(button(DELETE, Variant::Quiet, "Delete chat".into())),
                true => end.push(button(DELETE, Variant::Danger, "Delete for good".into())),
            }
        } else {
            end.extend((!blank).then(|| button(NEW, Variant::Quiet, "New chat".into())));
            let compact = compactable(memory);
            end.extend(compact.then(|| button(COMPACT, Variant::Quiet, "Compact".into())));
        }
        // The others, the one left last first, while they fit with "N more" for the rest.
        let others = if self.listing { 0 } else { self.list.len().saturating_sub(1) };
        let more = |n: usize| button(MORE, Variant::Chip, format!("{n} more"));
        let mut used: u32 = row.iter().chain(&end).map(wide).sum();
        for (k, c) in self.list.iter().skip(1).take(others).enumerate() {
            let (b, left) = (chip(c, Variant::Chip), others - k - 1);
            let need = wide(&b) + if left > 0 { wide(&more(left)) } else { 0 };
            if used + need > room {
                break;
            }
            used += wide(&b);
            row.push(b);
        }
        let left = others + 1 - row.len();
        if left > 0 && used + wide(&more(left)) <= room {
            row.push(more(left));
        }
        row.extend(end);
        out.push(Node::Row { id: 0, gap: GAP as u8, children: row });
        out
    }

    /// The chats as the file keeps them, `turns` and `memory` being the current one's.
    pub fn encode(&self, turns: &[Turn], memory: &Memory) -> String {
        let mut out = [HEAD, "\n"].concat();
        for (i, c) in self.list.iter().enumerate() {
            let (turns, memory) = if i == 0 { (turns, memory) } else { (&c.turns[..], &c.memory) };
            out += "c ";
            esc(&mut out, clip(&c.name, NAME));
            out.push('\n');
            for (prompt, answer) in memory {
                out += "m ";
                esc(&mut out, clip(prompt, TASK));
                out.push('\t');
                esc(&mut out, clip(answer, TASK));
                out.push('\n');
            }
            // The newest turns that fit, oldest first; each its last lines that fit alone, so
            // the newest is never lost to its size.
            let (mut kept, mut size) = (Vec::new(), 0);
            for t in turns.iter().rev() {
                let mut s = String::from("t ");
                esc(&mut s, &fit(&t.prompt, LINE));
                s.push('\n');
                let (mut last, mut n) = (Vec::new(), s.len());
                for (style, line) in t.lines.iter().rev().take(LINES) {
                    let mut l = ["l ", &(*style as u8).to_string(), " "].concat();
                    esc(&mut l, &fit(line, LINE));
                    l.push('\n');
                    n += l.len();
                    if n > TURNS_KEPT {
                        break;
                    }
                    last.push(l);
                }
                last.iter().rev().for_each(|l| s += l);
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

    /// The chats `text` keeps (see the crate docs), if it holds one, with the current one's
    /// transcript and memory; at most `most` turns and tasks a chat.
    pub fn load(text: &str, most: (usize, usize)) -> Option<(Chats, Vec<Turn>, Memory)> {
        let mut lines = text.lines();
        if text.len() > MAX_FILE || lines.next() != Some(HEAD) {
            return None;
        }
        let (mut list, mut skip) = (Vec::<Chat>::new(), false);
        for line in lines {
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            if kind == "c" && list.len() == MAX {
                break;
            }
            if kind == "c" {
                let (n, name) = (list.len() as u32, back(rest, NAME));
                list.push(Chat { n, name, ..Chat::default() });
                skip = false;
                continue;
            }
            let Some(c) = list.last_mut() else { continue };
            match kind {
                "m" if c.memory.len() < most.1 => {
                    if let Some((prompt, answer)) = rest.split_once('\t') {
                        c.memory.push((back(prompt, TASK), back(answer, TASK)));
                    }
                }
                "t" => {
                    skip = c.turns.len() == most.0;
                    if !skip {
                        c.turns.push(Turn { prompt: back(rest, LINE), lines: Vec::new() });
                    }
                }
                "l" => {
                    let (code, s) = rest.split_once(' ').unwrap_or((rest, ""));
                    let style = code.parse().ok().and_then(Style::from_u8);
                    if let (Some(t), Some(style), false) = (c.turns.last_mut(), style, skip) {
                        if t.lines.len() < LINES {
                            t.lines.push((style, back(s, LINE)));
                        }
                    }
                }
                _ => {}
            }
        }
        let next = list.len() as u32;
        let mut chats = Chats { list, next, listing: false, asked: false };
        let (mut turns, mut memory) = (Vec::new(), Vec::new());
        chats.take(&mut turns, &mut memory);
        (!chats.list.is_empty()).then_some((chats, turns, memory))
    }
}

/// Whether `text` is chats a newer Assistant kept, its head's version past this one's (1): not
/// damaged, but not for this one to read or write over.
pub fn newer(text: &str) -> bool {
    let version = text.lines().next().and_then(|head| head.strip_prefix("compusophy chats "));
    version.and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v > 1)
}

/// Remembers a task, `prompt` answered `answer`, each clipped to 1,000 bytes as the file keeps
/// them (an ellipsis, within them, where cut); the memory is then its last `n` tasks ([`forget`]).
pub fn remember(memory: &mut Memory, prompt: &str, answer: &str, n: usize) {
    memory.push((fit(prompt, TASK), fit(answer, TASK)));
    forget(memory, n);
}

/// Whether `answer`, to a question that waits on the person's yes, is one: yes words alone ("yes",
/// "ok, send it", "sure, why not", "that's fine", a thumbs up), never a question ("sure?"), and
/// never how the person is ("I'm good", "fine, thanks": a no thanks). So an answer that asks for
/// a change, or says no, is none.
pub fn yes(answer: &str) -> bool {
    if answer.contains(['?', '\u{ff1f}']) {
        return false;
    }
    // The yes words; "good" and "fine" alone, or saying how a thing is ("sounds good", "it's
    // fine"); the words that may go with them; and how the person may say they are.
    const YES: &str = "yes y ya yea yeah yep yup aye ok okay k kk sure alright right great cool \
                       perfect absolutely definitely certainly course correct agreed confirm \
                       confirmed approve approved lgtm go do send write save replace type click \
                       press tap submit proceed continue please works";
    const HOW: &str = "good fine";
    const IS: &str = "sounds looks seems thats its is";
    const ALSO: &str = "i im am it its thats that this them the is ahead on for now of all and \
                        just lets thing thanks thank you me can end sounds looks seems";
    const STATE: &str = "good fine ok okay alright right";
    let is = |list: &str, w: &str| list.split(' ').any(|v| v == w);
    let answer = answer.to_ascii_lowercase().replace(['\'', '\u{2019}'], "");
    // A thumbs up, an OK hand, a check mark.
    let answer = answer.replace(['\u{1f44d}', '\u{1f44c}', '\u{2705}', '\u{2714}'], " ok ");
    let words: Vec<&str> =
        answer.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let at = |k: usize| words.get(k).copied().unwrap_or_default();
    let (mut said, mut i) = (false, 0);
    while i < words.len() {
        let (w, next) = (at(i), at(i + 1));
        let (back, back2) = (i.checked_sub(1).map_or("", at), i.checked_sub(2).map_or("", at));
        // "I'm good", "I am fine", "I'm all right": how the person is.
        let me = |x: &str| is("i im am", x);
        if is(STATE, w) && (me(back) || back == "all" && me(back2)) {
            return false;
        }
        i += 1;
        if let ("why", "not") | ("no", "problem" | "worries") | ("you", "can") = (w, next) {
            (said, i) = (true, i + 1);
        } else if is(YES, w) || is(HOW, w) && (words.len() == 1 || is(IS, back)) {
            said = true;
        } else if !is(ALSO, w) && !is(HOW, w) {
            return false;
        }
    }
    said
}

/// `said`, a model's words, as plain text: the inline markdown it may write though told not to
/// (code's backticks, bold's `**`) taken out, every word kept.
pub fn plain(said: &str) -> String {
    said.replace("**", "").replace('`', "")
}

/// Whether `prompt`, the person's, asks to be told something, not only to have it done: a
/// question mark, a question's word (or tell, explain) anywhere, or is, are, does or did first.
pub fn asks(prompt: &str) -> bool {
    const ANY: &str = "what which who whose when where why how tell explain";
    const FIRST: &str = "is are was were does did";
    let is = |list: &str, w: &str| list.split(' ').any(|v| v == w);
    let words = prompt.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty());
    let mut words = words.map(str::to_ascii_lowercase);
    let first = words.next().unwrap_or_default();
    prompt.contains(['?', '\u{ff1f}', '\u{bf}'])
        || is(FIRST, &first)
        || [first].into_iter().chain(words).any(|w| is(ANY, &w))
}

/// Keeps `memory` to its last `n` tasks, a note (the one with no prompt) first while it is there,
/// one of them.
pub fn forget(memory: &mut Memory, n: usize) {
    let note = usize::from(memory.first().is_some_and(|m| m.0.is_empty()));
    let over = memory.len().saturating_sub(n);
    memory.drain(note..(note + over).min(memory.len()));
}

/// Whether `memory` is worth compacting: past [`LEAN`] bytes.
pub fn compactable(memory: &Memory) -> bool {
    memory.iter().map(|(prompt, answer)| prompt.len() + answer.len()).sum::<usize>() > LEAN
}

/// `c`'s name as its chip or item says it, `n` chars at most.
fn named(c: &Chat, n: usize) -> String {
    if c.name.is_empty() { "New chat".into() } else { short(&c.name, n) }
}

/// A button's width, a little over its label's in Inter (a chip's 12 px with 12 px either side, a
/// quiet button's 14 px with 8 px, another's 14 px with 14 px), and the gap after it.
fn wide(b: &Node) -> u32 {
    let Node::Button { variant, label, .. } = b else { return 0 };
    let n = label.chars().count() as u32;
    GAP + match variant {
        Variant::Chip | Variant::On => 24 + 15 * n / 2,
        Variant::Quiet => (16 + 9 * n).max(32),
        _ => 28 + 9 * n,
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

/// `s` within `max` bytes as the file keeps it: whole if it fits, else cut with an ellipsis.
fn fit(s: &str, max: usize) -> String {
    match s.len() > max {
        true => [clip(s, max - '\u{2026}'.len_utf8()), "\u{2026}"].concat(),
        false => s.to_string(),
    }
}

/// `s` cut to at most `max` bytes, on a char boundary.
fn clip(s: &str, max: usize) -> &str {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    s.get(..end).unwrap_or_default()
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

/// `s` as the file holds it, read back: unescaped, clipped to `max` bytes as it was kept.
fn back(s: &str, max: usize) -> String {
    clip(&unesc(s), max).to_string()
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

#[cfg(test)]
mod tests;
