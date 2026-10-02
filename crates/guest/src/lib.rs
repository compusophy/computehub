//! The guest shell the Terminal runs: a line editor and Unix-like commands
//! over the VFS, answering in ANSI text (lines end in `\n`; the Terminal
//! makes that CR LF). Commands are rows of a table; what they do to paths is
//! an `Op`; any other word starts a [`program`], whose input the editor then
//! edits. Output is built with `push_str`, not `format!`: smaller wasm.

#![forbid(unsafe_code)]

use term::char_width;
use ui::kernel::wire::{self, Stdout};
use ui::kernel::{Program, Spawn};
use ui::{Cx, Key, Mods, THEMES};
use vfs::Vfs;
use vfs::VfsError::{self, IsADir, NotADir, NotFound};

/// The apps `open` knows by name.
const BUILTIN: [&str; 8] =
    ["studio", "assistant", "terminal", "files", "settings", "feedback", "about", "welcome"];
const KEYS: &str = "Up and Down recall history, Ctrl+C cancels the line, Ctrl+L clears the \
screen. Quotes group words: \"a b\" or 'a b'.";
/// `uname`, then `uname -a`.
const UNAME: [&str; 2] = ["compusophyOS\n", "compusophyOS 0.2 wasm32\n"];

/// A command: gets the shell, its operands (flags taken off), its flags (bit
/// `i` for the `i`th letter its row allows), its output and the world.
type Cmd = fn(&mut Guest, &[&str], u32, &mut Io<'_>, &mut Cx<'_>);
/// Any number of operands.
const ANY: usize = usize::MAX;

/// Every command: name, flag letters, fewest and most operands (flags included), synopsis
/// (for `help` and usage errors), what it does (for `help`; empty is left out), what it runs.
#[rustfmt::skip]
const COMMANDS: [(&str, &str, usize, usize, &str, &str, Cmd); 20] = [
    ("ls", "a", 0, ANY, "ls [-a] [path]", "list a directory; -a shows dot files", ls),
    ("cd", "", 0, 1, "cd [dir]", "change directory; ~ is home", |g, a, _, io, cx| {
        g.each(if a.is_empty() { &["~"][..] } else { a }, io, cx, Op::Cd);
    }),
    ("pwd", "", 0, ANY, "pwd", "print the working directory",
        |g, _, _, io, _| io.line(&[&g.cwd], "")),
    ("cat", "", 1, ANY, "cat <file>...", "print files",
        |g, a, _, io, cx| g.each(a, io, cx, Op::Cat)),
    ("echo", "", 0, ANY, "echo <text>", "print text; > file writes it, >> file appends",
        |_, a, _, io, _| io.line(a, " ")),
    ("mkdir", "p", 1, ANY, "mkdir [-p] <dir>...", "make directories; -p makes parents too",
        |g, a, f, io, cx| g.each(a, io, cx, Op::Mkdir(f))),
    ("touch", "", 1, ANY, "touch <file>...", "make empty files",
        |g, a, _, io, cx| g.each(a, io, cx, Op::Touch)),
    ("rm", "rRf", 1, ANY, "rm [-r] <path>...", "remove files; -r removes directories",
        |g, a, f, io, cx| g.each(a, io, cx, Op::Rm(f))),
    ("mv", "", 2, 2, "mv <from> <to>", "move or rename",
        |g, a, _, io, cx| g.each(&a[..1], io, cx, Op::Mv(a[1]))),
    ("apps", "", 0, ANY, "apps", "list the apps you can open", |_, _, _, io, cx| {
        io.line(&BUILTIN, "  ");
        for dir in ["/apps", Vfs::HOME] {
            for e in cx.vfs.list(dir).unwrap_or_default() {
                if !e.is_dir && e.name.ends_with(".app") {
                    io.line(&[dir, "/", &e.name], "");
                }
            }
        }
    }),
    ("open", "", 1, 1, "open <app|file.app>", "open one in a new window", open),
    ("run", "", 1, 1, "run <file.app>", "run an applang app",
        |g, a, _, io, cx| g.each(a, io, cx, Op::Run(a[0]))),
    ("edit", "", 1, 1, "edit <file>", "edit a file in Studio",
        |g, a, _, io, cx| g.each(a, io, cx, Op::Edit)),
    ("history", "", 0, ANY, "history", "list past commands", |g, _, _, io, _| {
        for (i, h) in g.history.iter().enumerate() {
            let mut n = String::new();
            push_int(&mut n, i as u64 + 1, 4);
            io.line(&[&n, h], "  ");
        }
    }),
    ("theme", "", 0, 1, "theme [name]", "list the themes, or switch to one", theme),
    ("clear", "", 0, ANY, "clear", "clear the screen",
        |_, _, _, io, _| io.out("\x1b[3J\x1b[H\x1b[2J")),
    ("whoami", "", 0, ANY, "whoami", "print your user name", |_, _, _, io, _| io.out("guest\n")),
    ("uname", "", 0, ANY, "uname [-a]", "print the system's name",
        |_, a, _, io, _| io.out(UNAME[usize::from(matches!(a, ["-a"]))])),
    ("exit", "", 0, ANY, "exit", "close this terminal", |_, _, _, _, cx| cx.close_self()),
    ("help", "", 0, ANY, "", "", |g, _, _, io, _| help(io, g.cols.into())),
];
/// `help`'s sections: a title and the row of [`COMMANDS`] it starts at.
const SECTIONS: [(&str, usize); 3] = [("Files", 0), ("Apps", 9), ("Shell", 13)];

