//! Feedback: what to fix, what to build, what to keep, straight to compusophy.

use uiwire::{Event, Frame, Key, Node, Request, Style, Variant, mods};

use crate::{Disk, View, center, space, text};

/// The kinds as (chip, what is sent).
pub(crate) const KINDS: [(&str, &str); 3] = [("Bug", "bug"), ("Idea", "idea"), ("Love", "love")];
const INTRO: &str = "Tell compusophy what broke, what to build next, or what you love.";
const HINT: &str = "What happened, or what would make it better?";
const CONTEXT: &str = "Include what\u{2019}s open and recent events";
const CONTEXT_NOTE: &str = "The build, your browser and screen size, the theme, the apps open \
and the last 50 events. Never your files.";
const SEND: &str = "Send";
/// What Send says: true whether the report goes at once or waits in the page's outbox, as the app
/// cannot know which.
pub(crate) const THANKS: &str = "Thank you \u{2014} it goes when it can";
/// The most text sent, in bytes.
pub(crate) const MAX: usize = 8000;
/// Node ids: kind `i` is `KIND + i`; the context switch, Send; the text is `AREA` plus the
/// reports sent, so a fresh one starts empty.
pub(crate) const KIND: u32 = 1;
pub(crate) const BOX: u32 = 11;
pub(crate) const GO: u32 = 12;
pub(crate) const AREA: u32 = 100;
/// The column's widest.
const MAX_W: u16 = 610;

/// Feedback: chips for the kind (Bug, Idea, Love), a text that wraps and grows, a switch to
/// include the desktop's context, and Send, which hands it all to the page
/// ([`Request::Feedback`]) and thanks; Ctrl+Enter sends too.
#[derive(Debug)]
pub struct Feedback {
    /// An index into [`KINDS`]: Idea at first.
    pub(crate) kind: usize,
    /// The text as the desktop last said, and the reports sent.
    pub(crate) text: String,
    pub(crate) sent: u32,
    pub(crate) context: bool,
    /// What the last Send said, until the next edit.
    pub(crate) status: Option<&'static str>,
    requests: Vec<Request>,
    framed: bool,
}

impl Default for Feedback {
    fn default() -> Feedback {
        let (text, requests) = (String::new(), Vec::new());
        #[rustfmt::skip]
        let f = Feedback { kind: 1, text, sent: 0, context: true, status: None, requests,
            framed: false };
        f
    }
}

impl Feedback {
    fn area(&self) -> u32 {
        AREA.wrapping_add(self.sent)
    }

    /// Sends the text, if there is any, and starts a fresh one; whether it went.
    fn send(&mut self) -> bool {
        let mut text = self.text.trim();
        if text.is_empty() {
            return false;
        }
        if text.len() > MAX {
            let cut = (0..=MAX).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
            text = &text[..cut];
        }
        let (kind, context) = (KINDS[self.kind].1.to_string(), self.context);
        self.requests.push(Request::Feedback { kind, text: text.to_string(), context });
        (self.text, self.sent, self.status) =
            (String::new(), self.sent.wrapping_add(1), Some(THANKS));
        self.requests.push(Request::Focus { id: self.area() });
        true
    }
}

impl View for Feedback {
    fn event(&mut self, ev: &Event, _: &mut dyn Disk) -> bool {
        match *ev {
            Event::Resize { .. } => !self.framed,
            Event::Click { id } if (KIND..KIND + 3).contains(&id) => {
                let kind = (id - KIND) as usize;
                std::mem::replace(&mut self.kind, kind) != kind
            }
            Event::Click { id: BOX } => {
                self.context = !self.context;
                true
            }
            Event::Click { id: GO } => self.send(),
            Event::Key { key: Key::Enter, mods: m, .. } if m & (mods::CTRL | mods::META) != 0 => {
                self.send()
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Event::Change { id, ref text, .. } => {
                if id == self.area() {
                    (self.text, self.status) = (text.clone(), None);
                }
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self) -> Frame {
        let chip = |(i, (label, _)): (usize, &(&str, &str))| {
            let variant = if i == self.kind { Variant::On } else { Variant::Chip };
            Node::Button { id: KIND + i as u32, variant, label: label.to_string() }
        };
        let chips = KINDS.iter().enumerate().map(chip).collect();
        let (value, placeholder) = (self.text.clone(), HINT.to_string());
        let note = vec![space(8), text(Style::Small, CONTEXT_NOTE)];
        let context = vec![
            Node::Toggle { id: BOX, on: self.context, label: CONTEXT.into() },
            Node::Row { id: 0, gap: 0, children: note },
        ];
        // Send at the right, in the accent once there is something to send; what the last one
        // did in the rest of the row, its first line level with Send's label.
        let variant = if self.text.trim().is_empty() { Variant::Normal } else { Variant::Primary };
        let said = self.status.iter().flat_map(|s| [space(7), text(Style::Body, s)]).collect();
        let end = vec![
            Node::Col { id: 0, gap: 0, children: said },
            Node::Button { id: GO, variant, label: SEND.into() },
        ];
        let column = vec![
            space(5),
            text(Style::Small, INTRO),
            Node::Row { id: 0, gap: 8, children: chips },
            Node::Area { id: self.area(), value, placeholder },
            Node::Col { id: 0, gap: 0, children: context },
            Node::Row { id: 0, gap: 13, children: end },
        ];
        let pane = center(Node::Pane { id: 0, w: MAX_W, children: column });
        let first = [Request::Focus { id: self.area() }];
        let first = (!std::mem::replace(&mut self.framed, true)).then_some(first);
        let requests = first.into_iter().flatten().chain(std::mem::take(&mut self.requests));
        Frame { seq: 0, title: "Feedback".into(), requests: requests.collect(), nodes: vec![pane] }
    }
}
