use super::*;
use ui::Request;

/// Runs each `$ ` line of `script` in a guest shell over a VFS holding three
/// files and a program, and checks that the transcript (output without
/// escapes, requests in brackets, the home as `~`) is `script`.
fn transcript(script: &str) {
    let (mut g, mut got, mut fs) = (Guest::new(), String::new(), Vfs::new());
    let mut kernel = ui::kernel::Kernel::new();
    for path in ["/apps/demo.app", "~/counter.app", "~/notes.txt"] {
        fs.write(&path.replace('~', Vfs::HOME), b"").unwrap();
    }
    fs.mkdir("/bin").and(fs.write("/bin/hi", b"#!wasm bin/toolbox.wasm")).unwrap();
    for cmd in script.lines().filter_map(|l| l.strip_prefix("$ ")) {
        let mut cx = Cx::new(&mut fs, &mut kernel, 0.0);
        g.run(cmd, &mut cx);
        got += &format!("$ {cmd}\n{}", plain(&std::mem::take(&mut g.out)));
        for r in cx.take_requests() {
            got += &format!("[{}]\n", show(&r));
        }
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
        Request::SetTheme(name) => format!("theme {name}"),
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
    transcript(
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
$ hi a b > x
sh: hi: programs need a cross-origin isolated page (COOP/COEP headers)
$ ./notes.txt
sh: ./notes.txt: cannot execute
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
$ open settings
[open settings]
$ open launcher
open: launcher: no such app (see 'apps')
$ run counter.app
[open ~/counter.app]
$ run notes.txt
run: notes.txt: not an .app file
$ edit notes.txt
[open studio:~/notes.txt]
$ open nope
open: nope: no such app (see 'apps')
$ apps
terminal  studio  welcome  settings
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
[close]
"#,
    );
}

#[test]
fn help_lists_one_command_a_line_at_any_width() {
    for cols in [100, 80, 40, 30] {
        let mut g = Guest::new();
        g.cols = cols;
        let mut fs = Vfs::new();
        g.run("help", &mut Cx::new(&mut fs, &mut ui::kernel::Kernel::new(), 0.0));
        let out = plain(&g.out);
        for line in out.lines() {
            assert!(width(line) <= usize::from(cols), "{cols} cols: {line:?}\n{out}");
        }
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
}

#[test]
fn programs_come_from_bin_then_local_bin() {
    let mut fs = Vfs::new();
    fs.mkdir_all(&[Vfs::HOME, "/.local/bin"].concat()).and(fs.mkdir("/bin")).unwrap();
    #[rustfmt::skip]
    let files = [
        ("/bin/a", "#!wasm bin/toolbox.wasm\n"), ("~/.local/bin/a", "\0asm"), ("/bin/b.wasm", "\0asm"),
        ("~/.local/bin/c", "#!wasm  /tmp/c 12ab\n"), ("~/.local/bin/d.wasm", "#!wasm x/../y"),
        ("/bin/e", "#!wasm https://x"), ("/bin/f", "#!wasm\tf"), ("~/f", "\0asm"),
    ];
    for (path, data) in files {
        fs.write(&path.replace('~', Vfs::HOME), data.as_bytes()).unwrap();
    }
    let at = |p: &str| Ok(Program::Vfs(p.replace('~', Vfs::HOME)));
    #[rustfmt::skip]
    let want = [Ok(Program::Url("bin/toolbox.wasm".into())), at("/bin/b.wasm"), at("/tmp/c"),
        Err(false), Err(false), Err(false), at("~/f"), Err(true)];
    for (w, want) in ["a", "b", "c", "d", "e", "f", "./f", "g"].into_iter().zip(want) {
        assert_eq!(program(&fs, Vfs::HOME, w), want, "{w}");
    }
}

#[test]
fn a_program_runs_in_the_foreground() {
    use ui::kernel::{Effect, Kernel, Load, wire::Msg};
    let (mut g, mut fs, mut kernel) = (Guest::new(), Vfs::new(), Kernel::new());
    fs.mkdir("/bin").and(fs.write("/bin/hi", b"#!wasm bin/toolbox.wasm")).unwrap();
    kernel.set_isolated(true);
    let mut cx = Cx::new(&mut fs, &mut kernel, 0.0);
    g.text("hi 'b c' >> /tmp/o\n", &mut cx);
    cx.kernel.message(cx.vfs, 2, &Msg::Ready { version: wire::VERSION }.encode());
    let s = cx.kernel.take_effects().into_iter().find_map(|e| match e {
        Effect::Start { pid: 2, msg, program: Load::Url(_) } => wire::Start::decode(&msg),
        _ => None,
    });
    let s = s.expect("a Start for the URL");
    assert_eq!((s.argv.join(","), s.tty, &s.cwd[..]), ("hi,b c".into(), Some((80, 24)), Vfs::HOME));
    assert_eq!(s.stdout, Stdout::File { path: "/tmp/o".into(), append: true });
    // Typed ahead: echoed at the cursor, moved below output, sent on Enter.
    g.out.clear();
    g.text("xy", &mut cx);
    g.hide();
    g.show(5);
    g.key(Key::Enter, Mods::default(), &mut cx);
    let echo = "\r\x1b[Jxy\r\x1b[2C\r\x1b[Jxy\r\x1b[7C\r\x1b[5C\x1b[Jxy\r\x1b[7C\n\r";
    assert_eq!(std::mem::take(&mut g.out), echo);
    // Its end gives the shell its line and history back (apps tests the rest).
    g.finished(0, 0);
    assert!(g.running().is_none() && g.history == ["hi 'b c' >> /tmp/o"] && g.other == ["xy"]);
}
