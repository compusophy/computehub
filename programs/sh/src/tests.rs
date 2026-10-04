use kernel::wire::{Job, Stdin, Stdout};
use vfs::VfsError;

use super::*;

/// The shell's world in a test: a VFS, and the kernel faked: each job is kept with what was
/// shown before it, and ends with `status` (a stage outside /bin is no program: ENOEXEC).
struct World {
    fs: Vfs,
    shown: String,
    jobs: Vec<Vec<u8>>,
    status: Result<i32, u16>,
}

fn why(e: VfsError) -> Why {
    match e {
        VfsError::NotFound => NOT_FOUND,
        VfsError::NotADir => NOT_A_DIR,
        VfsError::IsADir => IS_A_DIR,
        VfsError::Exists => "file exists",
        VfsError::NotEmpty => "directory not empty",
        VfsError::InvalidPath => "invalid path",
        VfsError::NoSpace => "no space left on device",
    }
}

impl Sys for World {
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Why> {
        self.fs.list(path).map_err(why)
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>, Why> {
        self.fs.read(path).map(<[u8]>::to_vec).map_err(why)
    }
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), Why> {
        if append { self.fs.append(path, data) } else { self.fs.write(path, data) }.map_err(why)
    }
    fn mkdir(&mut self, path: &str, parents: bool) -> Result<(), Why> {
        if parents { self.fs.mkdir_all(path) } else { self.fs.mkdir(path) }.map_err(why)
    }
    fn remove(&mut self, path: &str, all: bool) -> Result<(), Why> {
        self.fs.remove(path, all).map_err(why)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Why> {
        self.fs.rename(from, to).map_err(why)
    }
    fn kind(&mut self, path: &str) -> Option<bool> {
        self.fs.exists(path).then(|| self.fs.is_dir(path))
    }
    fn run(&mut self, shown: &str, job: &[u8]) -> Result<i32, u16> {
        self.shown.push_str(shown);
        self.jobs.push(job.to_vec());
        let job = Job::decode(job).expect("a job the kernel reads");
        if job.stages.iter().any(|s| !s.0.starts_with("/bin/")) {
            return Err(45);
        }
        self.status
    }
}

/// Three files, an app in /apps and the program `/bin/hi`.
fn world() -> World {
    let mut fs = Vfs::new();
    let files = ["/apps/demo.app", "~/counter.app", "~/notes.txt"];
    files.into_iter().for_each(|p| fs.write(&p.replace('~', Vfs::HOME), b"").unwrap());
    fs.mkdir("/bin").and(fs.write("/bin/hi", b"#!wasm bin/toolbox.wasm")).unwrap();
    World { fs, shown: String::new(), jobs: Vec::new(), status: Ok(0) }
}

/// A job as text: `[job cwd stdin|stage...|stdout]`.
fn show(job: &[u8]) -> String {
    let j = Job::decode(job).unwrap();
    let stdin = match j.stdin {
        Stdin::Console => "console".into(),
        Stdin::File(p) => format!("file {p}"),
        Stdin::Pipe => format!("pipe {:?}", String::from_utf8_lossy(j.data)),
    };
    let stdout = match j.stdout {
        Stdout::Console => "console".into(),
        Stdout::File { path, append } => {
            format!("{} {path}", if append { "append" } else { "file" })
        }
        Stdout::Pipe | Stdout::Null => format!("{:?}", j.stdout),
    };
    let stages: Vec<String> =
        j.stages.iter().map(|(p, argv)| format!("{p} {}", argv.join(" "))).collect();
    format!("[job {} {stdin}|{}|{stdout}]", j.cwd, stages.join("|"))
}

/// `s` without CSI sequences, its asks of the Terminal as `[verb arg]` lines.
fn plain(s: &str) -> String {
    let (mut out, mut it) = (String::new(), s.chars());
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.next() {
                Some('[') => _ = it.by_ref().find(|d| d.is_ascii_alphabetic()),
                Some(']') => {
                    let ask: String = it.by_ref().take_while(|&d| d != '\x07').collect();
                    let mut parts = ask.splitn(3, ';').skip(1);
                    out += &format!("[{} {}]\n", parts.next().unwrap(), parts.next().unwrap());
                }
                _ => {}
            },
            c => out.push(c),
        }
    }
    out
}

