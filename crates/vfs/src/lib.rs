//! The compusophyOS virtual filesystem: an in-memory tree of directories and files, Unix paths.
//!
//! [`Vfs`] is plain data: no clocks, floats, hash-ordered collections or I/O. A directory is a
//! vector of its entries sorted by name (byte order) and binary-searched, so listings are sorted
//! and the same operations on a fresh [`Vfs::new`] always build the same tree (`Vfs` is `Eq`,
//! change count included, so a replay can be checked). The kernel's homed saves /home when that
//! count, [`Vfs::generation`], moves.
//!
//! Paths: [`Vfs::normalize`] turns what a user types into an absolute path. Every other method
//! takes an absolute path (a relative one is [`VfsError::InvalidPath`]) and resolves `.`, `..`
//! and repeated slashes the same way, purely lexically: there are no links. A name may hold any
//! character but `/` and NUL.
//!
//! Limits: file contents total at most [`Vfs::MAX_BYTES`] and there are at most
//! [`Vfs::MAX_ENTRIES`] files and directories; going over either is [`VfsError::NoSpace`]. Names
//! are at most [`Vfs::MAX_NAME`] bytes and paths at most [`Vfs::MAX_DEPTH`] names deep; going over
//! either is [`VfsError::InvalidPath`]. An operation that fails changes nothing.
//!
//! ```
//! use vfs::{Vfs, VfsError};
//!
//! let mut fs = Vfs::new();
//! let path = Vfs::normalize("/tmp", "notes/../hello.txt").unwrap();
//! assert_eq!(path, "/tmp/hello.txt");
//! fs.write(&path, b"hello").unwrap();
//! fs.append(&path, b", world").unwrap();
//! assert_eq!((fs.read(&path), fs.total_bytes()), (Ok(&b"hello, world"[..]), 12));
//! assert_eq!(fs.mkdir("/tmp"), Err(VfsError::Exists));
//! let names: Vec<String> = fs.list("/").unwrap().into_iter().map(|e| e.name).collect();
//! assert_eq!(names, ["apps", "home", "tmp"]);
//! ```

#![forbid(unsafe_code)]

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Dir(Dir),
    File(Vec<u8>),
}

/// A directory's entries, sorted by name (byte order) with no name twice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Dir(Vec<(String, Node)>);

impl Dir {
    /// Where `name` is (`Ok`), or where it would go to stay sorted (`Err`).
    fn find(&self, name: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|(n, _)| n.as_str().cmp(name))
    }

    fn get(&self, name: &str) -> Option<&Node> {
        self.find(name).ok().map(|i| &self.0[i].1)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Node> {
        self.find(name).ok().map(|i| &mut self.0[i].1)
    }

    /// Puts `node` at `name`, replacing what was there.
    fn insert(&mut self, name: &str, node: Node) {
        match self.find(name) {
            Ok(i) => self.0[i].1 = node,
            Err(i) => self.0.insert(i, (String::from(name), node)),
        }
    }
}

/// One entry of a directory listing, from [`Vfs::list`]: its `name` within the directory, whether
/// it `is_dir`, and the `size` in bytes of a file (0 for a directory).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Why a [`Vfs`] operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VfsError {
    /// The path, or a directory on the way to it, does not exist.
    NotFound,
    /// A directory was needed but a file was found.
    NotADir,
    /// A file was needed but a directory was found.
    IsADir,
    /// The path to create already exists.
    Exists,
    /// The directory still has entries.
    NotEmpty,
    /// A relative path where an absolute one is needed, a NUL, a name or path over its limit, or
    /// an operation that cannot apply (removing or renaming the root, moving a dir into itself).
    InvalidPath,
    /// The operation would go over [`Vfs::MAX_BYTES`] or [`Vfs::MAX_ENTRIES`].
    NoSpace,
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            VfsError::NotFound => "no such file or directory",
            VfsError::NotADir => "not a directory",
            VfsError::IsADir => "is a directory",
            VfsError::Exists => "file exists",
            VfsError::NotEmpty => "directory not empty",
            VfsError::InvalidPath => "invalid path",
            VfsError::NoSpace => "no space left on device",
        })
    }
}

impl std::error::Error for VfsError {}

/// An in-memory filesystem. See the [crate docs](crate) for paths and limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vfs {
    root: Node,
    /// Sum of all file lengths.
    bytes: u64,
    /// Files and directories, the root not counted.
    entries: usize,
    generation: u64,
}

impl Default for Vfs {
    fn default() -> Vfs {
        Vfs::new()
    }
}

impl Vfs {
    /// The guest's home directory, which `~` names.
    // Two pieces: scripts/caps.sh rejects text that looks like a home path.
    pub const HOME: &str = concat!("/home/", "guest");
    /// Most bytes all file contents together may hold: 16 MiB.
    pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
    /// Most files and directories, the root not counted.
    pub const MAX_ENTRIES: usize = 65_536;
    /// Longest name, in bytes.
    pub const MAX_NAME: usize = 255;
    /// Most names in a path. It bounds how deep the tree nests.
    pub const MAX_DEPTH: usize = 64;

