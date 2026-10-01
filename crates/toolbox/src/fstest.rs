//! `fstest [dir]`: the file system test. In a fresh `fstest.d` under `dir`
//! (default /tmp) it crosses each WASI file call `std::fs` makes (not pread or
//! pwrite: unstable on WASI), `..`, isatty and /dev/winsize, printing a line
//! per check and a count; the exit status is the number of failures.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind::{self, AlreadyExists, NotFound};
use std::io::{self, IsTerminal, Read, Seek, SeekFrom, Write};

/// A check's verdict word (`"ok"` or `"skip"`), or what it found wrong.
type Check = Result<&'static str, String>;
type Test = fn(&Work) -> Check;
type Step = Result<(), String>;

const OK: Check = Ok("ok");

/// The checks in order; each starts from the state the one before it left.
#[rustfmt::skip]
const CHECKS: [(&str, Test); 9] = [
    ("mkdir creat excl", creat), ("write read 150000", write), ("seek", seek),
    ("trunc append set_len", trunc), ("stat isatty", stat), ("readdir paging", readdir),
    ("rename ..", rename), ("unlink rmdir", unlink), ("/dev/winsize", winsize),
];

/// Where the checks run: `dir` is `root/fstest.d`.
struct Work {
    root: String,
    dir: String,
}

impl Work {
    fn at(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }
}

/// Runs each check under `args[1]` (default /tmp), a line each and a count; the failures.
pub fn fstest(args: &[String], out: &mut dyn Write) -> u8 {
    let root = args.get(1).map_or("/tmp", String::as_str).to_owned();
    let w = Work { dir: format!("{root}/fstest.d"), root };
    // A run that was stopped leaves its tree behind.
    let _ = fs::remove_dir_all(&w.dir);
    let mut failed = 0;
    for (name, check) in CHECKS {
        // A failed stdout leaves nowhere to report; the status still counts.
        let _ = match check(&w) {
            Ok(word) => writeln!(out, "{word} {name}"),
            Err(why) => {
                failed += 1;
                writeln!(out, "FAIL {name}: {why}")
            }
        };
    }
    let _ = fs::remove_dir_all(&w.dir);
    let _ = writeln!(out, "fstest: {} checks, {failed} failed", CHECKS.len());
    failed
}

/// `r`'s value, or why `what` failed.
fn io<V>(what: &str, r: io::Result<V>) -> Result<V, String> {
    r.map_err(|e| format!("{what}: {e}"))
}

/// Ok if `r` failed, with `kind` when one is given.
fn fails<V>(what: &str, r: io::Result<V>, kind: Option<ErrorKind>) -> Step {
    match (r, kind) {
        (Ok(_), _) => Err(format!("{what}: worked, want an error")),
        (Err(e), Some(k)) if kind_of(&e) != k => Err(format!("{what}: {e}, want {k:?}")),
        (Err(_), _) => Ok(()),
    }
}

/// `e`'s kind. std on WASI files ENOTEMPTY (55) under no kind; here it is
/// DirectoryNotEmpty, as on every other target.
fn kind_of(e: &io::Error) -> ErrorKind {
    match e.raw_os_error() {
        Some(55) if cfg!(target_os = "wasi") => ErrorKind::DirectoryNotEmpty,
        _ => e.kind(),
    }
}

fn same(what: &str, got: u64, want: u64) -> Step {
    if got == want { Ok(()) } else { Err(format!("{what}: {got}, want {want}")) }
}

/// Ok if `holds`, else that `what` was wanted. Words, not Debug output:
/// Debug formatting would cost the program kilobytes.
fn want(holds: bool, what: &str) -> Step {
    if holds { Ok(()) } else { Err(format!("want {what}")) }
}

/// Ok if the file `path` holds `want`; else its length or the first byte that differs.
fn read(path: &str, want: &[u8]) -> Step {
    let got = io(path, fs::read(path))?;
    match got.iter().zip(want).position(|(a, b)| a != b) {
        None if got.len() == want.len() => Ok(()),
        None => Err(format!("{path}: {} bytes, want {}", got.len(), want.len())),
        Some(i) => Err(format!("{path}: differs at byte {i}")),
    }
}

/// 150,000 bytes: more than two 64 KiB requests.
fn data() -> Vec<u8> {
    (0..150_000u32).map(|i| (i % 251) as u8).collect()
}

fn creat(w: &Work) -> Check {
    io("mkdir", fs::create_dir(&w.dir))?;
    fails("mkdir again", fs::create_dir(&w.dir), Some(AlreadyExists))?;
    fails("mkdir in a missing dir", fs::create_dir(w.at("no/sub")), Some(NotFound))?;
    io("mkdir sub", fs::create_dir(w.at("sub")))?;
    let new = || OpenOptions::new().write(true).create_new(true).open(w.at("a"));
    io("write", io("create", new())?.write_all(b"hello\n"))?;
    fails("create again", new(), Some(AlreadyExists))?;
    fails("open a missing file", File::open(w.at("none")), Some(NotFound))?;
    read(&w.at("a"), b"hello\n")?;
    OK
}

