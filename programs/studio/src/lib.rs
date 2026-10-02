//! Studio: make applang apps by describing them. A wasm32-wasip1 GUI program
//! (`dist/bin/studio.wasm`) whose window is a [`uiwire`] widget tree the desktop draws in its own
//! theme.
//!
//! [`view`] reads the arguments: none is [`Studio`] with nothing open, asking what to make;
//! `edit <path>` is [`Studio`] on that file; `run <path>` is [`AppHost`], one `.app` file running.
//! [`serve`] runs a view until its window closes. Studio asks the AI (through the desktop, as the
//! Assistant does) for a program, checks it with [`applang`], sends it back with its problem at
//! most [`assistant::RETRIES`] times, then saves it and runs it live in its window ([`Live`]). A
//! view asks the desktop to open a `.app` path (to run it), `studio` or `studio:<path>`. Files go
//! through a [`Disk`]: [`Fs`] in the program, a map in tests.

#![forbid(unsafe_code)]

mod edit;
mod make;
mod run;
mod view;

pub use edit::{MAX_TEXT, Studio};
pub use run::{APP, AppHost, INPUT, Live};
use std::io::{self, ErrorKind, Read, Write};
use std::path::Path;
use uiwire::{Event, Frame, Node, Style, client::Client};

/// A window's program: events in, frames out.
pub trait View {
    /// Handles one event (`None`: one did not decode); whether the window
    /// changed. The first call loads the view and is always a change.
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool;

    /// Work that waits for a frame to show it is under way (Studio's check of a reply); whether
    /// the window changed again, and so whether there may be more.
    fn step(&mut self, _disk: &mut dyn Disk) -> bool {
        false
    }

    /// The window now, with the requests since the last frame; `seq` 0 for [`serve`] to fill.
    fn frame(&mut self) -> Frame;
}

/// Where a view's files live.
pub trait Disk {
    /// The file's text, invalid UTF-8 replaced.
    fn read(&mut self, path: &str) -> io::Result<String>;
    /// Writes the whole file, making its directory first.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()>;
    /// Adds `text` at the file's end, making it and its directory first.
    fn append(&mut self, path: &str, text: &str) -> io::Result<()>;
    /// Whether anything is at `path`.
    fn exists(&mut self, path: &str) -> bool;
}

/// The program's files: `std::fs`, which WASI serves from the desktop's VFS.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fs;

impl Disk for Fs {
    fn read(&mut self, path: &str) -> io::Result<String> {
        Ok(String::from_utf8_lossy(&std::fs::read(path)?).into_owned())
    }

    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        // If this fails, the write says why.
        let _ = Path::new(path).parent().map(std::fs::create_dir_all);
        std::fs::write(path, text)
    }

    fn append(&mut self, path: &str, text: &str) -> io::Result<()> {
        let _ = Path::new(path).parent().map(std::fs::create_dir_all);
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(text.as_bytes())
    }

    fn exists(&mut self, path: &str) -> bool {
        Path::new(path).exists()
    }
}

/// The view `args` ask for: none is [`Studio`] with nothing open; relative paths are in /apps.
pub fn view(args: &[String]) -> Option<Box<dyn View>> {
    let abs = |p: &str| {
        (!p.is_empty()).then(|| [if p.starts_with('/') { "" } else { "/apps/" }, p].concat())
    };
    match args {
        [] => Some(Box::new(Studio::new(""))),
        [mode, path] if mode == "edit" => Some(Box::new(Studio::new(&abs(path)?))),
        [mode, path] if mode == "run" => Some(Box::new(AppHost::new(&abs(path)?))),
        _ => None,
    }
}

/// Runs `view` on `ui`, a frame per event that changes it and per step after, until Close or
/// the end of the events; an event that does not decode is `None` to the view.
pub fn serve<R: Read, W: Write>(
    ui: &mut Client<R, W>,
    view: &mut dyn View,
    disk: &mut dyn Disk,
) -> io::Result<()> {
    let mut seq = 0u32;
    loop {
        let ev = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => Some(ev),
            Err(e) if e.kind() == ErrorKind::InvalidData => None,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut changed = view.event(ev.as_ref(), disk);
        while changed {
            ui.show(&Frame { seq, ..view.frame() })?;
            seq = seq.wrapping_add(1);
            changed = view.step(disk);
        }
    }
}

pub(crate) fn text(style: Style, text: &str) -> Node {
    Node::Text { id: 0, style, text: text.into() }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests;
