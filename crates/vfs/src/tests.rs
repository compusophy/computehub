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
        assert_eq!(Vfs::normalize(cwd, path).as_deref(), Ok(want), "{cwd:?} + {path:?}");
    }
    for path in ["~", "~/", "~//.", "~/docs/.."] {
        assert_eq!(Vfs::normalize("relative", path), Ok(home("")), "{path:?}");
    }
    assert_eq!(Vfs::normalize("/", "~//docs/./a/.."), Ok(home("/docs")));
}

#[test]
fn normalize_rejects_bad_paths() {
    for (cwd, path) in [("/tmp", "a\0b"), ("/tmp", "~/\0"), ("/\0", "a"), ("tmp", "a"), ("", "")] {
        assert_eq!(Vfs::normalize(cwd, path), Err(InvalidPath), "{cwd:?} + {path:?}");
    }
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
    let (mut fs, e) = (Vfs::new(), Err(InvalidPath));
    for p in ["", "tmp", "tmp/x", "~", "./tmp", "/tmp/a\0"] {
        assert!(!fs.exists(p) && !fs.is_dir(p) && !fs.is_file(p));
        let a = [fs.mkdir(p), fs.mkdir_all(p), fs.write(p, b"x"), fs.append(p, b"x")];
        let b =
            [fs.remove(p, true), fs.rename(p, "/tmp/y"), fs.rename("/tmp", p), fs.set_len(p, 0)];
        let c = [fs.read(p).map(drop), fs.list(p).map(drop), fs.write_at(p, 0, b"x").map(drop)];
        assert_eq!((a, b, c), ([e; 4], [e; 4], [e; 3]), "{p:?}");
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
    let cases = [("/tmp/a", Exists), ("/tmp/f", Exists), ("/", Exists), ("/tmp/x/y", NotFound)];
    for (path, err) in cases.into_iter().chain([("/tmp/f/y", NotADir)]) {
        assert_eq!(fs.mkdir(path), Err(err), "{path}");
    }
    assert_eq!((fs.mkdir_all("/tmp/f"), fs.mkdir_all("/tmp/f/g/h")), (Err(Exists), Err(NotADir)));
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
    let a = [fs.write("/tmp/f", b"hello"), fs.write("/tmp/f", b"hi"), fs.append("/tmp/f", b"!")];
    let b = [fs.append("/tmp/log", b"a"), fs.append("/tmp/log", b"bc")];
    assert_eq!((a, b, fs.write("/tmp/empty", b"")), ([Ok(()); 3], [Ok(()); 2], Ok(())));
    assert!(fs.is_file("/tmp/f") && !fs.is_dir("/tmp/f"));
    assert_eq!(fs.read("/tmp/f"), Ok(&b"hi!"[..]));
    assert_eq!(fs.read("/tmp/log"), Ok(&b"abc"[..]));
    assert_eq!((fs.read("/tmp/empty"), fs.total_bytes()), (Ok(&b""[..]), 6));
    let before = fs.clone();
    let cases = [("/tmp", IsADir), ("/", IsADir), ("/no/f", NotFound)];
    for (path, err) in cases.into_iter().chain([("/tmp/f/g", NotADir)]) {
        let a = [fs.write(path, b"x"), fs.append(path, b"x"), fs.set_len(path, 0)];
        let b = [fs.read(path).map(drop), fs.write_at(path, 0, b"").map(drop)];
        assert_eq!((a, b), ([Err(err); 3], [Err(err); 2]), "{path}");
    }
    assert_eq!(fs.read("/tmp/nope"), Err(NotFound));
    assert_eq!(fs, before);
}

#[test]
fn list_is_sorted_by_name() {
    let mut fs = Vfs::new();
    assert_eq!((fs.mkdir("/tmp/a"), fs.write("/tmp/a/inner", b"not counted")), (Ok(()), Ok(())));
    for name in ["b", "a.txt", "B", "a-x", "_", "é"] {
        fs.write(&format!("/tmp/{name}"), name.as_bytes()).unwrap();
    }
    let entries = fs.list("/tmp").unwrap();
    assert_eq!(ls(&fs, "/tmp"), ["B", "_", "a", "a-x", "a.txt", "b", "é"]);
    assert_eq!(entries[2], Entry { name: String::from("a"), is_dir: true, size: 0 });
    let kinds: Vec<(bool, u64)> = entries.iter().map(|e| (e.is_dir, e.size)).collect();
    assert_eq!(kinds[2..], [(true, 0), (false, 3), (false, 5), (false, 1), (false, 2)]);
    assert_eq!((fs.list("/tmp/b"), fs.list("/tmp/b/c")), (Err(NotADir), Err(NotADir)));
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
    assert_eq!((fs.rename("/tmp/2", "/tmp/0"), fs.rename("/tmp/3", "/tmp/4")), (Ok(()), Ok(())));
    want.retain(|n| !["1", "40", "25", "2", "3"].contains(&n.as_str()));
    want.insert(0, String::from("0"));
    assert_eq!(ls(&fs, "/tmp"), want);
    assert_eq!((fs.read("/tmp/4"), fs.read("/tmp/7")), (Ok(&b"3"[..]), Ok(&b"again"[..])));
}

#[test]
fn remove_and_its_errors() {
    let mut fs = Vfs::new();
    let made = [fs.mkdir_all("/tmp/d/e"), fs.write("/tmp/d/e/f", b"1234")];
    let more = [fs.write("/tmp/d/g", b"56"), fs.write("/tmp/h", b"7")];
    assert_eq!((made, more), ([Ok(()); 2], [Ok(()); 2]));
    let before = fs.clone();
    assert_eq!(fs.remove("/tmp/d", false), Err(NotEmpty));
    let cases = [("/", InvalidPath), ("/tmp/nope", NotFound), ("/nope/x", NotFound)];
    for (path, err) in cases.into_iter().chain([("/tmp/h/x", NotADir)]) {
        assert_eq!(fs.remove(path, true), Err(err), "{path}");
    }
    assert_eq!(fs, before);
    assert_eq!(fs.remove("/tmp/h", false), Ok(()));
    assert_eq!((fs.exists("/tmp/h"), fs.total_bytes()), (false, 6));
    assert_eq!(fs.remove("/tmp/d", true), Ok(()));
    assert_eq!((fs.exists("/tmp/d"), fs.total_bytes()), (false, 0));
    assert_eq!((fs.mkdir("/tmp/empty"), fs.write("/tmp/file", b"")), (Ok(()), Ok(())));
    assert_eq!((fs.remove("/tmp/empty", false), fs.remove("/tmp/file", true)), (Ok(()), Ok(())));
    assert_eq!(fs, Vfs { generation: fs.generation, ..Vfs::new() }, "counts are back");
}

#[test]
fn rename_moves_and_replaces() {
    let mut fs = Vfs::new();
    assert_eq!((fs.write("/tmp/f", b"abc"), fs.rename("/tmp/f", "/tmp/g")), (Ok(()), Ok(())));
    assert_eq!((fs.exists("/tmp/f"), fs.read("/tmp/g")), (false, Ok(&b"abc"[..])));
    assert_eq!((fs.mkdir_all("/tmp/d/e"), fs.rename("/tmp/g", "/tmp/d/e/g")), (Ok(()), Ok(())));
    fs.rename("/tmp/d", &home("/d")).unwrap();
    assert_eq!(fs.rename(&home("/d"), &home("/./d/")), Ok(()));
    assert_eq!(fs.read(&home("/d/e/g")), Ok(&b"abc"[..]));
    fs.rename(&home("/d/e"), &home("/e")).unwrap();
    assert_eq!(ls(&fs, Vfs::HOME), ["d", "e"]);
    assert!(ls(&fs, "/tmp").is_empty());
    // A file replaces a file, and a directory an empty directory.
    assert_eq!((fs.write("/tmp/a", b"aaaa"), fs.rename("/tmp/a", &home("/e/g"))), (Ok(()), Ok(())));
    assert_eq!((fs.read(&home("/e/g")), fs.total_bytes()), (Ok(&b"aaaa"[..]), 4));
    assert_eq!((fs.write("/tmp/b", b"b"), fs.mkdir("/tmp/full")), (Ok(()), Ok(())));
    let before = fs.clone();
    assert_eq!(fs.rename("/tmp/b", &home("/d")), Err(IsADir));
    assert_eq!(fs.rename(&home("/d"), "/tmp/b"), Err(NotADir));
    assert_eq!(fs.rename("/tmp/full", Vfs::HOME), Err(NotEmpty));
    assert_eq!(fs, before);
    fs.rename(&home("/e"), &home("/d")).unwrap();
    assert!(!fs.exists(&home("/e")) && fs.is_file(&home("/d/g")));
    let a = [fs.remove(&home("/d"), true), fs.remove("/tmp", true), fs.mkdir("/tmp")];
    assert_eq!(a, [Ok(()); 3]);
    assert_eq!(fs, Vfs { generation: fs.generation, ..Vfs::new() });
}

#[test]
fn rename_edge_cases() {
    let mut fs = Vfs::new();
    assert_eq!((fs.mkdir_all("/tmp/a/b"), fs.write("/tmp/f", b"x")), (Ok(()), Ok(())));
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
    let (big, e) = ("/tmp/big", Err(NoSpace));
    let a =
        [fs.append(big, b"xy"), fs.write("/tmp/other", b"xy"), fs.write(big, &vec![1; max + 1])];
    let b = [fs.write_at(big, Vfs::MAX_BYTES - 1, b"xy"), fs.write_at(big, u64::MAX - 1, b"xy")];
    assert_eq!((a, b, fs.set_len(big, u64::MAX)), ([e; 3], [Err(NoSpace); 2], e));
    assert_eq!(fs, before);
    fs.append("/tmp/big", b"x").unwrap();
    assert_eq!(fs.write_at("/tmp/big", 3, b"in place"), Ok(Vfs::MAX_BYTES));
    // Replacing frees the old contents first.
    let a = [fs.write("/tmp/big", &vec![1; max]), fs.write("/tmp/empty", b"")];
    assert_eq!((a, fs.rename("/tmp/empty", "/tmp/big")), ([Ok(()); 2], Ok(())));
    assert_eq!(fs.total_bytes(), 0);
    fs.write("/tmp/other", &vec![1; max]).unwrap();
}

#[test]
fn entry_limit() {
    let (mut fs, y) = (Vfs::new(), "/tmp/y");
    for i in 0..Vfs::MAX_ENTRIES - 5 {
        fs.write(&format!("/tmp/{i}"), b"").unwrap();
    }
    let before = fs.clone();
    assert_eq!(fs.mkdir_all("/tmp/x/y"), Err(NoSpace));
    assert_eq!(fs, before);
    fs.mkdir_all("/tmp/x").unwrap();
    let full = [fs.mkdir(y), fs.mkdir_all(y), fs.write(y, b""), fs.append(y, b"")];
    assert_eq!(full, [Err(NoSpace); 4]);
    // Replacing or moving an entry needs no new one.
    let a = [fs.write("/tmp/1", b"new"), fs.append("/tmp/1", b"er"), fs.rename("/tmp/1", y)];
    let b = [fs.rename(y, "/tmp/2"), fs.mkdir(y), fs.remove("/tmp", true)];
    assert_eq!((a, b, fs.mkdir_all("/tmp/x/y")), ([Ok(()); 3], [Ok(()); 3], Ok(())));
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