fn write(w: &Work) -> Check {
    io("write", fs::write(w.at("big"), data()))?;
    read(&w.at("big"), &data())?;
    OK
}

fn seek(w: &Work) -> Check {
    let (mut d, mut b, mut tail) = (data(), [0; 10], Vec::new());
    let mut f = io("open", OpenOptions::new().read(true).write(true).open(w.at("big")))?;
    same("seek start", io("seek", f.seek(SeekFrom::Start(70_000)))?, 70_000)?;
    io("read", f.read_exact(&mut b))?;
    want(d.get(70_000..70_010) == Some(&b), "bytes 70000 to 70010 back")?;
    same("seek current", io("seek", f.seek(SeekFrom::Current(-5)))?, 70_005)?;
    fails("seek before 0", f.seek(SeekFrom::Current(-80_000)), None)?;
    same("seek end", io("seek", f.seek(SeekFrom::End(-3)))?, 149_997)?;
    io("read to end", f.read_to_end(&mut tail))?;
    want(d.get(149_997..) == Some(&tail), "the last 3 bytes back")?;
    // In place, then past the end: the gap reads as zeros.
    io("seek", f.seek(SeekFrom::Start(10)))?;
    io("write", f.write_all(b"XYZ"))?;
    io("seek", f.seek(SeekFrom::End(5)))?;
    io("write", f.write_all(b"!"))?;
    d[10..13].copy_from_slice(b"XYZ");
    d.extend(b"\0\0\0\0\0!");
    read(&w.at("big"), &d)?;
    OK
}

fn trunc(w: &Work) -> Check {
    let p = w.at("a");
    let mut f = io("open", OpenOptions::new().write(true).truncate(true).open(&p))?;
    same("length after trunc", io("fstat", f.metadata())?.len(), 0)?;
    io("write", f.write_all(b"short"))?;
    let mut f = io("open", OpenOptions::new().append(true).open(&p))?;
    io("append", f.write_all(b" tail"))?;
    read(&p, b"short tail")?;
    let f = io("open", OpenOptions::new().write(true).open(&p))?;
    io("set_len 2", f.set_len(2))?;
    read(&p, b"sh")?;
    io("set_len 4", f.set_len(4))?;
    read(&p, b"sh\0\0")?;
    OK
}

fn stat(w: &Work) -> Check {
    let f = io("open", File::open(w.at("a")))?;
    same("fstat length", io("fstat", f.metadata())?.len(), 4)?;
    want(!f.is_terminal(), "a file not to be a tty")?;
    let m = io("stat file", fs::metadata(w.at("a")))?;
    want(m.is_file() && !m.is_dir() && m.len() == 4, "a file of 4 bytes")?;
    let m = io("stat dir", fs::metadata(&w.dir))?;
    want(m.is_dir() && !m.is_file(), "a directory")?;
    fails("stat a missing file", fs::metadata(w.at("none")), Some(NotFound))?;
    OK
}

/// 260 files with 243-byte names, and a directory: more than one 64 KiB
/// LIST reply, and many fd_readdir buffers.
fn readdir(w: &Work) -> Check {
    let dir = w.at("many");
    io("mkdir", fs::create_dir(&dir))?;
    // Sorted as made (digits sort before "sub/"), so no sort is linked in.
    let mut names: Vec<String> = (0..260).map(|i| format!("{i:03}{}", "x".repeat(240))).collect();
    for name in &names {
        io("create", File::create(format!("{dir}/{name}")))?;
    }
    io("mkdir", fs::create_dir(format!("{dir}/sub")))?;
    names.push("sub/".into());
    let mut seen = vec![false; names.len()];
    for e in io("readdir", fs::read_dir(&dir))? {
        let e = io("entry", e)?;
        let slash = if io("type", e.file_type())?.is_dir() { "/" } else { "" };
        let name = e.file_name().to_str().unwrap_or("?").to_owned() + slash;
        match names.binary_search(&name) {
            Ok(i) if !seen[i] => seen[i] = true,
            _ => return Err(format!("listed {name}: new, or twice")),
        }
    }
    same("entries missing", seen.iter().filter(|s| !**s).count() as u64, 0)?;
    OK
}