/// Runs each `$ ` line of `script` in a shell over [`world`]; the transcript (output without
/// escapes, asks and jobs in brackets, home as `~`) must be `script`.
fn transcript(script: &str) -> (Shell, World) {
    let (mut sh, mut w, mut got) = (Shell::default(), world(), String::new());
    for cmd in script.lines().filter_map(|l| l.strip_prefix("$ ")) {
        let jobs = w.jobs.len();
        sh.run(cmd, &mut w);
        got += &format!("$ {cmd}\n{}", plain(&(std::mem::take(&mut w.shown) + &sh.out)));
        sh.out.clear();
        w.jobs[jobs..].iter().for_each(|j| got += &(show(j) + "\n"));
    }
    assert_eq!(got.replace(Vfs::HOME, "~"), script);
    (sh, w)
}

#[test]
fn commands_jobs_redirects_and_errors() {
    let (sh, w) = transcript(
        r#"$ pwd
~
$ mkdir -p a/b c
$ echo hello   world > a/b/f.txt
$ echo more>>a/b/f.txt
$ cd a/b
$ touch .hidden g f.txt
$ ls -a
.hidden  f.txt  g
$ cat f.txt
hello world
more
$ mv g ~/c
$ ls ..
b
$ cd
$ ls
a  c  counter.app  notes.txt
$ rm a
rm: a: is a directory
$ rm -r a
$ rm nope c/g/x
rm: nope: no such file or directory
rm: c/g/x: not a directory
$ cd c/g
cd: c/g: not a directory
$ mv x
usage: mv <from> <to>
$ rm -rz x
rm: unknown option -z
$ frobnicate --now
frobnicate: command not found
$ hi a b > x
[job ~ console|/bin/hi hi a b|file ~/x]
$ hi < notes.txt | hi -n >> x
[job ~ file ~/notes.txt|/bin/hi hi|/bin/hi hi -n|append ~/x]
$ echo abc | hi|hi
[job ~ pipe "abc\n"|/bin/hi hi|/bin/hi hi|console]
$ hi < c
sh: c: is a directory
$ hi < nope
sh: nope: no such file or directory
$ hi > /nope/x
sh: /nope/x: no such file or directory
$ ls | echo
sh: echo: a command reads no pipe
$ cat < notes.txt
sh: cat: a command reads no file
$ ./notes.txt
sh: ./notes.txt: cannot execute
[job ~ console|~/notes.txt ./notes.txt|console]
$ ls /apps > listing
$ cat listing
demo.app
$ echo "a  b" 'c "d"' e\ f "x > y" 'it''s' "q\"\\" '' '\n' "a|b"
a  b c "d" e f x > y its q"\  \n a|b
$ echo "open
sh: unterminated quote
$ echo a >
sh: expected a file name after >
$ echo a <
sh: expected a file name after <
$ a | | b
sh: a | needs a program on each side
$ a |
sh: a | needs a program on each side
$ echo a > x > y
sh: only one > per line
$ open terminal
[open terminal]
$ open /apps/demo.app
[open /apps/demo.app]
$ open launcher
open: launcher: no such app (see 'apps')
$ run counter.app
[open ~/counter.app]
$ run notes.txt
run: notes.txt: not an .app file
$ edit notes.txt
[open editor:~/notes.txt]
$ edit counter.app
[open studio:~/counter.app]
$ apps
studio  assistant  terminal  files  editor  activity  settings  feedback  about  welcome
/apps/demo.app
~/counter.app
$ uname -a
compusophyOS 0.2 wasm32
$ theme
Midnight
Dawn
Mono
$ theme MONO
[theme Mono]
$ theme sepia
theme: sepia: no such theme (see 'theme')
$ theme a b
usage: theme [name]
$ whoami
guest
$ exit
"#,
    );
    assert!(sh.quit);
    // A redirect's file is made before the program runs (`>>` keeps what it held), and a
    // shown file never asks the Terminal for anything; a file it writes keeps the escape.
    assert_eq!(w.fs.read(&[Vfs::HOME, "/x"].concat()), Ok(&b""[..]));
    let (mut sh, mut w) = (Shell::default(), world());
    w.fs.write("/tmp/ask", b"\x1b]1729;open;studio\x07").unwrap();
    sh.run("cat /tmp/ask", &mut w);
    sh.run("cat /tmp/ask > /tmp/copy", &mut w);
    assert_eq!(sh.out, "^[]1729;open;studio\x07");
    assert_eq!(w.fs.read("/tmp/copy"), Ok(&b"\x1b]1729;open;studio\x07"[..]));
}