/// Writes the command list for a terminal `cols` wide: synopses in a column
/// with descriptions beside them, or below them, indented, when narrow.
fn help(io: &mut Io<'_>, cols: usize) {
    let color = io.file.is_none();
    let [bold, cyan, end] = if color { ["\x1b[1m", "\x1b[36m", "\x1b[m"] } else { [""; 3] };
    let syn = COMMANDS.iter().map(|r| r.4.len()).max().unwrap_or(0);
    // Two columns need room for a description of a few words.
    let lead = if cols >= syn + 4 + 24 { syn + 4 } else { 6 };
    let mut s = String::new();
    for (i, &(.., synopsis, what, _)) in COMMANDS.iter().enumerate() {
        if let Some(&(title, _)) = SECTIONS.iter().find(|t| t.1 == i) {
            push_all(&mut s, &[bold, title, end, "\n"]);
        }
        if what.is_empty() {
            continue;
        }
        let mut line = ["  ", cyan, synopsis, end].concat();
        if lead > 6 {
            line.extend(std::iter::repeat_n(' ', lead - 2 - synopsis.len()));
        } else {
            push_all(&mut s, &[&line, "\n"]);
            line = " ".repeat(lead);
        }
        line.push_str(what);
        wrap_line(&line, cols, lead, &mut s);
        s.push('\n');
    }
    s.push('\n');
    wrap_line(KEYS, cols, 0, &mut s);
    s.push('\n');
    io.out(&s);
}

/// Word-wraps `text` to `cols` columns into `out`, line by line.
pub fn wrap(text: &str, cols: usize, out: &mut String) {
    for (i, line) in text.split('\n').enumerate() {
        out.push_str(if i > 0 { "\n" } else { "" });
        wrap_line(line, cols, 0, out);
    }
}

/// Word-wraps one line, continuing at column `hang`; long words stay whole.
fn wrap_line(line: &str, cols: usize, hang: usize, out: &mut String) {
    if cols < hang + 12 {
        return out.push_str(line);
    }
    let mut col = 0;
    for (i, word) in line.split(' ').enumerate() {
        let w = width(word);
        if i > 0 && col + 1 + w > cols && col > hang {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', hang));
            col = hang;
        } else if i > 0 {
            out.push(' ');
            col += 1;
        }
        out.push_str(word);
        col += w;
    }
}

/// The columns `s` takes on screen, skipping CSI escape sequences.
fn width(s: &str) -> usize {
    let (mut w, mut it) = (0, s.chars());
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.next() == Some('[') {
                it.by_ref().find(|d| ('\x40'..='\x7e').contains(d));
            }
        } else {
            w += usize::from(char_width(c));
        }
    }
    w
}

type Res = Result<(), VfsError>;