/// Renames a file, onto a file and a directory; then `..` inside the tree
/// and above `root` (on WASI, above the preopen).
fn rename(w: &Work) -> Check {
    io("rename", fs::rename(w.at("a"), w.at("b")))?;
    fails("the old name", fs::metadata(w.at("a")), Some(NotFound))?;
    read(&w.at("b"), b"sh\0\0")?;
    io("write", fs::write(w.at("c"), b"c"))?;
    io("rename onto a file", fs::rename(w.at("c"), w.at("b")))?;
    io("write", fs::write(w.at("sub/f"), b"f"))?;
    io("rename a dir", fs::rename(w.at("sub"), w.at("sub2")))?;
    read(&w.at("sub2/f"), b"f")?;
    fails("rename a missing file", fs::rename(w.at("no"), w.at("x")), Some(NotFound))?;
    read(&w.at("sub2/../b"), b"c")?;
    io("mkdir", fs::create_dir(w.at("sub2/../d")))?;
    io("rmdir", fs::remove_dir(w.at("d")))?;
    let name = w.root.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next();
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        read(&format!("{}/../{name}/fstest.d/b", w.root), b"c")?;
    }
    OK
}

fn unlink(w: &Work) -> Check {
    io("unlink", fs::remove_file(w.at("b")))?;
    fails("stat it", fs::metadata(w.at("b")), Some(NotFound))?;
    fails("unlink again", fs::remove_file(w.at("b")), Some(NotFound))?;
    fails("unlink a dir", fs::remove_file(w.at("sub2")), None)?;
    let full = fs::remove_dir(w.at("sub2"));
    fails("rmdir a full dir", full, Some(ErrorKind::DirectoryNotEmpty))?;
    io("unlink", fs::remove_file(w.at("sub2/f")))?;
    io("rmdir", fs::remove_dir(w.at("sub2")))?;
    io("unlink", fs::remove_file(w.at("big")))?;
    io("rmdir many", fs::remove_dir_all(w.at("many")))?;
    io("rmdir", fs::remove_dir(&w.dir))?;
    fails("stat it", fs::metadata(&w.dir), Some(NotFound))?;
    OK
}

/// `<cols> <rows>\n`, both at least 1 when stdin is a terminal.
fn winsize(_: &Work) -> Check {
    let s = match fs::read_to_string("/dev/winsize") {
        Err(e) if e.kind() == NotFound => return Ok("skip"),
        r => io("read", r)?,
    };
    match size(&s) {
        Some((c, r)) if (c > 0 && r > 0) || !io::stdin().is_terminal() => OK,
        _ => Err(format!("got \"{}\", want \"<cols> <rows>\\n\"", s.trim_end())),
    }
}

fn size(s: &str) -> Option<(u16, u16)> {
    let (cols, rows) = s.strip_suffix('\n')?.split_once(' ')?;
    Some((cols.parse().ok()?, rows.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_print_a_line_each_and_the_status_counts_failures() {
        let root = std::env::temp_dir().join(format!("compusophy-fstest-{}", std::process::id()));
        let root = root.to_str().unwrap();
        let _ = fs::remove_dir_all(root);
        // A tree a stopped run left behind is cleared first.
        fs::create_dir_all(format!("{root}/fstest.d/x")).unwrap();
        let run = |dir: String| {
            let mut out = Vec::new();
            (fstest(&["fstest".into(), dir], &mut out), String::from_utf8(out).unwrap())
        };
        let (status, out) = run(root.into());
        assert_eq!(status, 0, "{out}");
        let mut want: Vec<String> = CHECKS.iter().map(|(name, _)| format!("ok {name}")).collect();
        // No host but compusophyOS has a /dev/winsize.
        want[8] = "skip /dev/winsize".into();
        want.push("fstest: 9 checks, 0 failed".into());
        assert_eq!(out.lines().collect::<Vec<_>>(), want);
        let (status, out) = run(format!("{root}/none"));
        assert_eq!(status, 8, "{out}");
        assert!(out.starts_with("FAIL mkdir creat excl: mkdir: "), "{out}");
        assert_eq!(out.lines().filter(|l| l.starts_with("FAIL ")).count(), 8);
        assert!(out.ends_with("skip /dev/winsize\nfstest: 9 checks, 8 failed\n"));
        // Both runs cleaned up after themselves: the root is empty.
        fs::remove_dir(root).unwrap();
        // `fails` wants an error, and /dev/winsize holds `<cols> <rows>\n`.
        let nf = || Err::<(), _>(io::Error::from(NotFound));
        assert_eq!(fails("f", nf(), Some(NotFound)), Ok(()));
        assert_eq!(fails("f", Ok(()), None), Err("f: worked, want an error".into()));
        assert!(fails("f", nf(), Some(AlreadyExists)).unwrap_err().ends_with("want AlreadyExists"));
        assert_eq!(size("80 24\n"), Some((80, 24)));
        for bad in ["80 24", "80x24\n", "80 \n", "a 24\n", "80 24 1\n", "70000 1\n"] {
            assert_eq!(size(bad), None, "{bad:?}");
        }
    }
}