#[test]
fn the_line_editor_takes_a_terminals_keys() {
    let (mut sh, mut w) = (Shell::default(), world());
    let typed = |sh: &mut Shell, w: &mut World, keys: &str| {
        sh.feed(keys.as_bytes(), w);
        plain(&std::mem::take(&mut sh.out))
    };
    // Arrows, Home, Delete and Backspace edit the line; Enter runs it, then a prompt.
    let out = typed(&mut sh, &mut w, "Xecho acd\x1b[D\x1b[Db\x01\x1b[3~\x05!\x7f\r");
    assert!(out.ends_with("$ echo abcd\r\nabcd\nguest@compusophy:~$ \r"), "{out:?}");
    // History: Up and Down (SS3 forms too); a paste of lines runs each.
    typed(&mut sh, &mut w, "pwd\recho 2\r");
    assert_eq!(
        (sh.history.len(), typed(&mut sh, &mut w, "\x1bOA\x1b[A").contains("pwd")),
        (3, true)
    );
    assert!(typed(&mut sh, &mut w, "\x1b[B\x1bOB").ends_with("$ \r") && sh.line.is_empty());
    // Ctrl+C drops the line, Ctrl+U what is before the caret, Ctrl+L clears the screen.
    assert!(typed(&mut sh, &mut w, "abc\x03").contains("abc\r^C\n") && sh.line.is_empty());
    assert_eq!(typed(&mut sh, &mut w, "abc\x15d\r").lines().nth(1), Some("d: command not found"));
    assert!(sh.out.is_empty() && typed(&mut sh, &mut w, "\x0c").starts_with("guest@"));
    // A job's status marks the prompt, which starts a row wherever the job left the cursor:
    // a row of spaces wraps past what it wrote, then goes.
    w.status = Ok(3);
    sh.feed(b"hi\r", &mut w);
    let after = [&" ".repeat(80), "\r\x1b[K\x1b[31m3 \x1b[1;32mguest"].concat();
    assert!(sh.out.starts_with(&after), "{:?}", sh.out);
    w.status = Err(6);
    assert!(typed(&mut sh, &mut w, "hi\r").contains("sh: hi: too many programs are running\n"));
    // An ask shows nothing: no line follows it.
    sh.feed(b"theme dawn\r", &mut w);
    assert!(std::mem::take(&mut sh.out).contains("Dawn\x07\x1b[31m"));
    // Ctrl+D on an empty line ends the shell; on a line, nothing.
    typed(&mut sh, &mut w, "a\x04\x7f");
    assert!(!sh.quit && sh.line.is_empty());
    typed(&mut sh, &mut w, "\x04");
    assert!(sh.quit);
}

