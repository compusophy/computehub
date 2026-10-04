//! An agent's file tools (the Assistant's `list_files`, `read_file` and `write_file`): the
//! person's files as a program's WASI filesystem holds them (the desktop's VFS, whose root `/`
//! every program sees), through the [`Disk`] the system program's Files and Editor reach ([`Fs`]
//! in a program, [`Mem`] in tests). A path goes from the home ([`Vfs::HOME`]; `~` names it, `/…`
//! the root), never into /dev (the program's devices, where a read may wait forever) nor the
//! Assistant's own folder ([`OWN`]: its chats, every conversation's, which a listing leaves out),
//! and a result names one in the home by `~`. A listing and a read reach the model clipped to
//! [`MAX_READ`] bytes, saying so; only text is read. Failures are coded in the Assistant's series:
//! E0918 a path that is none, E0925 nothing there, E0926 a folder where a file was named or a
//! file where a folder was, E0927 what the filesystem refused, E0928 a file that is not text.
//! Replacing a file, or writing outside the home, waits for the person's yes ([`asks`]); a
//! folder that will not list fails rather than taking a file there for none.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::io::{self, ErrorKind};
use std::rc::Rc;

pub use system::{Disk, Entry, Fs};
use vfs::{Vfs, VfsError};

/// The most bytes of a file, or of a listing, the model is given.
pub const MAX_READ: usize = 16 << 10;
/// The Assistant's own folder in the home, where it keeps its chats: none of the user's files.
pub const OWN: &str = "/.assistant";

/// The files in memory: a VFS each clone shares (a test keeps one to look).
#[derive(Clone, Debug, Default)]
pub struct Mem(pub Rc<RefCell<Vfs>>);

impl Disk for Mem {
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>> {
        self.0.borrow().list(path).map_err(kind)
    }

    fn read(&mut self, path: &str) -> io::Result<Vec<u8>> {
        self.0.borrow().read(path).map(<[u8]>::to_vec).map_err(kind)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let mut fs = self.0.borrow_mut();
        _ = fs.mkdir_all(path.rsplit_once('/').map_or("/", |d| d.0));
        fs.write(path, data).map_err(kind)
    }
}

/// A VFS error as WASI gives it a program, as far as the tools read its kind.
fn kind(e: VfsError) -> io::Error {
    match e {
        VfsError::NotFound => ErrorKind::NotFound.into(),
        VfsError::NotADir => ErrorKind::NotADirectory.into(),
        e => io::Error::other(e),
    }
}

/// `path` from the home, absolute; E0918 if it is none, in /dev, or in the Assistant's [`OWN`].
pub fn resolve(path: &str) -> Result<String, String> {
    let bad = |_| format!("E0918: {} is not a path", clip(path, 80));
    let p = Vfs::normalize(Vfs::HOME, path.trim()).map_err(bad)?;
    let under =
        |root: &str| p.strip_prefix(root).is_some_and(|r| r.is_empty() || r.starts_with('/'));
    if under("/dev") {
        return Err(format!("E0918: {p} is the program's devices, none of the user's files"));
    }
    if under(&[Vfs::HOME, OWN].concat()) {
        return Err(format!(
            "E0918: {} is the Assistant's own chats, none of the user's files",
            named(&p)
        ));
    }
    Ok(p)
}

/// Absolute `path` as the model and the person read it: `~/…` in the home.
pub fn named(path: &str) -> String {
    match path.strip_prefix(Vfs::HOME) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => ["~", rest].concat(),
        _ => path.to_string(),
    }
}

/// list_files: the folder at `path`, an entry a line (a folder's name ends in `/`, a file's
/// size follows it).
pub fn list(disk: &mut dyn Disk, path: &str) -> Result<String, String> {
    let p = resolve(path)?;
    let mut entries = disk.list(&p).map_err(|e| failed(disk, &e, &p, true))?;
    entries.retain(|e| p != Vfs::HOME || OWN.strip_prefix('/') != Some(e.name.as_str()));
    let count = match entries.len() {
        1 => "1 entry".into(),
        n => format!("{n} entries"),
    };
    let mut out = format!("ok: listed {} ({count})", named(&p));
    for (i, e) in entries.iter().enumerate() {
        let line = match e.is_dir {
            true => ["\n", &e.name, "/"].concat(),
            false => format!("\n{}, {} bytes", e.name, e.size),
        };
        if out.len() + line.len() > MAX_READ {
            out += &format!("\n(clipped: {} more)", entries.len() - i);
            break;
        }
        out += &line;
    }
    Ok(out)
}

