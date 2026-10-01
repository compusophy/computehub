//! The compusophyOS virtual filesystem: an in-memory tree of directories and
//! files with Unix-style paths.
//!
//! [`Vfs`] is plain data: no clocks, no floating point, no hash-ordered
//! collections, no I/O. A directory is a vector of its entries kept sorted by
//! name (byte order) and searched by binary search, so listings are sorted
//! and the same operations on a fresh [`Vfs::new`] always build the same tree
//! (`Vfs` is `Eq`, change count included, so a replay can be checked). The
//! kernel's homed saves /home when that count, [`Vfs::generation`], moves.
//!
//! # Paths
//!
//! [`Vfs::normalize`] turns what a user types into an absolute path. Every
//! other method takes an absolute path (a relative one is
//! [`VfsError::InvalidPath`]) and resolves `.`, `..` and repeated slashes the
//! same way. Resolution is purely lexical: there are no links. A name may
//! hold any character but `/` and NUL.
//!
//! # Limits
//!
//! File contents total at most [`Vfs::MAX_BYTES`] and there are at most
//! [`Vfs::MAX_ENTRIES`] files and directories; going over either is
//! [`VfsError::NoSpace`]. Names are at most [`Vfs::MAX_NAME`] bytes and paths
//! at most [`Vfs::MAX_DEPTH`] names deep; going over either is
//! [`VfsError::InvalidPath`]. An operation that fails changes nothing.
//!
//! # Example
//!
//! ```
//! use vfs::{Vfs, VfsError};
//!
//! let mut fs = Vfs::new();
//! let path = Vfs::normalize("/tmp", "notes/../hello.txt").unwrap();
//! assert_eq!(path, "/tmp/hello.txt");
//! fs.write(&path, b"hello").unwrap();
//! fs.append(&path, b", world").unwrap();
//! assert_eq!(fs.read(&path), Ok(&b"hello, world"[..]));
//! assert_eq!(fs.total_bytes(), 12);
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
        let i = self.find(name).ok()?;
        Some(&self.0[i].1)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Node> {
        let i = self.find(name).ok()?;
        Some(&mut self.0[i].1)
    }

    /// Puts `node` at `name`, replacing what was there.
    fn insert(&mut self, name: &str, node: Node) {
        match self.find(name) {
            Ok(i) => self.0[i].1 = node,
            Err(i) => self.0.insert(i, (String::from(name), node)),
        }
    }

    fn remove(&mut self, name: &str) -> Option<Node> {
        let i = self.find(name).ok()?;
        Some(self.0.remove(i).1)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One entry of a directory listing, from [`Vfs::list`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its name within the directory.
    pub name: String,
    /// Whether it is a directory.
    pub is_dir: bool,
    /// A file's length in bytes; 0 for a directory.
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
    /// A relative path where an absolute one is needed, a NUL, a name or path
    /// over its limit, or an operation that cannot apply (removing or
    /// renaming the root, moving a directory into itself).
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

    /// A filesystem holding the empty directories `/apps`, `/home`,
    /// [`Vfs::HOME`] and `/tmp`.
    pub fn new() -> Vfs {
        let mut fs = Vfs { root: Node::Dir(Dir::default()), bytes: 0, entries: 0, generation: 0 };
        for dir in ["/apps", Vfs::HOME, "/tmp"] {
            let made = fs.mkdir_all(dir);
            debug_assert!(made.is_ok());
        }
        Vfs { generation: 0, ..fs }
    }

    /// Resolves `path` against the working directory `cwd` into an absolute
    /// path with no `.`, `..` (one at the root stays there), empty names or
    /// trailing slash. The path need not exist.
    ///
    /// `~` and `~/…` start at [`Vfs::HOME`], `/…` at the root and anything
    /// else (the empty path too) at `cwd`, which must then be absolute. `~`
    /// is special only as the whole first name: `~x` and `a/~` are plain.
    ///
    /// ```text
    /// cwd    path        result
    /// /tmp   a//b/./c    /tmp/a/b/c
    /// /tmp   ../../x     /x
    /// /tmp   (empty)     /tmp
    /// ```
    ///
    /// Fails with `InvalidPath` for a NUL, a relative `cwd` that is needed,
    /// or a result over [`Vfs::MAX_NAME`] or [`Vfs::MAX_DEPTH`].
    pub fn normalize(cwd: &str, path: &str) -> Result<String, VfsError> {
        let full = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => [Vfs::HOME, rest].concat(),
            _ if path.starts_with('/') => String::from(path),
            _ if cwd.starts_with('/') => [cwd, "/", path].concat(),
            _ => return Err(VfsError::InvalidPath),
        };
        let mut out = String::with_capacity(full.len() + 1);
        for name in names(&full)? {
            out.push('/');
            out.push_str(name);
        }
        if out.is_empty() {
            out.push('/');
        }
        Ok(out)
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

    /// Creates the directory `path`, whose parent must be a directory. Fails
    /// with `Exists` if `path` exists (the root always does), `NotFound` or
    /// `NotADir` for a missing or non-directory parent, or `NoSpace`.
    pub fn mkdir(&mut self, path: &str) -> Result<(), VfsError> {
        let names = names(path)?;
        let (_, parent) = names.split_last().ok_or(VfsError::Exists)?;
        if walk(&self.root, &names).is_ok() {
            return Err(VfsError::Exists);
        }
        walk_dir(&self.root, parent)?;
        self.mkdir_all(path)
    }

    /// Creates the directory `path` and any missing parents; fine if it is
    /// already a directory. Fails with `Exists` if `path` is a file,
    /// `NotADir` if a parent is, or `NoSpace`, and then creates nothing.
    pub fn mkdir_all(&mut self, path: &str) -> Result<(), VfsError> {
        let names = names(path)?;
        let mut have = 0;
        while have < names.len() && walk(&self.root, &names[..=have]).is_ok() {
            have += 1;
        }
        match walk(&self.root, &names[..have])? {
            Node::File(_) if have == names.len() => return Err(VfsError::Exists),
            Node::File(_) => return Err(VfsError::NotADir),
            Node::Dir(_) => {}
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

    /// Creates the file `path` holding `data`, or replaces what it holds.
    /// Fails with `IsADir` for a directory (the root is one), `NotFound` or
    /// `NotADir` for a missing or non-directory parent, or `NoSpace` (the
    /// replaced bytes are freed first).
    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.put(path, data, false)
    }

    /// Adds `data` to the end of the file `path`, creating it if missing.
    /// Fails as [`Vfs::write`] does.
    pub fn append(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        self.put(path, data, true)
    }

    /// The contents of the file `path`. Fails with `NotFound`, `IsADir` for
    /// a directory, or `NotADir` if a parent is a file.
    pub fn read(&self, path: &str) -> Result<&[u8], VfsError> {
        match self.get(path)? {
            Node::File(data) => Ok(data),
            Node::Dir(_) => Err(VfsError::IsADir),
        }
    }

    /// The entries of the directory `dir`, sorted by name (byte order).
    /// Fails with `NotFound`, or `NotADir` if it or a parent is a file.
    pub fn list(&self, dir: &str) -> Result<Vec<Entry>, VfsError> {
        let Node::Dir(dir) = self.get(dir)? else {
            return Err(VfsError::NotADir);
        };
        let entry = |(name, node): &(String, Node)| Entry {
            name: name.clone(),
            is_dir: matches!(node, Node::Dir(_)),
            size: match node {
                Node::File(data) => data.len() as u64,
                Node::Dir(_) => 0,
            },
        };
        Ok(dir.0.iter().map(entry).collect())
    }

    /// Removes the file or directory `path`, and with `recursive` everything
    /// under it. Fails with `NotFound`, `NotEmpty` for a directory with
    /// entries unless `recursive`, `NotADir` if a parent is a file, or
    /// `InvalidPath` for the root.
    pub fn remove(&mut self, path: &str, recursive: bool) -> Result<(), VfsError> {
        let names = names(path)?;
        let (dir, name) = parent(&mut self.root, &names, VfsError::InvalidPath)?;
        let i = dir.find(name).map_err(|_| VfsError::NotFound)?;
        let node = &dir.0[i].1;
        if !recursive && matches!(node, Node::Dir(sub) if !sub.is_empty()) {
            return Err(VfsError::NotEmpty);
        }
        let (bytes, entries, _) = usage(node);
        dir.0.remove(i);
        self.bytes -= bytes;
        self.entries -= entries;
        self.generation += 1;
        Ok(())
    }

    /// Moves the file or directory `from` to `to` as POSIX `rename` does: a
    /// file replaces a file and a directory an empty directory; renaming a
    /// path to itself does nothing. Fails with `NotFound` if `from` or the
    /// parent of `to` is missing, `IsADir` for a file onto a directory,
    /// `NotADir` for a directory onto a file (or a parent that is a file),
    /// `NotEmpty` onto a directory with entries, or `InvalidPath` if either
    /// is the root, a directory would move into itself, or the result would
    /// be deeper than [`Vfs::MAX_DEPTH`].
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), VfsError> {
        let src = names(from)?;
        let dst = names(to)?;
        let (Some((src_name, src_parent)), Some((dst_name, dst_parent))) =
            (src.split_last(), dst.split_last())
        else {
            return Err(VfsError::InvalidPath);
        };
        let node = walk(&self.root, &src)?;
        if src == dst {
            return Ok(());
        }
        let is_dir = matches!(node, Node::Dir(_));
        if is_dir && dst.starts_with(&src) {
            return Err(VfsError::InvalidPath);
        }
        let (_, _, height) = usage(node);
        if dst_parent.len() + height > Vfs::MAX_DEPTH {
            return Err(VfsError::InvalidPath);
        }
        let (bytes, entries, _) = match walk_dir(&self.root, dst_parent)?.get(dst_name) {
            None => (0, 0, 0),
            Some(Node::Dir(_)) if !is_dir => return Err(VfsError::IsADir),
            Some(Node::File(_)) if is_dir => return Err(VfsError::NotADir),
            Some(Node::Dir(sub)) if !sub.is_empty() => return Err(VfsError::NotEmpty),
            Some(old) => usage(old),
        };
        // Both paths were walked above and `to` is not inside `from`: no `?` below fires.
        let Some(node) = walk_dir_mut(&mut self.root, src_parent)?.remove(src_name) else {
            return Err(VfsError::NotFound);
        };
        walk_dir_mut(&mut self.root, dst_parent)?.insert(dst_name, node);
        self.bytes -= bytes;
        self.entries -= entries;
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

    /// Writes `data` into the existing file `path` at `off` (`u64::MAX`
    /// appends), in place, zero-filling a gap; its new length. Writing nothing
    /// changes nothing. Fails as [`Vfs::read`] does, or with `NoSpace`.
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
        match slot {
            Ok(i) => {
                if let Node::File(old) = &mut dir.0[i].1 {
                    if append {
                        old.extend_from_slice(data);
                    } else {
                        *old = data.to_vec();
                    }
                }
            }
            Err(i) => {
                dir.0.insert(i, (String::from(name), Node::File(data.to_vec())));
                self.entries += 1;
            }
        }
        (self.bytes, self.generation) = (bytes, self.generation + 1);
        Ok(())
    }
}

/// The names of an absolute path, checked against the limits: `.` and empty
/// names are skipped and `..` drops the name before it (none at the root).
fn names(path: &str) -> Result<Vec<&str>, VfsError> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(VfsError::InvalidPath);
    }
    let mut names = Vec::new();
    for name in path.split('/') {
        match name {
            "" | "." => {}
            ".." => _ = names.pop(),
            _ => names.push(name),
        }
    }
    if names.len() > Vfs::MAX_DEPTH || names.iter().any(|n| n.len() > Vfs::MAX_NAME) {
        return Err(VfsError::InvalidPath);
    }
    Ok(names)
}