/// What a command does to each (absolute) path it names.
#[derive(Clone, Copy)]
enum Op<'a> {
    Cd,
    Cat,
    Touch,
    Write(&'a [u8], bool),
    Mkdir(u32),
    Rm(u32),
    Mv(&'a str),
    Open,
    Edit,
    Run(&'a str),
}

/// A shell: the working directory, the line being edited, the history.
#[derive(Debug, Default)]
pub struct Guest {
    /// What to show next; the Terminal takes it after each call.
    pub out: String,
    /// The terminal's width and height.
    pub cols: u16,
    pub rows: u16,
    cwd: String,
    line: Vec<char>,
    /// The caret, as an index into `line`.
    pos: usize,
    history: Vec<String>,
    /// The history entry shown while browsing, and the line typed before.
    browse: Option<usize>,
    draft: Vec<char>,
    /// The caret's row (below the prompt's first) and column, as last drawn.
    caret: (usize, usize),
    /// The foreground program: the line is its input, from column `base`,
    /// with the `other` history. A nonzero exit status `mark`s the prompt.
    pid: Option<u32>,
    base: usize,
    other: Vec<String>,
    mark: String,
}

/// A command's output: to the screen, or its stdout to a redirect's file.
struct Io<'a> {
    /// The command's name, which its errors start with.
    cmd: &'a str,
    screen: String,
    file: Option<String>,
}

impl Io<'_> {
    /// Writes to stdout.
    fn out(&mut self, s: &str) {
        self.file.as_mut().unwrap_or(&mut self.screen).push_str(s);
    }

    /// Writes `parts` separated by `sep`, then a line break, to stdout.
    fn line(&mut self, parts: &[&str], sep: &str) {
        for (i, part) in parts.iter().enumerate() {
            self.out(if i > 0 { sep } else { "" });
            self.out(part);
        }
        self.out("\n");
    }

    /// Writes `parts` and a line break to the screen.
    fn err(&mut self, parts: &[&str]) {
        push_all(&mut self.screen, parts);
        self.screen.push('\n');
    }
}

/// Pushes `ESC [ n c`; nothing for 0, which would mean 1.
fn csi(out: &mut String, n: usize, c: char) {
    if n > 0 {
        out.push_str("\x1b[");
        push_int(out, n as u64, 0);
        out.push(c);
    }
}

impl Guest {
    /// A shell in the guest's home, 80 columns by 24 rows.
    pub fn new() -> Guest {
        let mut g = Guest::default();
        (g.cwd, g.cols, g.rows) = (Vfs::HOME.to_string(), 80, 24);
        g
    }

    /// The pid of the program running in the foreground, if any.
    pub fn running(&self) -> Option<u32> {
        self.pid
    }

    /// Erases the program's input line typed so far, before its output.
    pub fn hide(&mut self) {
        if !self.line.is_empty() {
            self.erase();
        }
    }

    /// Draws the input line again from column `col`, after a program's output.
    pub fn show(&mut self, col: u16) {
        (self.base, self.caret) = (usize::from(col), (0, usize::from(col)));
        if !self.line.is_empty() {
            self.render();
        }
    }

    /// The foreground program ended with `status`: a line break unless at column 0 (`col`),
    /// then the prompt, marked by a nonzero status. Typed-ahead text stays as the shell's line.
    pub fn finished(&mut self, status: i32, col: u16) {
        self.out.push_str(if col > 0 { "\n" } else { "" });
        self.mark.clear();
        if status != 0 {
            push_int(&mut self.mark, u64::from(status as u32), 0);
            self.mark.push(' ');
        }
        std::mem::swap(&mut self.history, &mut self.other);
        (self.pid, self.base, self.browse) = (None, 0, None);
        self.render();
    }

