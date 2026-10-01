//! The guest shell the Terminal runs: a line editor and Unix-like commands
//! over the VFS, answering in ANSI text (lines end in `\n`; the Terminal
//! makes that CR LF). Commands are rows of a table; what they do to paths is
//! an `Op`. Output is built with `push_str`, not `format!`: smaller wasm.

#![forbid(unsafe_code)]

use term::char_width;
use ui::{Cx, Key, Mods, THEMES};
use vfs::Vfs;
use vfs::VfsError::{self, IsADir, NotADir, NotFound};

/// The apps `open` knows by name.
const BUILTIN: [&str; 4] = ["terminal", "studio", "welcome", "settings"];
const KEYS: &str = "Up and Down recall history, Ctrl+C cancels the line, Ctrl+L clears the \
screen. Quotes group words: \"a b\" or 'a b'.";
/// `uname`, then `uname -a`.
const UNAME: [&str; 2] = ["compusophyOS\n", "compusophyOS 0.2 wasm32\n"];

/// A command: gets the shell, its operands (flags taken off), its flags (bit
/// `i` for the `i`th letter its row allows), its output and the world.
type Cmd = fn(&mut Guest, &[&str], u32, &mut Io<'_>, &mut Cx<'_>);
/// Any number of operands.
const ANY: usize = usize::MAX;

/// Every command: its name, flag letters, fewest and most operands (flags
/// included), synopsis (for `help` and usage errors), what it does (for
/// `help`, which leaves out an empty one) and what it runs.
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
    ("help", "", 0, ANY, "", "", |g, _, _, io, _| help(io, g.cols)),
];
/// `help`'s sections: a title and the row of [`COMMANDS`] it starts at.
const SECTIONS: [(&str, usize); 3] = [("Files", 0), ("Apps", 9), ("Shell", 13)];

