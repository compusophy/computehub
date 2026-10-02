//! About, Feedback, Files and Welcome: compusophyOS's system apps as one wasm32-wasip1 GUI
//! program (`dist/bin/system.wasm`) on the [`uiwire`] protocol, fetched when one first opens and
//! never with the boot download. It runs the app its name says, as the /bin markers `about`,
//! `feedback`, `files` and `welcome` run it ([`view`]): [`About`], [`Feedback`], [`Files`] (at a
//! folder, if one follows) or [`Welcome`]. [`serve`] runs one until its window closes; run in a
//! terminal, which has no window for it, it says how to open one ([`hint`]). Files reads folders
//! through a [`Disk`]: [`Fs`] in the program (`std::fs`, which WASI serves from the desktop's
//! VFS), a VFS in tests.

#![forbid(unsafe_code)]

mod about;
mod feedback;
mod files;
#[cfg(test)]
mod tests;
mod welcome;

use std::io::{self, ErrorKind, Read, Write};

pub use about::About;
pub use feedback::Feedback;
pub use files::Files;
use uiwire::client::Client;
use uiwire::{Event, Frame, Node, Style};
pub use vfs::Entry;
pub use welcome::Welcome;

/// A window's app: events in, frames out.
pub trait View {
    /// Handles one event; whether the window changed (every Change does: the desktop sends the
    /// next one once a frame comes).
    fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool;

    /// The window now, with the requests since the last frame; `seq` 0, for [`serve`] to fill.
    fn frame(&mut self) -> Frame;
}

/// What Files reads.
pub trait Disk {
    /// The entries of the folder at `path`, in the order listed (`.` and `..` left out).
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>>;
}

/// The program's files: `std::fs`, which WASI serves from the desktop's VFS.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fs;

impl Disk for Fs {
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(path)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if name != "." && name != ".." {
                let meta = std::fs::metadata([path.trim_end_matches('/'), "/", &name].concat())?;
                out.push(Entry { name, is_dir: meta.is_dir(), size: meta.len() });
            }
        }
        Ok(out)
    }
}

/// The app `argv` names by its first word's file name (less a `.wasm`; `system <app> ...` names
/// it next), and the app: About, Feedback, Files at home or at the folder after it, or Welcome.
/// `None` for any other.
pub fn view(argv: &[String]) -> Option<(&'static str, Box<dyn View>)> {
    let base = |s: &str| {
        let name = s.rsplit('/').next().unwrap_or(s);
        name.strip_suffix(".wasm").unwrap_or(name).to_string()
    };
    let argv = match argv {
        [first, rest @ ..] if base(first) == "system" => rest,
        _ => argv,
    };
    let (app, args) = argv.split_first()?;
    Some(match (base(app).as_str(), args) {
        ("about", []) => ("about", Box::new(About::default())),
        ("feedback", []) => ("feedback", Box::new(Feedback::default())),
        ("files", []) => ("files", Box::new(Files::new("~"))),
        ("files", [dir]) => ("files", Box::new(Files::new(dir))),
        ("welcome", []) => ("welcome", Box::new(Welcome::default())),
        _ => return None,
    })
}

/// What the app `name` says run in a terminal: it has a window, and how to open it.
pub fn hint(name: &str) -> String {
    [name, ": an app with a window; open it with: open ", name].concat()
}

/// Runs `view` on `ui`, a frame per event that changes it, until Close or the end of the events;
/// an event that does not decode is skipped.
pub fn serve<R: Read, W: Write>(
    ui: &mut Client<R, W>,
    view: &mut dyn View,
    disk: &mut dyn Disk,
) -> io::Result<()> {
    let mut seq = 0u32;
    loop {
        let ev = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => ev,
            Err(e) if e.kind() == ErrorKind::InvalidData => continue,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if view.event(&ev, disk) {
            ui.show(&Frame { seq, ..view.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

pub(crate) fn text(style: Style, text: &str) -> Node {
    Node::Text { id: 0, style, text: text.into() }
}

/// Empty space, `px` logical px along the parent's axis.
pub(crate) fn space(px: u16) -> Node {
    Node::Spacer { px }
}

/// `n` centered across the width: flexible room either side of what takes its own width (a Text
/// on one line, a Glyph, a Button, a Pane).
pub(crate) fn center(n: Node) -> Node {
    let room = || Node::Col { id: 0, gap: 0, children: Vec::new() };
    Node::Row { id: 0, gap: 0, children: vec![room(), n, room()] }
}

/// `n` as people write it, its thousands set apart by commas: `8,000`.
pub(crate) fn group(n: usize) -> String {
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
