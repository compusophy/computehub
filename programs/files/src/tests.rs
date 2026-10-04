use super::*;

/// A disk that refuses all: its folders will not list, its files will not read or write.
struct Refusing;

impl Disk for Refusing {
    fn list(&mut self, _: &str) -> io::Result<Vec<Entry>> {
        Err(io::Error::other("busy"))
    }
    fn read(&mut self, _: &str) -> io::Result<Vec<u8>> {
        Err(ErrorKind::PermissionDenied.into())
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(ErrorKind::PermissionDenied.into())
    }
}

#[test]
fn the_tools_list_read_and_write_from_the_home_coded() {
    let (mut disk, at) = (Mem::default(), |p: &str| [Vfs::HOME, p].concat());
    let d: &mut dyn Disk = &mut disk;
    // A new file in the home asks nothing; its folders are made; listed and read back.
    assert_eq!(asks(d, "notes/a.txt", "hello"), Ok(None));
    assert_eq!(
        write(d, "notes/a.txt", "hello").as_deref(),
        Ok("ok: wrote ~/notes/a.txt (5 bytes, new)")
    );
    let listed = list(d, &at("/notes"));
    assert_eq!(listed.as_deref(), Ok("ok: listed ~/notes (1 entry)\na.txt, 5 bytes"));
    let got = read(d, "~/notes/a.txt");
    assert_eq!(got.as_deref(), Ok("ok: read ~/notes/a.txt (5 bytes):\nhello"));
    // A long file, and a long folder, reach the model clipped, saying so.
    write(d, "long.txt", &"ab".repeat(10_000)).unwrap();
    let got = read(d, "long.txt").unwrap();
    let head = "ok: read ~/long.txt (20000 bytes; clipped to its first 16384):\nabab";
    assert!(got.starts_with(head) && got.ends_with('\u{2026}') && got.len() < 16_500);
    for i in 0..100 {
        write(d, &format!("many/{i:03}{}", "x".repeat(200)), "").unwrap();
    }
    let many = list(d, "many").unwrap();
    assert!(many.starts_with("ok: listed ~/many (100 entries)\n000x") && many.len() <= MAX_READ);
    assert!(many.ends_with("\n(clipped: 24 more)"), "{many}");
    // Replacing a file, or writing outside the home, asks first.
    let q = asks(d, "notes/a.txt", "bye");
    assert_eq!(q, Ok(Some("Replace ~/notes/a.txt (5 bytes) with these 3 bytes?".into())));
    let q = asks(d, "/tmp/x", "hi");
    assert_eq!(q, Ok(Some("Write /tmp/x, outside your home, with these 2 bytes?".into())));
    // Failures, coded: nothing there, a folder for a file and a file for a folder, not text,
    // not a path, the program's devices (a read there may never end), the Assistant's own chats
    // (every conversation's; a listing of the home leaves them out).
    d.write(&at("/raw"), &[0xff, 0]).unwrap();
    d.write(
        &at("/.assistant/chats"),
        b"compusophy chats 1
",
    )
    .unwrap();
    assert!(!list(d, "~").unwrap().contains(".assistant") && list(d, "~").unwrap().contains("raw"));
    let errs = [
        read(d, "nope"),
        read(d, "notes"),
        list(d, "notes/a.txt"),
        asks(d, "notes", "x").map(|_| String::new()),
        read(d, "raw"),
        read(d, "a\0b"),
        read(d, "/dev/events"),
        list(d, "~/../../dev"),
        read(d, "~/.assistant/chats"),
        write(d, &at("/x/../.assistant/chats"), ""),
    ];
    let codes = errs.map(|e| e.unwrap_err().get(..5).unwrap_or_default().to_string());
    let paths = ["E0918"; 4];
    assert_eq!(codes[..6], ["E0925", "E0926", "E0926", "E0926", "E0928", "E0918"]);
    assert_eq!(codes[6..], paths);
    // A disk that refuses: E0927, and a write it cannot check for a file there never goes.
    let r: &mut dyn Disk = &mut Refusing;
    let refused = [read(r, "a.txt"), write(r, "a.txt", "x"), asks(r, "x", "").map(|_| "".into())];
    let denied = Err("E0927: ~/a.txt: permission denied".to_string());
    assert!(refused[0] == denied && refused[1] == denied);
    assert_eq!(refused[2], Err("E0927: ~/x: its folder did not list (busy)".into()));
}