    /// Handles a key; printable ones come as [`Guest::text`]. While a
    /// program runs, Ctrl+C ends it (status 130) and Ctrl+L does nothing.
    pub fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) {
        let n = self.line.len();
        match (key, mods.ctrl, self.pid) {
            (Key::Enter, ..) => return self.enter(cx),
            (Key::Char('c'), true, pid) => {
                self.pos = n;
                self.redraw();
                self.out.push_str("^C\n");
                (self.line, self.pos, self.browse, self.base) = (Vec::new(), 0, None, 0);
                if let Some(pid) = pid {
                    cx.kernel.kill(pid, wire::INTERRUPTED);
                }
                return self.render();
            }
            (Key::Char('l'), true, None) => {
                self.out.push_str("\x1b[H\x1b[2J");
                return self.render();
            }
            (Key::Home, ..) | (Key::Char('a'), true, _) => self.pos = 0,
            (Key::End, ..) | (Key::Char('e'), true, _) => self.pos = n,
            (Key::Left, ..) => self.pos = self.pos.saturating_sub(1),
            (Key::Right, ..) => self.pos = (self.pos + 1).min(n),
            (Key::Backspace, ..) if self.pos > 0 => {
                self.pos -= 1;
                self.line.remove(self.pos);
            }
            (Key::Delete, ..) if self.pos < n => _ = self.line.remove(self.pos),
            (Key::Up | Key::Down, ..) => self.recall(key == Key::Up),
            _ => return,
        }
        self.redraw();
    }

    /// Inserts typed or pasted text at the caret; a line break acts as Enter.
    pub fn text(&mut self, s: &str, cx: &mut Cx<'_>) {
        for c in s.chars() {
            if c == '\n' {
                self.enter(cx);
            } else if !c.is_control() {
                self.line.insert(self.pos, c);
                self.pos += 1;
            }
        }
        self.redraw();
    }

    /// Runs the line, then a prompt; while a program runs, sends it the line and `\n` (ICRNL).
    fn enter(&mut self, cx: &mut Cx<'_>) {
        self.pos = self.line.len();
        self.redraw();
        // Down a row, unless a full last row left the caret at the next.
        if self.caret.1 > 0 || self.caret.0 == 0 {
            self.out.push('\n');
        }
        let line: String = self.line.drain(..).collect();
        (self.pos, self.browse, self.base) = (0, None, 0);
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        if let Some(pid) = self.pid {
            cx.kernel.input(pid, (line + "\n").as_bytes());
            return self.render();
        }
        self.mark.clear();
        let start = self.out.len();
        self.run(&line, cx);
        let printed = self.out.get(start..).unwrap_or_default();
        if !printed.is_empty() && !printed.ends_with('\n') && !printed.ends_with("\x1b[2J") {
            self.out.push('\n');
        }
        self.render();
    }

    /// Up (`back`) or Down the history; past the newest is the typed line.
    fn recall(&mut self, back: bool) {
        let n = self.history.len();
        let next = match (self.browse, back) {
            (None, true) if n > 0 => Some(n - 1),
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < n => Some(i + 1),
            (Some(_), false) => None,
            _ => return,
        };
        if self.browse.is_none() {
            self.draft = std::mem::take(&mut self.line);
        }
        self.line = match next {
            Some(i) => self.history[i].chars().collect(),
            None => std::mem::take(&mut self.draft),
        };
        (self.browse, self.pos) = (next, self.line.len());
    }

    /// Moves back to where the line starts (prompt, or column `base` of input), erases down.
    fn erase(&mut self) {
        self.out.push('\r');
        csi(&mut self.out, self.caret.0, 'A');
        csi(&mut self.out, self.base, 'C');
        self.out.push_str("\x1b[J");
    }

    fn redraw(&mut self) {
        self.erase();
        self.render();
    }

    /// Draws the prompt and line from column 0 (a program's input from `base`, no prompt), then
    /// moves to the caret, wrapping as the terminal does (a char that does not fit starts a row).
    pub fn render(&mut self) {
        const USER: &str = "guest@compusophy:";
        const GREEN: &str = "\x1b[1;32mguest@compusophy\x1b[m:\x1b[1;34m";
        let cols = usize::from(self.cols.max(1));
        // The directory, the home as `~`.
        let (tilde, dir) = match self.cwd.strip_prefix(Vfs::HOME) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => ("~", rest),
            _ => ("", &self.cwd[..]),
        };
        let prompt = [&self.mark[..], USER, tilde, dir, "$ "];
        let prompt = if self.pid.is_some() { [""; 5] } else { prompt };
        let out = &mut self.out;
        if self.pid.is_none() {
            push_all(out, &["\x1b[31m", &self.mark, GREEN, tilde, dir, "\x1b[m$ "]);
        }
        out.extend(self.line.iter());
        let caret = prompt.iter().map(|s| s.chars().count()).sum::<usize>() + self.pos;
        let (mut i, mut end, mut at) = (0, (0, self.base), None);
        let mut step = |c: char| {
            if i == caret {
                at = Some(end);
            }
            let w = usize::from(char_width(c));
            if w > 0 && end.1 + w > cols {
                end = (end.0 + 1, 0);
            }
            end.1 += w;
            i += 1;
        };
        for s in &prompt {
            s.chars().for_each(&mut step);
        }
        self.line.iter().for_each(|&c| step(c));
        // The right edge is the next row's start (the terminal waits to wrap).
        let norm = |(r, c): (usize, usize)| if c >= cols { (r + 1, 0) } else { (r, c) };
        if end.1 >= cols {
            out.push_str("\r\n");
        }
        let (end, at) = (norm(end), norm(at.unwrap_or(end)));
        csi(out, end.0.saturating_sub(at.0), 'A');
        out.push('\r');
        csi(out, at.1, 'C');
        self.caret = at;
    }

    /// Runs one command line; `>` or `>>` sends its stdout to a file. Another word
    /// starts a [`program`], after the redirect's file is made (`>` empties it),
    /// as in POSIX: a path that cannot be written starts nothing.
    pub fn run(&mut self, line: &str, cx: &mut Cx<'_>) {
        let (words, redirect) = match parse(line) {
            Ok(parsed) => parsed,
            Err(e) => return push_all(&mut self.out, &["sh: ", e, "\n"]),
        };
        let file = redirect.as_ref().map(|_| String::new());
        let (cmd, screen) = (words.first().map_or("sh", String::as_str), String::new());
        let mut io = Io { cmd, screen, file };
        let args: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
        let mut program = false;
        match COMMANDS.iter().find(|c| c.0 == cmd) {
            Some(&(_, ok, min, max, .., run)) if (min..=max).contains(&args.len()) => {
                if let Some((f, operands)) = flags(&args, ok, &mut io) {
                    run(self, operands, f, &mut io, cx);
                }
            }
            Some(&(.., synopsis, _, _)) => io.err(&["usage: ", synopsis]),
            None => program = !words.is_empty(),
        }
        if let (Some((path, append)), Some(data)) = (&redirect, io.file.take()) {
            io.cmd = "sh";
            self.each(&[path], &mut io, cx, Op::Write(data.as_bytes(), *append));
        }
        self.out.push_str(&io.screen);
        if program && io.screen.is_empty() {
            self.exec(words, redirect, cx);
        }
    }

    /// Starts the [`program`] `argv[0]` names in the foreground, its stdout to the
    /// redirect's file by absolute path; [`Guest::finished`] follows its end.
    fn exec(&mut self, argv: Vec<String>, to: To, cx: &mut Cx<'_>) {
        let name = argv[0].clone();
        let program = match program(cx.vfs, &self.cwd, &name) {
            Ok(program) => program,
            Err(true) => return push_all(&mut self.out, &[&name, ": command not found\n"]),
            Err(false) => return push_all(&mut self.out, &["sh: ", &name, ": cannot execute\n"]),
        };
        let stdout = match to {
            Some((p, append)) => Stdout::File { path: self.abs(&p).unwrap_or(p), append },
            None => Stdout::Console,
        };
        let (cwd, tty, roots) = (self.cwd.clone(), Some((self.cols, self.rows)), vec!["/".into()]);
        match cx.kernel.spawn(Spawn { argv, program, cwd, tty, stdout, roots }) {
            Ok(pid) => {
                self.pid = Some(pid);
                std::mem::swap(&mut self.history, &mut self.other);
            }
            Err(e) => push_all(&mut self.out, &["sh: ", &name, ": ", e, "\n"]),
        }
    }

    fn abs(&self, path: &str) -> Result<String, VfsError> {
        Vfs::normalize(&self.cwd, path)
    }

    /// Does `op` to each path, reporting failures as `cmd: path: error`.
    fn each(&mut self, paths: &[&str], io: &mut Io<'_>, cx: &mut Cx<'_>, op: Op<'_>) {
        for p in paths {
            if let Err(e) = self.abs(p).and_then(|a| self.apply(op, &a, io, cx)) {
                let cmd = io.cmd;
                io.err(&[cmd, ": ", p, ": ", &e.to_string()]);
            }
        }
    }

    /// Does `op` to the absolute path `a`.
    fn apply(&mut self, op: Op<'_>, a: &str, io: &mut Io<'_>, cx: &mut Cx<'_>) -> Res {
        let fs = &mut *cx.vfs;
        match op {
            Op::Cd if fs.is_dir(a) => self.cwd = a.to_string(),
            Op::Cd if fs.exists(a) => return Err(NotADir),
            Op::Cat => io.out(&String::from_utf8_lossy(fs.read(a)?)),
            // Appending nothing makes a missing file and keeps an existing one.
            Op::Touch => fs.append(a, b"")?,
            Op::Write(data, true) => fs.append(a, data)?,
            Op::Write(data, false) => fs.write(a, data)?,
            Op::Mkdir(0) => fs.mkdir(a)?,
            Op::Mkdir(_) => fs.mkdir_all(a)?,
            Op::Rm(f) if f & 3 == 0 && fs.is_dir(a) => return Err(IsADir),
            Op::Rm(f) => match fs.remove(a, f & 3 != 0) {
                Err(NotFound) if f & 4 != 0 => {}
                r => r?,
            },
            Op::Mv(to) => {
                let mut dst = self.abs(to)?;
                if fs.is_dir(&dst) && a != dst {
                    dst.push_str(if dst.ends_with('/') { "" } else { "/" });
                    dst.push_str(a.get(a.rfind('/').map_or(0, |i| i + 1)..).unwrap_or_default());
                }
                fs.rename(a, &dst)?;
            }
            Op::Open | Op::Run(_) if fs.is_file(a) && a.ends_with(".app") => cx.open(a),
            Op::Run(p) if fs.is_file(a) => io.err(&["run: ", p, ": not an .app file"]),
            Op::Run(_) | Op::Edit if fs.is_dir(a) => return Err(IsADir),
            Op::Edit
                if fs.is_dir(a.get(..a.rfind('/').unwrap_or(0).max(1)).unwrap_or_default()) =>
            {
                cx.open(&("studio:".to_string() + a));
            }
            Op::Cd | Op::Open | Op::Run(_) | Op::Edit => return Err(NotFound),
        }
        Ok(())
    }
}

