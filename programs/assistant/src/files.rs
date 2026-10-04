//! The file tools: the person's files as the program's own WASI filesystem holds them (the
//! desktop's VFS, whose root `/` every program sees). A path goes from the home ([`Vfs::HOME`];
//! `~` names it, `/…` the root) and a result names one in the home by `~`. A listing and a read
//! reach the model clipped to [`MAX_READ`] bytes, saying so; only text is read. Failures are
//! coded: E0918 a path that is none, E0925 nothing there, E0926 a folder where a file was named
//! or a file where a folder was, E0927 what the filesystem refused (or no files at all), E0928 a
//! file that is not text. Replacing a file, or writing outside the home, waits for the person's
//! yes ([`asks`]).

use std::io::{self, ErrorKind};

use vfs::{Entry, Vfs};

use crate::ai::clip;

/// The most bytes of a file, or of a listing, the model is given.
pub const MAX_READ: usize = 16 << 10;

/// What the file tools reach: [`Fs`] in the program, a VFS in tests.
pub trait Disk: std::fmt::Debug {
    /// The entries of the folder at `path`.
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

    /// Written beside it (`.<name>.saving`), then moved over it, as the Editor saves.
    fn write(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let (dir, name) = path.rsplit_once('/').ok_or(ErrorKind::InvalidInput)?;
        for (i, b) in path.bytes().enumerate().skip(1) {
            if b == b'/' {
                _ = std::fs::create_dir(path.get(..i).unwrap_or_default());
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

/// `path` from the home, absolute; E0918 if it is none.
pub fn resolve(path: &str) -> Result<String, String> {
    let bad = |_| format!("E0918: {} is not a path", clip(path, 80));
    Vfs::normalize(Vfs::HOME, path.trim()).map_err(bad)
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
    let entries = disk.list(&p).map_err(|e| failed(disk, &e, &p, true))?;
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
/// write outside the home. E0926 for a folder there.
pub fn asks(disk: &mut dyn Disk, path: &str, text: &str) -> Result<Option<String>, String> {
    let p = resolve(path)?;
    let (n, size) = (named(&p), text.len());
    Ok(match entry(disk, &p) {
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
        Some(e) => format!("; it held {}", e.size),
        None => ", new".into(),
    };
    disk.write(&p, text.as_bytes()).map_err(|e| failed(disk, &e, &p, false))?;
    Ok(format!("ok: wrote {} ({} bytes{was})", named(&p), text.len()))
}

/// The entry at absolute `path`, as its folder lists it.
fn entry(disk: &mut dyn Disk, path: &str) -> Option<Entry> {
    let (dir, name) = path.rsplit_once('/')?;
    let dir = if dir.is_empty() { "/" } else { dir };
    disk.list(dir).ok()?.into_iter().find(|e| e.name == name)
}

/// Why `path` failed with `e`, coded; `folder`: whether a folder was named.
fn failed(disk: &mut dyn Disk, e: &io::Error, path: &str, folder: bool) -> String {
    let n = named(path);
    match entry(disk, path) {
        Some(x) if x.is_dir && !folder => format!("E0926: {n} is a folder, not a file"),
        Some(x) if !x.is_dir && folder => format!("E0926: {n} is a file, not a folder"),
        None if e.kind() == ErrorKind::NotFound => {
            format!("E0925: nothing is at {n}; list_files shows what is")
        }
        _ => format!("E0927: {n}: {e}"),
    }
}
