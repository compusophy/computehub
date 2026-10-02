//! About, Editor, Feedback, Files, Welcome and Activity: compusophyOS's system apps as one
//! wasm32-wasip1 GUI program (`dist/bin/system.wasm`) on the [`uiwire`] protocol, fetched when
//! one first opens and never with the boot download. It runs the app its name says, as the /bin
//! markers `about`, `editor`, `feedback`, `files`, `welcome` and `activity` run it ([`view`]):
//! [`About`], [`Editor`] (on a file, if one follows), [`Feedback`], [`Files`] (at a folder, if
//! one follows), [`Welcome`] or [`Activity`]. [`serve`] runs one until its window closes; run in
//! a terminal, which has no window for it, it says how to open one ([`hint`]). Files and Editor
//! reach files through a [`Disk`]: [`Fs`] in the program (`std::fs`, which WASI serves from the
//! desktop's VFS), a VFS in tests.

#![forbid(unsafe_code)]

mod about;
mod activity;
mod editor;
mod feedback;
mod files;
#[cfg(test)]
mod tests;
mod welcome;

use std::io::{self, ErrorKind, Read, Write};

pub use about::About;
pub use activity::Activity;
pub use editor::Editor;
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

    /// An event did not decode (it may have been an edit); whether the window changed.
    fn garbled(&mut self) -> bool {
        false
    }

    /// The window is closing: the last moment to keep what it holds.
    fn close(&mut self, disk: &mut dyn Disk) {
        _ = disk;
    }
}

/// What Files and Editor reach.
pub trait Disk {
    /// The entries of the folder at `path`, in the order listed (`.` and `..` left out).
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>>;
    /// The file at `path`, whole.
    fn read(&mut self, path: &str) -> io::Result<Vec<u8>>;
    /// Puts `data` in the file at `path`, whole, making the folders above it; a write that fails
    /// leaves what was there.
    fn write(&mut self, path: &str, data: &[u8]) -> io::Result<()>;
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

    fn read(&mut self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    /// Written beside it (`.<name>.saving`), then moved over it, so it is never half written.
    fn write(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let (dir, name) = path.rsplit_once('/').ok_or(ErrorKind::InvalidInput)?;
        // Each folder above it (one that cannot be made, the write says why); not
        // create_dir_all, whose path parsing would ship.
        for (i, b) in path.bytes().enumerate().skip(1) {
            if b == b'/' {
                _ = std::fs::create_dir(&path[..i]);
            }
        }
        let part = [dir, "/.", name, ".saving"].concat();
        let wrote = std::fs::write(&part, data).and_then(|()| std::fs::rename(&part, path));
        if wrote.is_err() {
            _ = std::fs::remove_file(&part);
        }
        wrote
    }
}

/// The app `argv` names by its first word's file name (less a `.wasm`; `system <app> ...` names
/// it next), and the app: About, Editor (a new note, or the file after it), Feedback, Files at
/// home or at the folder after it, Welcome or Activity. `None` for any other.
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
        ("editor", []) => ("editor", Box::new(Editor::new(""))),
        ("editor", [file]) => ("editor", Box::new(Editor::new(file))),
        ("feedback", []) => ("feedback", Box::new(Feedback::default())),
        ("files", []) => ("files", Box::new(Files::new("~"))),
        ("files", [dir]) => ("files", Box::new(Files::new(dir))),
        ("welcome", []) => ("welcome", Box::new(Welcome::default())),
        ("activity", []) => ("activity", Box::new(Activity::default())),
        _ => return None,
    })
}

/// What the app `name` says run in a terminal: it has a window, and how to open it.
pub fn hint(name: &str) -> String {
    [name, ": an app with a window; open it with: open ", name].concat()
}

/// Where the hint goes, by which of stdin, stdout and stderr are a terminal (`ttys`): to
/// stdout (1) when it is one, else to stderr (2) when any is (`about > out.txt` leaves stdin and
/// stderr on the terminal); `None` when none is, as in a window, whose console no fd is.
pub fn hint_to(ttys: [bool; 3]) -> Option<u8> {
    match ttys {
        [_, true, _] => Some(1),
        [true, ..] | [.., true] => Some(2),
        _ => None,
    }
}

/// Runs `view` on `ui`, a frame per event that changes it, until Close (told to the view first)
/// or the end of the events; an event that does not decode is the view's to note
/// ([`View::garbled`]).
pub fn serve<R: Read, W: Write>(
    ui: &mut Client<R, W>,
    view: &mut dyn View,
    disk: &mut dyn Disk,
) -> io::Result<()> {
    let mut seq = 0u32;
    loop {
        let changed = match ui.next_event() {
            Ok(Event::Close) => {
                view.close(disk);
                return Ok(());
            }
            Ok(ev) => view.event(&ev, disk),
            Err(e) if e.kind() == ErrorKind::InvalidData => view.garbled(),
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if changed {
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