/// read_file: the text of the file at `path`, at most its first [`MAX_READ`] bytes.
pub fn read(disk: &mut dyn Disk, path: &str) -> Result<String, String> {
    let p = resolve(path)?;
    let data = disk.read(&p).map_err(|e| failed(disk, &e, &p, false))?;
    let (n, size) = (named(&p), data.len());
    let text = std::str::from_utf8(&data)
        .map_err(|_| format!("E0928: {n} is not text ({size} bytes); read_file reads text"))?;
    Ok(match size > MAX_READ {
        true => {
            format!("ok: read {n} ({size} bytes; clipped to its first {MAX_READ}):\n")
                + &clip(text, MAX_READ)
        }
        false => format!("ok: read {n} ({size} bytes):\n{text}"),
    })
}

/// What writing `text` at `path` asks the person first, if anything: to replace a file, or to
/// write outside the home. E0926 for a folder there; E0927 if its folder is there and will not
/// list, since a file may be.
pub fn asks(disk: &mut dyn Disk, path: &str, text: &str) -> Result<Option<String>, String> {
    let p = resolve(path)?;
    let (n, size) = (named(&p), text.len());
    let there =
        entry(disk, &p).map_err(|e| format!("E0927: {n}: its folder did not list ({e})"))?;
    Ok(match there {
        Some(e) if e.is_dir => return Err(format!("E0926: {n} is a folder, not a file")),
        Some(e) => Some(format!("Replace {n} ({} bytes) with these {size} bytes?", e.size)),
        None if !n.starts_with('~') => {
            Some(format!("Write {n}, outside your home, with these {size} bytes?"))
        }
        None => None,
    })
}

/// write_file: `text` put in the file at `path`, whole, the folders above it made.
pub fn write(disk: &mut dyn Disk, path: &str, text: &str) -> Result<String, String> {
    let p = resolve(path)?;
    let was = match entry(disk, &p) {
        Ok(Some(e)) => format!("; it held {}", e.size),
        Ok(None) => ", new".into(),
        Err(_) => String::new(),
    };
    disk.write(&p, text.as_bytes()).map_err(|e| failed(disk, &e, &p, false))?;
    Ok(format!("ok: wrote {} ({} bytes{was})", named(&p), text.len()))
}

/// The entry at absolute `path`, as its folder lists it: none if that folder is not there (or
/// is a file); the folder's error if it is there and will not list.
fn entry(disk: &mut dyn Disk, path: &str) -> io::Result<Option<Entry>> {
    let (dir, name) = path.rsplit_once('/').ok_or(ErrorKind::InvalidInput)?;
    match disk.list(if dir.is_empty() { "/" } else { dir }) {
        Ok(all) => Ok(all.into_iter().find(|e| e.name == name)),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Why `path` failed with `e`, coded; `folder`: whether a folder was named.
fn failed(disk: &mut dyn Disk, e: &io::Error, path: &str, folder: bool) -> String {
    let n = named(path);
    match entry(disk, path).ok().flatten() {
        Some(x) if x.is_dir && !folder => format!("E0926: {n} is a folder, not a file"),
        Some(x) if !x.is_dir && folder => format!("E0926: {n} is a file, not a folder"),
        None if e.kind() == ErrorKind::NotFound => {
            format!("E0925: nothing is at {n}; list_files shows what is")
        }
        _ => format!("E0927: {n}: {e}"),
    }
}

/// `s` cut to at most `max` bytes at a character's start, `…` after it if anything was cut.
fn clip(s: &str, max: usize) -> String {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    let mut out = s.get(..end).unwrap_or_default().to_string();
    if end < s.len() {
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
mod tests;