/// Writes the command list for a terminal `cols` wide: synopses in a column
/// with descriptions beside them, or below them, indented, when narrow.
fn help(io: &mut Io<'_>, cols: u16) {
    let color = io.file.is_none();
    let [bold, cyan, end] = if color { ["\x1b[1m", "\x1b[36m", "\x1b[m"] } else { [""; 3] };
    let cols = usize::from(cols);
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
    /// The terminal's width.
    pub cols: u16,
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

/// Pushes `ESC [ n c`.
fn csi(out: &mut String, n: usize, c: char) {
    out.push_str("\x1b[");
    push_int(out, n as u64, 0);
    out.push(c);
}

impl Guest {
    /// A shell in the guest's home, 80 columns wide.
    pub fn new() -> Guest {
        let mut g = Guest::default();
        (g.cwd, g.cols) = (Vfs::HOME.to_string(), 80);
        g
    }

    /// Handles a key; printable ones come as [`Guest::text`].
    pub fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) {
        let n = self.line.len();
        match (key, mods.ctrl) {
            (Key::Enter, _) => return self.enter(cx),
            (Key::Char('c'), true) => {
                self.pos = n;
                self.redraw();
                self.out.push_str("^C\n");
                (self.line, self.pos, self.browse) = (Vec::new(), 0, None);
                return self.render();
            }
            (Key::Char('l'), true) => {
                self.out.push_str("\x1b[H\x1b[2J");
                return self.render();
            }
            (Key::Home, _) | (Key::Char('a'), true) => self.pos = 0,
            (Key::End, _) | (Key::Char('e'), true) => self.pos = n,
            (Key::Left, _) => self.pos = self.pos.saturating_sub(1),
            (Key::Right, _) => self.pos = (self.pos + 1).min(n),
            (Key::Backspace, _) if self.pos > 0 => {
                self.pos -= 1;
                self.line.remove(self.pos);
            }
            (Key::Delete, _) if self.pos < n => _ = self.line.remove(self.pos),
            (Key::Up | Key::Down, _) => self.recall(key == Key::Up),
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

    /// Runs the line; the next prompt follows.
    fn enter(&mut self, cx: &mut Cx<'_>) {
        self.pos = self.line.len();
        self.redraw();
        // Down a row, unless a full last row left the caret at the next.
        if self.caret.1 > 0 || self.caret.0 == 0 {
            self.out.push('\n');
        }
        let line: String = self.line.drain(..).collect();
        (self.pos, self.browse) = (0, None);
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        let start = self.out.len();
        self.run(&line, cx);
        let printed = &self.out[start..];
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

    /// Moves back to the prompt's start, erases below and draws it again.
    fn redraw(&mut self) {
        self.out.push('\r');
        if self.caret.0 > 0 {
            csi(&mut self.out, self.caret.0, 'A');
        }
        self.out.push_str("\x1b[J");
        self.render();
    }

    /// Draws the prompt and the line from column 0, then moves to the caret,
    /// wrapping as the terminal does (a char that does not fit starts a row).
    pub fn render(&mut self) {
        const USER: &str = "guest@compusophy:";
        const GREEN: &str = "\x1b[1;32mguest@compusophy\x1b[m:\x1b[1;34m";
        let cols = usize::from(self.cols.max(1));
        // The directory, the home as `~`.
        let (tilde, dir) = match self.cwd.strip_prefix(Vfs::HOME) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => ("~", rest),
            _ => ("", &self.cwd[..]),
        };
        let out = &mut self.out;
        push_all(out, &[GREEN, tilde, dir, "\x1b[m$ "]);
        out.extend(self.line.iter());
        let caret = USER.len() + tilde.len() + dir.chars().count() + 2 + self.pos;
        let (mut i, mut end, mut at) = (0, (0, 0), None);
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
        for s in &[USER, tilde, dir, "$ "] {
            s.chars().for_each(&mut step);
        }
        self.line.iter().for_each(|&c| step(c));
        // The right edge is the next row's start (the terminal waits to wrap).
        let norm = |(r, c): (usize, usize)| if c >= cols { (r + 1, 0) } else { (r, c) };
        if end.1 >= cols {
            out.push_str("\r\n");
        }
        let (end, at) = (norm(end), norm(at.unwrap_or(end)));
        if end.0 > at.0 {
            csi(out, end.0 - at.0, 'A');
        }
        out.push('\r');
        if at.1 > 0 {
            csi(out, at.1, 'C');
        }
        self.caret = at;
    }

    /// Runs one command line; `>` or `>>` sends its stdout to a file.
    pub fn run(&mut self, line: &str, cx: &mut Cx<'_>) {
        let (words, redirect) = match parse(line) {
            Ok(parsed) => parsed,
            Err(e) => return push_all(&mut self.out, &["sh: ", e, "\n"]),
        };
        let file = redirect.as_ref().map(|_| String::new());
        let (cmd, screen) = (words.first().map_or("sh", String::as_str), String::new());
        let mut io = Io { cmd, screen, file };
        let args: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
        match COMMANDS.iter().find(|c| c.0 == cmd) {
            Some(&(_, ok, min, max, .., run)) if (min..=max).contains(&args.len()) => {
                if let Some((f, operands)) = flags(&args, ok, &mut io) {
                    run(self, operands, f, &mut io, cx);
                }
            }
            Some(&(.., synopsis, _, _)) => io.err(&["usage: ", synopsis]),
            None if words.is_empty() => {}
            None => io.err(&[cmd, ": command not found"]),
        }
        if let (Some((path, append)), Some(data)) = (redirect, io.file.take()) {
            io.cmd = "sh";
            self.each(&[&path], &mut io, cx, Op::Write(data.as_bytes(), append));
        }
        self.out.push_str(&io.screen);
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
                    dst.push_str(&a[a.rfind('/').map_or(0, |i| i + 1)..]);
                }
                fs.rename(a, &dst)?;
            }
            Op::Open | Op::Run(_) if fs.is_file(a) && a.ends_with(".app") => cx.open(a),
            Op::Run(p) if fs.is_file(a) => io.err(&["run: ", p, ": not an .app file"]),
            Op::Run(_) | Op::Edit if fs.is_dir(a) => return Err(IsADir),
            Op::Edit if fs.is_dir(&a[..a.rfind('/').unwrap_or(0).max(1)]) => {
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
        for c in arg[1..].chars() {
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

type Parsed = (Vec<String>, Option<(String, bool)>);

/// Splits a command line into words and a `>` or `>>` redirect (path,
/// append). `'…'` is literal, `"…"` takes `\"` and `\\`, a bare `\` escapes.
fn parse(line: &str) -> Result<Parsed, &'static str> {
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