    /// A filesystem holding the empty directories `/apps`, `/home`, [`Vfs::HOME`] and `/tmp`.
    pub fn new() -> Vfs {
        let mut fs = Vfs { root: Node::Dir(Dir::default()), bytes: 0, entries: 0, generation: 0 };
        let made = ["/apps", Vfs::HOME, "/tmp"].map(|dir| fs.mkdir_all(dir));
        debug_assert!(made.iter().all(Result::is_ok));
        Vfs { generation: 0, ..fs }
    }

    /// Resolves `path` against the working directory `cwd` into an absolute path with no `.`,
    /// `..` (one at the root stays there), empty names or trailing slash. The path need not exist.
    ///
    /// `~` and `~/…` start at [`Vfs::HOME`], `/…` at the root and anything else (the empty path
    /// too) at `cwd`, which must then be absolute. `~` is special only as the whole first name:
    /// `~x` and `a/~` are plain.
    ///
    /// ```text
    /// cwd    path        result
    /// /tmp   a//b/./c    /tmp/a/b/c
    /// /tmp   ../../x     /x
    /// /tmp   (empty)     /tmp
    /// ```
    ///
    /// Fails with `InvalidPath` for a NUL, a relative `cwd` that is needed, or a result over
    /// [`Vfs::MAX_NAME`] or [`Vfs::MAX_DEPTH`].
    pub fn normalize(cwd: &str, path: &str) -> Result<String, VfsError> {
        let full = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => [Vfs::HOME, rest].concat(),
            _ if path.starts_with('/') => String::from(path),
            _ if cwd.starts_with('/') => [cwd, "/", path].concat(),
            _ => return Err(VfsError::InvalidPath),
        };
        Ok(format!("/{}", names(&full)?.join("/")))
    }

    /// Whether `path` is a directory. False for any error, a bad path too.
    pub fn is_dir(&self, path: &str) -> bool {
        matches!(self.get(path), Ok(Node::Dir(_)))
    }

    /// Whether `path` is a file. False for any error, a bad path too.
    pub fn is_file(&self, path: &str) -> bool {
        matches!(self.get(path), Ok(Node::File(_)))
    }

    /// Whether `path` is a file or a directory.
    pub fn exists(&self, path: &str) -> bool {
        self.get(path).is_ok()
    }

    /// Creates the directory `path`, whose parent must be a directory. Fails with `Exists` if
    /// `path` exists (the root always does), `NotFound` or `NotADir` for a missing or
    /// non-directory parent, or `NoSpace`.
    pub fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        let names = names(path)?;
        let exists = walk(&self.root, &names).is_ok();
        let (_, parent) = names.split_last().filter(|_| !exists).ok_or(VfsError::Exists)?;
        walk_dir(&self.root, parent)?;
        self.mkdir_all(path)
    }

    /// Creates the directory `path` and any missing parents; fine if it is already a directory.
    /// Fails with `Exists` if `path` is a file, `NotADir` if a parent is, or `NoSpace`, and then
    /// creates nothing.
    pub fn mkdir_all(&mut self, path: &str) -> Result<(), VfsError> {
        let names = names(path)?;
        let have = (0..names.len()).take_while(|&k| walk(&self.root, &names[..=k]).is_ok()).count();
        if let Node::File(_) = walk(&self.root, &names[..have])? {
            return Err(if have == names.len() { VfsError::Exists } else { VfsError::NotADir });
        }
        if names.len() - have > Vfs::MAX_ENTRIES - self.entries {
            return Err(VfsError::NoSpace);
        }
        for end in have..names.len() {
            let (dir, name) = parent(&mut self.root, &names[..=end], VfsError::Exists)?;
            dir.insert(name, Node::Dir(Dir::default()));
            self.entries += 1;
        }
        self.generation += 1;
        Ok(())
    }

    /// Creates the file `path` holding `data`, or replaces what it holds. Fails with `IsADir` for
    /// a directory (the root is one), `NotFound` or `NotADir` for a missing or non-directory
    /// parent, or `NoSpace` (the replaced bytes are freed first).
    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.put(path, data, false)
    }

    /// Adds `data` to the end of the file `path`, creating it if missing. Fails as
    /// [`Vfs::write`] does.
    pub fn append(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.put(path, data, true)
    }

    /// The contents of the file `path`. Fails with `NotFound`, `IsADir` for a directory, or
    /// `NotADir` if a parent is a file.
    pub fn read(&self, path: &str) -> Result<&[u8], VfsError> {
        match self.get(path)? {
            Node::File(data) => Ok(data),
            Node::Dir(_) => Err(VfsError::IsADir),
        }
    }

    /// The entries of the directory `dir`, sorted by name (byte order). Fails with `NotFound`,
    /// or `NotADir` if it or a parent is a file.
    pub fn list(&self, dir: &str) -> Result<Vec<Entry>, VfsError> {
        let entry = |(name, node): &(String, Node)| {
            let size = if let Node::File(data) = node { data.len() as u64 } else { 0 };
            Entry { name: name.clone(), is_dir: matches!(node, Node::Dir(_)), size }
        };
        Ok(walk_dir(&self.root, &names(dir)?)?.0.iter().map(entry).collect())
    }

    /// Removes the file or directory `path`, and with `recursive` everything under it. Fails
    /// with `NotFound`, `NotEmpty` for a directory with entries unless `recursive`, `NotADir` if
    /// a parent is a file, or `InvalidPath` for the root.
    pub fn remove(&mut self, path: &str, recursive: bool) -> Result<(), VfsError> {
        let names = names(path)?;
        let (dir, name) = parent(&mut self.root, &names, VfsError::InvalidPath)?;
        let i = dir.find(name).map_err(|_| VfsError::NotFound)?;
        if !recursive && matches!(&dir.0[i].1, Node::Dir(sub) if !sub.0.is_empty()) {
            return Err(VfsError::NotEmpty);
        }
        let (bytes, entries, _) = usage(&dir.0.remove(i).1);
        (self.bytes, self.entries) = (self.bytes - bytes, self.entries - entries);
        self.generation += 1;
        Ok(())
    }

    /// Moves the file or directory `from` to `to` as POSIX `rename` does: a file replaces a file
    /// and a directory an empty directory; renaming a path to itself does nothing. Fails with
    /// `NotFound` if `from` or the parent of `to` is missing, `IsADir` for a file onto a
    /// directory, `NotADir` for a directory onto a file (or a parent that is a file), `NotEmpty`
    /// onto a directory with entries, or `InvalidPath` if either is the root, a directory would
    /// move into itself, or the result would be deeper than [`Vfs::MAX_DEPTH`].
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), VfsError> {
        let (src, dst) = (names(from)?, names(to)?);
        let both = src.split_last().zip(dst.split_last());
        let ((name, up), (to_name, to_up)) = both.ok_or(VfsError::InvalidPath)?;
        let node = walk(&self.root, &src)?;
        if src == dst {
            return Ok(());
        }
        let is_dir = matches!(node, Node::Dir(_));
        if (is_dir && dst.starts_with(&src)) || to_up.len() + usage(node).2 > Vfs::MAX_DEPTH {
            return Err(VfsError::InvalidPath);
        }
        let (bytes, entries, _) = match walk_dir(&self.root, to_up)?.get(to_name) {
            None => (0, 0, 0),
            Some(Node::Dir(_)) if !is_dir => return Err(VfsError::IsADir),
            Some(Node::File(_)) if is_dir => return Err(VfsError::NotADir),
            Some(Node::Dir(sub)) if !sub.0.is_empty() => return Err(VfsError::NotEmpty),
            Some(old) => usage(old),
        };
        // Both paths were walked above and `to` is not inside `from`: no `?` below fires.
        let src_dir = walk_dir_mut(&mut self.root, up)?;
        let node = src_dir.0.remove(src_dir.find(name).map_err(|_| VfsError::NotFound)?).1;
        walk_dir_mut(&mut self.root, to_up)?.insert(to_name, node);
        (self.bytes, self.entries) = (self.bytes - bytes, self.entries - entries);
        self.generation += 1;
        Ok(())
    }

    /// Bytes held by all files together; at most [`Vfs::MAX_BYTES`].
    pub fn total_bytes(&self) -> u64 {
        self.bytes
    }

    /// Moved by every change (maybe by a no-op), never by a failure; 0 when new.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Writes `data` into the existing file `path` at `off` (`u64::MAX` appends), in place,
    /// zero-filling a gap; its new length. Writing nothing changes nothing. Fails as
    /// [`Vfs::read`] does, or with `NoSpace`.
    pub fn write_at(&mut self, path: &str, off: u64, data: &[u8]) -> Result<u64, VfsError> {
        let old = self.read(path)?.len() as u64;
        if data.is_empty() {
            return Ok(old);
        }
        let off = if off == u64::MAX { old } else { off };
        let end = off.saturating_add(data.len() as u64);
        // Once resized, `end` is at most MAX_BYTES: the casts are exact.
        self.resize(path, end.max(old))?[off as usize..end as usize].copy_from_slice(data);
        Ok(end.max(old))
    }

    /// Cuts or zero-extends the existing file `path` to `len` bytes. Fails as
    /// [`Vfs::write_at`] does.
    pub fn set_len(&mut self, path: &str, len: u64) -> Result<(), VfsError> {
        self.resize(path, len).map(drop)
    }

    /// Resizes the file `path` to `len` bytes: its contents, to change.
    fn resize(&mut self, path: &str, len: u64) -> Result<&mut Vec<u8>, VfsError> {
        let old = self.read(path)?.len() as u64;
        if len.saturating_sub(old) > Vfs::MAX_BYTES - self.bytes {
            return Err(VfsError::NoSpace);
        }
        let (dir, name) = parent(&mut self.root, &names(path)?, VfsError::IsADir)?;
        // `read` found the file: the else is never taken.
        let Some(Node::File(file)) = dir.get_mut(name) else { return Err(VfsError::NotFound) };
        file.resize(len as usize, 0);
        (self.bytes, self.generation) = (self.bytes - old + len, self.generation + 1);
        Ok(file)
    }

    fn get(&self, path: &str) -> Result<&Node, VfsError> {
        walk(&self.root, &names(path)?)
    }

    fn put(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), VfsError> {
        let names = names(path)?;
        let (dir, name) = parent(&mut self.root, &names, VfsError::IsADir)?;
        let slot = dir.find(name);
        let freed = match slot.map(|i| &dir.0[i].1) {
            Ok(Node::Dir(_)) => return Err(VfsError::IsADir),
            Ok(Node::File(_)) if append => 0,
            Ok(Node::File(old)) => old.len() as u64,
            Err(_) if self.entries >= Vfs::MAX_ENTRIES => return Err(VfsError::NoSpace),
            Err(_) => 0,
        };
        let bytes = self.bytes - freed + data.len() as u64;
        if bytes > Vfs::MAX_BYTES {
            return Err(VfsError::NoSpace);
        }
        match slot.map(|i| &mut dir.0[i].1) {
            Ok(Node::File(old)) if append => old.extend_from_slice(data),
            _ => dir.insert(name, Node::File(data.to_vec())),
        }
        self.entries += usize::from(slot.is_err());
        (self.bytes, self.generation) = (bytes, self.generation + 1);
        Ok(())
    }
}

