//! Compact, and the rest of the card's chats ([`chats`] holds them; the current one's transcript
//! and memory are the agent's own). Compact, there once the memory is past [`chats::LEAN`]
//! bytes, asks the model, with no tools, to condense the chat into a note of [`NOTE`] bytes at
//! most. The note then stands for the memory as its one task with no prompt (asked as [`SUM`]),
//! first while later tasks come and go, until the next Compact folds it in. A failure (E0901 to
//! E0905, E0907 for a reply out of room, a note cut short too, or E0929 for no note) leaves the
//! memory as it was, and Compact there to ask again. Compacting shows as a task does, the pill with Stop. The chats stay as they are while
//! a task is in hand; they are kept after each change ([`Agent::kept`]) and read at the start
//! ([`Agent::load`]).

use chats::{COMPACT, Chats, Clicked, compactable};
use uiwire::{Event, Node, Request, Style};

use super::{Agent, MAX_TURNS, MEMORY, RETRY, SEND, STOP, Turn, tokens};
use crate::ai::{ROOM, chat, clip, failure, put_clip};
use crate::calls::Calls;

/// The most bytes of a compaction's note, and the prompt it answers in a request.
pub const NOTE: usize = 600;
pub const SUM: &str = "Sum up our conversation so far.";
/// What a compaction asks of the model, and the most bytes of each prompt and answer it reads.
const CONDENSE: &str = "You condense a conversation between a user and the assistant that \
operates compusophyOS, a desktop in a browser tab, for them, into the note the assistant keeps in \
its place: what the user wants, what was done and changed, names and choices, and anything left \
open. Plain text, shorter than the conversation and under 500 characters, nothing before or after \
the note.";
const READ: usize = 1500;
/// A compaction's reply that held no note.
const NONE: &str = "E0929 the AI wrote no note; the memory is as it was";

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

    /// The chats' events, before the rest: the card's width, its chats' buttons, Compact, and
    /// while compacting its reply, its Stop, and the prompts it holds off (a prompt hides the
    /// list). Whether the window changed; `None` for the rest's.
    pub(super) fn chats_event(&mut self, ev: &Event) -> Option<bool> {
        let compacting = self.compacting.as_ref().map(|c| c.0);
        Some(match ev {
            // The card's, not the pill's: its row follows it.
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
            Event::Click { id: COMPACT } => self.compact(),
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
            // Held off while it compacts; else the agent's, the list hidden.
            Event::Ask { .. } | Event::Submit { .. } | Event::Click { id: SEND | RETRY } => {
                self.chats.close();
                compacting.map(|_| false)?
            }
            Event::Click { id } if !self.busy() => {
                let clicked = self.chats.click(*id, &mut self.turns, &mut self.memory)?;
                if clicked == Clicked::New {
                    self.requests.push(Request::Focus { id: self.input_id() });
                }
                if clicked != Clicked::Shown {
                    (self.retry, self.unkept) = (None, true);
                }
                true
            }
            _ => return None,
        })
    }

    /// Asks the model for a note of the chat so far, with no tools: it shows as a task does.
    fn compact(&mut self) -> bool {
        if self.busy() || !compactable(&self.memory) {
            return false;
        }
        let mut talk = String::from("The conversation so far:\n");
        for (prompt, answer) in &self.memory {
            talk += "\nUser: ";
            put_clip(&mut talk, if prompt.is_empty() { SUM } else { prompt }, READ);
            talk += "\nAssistant: ";
            put_clip(&mut talk, answer, READ);
            talk += "\n";
        }
        // A task's room: the free AI's proxy lets the model think through 1,024 tokens of it.
        let body = chat(self.model(), ",\"max_tokens\":2048,\"temperature\":0.2", CONDENSE, &talk);
        let id = self.next_id();
        (self.retry, self.compacting) = (None, Some((id, Calls::default())));
        self.chats.close();
        self.turns.push(Turn { prompt: "Compact".into(), lines: Vec::new() });
        self.turns.drain(..self.turns.len().saturating_sub(MAX_TURNS));
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
            // Out of room, even mid-note: a note cut short is none.
            None if c.finish == "length" => (Style::Error, ROOM.into(), ""),
            None if note.is_empty() => (Style::Error, NONE.into(), ""),
            None => {
                // Cut short, its ellipsis within the most.
                let note = clip(note, NOTE - '\u{2026}'.len_utf8());
                self.memory = vec![(String::new(), note.clone())];
                (Style::Dim, note, "Compacted: ")
            }
        };
        self.note(style, line);
        self.note(Style::Small, format!("{done}{} tokens in, {} out", tokens(i), tokens(o)));
        self.requests.push(Request::Status { working: false });
        self.unkept = true;
        true
    }

    /// A task ended: the chat is named by its first prompt if it has no name yet, and to be kept.
    pub(super) fn changed(&mut self) {
        if let Some(t) = self.turns.first() {
            self.chats.name(&t.prompt);
        }
        self.unkept = true;
    }

    /// The chats over the prompt ([`Chats::row`]), none while a task is in hand.
    pub(super) fn chips(&self) -> Vec<Node> {
        match self.task {
            Some(_) => Vec::new(),
            None => self.chats.row(self.width, &self.turns, &self.memory),
        }
    }

    /// The chats as their file keeps them ([`Chats::encode`]), if they changed since last asked.
    pub fn kept(&mut self) -> Option<String> {
        std::mem::take(&mut self.unkept).then(|| self.chats.encode(&self.turns, &self.memory))
    }

    /// Puts back the chats `text` keeps ([`Chats::load`]), if it holds them (never while a task
    /// is in hand); whether it does.
    pub fn load(&mut self, text: &str) -> bool {
        let Some((chats, turns, memory)) = Chats::load(text, (MAX_TURNS, MEMORY)) else {
            return false;
        };
        if !self.busy() {
            (self.chats, self.turns, self.memory) = (chats, turns, memory);
        }
        true
    }
}
