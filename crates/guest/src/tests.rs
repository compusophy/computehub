use super::*;
use ui::{Pairing, Request};

const PAIR: Pairing = Pairing { port: 7878, token: [7; 32] };

/// Runs each `$ ` line of `script` in a guest shell over `fs` and checks
/// that the transcript (output without escapes, requests in brackets, the
/// home as `~`) is `script`.
fn transcript(fs: &mut Vfs, now: f64, pairing: Option<Pairing>, script: &str) {
    let (mut g, mut got, mut next) = (Guest::new(), String::new(), 1);
    for cmd in script.lines().filter_map(|l| l.strip_prefix("$ ")) {
        let mut cx = Cx::new(fs, now, pairing, &mut next);
        g.run(cmd, &mut cx);
        got += &format!("$ {cmd}\n{}", plain(&std::mem::take(&mut g.out)));
        for r in cx.take_requests() {
            got += &format!("[{}]\n", show(&r));
        }
        got += if std::mem::take(&mut g.connect) { "[connect]\n" } else { "" };
    }
    assert_eq!(got.replace(Vfs::HOME, "~"), script);
}

/// A request as text: the ones the guest shell makes.
fn show(r: &Request) -> String {
    match r {
        Request::Open { name, floating } => {
            format!("{} {name}", ["open", "float"][*floating as usize])
        }
        Request::CloseSelf => "close".into(),
        other => format!("{other:?}"),
    }
}
/// `s` without CSI sequences.
fn plain(s: &str) -> String {
    let (mut out, mut it) = (String::new(), s.chars());
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
        } else if it.next() == Some('[') {
            it.by_ref().find(|d| d.is_ascii_alphabetic());
        }
    }
    out
}

#[test]
fn guest_shell_commands_and_errors() {
    let (mut fs, now) = (Vfs::new(), 1_700_000_000_000.0);
    for path in ["/apps/demo.app", "~/counter.app", "~/notes.txt"] {
        fs.write(&path.replace('~', Vfs::HOME), b"").unwrap();
    }
    transcript(
        &mut fs,
        now,
        None,
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
$ mkdir
usage: mkdir [-p] <dir>...
$ rm -rz x
rm: unknown option -z
$ frobnicate --now
frobnicate: command not found
$ ls /apps > listing
$ cat listing
demo.app
$ echo "a  b" 'c "d"' e\ f "x > y" 'it''s' "q\"\\" '' '\n'
a  b c "d" e f x > y its q"\  \n
$ echo "open
sh: unterminated quote
$ echo a >
sh: expected a file name after >
$ open terminal
[open terminal]
$ open launcher
[float launcher]
$ run counter.app
[open ~/counter.app]
$ run notes.txt
run: notes.txt: not an .app file
$ edit notes.txt
[open studio:~/notes.txt]
$ open nope
open: nope: no such app (see 'apps')
$ apps
terminal  studio  welcome  launcher  about
/apps/demo.app
~/counter.app
$ uname -a
compusophyOS 0.1 wasm32
$ connect
No node is paired with this page. Start computehub-node and open the pairing
link it prints; type 'node' for how.
$ exit
[close]
"#,
    );
    let script = "$ whoami\nguest\n$ connect\n[connect]\n";
    transcript(&mut fs, now, Some(PAIR), script);
}

#[test]
fn help_lists_one_command_a_line_at_any_width() {
    for cols in [100, 80, 40, 30] {
        let mut g = Guest::new();
        g.cols = cols;
        let (mut fs, mut next) = (Vfs::new(), 1);
        g.run("help", &mut Cx::new(&mut fs, 0.0, None, &mut next));
        let out = plain(&g.out);
        for line in out.lines() {
            assert!(width(line) <= usize::from(cols), "{cols} cols: {line:?}\n{out}");
        }
        for (_, rows) in HELP {
            for (synopsis, what) in rows {
                // Each synopsis starts its own line, and its description
                // follows it, beside it or below it, whole.
                let at = out.find(&format!("\n  {synopsis}")).expect(synopsis);
                let flat: String = out[at..].split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(flat.starts_with(&format!("{synopsis} {what}")), "{cols}: {synopsis}");
            }
        }
    }
    let mut out = String::new();
    wrap("  1. one two three four five six seven eight nine ten", 24, &mut out);
    assert_eq!(out, "  1. one two three four\n     five six seven\n     eight nine ten");
}