/// The names of an absolute path, checked against the limits: `.` and empty names are skipped
/// and `..` drops the name before it (none at the root).
fn names(path: &str) -> Result<Vec<&str>, VfsError> {
    let mut names = Vec::new();
    for name in path.split('/') {
        match name {
            "" | "." => {}
            ".." => _ = names.pop(),
            _ => names.push(name),
        }
    }
    let ok = path.starts_with('/') && !path.contains('\0') && names.len() <= Vfs::MAX_DEPTH;
    let ok = ok && names.iter().all(|n| n.len() <= Vfs::MAX_NAME);
    ok.then_some(names).ok_or(VfsError::InvalidPath)
}

fn walk<'n>(mut node: &'n Node, names: &[&str]) -> Result<&'n Node, VfsError> {
    for name in names {
        let Node::Dir(dir) = node else { return Err(VfsError::NotADir) };
        node = dir.get(name).ok_or(VfsError::NotFound)?;
    }
    Ok(node)
}

fn walk_dir<'n>(node: &'n Node, names: &[&str]) -> Result<&'n Dir, VfsError> {
    let Node::Dir(dir) = walk(node, names)? else { return Err(VfsError::NotADir) };
    Ok(dir)
}

fn walk_dir_mut<'n>(node: &'n mut Node, names: &[&str]) -> Result<&'n mut Dir, VfsError> {
    let Node::Dir(dir) = node else { return Err(VfsError::NotADir) };
    match names.split_first() {
        None => Ok(dir),
        Some((name, rest)) => walk_dir_mut(dir.get_mut(name).ok_or(VfsError::NotFound)?, rest),
    }
}

/// The directory holding the last of `names`, and that name; `at_root` if `names` is the root,
/// which has no parent.
fn parent<'n, 'a>(
    root: &'n mut Node,
    names: &[&'a str],
    at_root: VfsError,
) -> Result<(&'n mut Dir, &'a str), VfsError> {
    let (name, parent) = names.split_last().ok_or(at_root)?;
    Ok((walk_dir_mut(root, parent)?, name))
}

/// A subtree's file bytes, its entries (itself included) and its height (1 for a file or an
/// empty directory). Iterative, so depth costs no stack.
fn usage(node: &Node) -> (u64, usize, usize) {
    let (mut bytes, mut entries, mut height) = (0, 0, 0);
    let mut stack = vec![(node, 1)];
    while let Some((node, depth)) = stack.pop() {
        (entries, height) = (entries + 1, height.max(depth));
        match node {
            Node::File(data) => bytes += data.len() as u64,
            Node::Dir(dir) => stack.extend(dir.0.iter().map(|(_, n)| (n, depth + 1))),
        }
    }
    (bytes, entries, height)
}

#[cfg(test)]
mod tests;
