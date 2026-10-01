//! Studio: write applang apps, check them and run them, each in its own window. A wasm32-wasip1
//! GUI program (`dist/bin/studio.wasm`) whose window is a [`uiwire`] widget tree the desktop draws
//! in its own theme.
//!
//! [`view`] reads the arguments: `edit <path>` is [`Studio`], the editor; `run <path>` is
//! [`AppHost`], one `.app` file running. [`serve`] runs a view until its window closes. A view asks
//! the desktop to open a `.app` path (to run it) or `studio:<path>` (to edit it). Files go through
//! a [`Disk`]: [`Fs`] in the program, a map in tests. The samples are files in `samples/`, so the
//! desktop can install them without this crate.

#![forbid(unsafe_code)]

mod edit;
mod run;

use applang::Diag;
pub use edit::{MAX_TEXT, Studio};
pub use run::AppHost;
use std::io::{self, ErrorKind, Read, Write};
use std::path::Path;
use uiwire::{Event, Frame, client::Client};

/// The file Studio edits when it is given none.
pub const DEFAULT_FILE: &str = "/apps/counter.app";

/// The sample apps as `(path, source)`: the files in `samples/`.
pub const SAMPLES: [(&str, &str); 3] = [
    (DEFAULT_FILE, include_str!("../samples/counter.app")),
    ("/apps/greeter.app", include_str!("../samples/greeter.app")),
    ("/apps/clicker.app", include_str!("../samples/clicker.app")),
];

/// What New starts a file with.
const NEW_APP: &str = include_str!("new.app");

/// A window's program: events in, frames out.
pub trait View {
    /// Handles one event (`None`: one did not decode); whether the window
    /// changed. The first call loads the view and is always a change.
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool;

    /// The window now, with the requests since the last frame; `seq` 0 for [`serve`] to fill.
    fn frame(&mut self) -> Frame;
}

/// Where a view's files live.
pub trait Disk {
    /// The file's text, invalid UTF-8 replaced.
    fn read(&mut self, path: &str) -> io::Result<String>;
    /// Writes the whole file, making its directory first.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()>;
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

    fn exists(&mut self, path: &str) -> bool {
        Path::new(path).exists()
    }
}

/// The view `args` ask for: none is [`Studio`] on [`DEFAULT_FILE`]; relative paths are in /apps.
pub fn view(args: &[String]) -> Option<Box<dyn View>> {
    let abs = |p: &str| {
        (!p.is_empty()).then(|| [if p.starts_with('/') { "" } else { "/apps/" }, p].concat())
    };
    match args {
        [] => Some(Box::new(Studio::new(DEFAULT_FILE))),
        [mode, path] if mode == "edit" => Some(Box::new(Studio::new(&abs(path)?))),
        [mode, path] if mode == "run" => Some(Box::new(AppHost::new(&abs(path)?))),
        _ => None,
    }
}

/// Runs `view` on `ui`, a frame per event that changes it, until Close or the
/// end of the events; an event that does not decode is `None` to the view.
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
        if view.event(ev.as_ref(), disk) {
            ui.show(&Frame { seq, ..view.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

/// A diagnostic as a line cut to 1 KiB, `E0302 3:7 message` (line from 1,
/// column in chars); `error` for no code, and no position for no span.
fn problem(d: &Diag, src: &str) -> String {
    let code = d.code.map_or_else(|| "error".to_string(), |c| format!("E{c:04}"));
    let at = d.span.map(|s| lang::diag::line_col(src, s.start));
    let at = at.map_or_else(String::new, |(line, col)| format!(" {line}:{col}"));
    clip(format!("{code}{at} {}", d.message), 1024)
}

/// `s` cut to at most `max` bytes on a char boundary, ending in `…` if cut.
fn clip(mut s: String, max: usize) -> String {
    if s.len() > max {
        s.truncate((0..=max).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0));
        s.push('…');
    }
    s
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests;
