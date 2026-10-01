//! About, Feedback and Files: compusophyOS's system apps as one wasm32-wasip1 GUI program
//! (`dist/bin/system.wasm`) on the [`uiwire`] protocol, fetched when one first opens and never
//! with the boot download. It runs the app its name says, as the /bin markers `about`,
//! `feedback` and `files` run it ([`view`]): [`About`], [`Feedback`] or [`Files`] (at a folder,
//! if one follows). [`serve`] runs one until its window closes. Files reads folders through a
//! [`Disk`]: [`Fs`] in the program (`std::fs`, which WASI serves from the desktop's VFS), a VFS
//! in tests.

#![forbid(unsafe_code)]

mod about;
mod feedback;
mod files;
#[cfg(test)]
mod tests;

use std::io::{self, ErrorKind, Read, Write};

pub use about::About;
pub use feedback::Feedback;
pub use files::Files;
use uiwire::client::Client;
use uiwire::{Event, Frame, Node, Style};
pub use vfs::Entry;

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
/// it next): About, Feedback, or Files at home or at the folder after it. `None` for any other.
pub fn view(argv: &[String]) -> Option<Box<dyn View>> {
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
        ("about", []) => Box::new(About::default()),
        ("feedback", []) => Box::new(Feedback::default()),
        ("files", []) => Box::new(Files::new("~")),
        ("files", [dir]) => Box::new(Files::new(dir)),
        _ => return None,
    })
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