#[test]
fn keys_decode_from_the_bytes_a_terminal_sends() {
    #[rustfmt::skip]
    let cases: [(&[u8], (Key, usize)); 12] = [
        (b"\x1bOA", (Key::Up, 3)), (b"\x1b[1;5C", (Key::Right, 6)), (b"\x1b[7~", (Key::Home, 4)),
        (b"\x1b[4~x", (Key::End, 4)), (b"\x1b[3~", (Key::Delete, 4)), (b"\x1b[5~", (Key::None, 4)),
        ("é!".as_bytes(), (Key::Char('é'), 2)), (b"\xc3", (Key::None, 1)), (b"\x1b[", (Key::None, 2)),
        (b"\x1bx", (Key::None, 2)), (b"\r\n", (Key::Enter, 2)), (b"\t", (Key::None, 1)),
    ];
    for (bytes, want) in cases {
        assert_eq!(key(bytes), want, "{bytes:?}");
    }
}

#[test]
fn help_lists_one_command_a_line_at_any_width() {
    for cols in [100, 80, 40, 30] {
        let (mut sh, mut w) = (Shell { cols, ..Shell::default() }, world());
        sh.run("help", &mut w);
        let out = plain(&sh.out);
        out.lines().for_each(|l| assert!(width(l) <= cols.into(), "{cols} cols: {l:?}\n{out}"));
        // Each synopsis starts a line; its description follows, whole.
        for &(.., synopsis, what, _) in COMMANDS.iter().filter(|c| !c.5.is_empty()) {
            let at = out.find(&format!("\n  {synopsis}")).expect(synopsis);
            let flat: String = out[at..].split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat.starts_with(&format!("{synopsis} {what}")), "{cols}: {synopsis}");
        }
    }
    let mut out = String::new();
    wrap("one two three four five six seven eight\nnine ten", 24, &mut out);
    assert_eq!(out, "one two three four five\nsix seven eight\nnine ten");
    let mut sh = Shell { cols: 30, ..Shell::default() };
    sh.greet();
    assert!(plain(&sh.out).starts_with("compusophyOS terminal \u{2014} type\n'help'.\n"));
}

#[test]
fn programs_come_from_bin_then_local_bin() {
    let mut w = world();
    w.fs.mkdir_all(&[Vfs::HOME, "/.local/bin"].concat()).unwrap();
    let files = ["/bin/a", "~/.local/bin/a", "/bin/b.wasm", "~/.local/bin/c", "~/f", "/bin/d/x"];
    for path in files {
        w.fs.mkdir_all(path.rsplit_once('/').unwrap().0).ok();
        w.fs.write(&path.replace('~', Vfs::HOME), b"").unwrap();
    }
    let sh = Shell::default();
    let found = ["a", "b", "c", "./f", "d", "g"].map(|w2| sh.program(w2, &mut w));
    let at = |p: &str| Some(p.replace('~', Vfs::HOME));
    assert_eq!(
        found,
        [at("/bin/a"), at("/bin/b.wasm"), at("~/.local/bin/c"), at("~/f"), None, None]
    );
}

#[test]
fn jobs_encode_as_the_kernel_reads_them() {
    let stages = [("/bin/a".to_string(), vec!["a".to_string(), "é".into()])];
    let bytes = job("/tmp", (1, "/tmp/in"), (2, "/tmp/out"), &stages, b"").unwrap();
    let want = Job {
        cwd: "/tmp",
        stdin: Stdin::File("/tmp/in".into()),
        stdout: Stdout::File { path: "/tmp/out".into(), append: true },
        stages: vec![("/bin/a", vec!["a".into(), "é".into()])],
        data: b"",
    };
    assert_eq!((Job::decode(&bytes), &want.encode()), (Some(want.clone()), &bytes));
    let piped = job("/", (2, ""), (0, ""), &stages, b"data").unwrap();
    assert_eq!(Job::decode(&piped).map(|j| (j.stdin, j.data)), Some((Stdin::Pipe, &b"data"[..])));
    let long = [("/bin/a".to_string(), vec!["x".repeat(70_000)])];
    assert_eq!(job("/", (0, ""), (0, ""), &long, b""), None);
}