fn ls(g: &mut Guest, paths: &[&str], all: u32, io: &mut Io<'_>, cx: &mut Cx<'_>) {
    const BLUE: [&str; 2] = ["\x1b[1;34m", "\x1b[m"];
    let (p, fs) = (paths.first().copied().unwrap_or("."), &*cx.vfs);
    let entries = match g.abs(p).and_then(|a| fs.list(&a)) {
        Ok(entries) => entries,
        Err(_) if g.abs(p).is_ok_and(|a| fs.is_file(&a)) => return io.line(&[p], ""),
        Err(e) => return io.err(&["ls: ", p, ": ", &e.to_string()]),
    };
    let (color, mut names) = (io.file.is_none(), String::new());
    let shown = |e: &&vfs::Entry| all != 0 || !e.name.starts_with('.');
    for e in entries.iter().filter(shown) {
        let sep = if names.is_empty() { "" } else { "  " };
        let [open, close] = if e.is_dir && color { BLUE } else { ["", ""] };
        push_all(&mut names, &[sep, open, &e.name, close]);
    }
    if !names.is_empty() {
        io.line(&[&names], "");
    }
}

fn open(g: &mut Guest, args: &[&str], _: u32, io: &mut Io<'_>, cx: &mut Cx<'_>) {
    match args[0] {
        app if BUILTIN.contains(&app) => cx.open(app),
        app if app.ends_with(".app") => g.each(args, io, cx, Op::Open),
        app => io.err(&["open: ", app, ": no such app (see 'apps')"]),
    }
}