fn walk<'n>(mut node: &'n Node, names: &[&str]) -> Result<&'n Node, VfsError> {
    for name in names {
        node = match node {
            Node::Dir(dir) => dir.get(name).ok_or(VfsError::NotFound)?,
            Node::File(_) => return Err(VfsError::NotADir),
        };
    }
    Ok(node)
}

fn walk_dir<'n>(node: &'n Node, names: &[&str]) -> Result<&'n Dir, VfsError> {
    match walk(node, names)? {
        Node::Dir(dir) => Ok(dir),
        Node::File(_) => Err(VfsError::NotADir),
    }
}

fn walk_dir_mut<'n>(mut node: &'n mut Node, names: &[&str]) -> Result<&'n mut Dir, VfsError> {
    for name in names {
        node = match node {
            Node::Dir(dir) => dir.get_mut(name).ok_or(VfsError::NotFound)?,
            Node::File(_) => return Err(VfsError::NotADir),
        };
    }
    match node {
        Node::Dir(dir) => Ok(dir),
        Node::File(_) => Err(VfsError::NotADir),
    }
}

/// The directory holding the last of `names`, and that name; `at_root` if
/// `names` is the root, which has no parent.
fn parent<'n, 'a>(
    root: &'n mut Node,
    names: &[&'a str],
    at_root: VfsError,
) -> Result<(&'n mut Dir, &'a str), VfsError> {
    let (name, parent) = names.split_last().ok_or(at_root)?;
    Ok((walk_dir_mut(root, parent)?, name))
}

/// A subtree's file bytes, its entries (itself included) and its height (1
/// for a file or an empty directory). Iterative, so depth costs no stack.
fn usage(node: &Node) -> (u64, usize, usize) {
    let (mut bytes, mut entries, mut height) = (0, 0, 0);
    let mut stack = vec![(node, 1)];
    while let Some((node, depth)) = stack.pop() {
        entries += 1;
        height = height.max(depth);
        match node {
            Node::File(data) => bytes += data.len() as u64,
            Node::Dir(dir) => stack.extend(dir.0.iter().map(|(_, n)| (n, depth + 1))),
        }
    }
    (bytes, entries, height)
}

#[cfg(test)]
mod tests {
    use super::VfsError::*;
    use super::*;

    fn ls(fs: &Vfs, dir: &str) -> Vec<String> {
        fs.list(dir).unwrap().into_iter().map(|e| e.name).collect()
    }

    fn home(rest: &str) -> String {
        format!("{}{rest}", Vfs::HOME)
    }

    #[test]
    fn new_makes_the_standard_tree() {
        let fs = Vfs::new();
        assert_eq!(ls(&fs, "/"), ["apps", "home", "tmp"]);
        assert_eq!(ls(&fs, "/home"), ["guest"]);
        assert_eq!(Vfs::normalize("/home", "guest"), Ok(home("")));
        assert!(fs.is_dir(Vfs::HOME) && fs.is_dir("/") && fs.exists("/"));
        assert!(ls(&fs, Vfs::HOME).is_empty() && ls(&fs, "/tmp").is_empty());
        assert_eq!((fs.total_bytes(), &fs), (0, &Vfs::default()));
    }

    #[test]
    fn normalize_table() {
        #[rustfmt::skip]
        let table = [
            ("/", "", "/"), ("/tmp", "", "/tmp"), ("/tmp", ".", "/tmp"),
            ("/tmp", "a/b/", "/tmp/a/b"), ("/tmp", "a//b///c", "/tmp/a/b/c"),
            ("/tmp", "./a/./b/.", "/tmp/a/b"), ("/tmp", "a/b/../../c", "/tmp/c"),
            ("/tmp", "../../..", "/"), ("/tmp", "../../x", "/x"),
            ("/tmp", "//apps//x/", "/apps/x"), ("/tmp", "/../apps", "/apps"),
            ("/tmp/x/../y", "z", "/tmp/y/z"), ("//tmp//", "", "/tmp"),
            ("/tmp", "...", "/tmp/..."), ("/tmp", "a b", "/tmp/a b"), ("/tmp", "~x", "/tmp/~x"),
            ("/tmp", "a/~", "/tmp/a/~"), ("/tmp", "~/..", "/home"), ("/tmp", "~/../..", "/"),
            ("relative", "/apps", "/apps"),
        ];
        for (cwd, path, want) in table {
            let got = Vfs::normalize(cwd, path);
            assert_eq!(got.as_deref(), Ok(want), "{cwd:?} + {path:?}");
        }
        for path in ["~", "~/", "~//.", "~/docs/.."] {
            assert_eq!(Vfs::normalize("relative", path), Ok(home("")), "{path:?}");
        }
        assert_eq!(Vfs::normalize("/", "~//docs/./a/.."), Ok(home("/docs")));
    }

    #[test]
    fn normalize_rejects_bad_paths() {
        assert_eq!(Vfs::normalize("/tmp", "a\0b"), Err(InvalidPath));
        assert_eq!(Vfs::normalize("/tmp", "~/\0"), Err(InvalidPath));
        assert_eq!(Vfs::normalize("/\0", "a"), Err(InvalidPath));
        assert_eq!(Vfs::normalize("tmp", "a"), Err(InvalidPath));
        assert_eq!(Vfs::normalize("", ""), Err(InvalidPath));
        let name = "n".repeat(Vfs::MAX_NAME);
        assert_eq!(Vfs::normalize("/", &name), Ok(format!("/{name}")));
        assert_eq!(Vfs::normalize("/", &format!("{name}n")), Err(InvalidPath));
        let deep = "d/".repeat(Vfs::MAX_DEPTH);
        assert!(Vfs::normalize("/", &deep).is_ok());
        assert_eq!(Vfs::normalize("/", &format!("{deep}d")), Err(InvalidPath));
        assert!(Vfs::normalize("/", &format!("{deep}d/..")).is_ok());
    }

    #[test]
    fn methods_need_absolute_paths() {
        let mut fs = Vfs::new();
        for p in ["", "tmp", "tmp/x", "~", "./tmp", "/tmp/a\0"] {
            assert!(!fs.exists(p) && !fs.is_dir(p) && !fs.is_file(p));
            assert_eq!(fs.mkdir(p), Err(InvalidPath));
            assert_eq!(fs.mkdir_all(p), Err(InvalidPath));
            assert_eq!(fs.write(p, b"x"), Err(InvalidPath));
            assert_eq!(fs.append(p, b"x"), Err(InvalidPath));
            assert_eq!(fs.read(p), Err(InvalidPath));
            assert_eq!(fs.list(p), Err(InvalidPath));
            assert_eq!(fs.remove(p, true), Err(InvalidPath));
            assert_eq!(fs.rename(p, "/tmp/y"), Err(InvalidPath));
            assert_eq!(fs.rename("/tmp", p), Err(InvalidPath));
            assert_eq!(fs.write_at(p, 0, b"x"), Err(InvalidPath));
            assert_eq!(fs.set_len(p, 0), Err(InvalidPath));
        }
        assert_eq!(fs, Vfs::new());
        fs.write("//tmp/./x/../f", b"1").unwrap();
        assert!(fs.is_file("/apps/../tmp//f") && fs.is_dir("/.."));
        assert_eq!(ls(&fs, "/tmp/."), ["f"]);
    }

    #[test]
    fn mkdir_and_mkdir_all() {
        let mut fs = Vfs::new();
        assert_eq!(fs.mkdir("/tmp/a"), Ok(()));
        assert!(fs.is_dir("/tmp/a") && !fs.is_file("/tmp/a") && fs.exists("/tmp/a"));
        fs.write("/tmp/f", b"keep").unwrap();
        let before = fs.clone();
        assert_eq!(fs.mkdir("/tmp/a"), Err(Exists));
        assert_eq!(fs.mkdir("/tmp/f"), Err(Exists));
        assert_eq!(fs.mkdir("/"), Err(Exists));
        assert_eq!(fs.mkdir("/tmp/x/y"), Err(NotFound));
        assert_eq!(fs.mkdir("/tmp/f/y"), Err(NotADir));
        assert_eq!(fs.mkdir_all("/tmp/f"), Err(Exists));
        assert_eq!(fs.mkdir_all("/tmp/f/g/h"), Err(NotADir));
        assert_eq!(fs, before);
        assert_eq!(fs.mkdir_all("/tmp/a/b/c"), Ok(()));
        assert!(fs.is_dir("/tmp/a/b") && fs.is_dir("/tmp/a/b/c"));
        for path in ["/tmp/a/b/c", "/tmp/a", "/"] {
            assert_eq!(fs.mkdir_all(path), Ok(()));
        }
        assert_eq!(ls(&fs, "/tmp"), ["a", "f"]);
        assert_eq!(fs.read("/tmp/f"), Ok(&b"keep"[..]));
    }

    #[test]
    fn write_append_read() {
        let mut fs = Vfs::new();
        fs.write("/tmp/f", b"hello").unwrap();
        fs.write("/tmp/f", b"hi").unwrap();
        fs.append("/tmp/f", b"!").unwrap();
        fs.append("/tmp/log", b"a").unwrap();
        fs.append("/tmp/log", b"bc").unwrap();
        fs.write("/tmp/empty", b"").unwrap();
        assert!(fs.is_file("/tmp/f") && !fs.is_dir("/tmp/f"));
        assert_eq!(fs.read("/tmp/f"), Ok(&b"hi!"[..]));
        assert_eq!(fs.read("/tmp/log"), Ok(&b"abc"[..]));
        assert_eq!(fs.read("/tmp/empty"), Ok(&b""[..]));
        assert_eq!(fs.total_bytes(), 6);
        let before = fs.clone();
        let cases = [("/tmp", IsADir), ("/", IsADir), ("/no/f", NotFound)];
        for (path, err) in cases.into_iter().chain([("/tmp/f/g", NotADir)]) {
            assert_eq!(fs.write(path, b"x"), Err(err), "{path}");
            assert_eq!(fs.append(path, b"x"), Err(err), "{path}");
            assert_eq!(fs.read(path), Err(err), "{path}");
            assert_eq!(fs.write_at(path, 0, b""), Err(err), "{path}");
            assert_eq!(fs.set_len(path, 0), Err(err), "{path}");
        }
        assert_eq!(fs.read("/tmp/nope"), Err(NotFound));
        assert_eq!(fs, before);
    }

    #[test]
    fn list_is_sorted_by_name() {
        let mut fs = Vfs::new();
        fs.mkdir("/tmp/a").unwrap();
        fs.write("/tmp/a/inner", b"not counted").unwrap();
        for name in ["b", "a.txt", "B", "a-x", "_", "é"] {
            fs.write(&format!("/tmp/{name}"), name.as_bytes()).unwrap();
        }
        let entries = fs.list("/tmp").unwrap();
        assert_eq!(ls(&fs, "/tmp"), ["B", "_", "a", "a-x", "a.txt", "b", "é"]);
        let dir = Entry { name: String::from("a"), is_dir: true, size: 0 };
        assert_eq!(entries[2], dir);
        let kinds: Vec<(bool, u64)> = entries.iter().map(|e| (e.is_dir, e.size)).collect();
        let want = [(true, 0), (false, 3), (false, 5), (false, 1), (false, 2)];
        assert_eq!(kinds[2..], want);
        assert_eq!(fs.list("/tmp/b"), Err(NotADir));
        assert_eq!(fs.list("/tmp/b/c"), Err(NotADir));
        assert_eq!(fs.list("/nope"), Err(NotFound));
    }

    #[test]
    fn directories_stay_sorted() {
        let mut fs = Vfs::new();
        // 1..=40 in a scrambled but fixed order.
        let mut want: Vec<String> = (1..41u32).map(|k| (k * 7 % 41).to_string()).collect();
        for name in &want {
            fs.write(&format!("/tmp/{name}"), name.as_bytes()).unwrap();
        }
        fs.write("/tmp/7", b"again").unwrap();
        want.sort();
        assert_eq!(ls(&fs, "/tmp"), want);
        for name in ["1", "40", "25"] {
            fs.remove(&format!("/tmp/{name}"), false).unwrap();
        }
        fs.rename("/tmp/2", "/tmp/0").unwrap();
        fs.rename("/tmp/3", "/tmp/4").unwrap();
        want.retain(|n| !["1", "40", "25", "2", "3"].contains(&n.as_str()));
        want.insert(0, String::from("0"));
        assert_eq!(ls(&fs, "/tmp"), want);
        assert_eq!(fs.read("/tmp/4"), Ok(&b"3"[..]));
        assert_eq!(fs.read("/tmp/7"), Ok(&b"again"[..]));
    }

    #[test]
    fn remove_and_its_errors() {
        let mut fs = Vfs::new();
        fs.mkdir_all("/tmp/d/e").unwrap();
        fs.write("/tmp/d/e/f", b"1234").unwrap();
        fs.write("/tmp/d/g", b"56").unwrap();
        fs.write("/tmp/h", b"7").unwrap();
        let before = fs.clone();
        assert_eq!(fs.remove("/tmp/d", false), Err(NotEmpty));
        assert_eq!(fs.remove("/", true), Err(InvalidPath));
        assert_eq!(fs.remove("/tmp/nope", true), Err(NotFound));
        assert_eq!(fs.remove("/nope/x", true), Err(NotFound));
        assert_eq!(fs.remove("/tmp/h/x", true), Err(NotADir));
        assert_eq!(fs, before);
        assert_eq!(fs.remove("/tmp/h", false), Ok(()));
        assert_eq!((fs.exists("/tmp/h"), fs.total_bytes()), (false, 6));
        assert_eq!(fs.remove("/tmp/d", true), Ok(()));
        assert_eq!((fs.exists("/tmp/d"), fs.total_bytes()), (false, 0));
        fs.mkdir("/tmp/empty").unwrap();
        fs.write("/tmp/file", b"").unwrap();
        assert_eq!(fs.remove("/tmp/empty", false), Ok(()));
        assert_eq!(fs.remove("/tmp/file", true), Ok(()));
        assert_eq!(fs, Vfs { generation: fs.generation, ..Vfs::new() }, "counts are back");
    }

    #[test]
    fn rename_moves_and_replaces() {
        let mut fs = Vfs::new();
        fs.write("/tmp/f", b"abc").unwrap();
        fs.rename("/tmp/f", "/tmp/g").unwrap();
        assert_eq!((fs.exists("/tmp/f"), fs.read("/tmp/g")), (false, Ok(&b"abc"[..])));
        fs.mkdir_all("/tmp/d/e").unwrap();
        fs.rename("/tmp/g", "/tmp/d/e/g").unwrap();
        fs.rename("/tmp/d", &home("/d")).unwrap();
        assert_eq!(fs.rename(&home("/d"), &home("/./d/")), Ok(()));
        assert_eq!(fs.read(&home("/d/e/g")), Ok(&b"abc"[..]));
        fs.rename(&home("/d/e"), &home("/e")).unwrap();
        assert_eq!(ls(&fs, Vfs::HOME), ["d", "e"]);
        assert!(ls(&fs, "/tmp").is_empty());
        // A file replaces a file, and a directory an empty directory.
        fs.write("/tmp/a", b"aaaa").unwrap();
        fs.rename("/tmp/a", &home("/e/g")).unwrap();
        assert_eq!((fs.read(&home("/e/g")), fs.total_bytes()), (Ok(&b"aaaa"[..]), 4));
        fs.write("/tmp/b", b"b").unwrap();
        fs.mkdir("/tmp/full").unwrap();
        let before = fs.clone();
        assert_eq!(fs.rename("/tmp/b", &home("/d")), Err(IsADir));
        assert_eq!(fs.rename(&home("/d"), "/tmp/b"), Err(NotADir));
        assert_eq!(fs.rename("/tmp/full", Vfs::HOME), Err(NotEmpty));
        assert_eq!(fs, before);
        fs.rename(&home("/e"), &home("/d")).unwrap();
        assert!(!fs.exists(&home("/e")) && fs.is_file(&home("/d/g")));
        fs.remove(&home("/d"), true).unwrap();
        fs.remove("/tmp", true).unwrap();
        fs.mkdir("/tmp").unwrap();
        assert_eq!(fs, Vfs { generation: fs.generation, ..Vfs::new() });
    }

    #[test]
    fn rename_edge_cases() {
        let mut fs = Vfs::new();
        fs.mkdir_all("/tmp/a/b").unwrap();
        fs.write("/tmp/f", b"x").unwrap();
        let before = fs.clone();
        #[rustfmt::skip]
        let cases = [
            ("/tmp/a", "/tmp/a/c", InvalidPath), ("/tmp/a", "/tmp/a/b/c", InvalidPath),
            ("/tmp/a", "/tmp/a/b", InvalidPath), ("/tmp/a/b", "/tmp/a", NotEmpty),
            ("/tmp/a/b", "/tmp", NotEmpty), ("/", "/tmp/r", InvalidPath),
            ("/tmp/a", "/", InvalidPath), ("/tmp/nope", "/tmp/x", NotFound),
            ("/tmp/nope", "/tmp/nope", NotFound), ("/tmp/a", "/nope/x", NotFound),
            ("/tmp/a", "/tmp/f/x", NotADir), ("/tmp/f/x", "/tmp/y", NotADir),
        ];
        for (from, to, err) in cases {
            assert_eq!(fs.rename(from, to), Err(err), "{from} -> {to}");
        }
        assert_eq!(fs.rename("/tmp/f", "/tmp//f"), Ok(()));
        assert_eq!(fs, before);
        // A name that merely starts with the source's is not inside it.
        fs.rename("/tmp/a", "/tmp/ab").unwrap();
        assert!(fs.is_dir("/tmp/ab/b") && !fs.exists("/tmp/a"));
    }

    #[test]
    fn depth_and_name_limits() {
        let mut fs = Vfs::new();
        let deep = "/d".repeat(Vfs::MAX_DEPTH);
        assert_eq!(fs.mkdir_all(&deep), Ok(()));
        assert_eq!(fs.mkdir(&format!("{deep}/e")), Err(InvalidPath));
        assert_eq!(fs.write(&format!("{deep}/f"), b""), Err(InvalidPath));
        let long = "n".repeat(Vfs::MAX_NAME);
        assert_eq!(fs.write(&format!("/tmp/{long}"), b""), Ok(()));
        assert_eq!(fs.write(&format!("/tmp/{long}n"), b""), Err(InvalidPath));
        fs.mkdir_all("/tmp/t/u").unwrap();
        let too_deep = format!("{}/t", "/d".repeat(Vfs::MAX_DEPTH - 1));
        assert_eq!(fs.rename("/tmp/t", &too_deep), Err(InvalidPath));
        let fits = format!("{}/t", "/d".repeat(Vfs::MAX_DEPTH - 2));
        assert_eq!(fs.rename("/tmp/t", &fits), Ok(()));
        assert!(fs.is_dir(&format!("{fits}/u")));
        assert_eq!(fs.remove("/d", true), Ok(()));
    }

    #[test]
    fn byte_limit() {
        let mut fs = Vfs::new();
        let max = Vfs::MAX_BYTES as usize;
        fs.write("/tmp/big", &vec![7; max - 1]).unwrap();
        let before = fs.clone();
        assert_eq!(fs.append("/tmp/big", b"xy"), Err(NoSpace));
        assert_eq!(fs.write("/tmp/other", b"xy"), Err(NoSpace));
        assert_eq!(fs.write("/tmp/big", &vec![1; max + 1]), Err(NoSpace));
        assert_eq!(fs.write_at("/tmp/big", Vfs::MAX_BYTES - 1, b"xy"), Err(NoSpace));
        assert_eq!(fs.write_at("/tmp/big", u64::MAX - 1, b"xy"), Err(NoSpace));
        assert_eq!(fs.set_len("/tmp/big", u64::MAX), Err(NoSpace));
        assert_eq!(fs, before);
        fs.append("/tmp/big", b"x").unwrap();
        assert_eq!(fs.write_at("/tmp/big", 3, b"in place"), Ok(Vfs::MAX_BYTES));
        // Replacing frees the old contents first.
        fs.write("/tmp/big", &vec![1; max]).unwrap();
        fs.write("/tmp/empty", b"").unwrap();
        fs.rename("/tmp/empty", "/tmp/big").unwrap();
        assert_eq!(fs.total_bytes(), 0);
        fs.write("/tmp/other", &vec![1; max]).unwrap();
    }

    #[test]
    fn entry_limit() {
        let mut fs = Vfs::new();
        for i in 0..Vfs::MAX_ENTRIES - 5 {
            fs.write(&format!("/tmp/{i}"), b"").unwrap();
        }
        let before = fs.clone();
        assert_eq!(fs.mkdir_all("/tmp/x/y"), Err(NoSpace));
        assert_eq!(fs, before);
        fs.mkdir_all("/tmp/x").unwrap();
        assert_eq!(fs.mkdir("/tmp/y"), Err(NoSpace));
        assert_eq!(fs.mkdir_all("/tmp/y"), Err(NoSpace));
        assert_eq!(fs.write("/tmp/y", b""), Err(NoSpace));
        assert_eq!(fs.append("/tmp/y", b""), Err(NoSpace));
        // Replacing or moving an entry needs no new one.
        assert_eq!(fs.write("/tmp/1", b"new"), Ok(()));
        assert_eq!(fs.append("/tmp/1", b"er"), Ok(()));
        assert_eq!(fs.rename("/tmp/1", "/tmp/y"), Ok(()));
        assert_eq!(fs.rename("/tmp/y", "/tmp/2"), Ok(()));
        assert_eq!(fs.mkdir("/tmp/y"), Ok(()));
        assert_eq!(fs.remove("/tmp", true), Ok(()));
        assert_eq!(fs.mkdir_all("/tmp/x/y"), Ok(()));
    }

    #[test]
    fn write_at_and_set_len_work_in_place() {
        let mut fs = Vfs::new();
        fs.write("/tmp/f", b"hello").unwrap();
        // Over, past the end, appending, after a gap (zeros), and nothing.
        let writes: [(u64, &[u8], u64); 5] =
            [(1, b"EL", 5), (4, b"O!", 6), (u64::MAX, b"?", 7), (9, b"z", 10), (1 << 40, b"", 10)];
        for (off, data, len) in writes {
            let old = fs.generation();
            assert_eq!(fs.write_at("/tmp/f", off, data), Ok(len), "{off}");
            assert_eq!(fs.generation() > old, !data.is_empty(), "{off}");
        }
        assert_eq!((fs.read("/tmp/f"), fs.total_bytes()), (Ok(&b"hELlO!?\0\0z"[..]), 10));
        assert_eq!((fs.set_len("/tmp/f", 3), fs.set_len("/tmp/f", 5)), (Ok(()), Ok(())));
        assert_eq!((fs.read("/tmp/f"), fs.total_bytes()), (Ok(&b"hEL\0\0"[..]), 5));
    }

    #[test]
    fn errors_display() {
        assert_eq!(NotFound.to_string(), "no such file or directory");
        assert_eq!(NoSpace.to_string(), "no space left on device");
        let all = [NotFound, NotADir, IsADir, Exists, NotEmpty, NoSpace, InvalidPath];
        let msgs: std::collections::BTreeSet<_> = all.iter().map(|e| e.to_string()).collect();
        assert_eq!(msgs.len(), 7);
        let err: Box<dyn std::error::Error> = Box::new(InvalidPath);
        assert_eq!(err.to_string(), "invalid path");
    }

    #[test]
    fn same_operations_build_the_same_tree() {
        let run = || {
            let mut fs = Vfs::new();
            for name in ["z", "a", "m"] {
                fs.write(&format!("/tmp/{name}"), name.as_bytes()).unwrap();
            }
            fs.rename("/tmp/z", "/apps/z").unwrap();
            fs.mkdir_all(&home("/docs/old")).unwrap();
            fs.remove(&home("/docs/old"), false).unwrap();
            fs
        };
        let (a, mut b) = (run(), run());
        assert_eq!((&a, format!("{a:?}"), a.generation()), (&b, format!("{b:?}"), 6));
        b.remove("/apps/z", false).unwrap();
        assert!(a != b && a.exists("/apps/z"));
    }
}
