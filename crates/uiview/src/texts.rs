//! [`Texts`]: the text the person edits in a window, which the host owns while they do.

use std::mem;

use ui::Code;
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
}

impl Texts {
    /// Whether the frame holds an Input, Code or Area `id`.
    pub fn has(&self, id: u32) -> bool {
        let (i, c) = (self.inputs.iter().any(|i| i.0 == id), self.codes.iter().any(|c| c.0 == id));
        i || c || self.areas.iter().any(|a| a.0 == id)
    }

    /// A press on node `id` (0: none) at `(x, y)` in the content. An Input, Code or Area takes
    /// the keyboard (a Code or Area its caret there too); any other node leaves it where it is,
    /// so a chip or a switch pressed while typing loses nothing; empty space takes it away.
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
        mem::replace(&mut self.focus, focus) != focus || focus != 0
    }

    /// Takes the Inputs, Codes and Areas of a frame's `nodes`. One the person is in, or whose
    /// edit has not reached the program yet (`dirty`: its Change waits), keeps the host's text;
    /// the others take the frame's. A Code at its version takes the frame's spans, above it its
    /// text. The keyboard stays where it was while that is in the frame.
    pub fn adopt(&mut self, nodes: &[Node], dirty: &[u32]) {
        let mut old = mem::take(self);
        let keep = |focus: u32, id: u32| focus == id || dirty.contains(&id);
        each(nodes, &mut |n| match n {
            Node::Input { id, value, .. } if *id != 0 => {
                let i = old.inputs.iter().position(|i| i.0 == *id).map(|i| old.inputs.remove(i));
                let mut i = i.unwrap_or((*id, String::new(), 0));
                if !keep(old.focus, *id) {
                    i.1.clone_from(value);
                }
                self.inputs.push(i);
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
                if !keep(old.focus, *id) {
                    a.1.set(value);
                }
                self.areas.push(a);
            }
            _ => {}
        });
        if self.has(old.focus) {
            self.focus = old.focus;
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