/// The program the word `w` names: the file `w` if it has a `/`, else the
/// first of `/bin/w`, `/bin/w.wasm`, `~/.local/bin/w`, `~/.local/bin/w.wasm`,
/// holding wasm or a marker `#!wasm <target> [<sha256>]` (an absolute VFS
/// path or a URL `[A-Za-z0-9._/-]+` without `..`). Err: whether none is found.
pub fn program(fs: &Vfs, cwd: &str, w: &str) -> Result<Program, bool> {
    const BIN: [(&str, &str); 4] =
        [("/bin/", ""), ("/bin/", ".wasm"), ("~/.local/bin/", ""), ("~/.local/bin/", ".wasm")];
    let tries = if w.contains('/') { &[("", "")][..] } else { &BIN };
    let mut paths = tries.iter().filter_map(|(d, x)| Vfs::normalize(cwd, &[d, w, x].concat()).ok());
    let path = paths.find(|p| fs.is_file(p)).ok_or(true)?;
    let data = fs.read(&path).unwrap_or_default();
    if data.starts_with(b"\0asm") {
        return Ok(Program::Vfs(path));
    }
    let line = data.strip_prefix(b"#!wasm ").ok_or(false)?.split(|&b| b == b'\n').next();
    let t = line.unwrap_or_default().split(u8::is_ascii_whitespace).find(|t| !t.is_empty());
    // Lossy, as `cat` already is: `str::from_utf8` would add 0.3 KB of boot
    // wasm. Bad UTF-8 fails the URL check, and a VFS path with it is not found.
    let t = String::from_utf8_lossy(t.ok_or(false)?);
    let ok = |&b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-');
    // Not `contains("..")`: a substring search costs 2 KB of wasm.
    let url = t.as_bytes().iter().all(ok) && !t.as_bytes().windows(2).any(|p| p == b"..");
    match t.starts_with('/') {
        true => Ok(Program::Vfs(t.into_owned())),
        false if url => Ok(Program::Url(t.into_owned())),
        false => Err(false),
    }
}

