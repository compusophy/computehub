//! [`Texts`]: the text the person edits in a window, which the host owns while they do.

use std::mem;

use ui::{Code, Key};
use uiwire::Node;

use crate::Area;

/// The text the host owns: each Input's id, text and version, each Code and Area by id, and the
/// focused one (0 for none).
#[derive(Debug, Default)]
pub struct Texts {
    pub inputs: Vec<(u32, String, u32)>,
    pub codes: Vec<(u32, Code)>,
    pub areas: Vec<(u32, Area)>,
    pub focus: u32,
    /// Each Input's and Area's value as the program last sent it.
    pub(crate) said: Vec<(u32, String)>,
    /// The focused Input or Area, at its version, when a press on another node left it the
    /// keyboard: until its next edit, a value the program changes is the program's.
    pub(crate) left: Option<(u32, u32)>,
}

impl Texts {
    /// Whether the frame holds an Input, Code or Area `id`.
    pub fn has(&self, id: u32) -> bool {
        let (i, c) = (self.inputs.iter().any(|i| i.0 == id), self.codes.iter().any(|c| c.0 == id));
        i || c || self.areas.iter().any(|a| a.0 == id)
    }

    /// Key `key` for the focused Code, Area or Input (none of them takes Escape or a chord):
    /// whether one took it (`Some`), and whether that edited it. Enter in an Input it leaves
    /// to the window: its Submit.
    pub fn key(&mut self, key: Key, chord: bool) -> Option<bool> {
        let id = self.focus;
        if chord || key == Key::Escape {
            return None;
        }
        let code = self.codes.iter_mut().find(|c| c.0 == id).and_then(|c| c.1.key(key));
        let typed = code.or_else(|| self.areas.iter_mut().find(|a| a.0 == id)?.1.key(key));
        typed.or_else(|| {
            let i = self.inputs.iter_mut().find(|i| i.0 == id && key == Key::Backspace)?;
            let popped = i.1.pop().is_some();
            i.2 = i.2.wrapping_add(u32::from(popped));
            Some(popped)
        })
    }

    /// Typed or pasted `s` into the focused Code, Area or Input (into a Code or an Area none of
    /// it if `echo`: the Enter or Tab just handled); whether it edited it.
    pub fn insert(&mut self, s: &str, echo: bool) -> bool {
        let id = self.focus;
        let area = self.areas.iter_mut().find(|a| a.0 == id).map(|a| !echo && a.1.insert(s));
        match self.codes.iter_mut().find(|c| c.0 == id) {
            Some(c) => !echo && c.1.insert(s),
            None if area.is_some() => area == Some(true),
            None => self.inputs.iter_mut().find(|i| i.0 == id).is_some_and(|i| {
                let n = i.1.len();
                i.1.extend(s.chars().filter(|c| !c.is_control()));
                i.1.truncate(if i.1.len() > ui::CODE_MAX { n } else { i.1.len() });
                i.2 = i.2.wrapping_add(u32::from(i.1.len() != n));
                i.1.len() != n
            }),
        }
    }

    /// The edits so far of Input or Area `id`.
    fn version(&self, id: u32) -> Option<u32> {
        let input = self.inputs.iter().find(|i| i.0 == id).map(|i| i.2);
        input.or_else(|| self.areas.iter().find(|a| a.0 == id).map(|a| a.1.version))
    }

    /// A press on node `id` (0: none) at `(x, y)` in the content. An Input, Code or Area takes
    /// the keyboard (a Code or Area its caret there too); any other node leaves it where it is,
    /// so a chip or a switch pressed while typing loses nothing, but what the program then does
    /// to the text (a Clear button) shows ([`Texts::adopt`]); empty space takes it away.
    /// Whether to redraw.
    pub fn press(&mut self, id: u32, x: f32, y: f32) -> bool {
        if let Some(c) = self.codes.iter_mut().find(|c| c.0 == id && id != 0) {
            c.1.click(x, y);
        }
        if let Some(a) = self.areas.iter_mut().find(|a| a.0 == id && id != 0) {
            a.1.click(x, y);
        }
        let focus = match id {
            0 => 0,
            id if self.has(id) => id,
            _ => self.focus,
        };
        let control = id != 0 && !self.has(id);
        self.left = self.version(focus).filter(|_| control).map(|v| (focus, v));
        mem::replace(&mut self.focus, focus) != focus || focus != 0
    }

    /// Takes the Inputs, Codes and Areas of a frame's `nodes`. One whose edit has not reached
    /// the program yet (`dirty`: its Change waits) keeps the host's text, and so does the one the
    /// person is in, unless a press on another node left it the keyboard, no edit came since, and
    /// the program changed the value it sent: then the program's value wins (a button that
    /// clears it). The others take the frame's. A Code at its version takes the frame's spans,
    /// above it its text. The keyboard stays where it was while that is in the frame.
    pub fn adopt(&mut self, nodes: &[Node], dirty: &[u32]) {
        let mut old = mem::take(self);
        let (said, left, focus) = (mem::take(&mut old.said), old.left, old.focus);
        let keep = |id: u32, version: u32, value: &str| {
            let changed = said.iter().find(|s| s.0 == id).is_none_or(|s| s.1 != value);
            let theirs = left == Some((id, version)) && changed;
            dirty.contains(&id) || (focus == id && !theirs)
        };
        each(nodes, &mut |n| match n {
            Node::Input { id, value, .. } if *id != 0 => {
                let i = old.inputs.iter().position(|i| i.0 == *id).map(|i| old.inputs.remove(i));
                let mut i = i.unwrap_or((*id, String::new(), 0));
                if !keep(*id, i.2, value) {
                    i.1.clone_from(value);
                }
                self.inputs.push(i);
                self.said.push((*id, value.clone()));
            }
            Node::Code { id, version, text, spans, .. } if *id != 0 => {
                let c = old.codes.iter().position(|c| c.0 == *id).map(|c| old.codes.remove(c).1);
                let mut c = c.unwrap_or_else(|| Code::new(text, *version));
                if *version > c.version {
                    c.set_text(text, *version);
                }
                if *version == c.version {
                    let s: Vec<_> = spans.iter().map(|s| (s.start, s.len, s.class as u8)).collect();
                    c.set_spans(&s);
                }
                self.codes.push((*id, c));
            }
            Node::Area { id, value, .. } if *id != 0 => {
                let a = old.areas.iter().position(|a| a.0 == *id).map(|a| old.areas.remove(a));
                let mut a = a.unwrap_or_else(|| (*id, Area::new(value)));
                if !keep(*id, a.1.version, value) {
                    a.1.set(value);
                }
                self.areas.push(a);
                self.said.push((*id, value.clone()));
            }
            _ => {}
        });
        if self.has(focus) {
            (self.focus, self.left) = (focus, left);
        }
    }
}

/// `f` for every node of `nodes` and their children, in pre-order.
fn each(nodes: &[Node], f: &mut dyn FnMut(&Node)) {
    for n in nodes {
        f(n);
        each(n.children(), f);
    }
}
