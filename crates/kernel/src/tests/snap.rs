use vfs::{Vfs, VfsError};

use crate::snap::{SnapError, fnv64, give, take};
use crate::wire::Writer;

/// A path in the guest's home (built, so no home path shows in the source).
fn at(rel: &str) -> String {
    [Vfs::HOME, "/", rel].concat()
}

/// A snapshot of `entries` (kind, path, bytes) with version `v`, summed.
fn made(v: u8, entries: &[(u8, &str, &[u8])]) -> Vec<u8> {
    let mut w = Writer::default().bytes(b"CSHM").u8(v).u8(0).u16(0).u64(9);
    w = w.u32(entries.len() as u32);
    for &(kind, path, data) in entries {
        w = w.u8(kind).str(path);
        if kind == 1 {
            w = w.u32(data.len() as u32).bytes(data);
        }
    }
    let sum = fnv64(&w.0);
    w.u64(sum).done()
}

fn home() -> Vfs {
    let mut fs = Vfs::new();
    fs.mkdir_all(&at("apps")).unwrap();
    fs.mkdir_all(&at("empty")).unwrap();
    fs.write(&at("apps/tetris.app"), b"state n = 0;\nlabel n;\n").unwrap();
    fs.write(&at("raw"), &[0, 255, 10, 128]).unwrap();
    fs.write(&at("../note"), b"top").unwrap();
    fs.write("/tmp/scratch", b"not kept").unwrap();
    fs
}

#[test]
fn a_snapshot_puts_home_back_and_only_home() {
    let fs = home();
    let snap = take(&fs, 41);
    let mut back = Vfs::new();
    assert_eq!(give(&mut back, &snap), Ok(41));
    for path in ["apps/tetris.app", "raw", "../note"].map(at) {
        let path = Vfs::normalize("/", &path).unwrap();
        assert_eq!(back.read(&path), fs.read(&path), "{path}");
    }
    assert!(back.is_dir(&at("empty")) && !back.exists("/tmp/scratch"));
    assert_eq!(take(&back, 41), snap, "the same tree, the same bytes");
    let first = &snap[16..];
    assert_eq!(&first[..4], [6, 0, 0, 0], "six entries: guest, its four, note");
    assert_eq!(&first[4..12], b"\x00\x05\x00guest");
    let mut empty = Vfs::new();
    let none = take(&Vfs::new(), 0);
    assert_eq!((give(&mut empty, &none), take(&empty, 0)), (Ok(0), none));
}

#[test]
fn damage_and_newer_snapshots_change_nothing() {
    let snap = take(&home(), 1);
    let mut flipped = snap.clone();
    flipped[30] ^= 1;
    let file = |p: &'static str| made(1, &[(1, p, b"x")]);
    let damaged = [
        &[][..],
        &snap[..7],
        &snap[..snap.len() - 1],
        &flipped,
        &made(0, &[]),
        &file("../etc/passwd"),
        &file("/bin/hello"),
        &file(""),
        &made(1, &[(2, "guest", b"")]),
    ];
    for b in damaged {
        let mut fs = Vfs::new();
        assert_eq!(give(&mut fs, b), Err(SnapError::Damaged), "{b:?}");
        assert_eq!(fs, Vfs::new());
    }
    let mut fs = Vfs::new();
    assert_eq!(give(&mut fs, &made(2, &[(1, "guest/x", b"x")])), Err(SnapError::Newer));
    assert_eq!(fs, Vfs::new());
    // An entry the filesystem refuses puts back none, not even those before it.
    fs.write(&at("apps"), b"a file").unwrap();
    let before = fs.clone();
    let clash = made(1, &[(1, "a", b"x"), (0, "guest/apps", b""), (1, "guest/apps/a.app", b"")]);
    assert_eq!(give(&mut fs, &clash), Err(SnapError::Vfs(VfsError::Exists)));
    assert_eq!(fs, before);
}

#[test]
fn names_come_back_as_they_were_written() {
    // `~` is a plain name in /home, not the guest's home: neither overwrites the other.
    let mut fs = Vfs::new();
    fs.write(&at("note"), b"the guest's").unwrap();
    fs.mkdir_all("/home/~/~").unwrap();
    fs.write("/home/~/note", b"the tilde's").unwrap();
    fs.write("/home/~x", b"").unwrap();
    let mut back = Vfs::new();
    assert_eq!(give(&mut back, &take(&fs, 3)), Ok(3));
    assert_eq!(back.read(&at("note")), Ok(&b"the guest's"[..]));
    assert_eq!(back.read("/home/~/note"), Ok(&b"the tilde's"[..]));
    assert!(back.is_dir("/home/~/~") && back.is_file("/home/~x"));
    assert_eq!(take(&back, 3), take(&fs, 3));
    // A file named `~` is a file there too, and the names after it come back.
    let mut fs = Vfs::new();
    let snap = made(1, &[(1, "~", b"t"), (1, "\u{e9}t\u{e9}", b"e")]);
    assert_eq!(give(&mut fs, &snap), Ok(9));
    assert_eq!(fs.read("/home/~"), Ok(&b"t"[..]));
    assert_eq!(fs.read("/home/\u{e9}t\u{e9}"), Ok(&b"e"[..]));
}