/// `theme`: lists the themes; `theme <name>` switches to one (any case).
fn theme(_: &mut Guest, args: &[&str], _: u32, io: &mut Io<'_>, cx: &mut Cx<'_>) {
    let Some(&want) = args.first() else {
        return THEMES.iter().for_each(|t| io.line(&[t.name], ""));
    };
    match THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(want)) {
        Some(t) => cx.set_theme(t.name),
        None => io.err(&["theme: ", want, ": no such theme (see 'theme')"]),
    }
}

/// Splits leading `-xyz` flags (chars of `ok`, as bits) from the operands;
/// `None`, after reporting it, for an unknown flag.
fn flags<'a, 'b>(args: &'a [&'b str], ok: &str, io: &mut Io<'_>) -> Option<(u32, &'a [&'b str])> {
    let mut set = 0;
    for (n, arg) in args.iter().enumerate() {
        if ok.is_empty() || arg.len() < 2 || !arg.starts_with('-') {
            return Some((set, &args[n..]));
        }
        for c in arg.chars().skip(1) {
            let Some(i) = ok.bytes().position(|o| char::from(o) == c) else {
                let cmd = io.cmd;
                io.err(&[cmd, ": unknown option -", c.encode_utf8(&mut [0; 4])]);
                return None;
            };
            set |= 1 << i;
        }
    }
    Some((set, &[]))
}

/// A redirect: its path, and whether it appends (`>>`).
type To = Option<(String, bool)>;

/// Splits a command line into words and a `>` or `>>` redirect (path,
/// append). `'…'` is literal, `"…"` takes `\"` and `\\`, a bare `\` escapes.
fn parse(line: &str) -> Result<(Vec<String>, To), &'static str> {
    let (mut words, mut redirect, mut to) = (Vec::new(), None, None);
    let mut word: Option<String> = None;
    // Trailing spaces end the last word (two, so a final `\` escapes one).
    let mut chars = line.chars().chain([' ', ' ']).peekable();
    while let Some(c) = chars.next() {
        if matches!(c, ' ' | '\t' | '>') {
            match (word.take(), to.take()) {
                (Some(w), Some(append)) => redirect = Some((w, append)),
                (Some(w), None) => words.push(w),
                (None, pending) => to = pending,
            }
            if c == '>' && (to.is_some() || redirect.is_some()) {
                return Err("only one redirect per line");
            } else if c == '>' {
                to = Some(chars.next_if_eq(&'>').is_some());
            }
            continue;
        }
        let w = word.get_or_insert_with(String::new);
        match c {
            '\\' => w.extend(chars.next()),
            '\'' | '"' => loop {
                match chars.next() {
                    None => return Err("unterminated quote"),
                    Some(q) if q == c => break,
                    Some('\\') if c == '"' && matches!(chars.peek(), Some('"' | '\\')) => {
                        w.extend(chars.next());
                    }
                    Some(x) => w.push(x),
                }
            },
            _ => w.push(c),
        }
    }
    match to {
        Some(_) => Err("expected a file name after >"),
        None => Ok((words, redirect)),
    }
}

/// Pushes each of `parts` (one loop for every caller).
fn push_all(out: &mut String, parts: &[&str]) {
    parts.iter().for_each(|p| out.push_str(p));
}

/// Pushes `n` in decimal, right-aligned in `width` chars, without `core::fmt`.
fn push_int(out: &mut String, n: u64, width: usize) {
    let len = n.checked_ilog10().unwrap_or(0) as usize + 1;
    (len..width).for_each(|_| out.push(' '));
    if n >= 10 {
        push_int(out, n / 10, 0);
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

#[cfg(test)]
mod tests;
