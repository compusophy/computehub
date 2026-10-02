//! Welcome: the first screen.

use std::mem;

use icons::Glyph;
use uiwire::{Event, Frame, Node, REVEAL, Request, Style};

use crate::{Disk, View, center, space, text};

const TITLE: &str = "compusophy";
const LINE: &str = "a computer in your browser \u{2014} free AI, nothing to install.";
pub(crate) const HINT: &str = "Every app is on the home screen; the sparkle at the bottom \
right opens the Assistant.";
/// The apps, rows 1 to 7: the name to open, what it is called, what it is for, its icon.
#[rustfmt::skip]
pub(crate) const APPS: [(&str, &str, &str, Glyph, u32); 7] = [
    ("studio", "Studio", "build apps with AI", Glyph::Studio, 0x8b7bff),
    ("assistant", "Assistant", "ask anything", Glyph::Assistant, 0xa78bfa),
    ("terminal", "Terminal", "a shell and your files", Glyph::Terminal, 0x2dd4bf),
    ("files", "Files", "your home folder", Glyph::Folder, 0x60a5fa),
    ("settings", "Settings", "themes, AI, privacy", Glyph::Cog, 0x94a3b8),
    ("about", "About", "what this is", Glyph::About, 0xfbbf24),
    ("feedback", "Feedback", "tell compusophy what to fix", Glyph::Bug, 0x34d399),
];
/// The column's widest, and the mark's largest side.
const MAX_W: u16 = 466;
pub(crate) const MARK_MAX: u16 = 144;

/// The first screen: the mark (1/φ of the window's shorter side, at most 144 px), which comes in
/// ring by ring when it first shows, the name, a line, where the apps live, and the apps as a
/// list, each row opening its app. Centered in a tall window; a short one scrolls.
#[derive(Debug, Default)]
pub struct Welcome {
    /// The mark's side, from the last Resize; whether a frame went yet; the requests since.
    pub(crate) side: u16,
    framed: bool,
    requests: Vec<Request>,
}

impl View for Welcome {
    fn event(&mut self, ev: &Event, _: &mut dyn Disk) -> bool {
        match *ev {
            Event::Resize { w, h } => {
                // 1/φ in whole numbers.
                let side = (u32::from(w.min(h)) * 1000 / 1618).min(u32::from(MARK_MAX)) as u16;
                mem::replace(&mut self.side, side) != side || !self.framed
            }
            Event::Click { id: id @ 1..=7 } => {
                let name = APPS[id as usize - 1].0.to_string();
                self.requests.push(Request::Open { name });
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self) -> Frame {
        let head = vec![
            Node::Glyph { glyph: Glyph::Mark as u8 | REVEAL, size: self.side },
            space(34),
            text(Style::Display, TITLE),
            space(8),
            text(Style::Dim, LINE),
            space(13),
            text(Style::Small, HINT),
        ];
        let rows = APPS.iter().zip(1..).map(|(&(_, name, what, glyph, hue), id)| {
            let (text, detail) = ([name, "\n", what].concat(), String::new());
            Node::Entry { id, glyph: glyph as u8, hue, text, detail, more: true }
        });
        let column = vec![
            Node::Center { id: 0, gap: 0, children: head },
            space(18),
            Node::Col { id: 0, gap: 0, children: rows.collect() },
        ];
        // Room above and below shares what the window has spare: the column sits in its middle.
        let room = || Node::Fill { id: 0, children: Vec::new() };
        let nodes = vec![room(), center(Node::Pane { id: 0, w: MAX_W, children: column }), room()];
        self.framed = true;
        Frame { seq: 0, title: "Welcome".into(), requests: mem::take(&mut self.requests), nodes }
    }
}
