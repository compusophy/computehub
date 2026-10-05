//! `sh`, the shell the Terminal runs: a line editor and Unix-like commands over the files,
//! answering in ANSI text (lines end in `\n`; the program puts a CR before each LF on its raw
//! console). [`Shell::feed`] takes the console's bytes as a terminal sends them. Commands are rows
//! of a table; what they do to paths is an `Op`. Any other word runs a program as one of the
//! kernel's jobs ([`job`]): programs joined by `|`, the first of which may be a command (its
//! output the job's input), reading a file with `<`, writing one with `>` (`>>` appends). What
//! only the desktop does (open an app, switch the theme) the shell asks of its Terminal in an
//! escape, `OSC 1729 ; verb ; arg BEL`. Files and jobs are reached through a [`Sys`]: the
//! program's WASI calls, or a VFS in tests.

#![forbid(unsafe_code)]

pub use vfs::Entry;
use vfs::Vfs;

/// The apps `open` knows by name.
#[rustfmt::skip]
pub const APPS: [&str; 10] = ["studio", "assistant", "terminal", "files", "editor",
    "activity", "settings", "feedback", "about", "welcome"];
/// The desktop's themes, in its order.
const THEMES: [&str; 3] = ["Midnight", "Dawn", "Mono"];
const GREETING: &str = "\x1b[1mcompusophyOS terminal\x1b[m \u{2014} type 'help'.\n";
const KEYS: &str = "Up and Down recall history, Ctrl+C cancels the line, Ctrl+L clears the \
screen, Ctrl+D on an empty line closes the terminal. Quotes group words: \"a b\" or 'a b'; ~ \
is home, # starts a comment. Other words run programs from /bin or ~/.local/bin, as \
wc [-lwmc] [file]... and rev [file]... (no file: what they are given); they join with |, read \
a file with <, write one with > (>> appends).";
/// What `uname` prints: the system's name (`-s`, alone the default), its node's (`-n`), its
/// release (`-r`) and its machine (`-m`); `-a` all.
const UNAME: [&str; 4] = ["compusophyOS", "compusophy", "0.2", "wasm32"];
/// The escape that asks the Terminal for something, and what a file shown on screen has in its
/// place: `cat` never asks.
const ASK: &str = "\x1b]1729;";
const DEFUSED: &str = "^[]1729;";

/// Why a file operation failed, as the VFS words it.
pub type Why = &'static str;
pub const NOT_FOUND: Why = "no such file or directory";
pub const NOT_A_DIR: Why = "not a directory";
pub const IS_A_DIR: Why = "is a directory";

/// What the shell reaches: the files by absolute path, and the kernel, which runs jobs.
pub trait Sys {
    /// The entries of the directory at `path`, in its order.
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Why>;
    fn read(&mut self, path: &str) -> Result<Vec<u8>, Why>;
    /// Writes `data` to the file at `path`, made if missing: at its end if `append`, else in
    /// place of what it held.
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), Why>;
    /// Makes the directory at `path`, and its parents if `parents`.
    fn mkdir(&mut self, path: &str, parents: bool) -> Result<(), Why>;
    /// Removes the file at `path`, or the directory with all it holds (`all`; else an empty one).
    fn remove(&mut self, path: &str, all: bool) -> Result<(), Why>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Why>;
    /// Whether `path` is a directory (`Some(true)`), a file (`Some(false)`) or nothing.
    fn kind(&mut self, path: &str) -> Option<bool>;
    /// Shows `shown`, then runs `job` (its bytes, [`job`]) to its end: its status, or the errno
    /// it could not start with.
    fn run(&mut self, shown: &str, job: &[u8]) -> Result<i32, u16>;
}

/// A command: gets the shell, its operands (flags taken off), its flags (bit `i` for the `i`th
/// letter its row allows), its output and the world.
type Cmd = fn(&mut Shell, &[&str], u32, &mut Io<'_>, &mut dyn Sys);
/// Any number of operands.
const ANY: usize = usize::MAX;

/// Every command: name, flag letters, fewest and most operands (flags not counted), synopsis
/// (for `help` and usage errors), what it does (for `help`; empty is left out), what it runs.
#[rustfmt::skip]
const COMMANDS: [(&str, &str, usize, usize, &str, &str, Cmd); 20] = [
    ("ls", "aAlh1", 0, ANY, "ls [-alh1] [path]...",
        "list directories; -a shows dot files, -l kinds (d a directory) and sizes (-h as \
        1.5K), -1 a name a line", ls),
    ("cd", "", 0, 1, "cd [dir]", "change directory; ~ is home", |s, a, _, io, sys| {
        s.each(if a.is_empty() { &["~"][..] } else { a }, io, sys, Op::Cd);
    }),
    ("pwd", "", 0, ANY, "pwd", "print the working directory",
        |s, _, _, io, _| io.line(&[&s.cwd], "")),
    ("cat", "n", 1, ANY, "cat [-n] <file>...", "print files; -n numbers the lines",
        |s, a, f, io, sys| s.each(a, io, sys, Op::Cat(f != 0))),
    ("echo", "", 0, ANY, "echo [-n] <text>",
        "print text (-n with no line break); > file writes it, >> file appends",
        |_, a, _, io, _| match a {
            ["-n", rest @ ..] => io.out(&rest.join(" ")),
            _ => io.line(a, " "),
        }),
    ("mkdir", "p", 1, ANY, "mkdir [-p] <dir>...", "make directories; -p makes parents too",
        |s, a, f, io, sys| s.each(a, io, sys, Op::Mkdir(f))),
    ("touch", "", 1, ANY, "touch <file>...", "make empty files",
        |s, a, _, io, sys| s.each(a, io, sys, Op::Touch)),
    ("rm", "rRf", 1, ANY, "rm [-rf] <path>...",
        "remove files; -r removes directories, -f is quiet about missing ones",
        |s, a, f, io, sys| s.each(a, io, sys, Op::Rm(f))),
    // mv never asks, so -f (do not ask) is taken and changes nothing.
    ("mv", "f", 2, ANY, "mv <from>... <to>",
        "move or rename; several move into the directory <to>", |s, a, _, io, sys| {
            let [from @ .., to] = a else { return };
            if from.len() > 1 && s.abs(to).map(|t| sys.kind(&t)) != Ok(Some(true)) {
                return io.err(&["mv: ", to, ": ", NOT_A_DIR]);
            }
            s.each(from, io, sys, Op::Mv(to));
        }),
    ("apps", "", 0, ANY, "apps", "list the apps you can open", |_, _, _, io, sys| {
        io.line(&APPS, "  ");
        for dir in ["/apps", Vfs::HOME] {
            for e in sys.list(dir).unwrap_or_default() {
                if !e.is_dir && e.name.ends_with(".app") {
                    io.line(&[dir, "/", &e.name], "");
                }
            }
        }
    }),
    ("open", "", 1, 1, "open <app|file.app>", "open one in a new window", open),
    ("run", "", 1, 1, "run <file.app>", "run an applang app",
        |s, a, _, io, sys| s.each(a, io, sys, Op::Run(a[0]))),
    ("edit", "", 1, 1, "edit <file>", "edit a file (an app in Studio)",
        |s, a, _, io, sys| s.each(a, io, sys, Op::Edit)),
    ("history", "", 0, ANY, "history", "list past commands", |s, _, _, io, _| {
        for (i, h) in s.history.iter().enumerate() {
            let mut n = String::new();
            push_int(&mut n, i as u64 + 1, 4);
            io.line(&[&n, h], "  ");
        }
    }),
    ("theme", "", 0, 1, "theme [name]", "list the themes, or switch to one", theme),
    ("clear", "", 0, ANY, "clear", "clear the screen",
        |_, _, _, io, _| io.out("\x1b[3J\x1b[H\x1b[2J")),
    ("whoami", "", 0, ANY, "whoami", "print your user name", |_, _, _, io, _| io.out("guest\n")),
    ("uname", "asnrm", 0, 0, "uname [-asnrm]", "print the system's name; -a all about it",
        |_, _, f, io, _| {
            // Bit 0 is -a, bits 1 to 4 the fields; none is -s.
            let f = if f & 1 != 0 { 30 } else { f.max(2) };
            let parts: Vec<&str> = (0..4).filter(|i| f & (2 << i) != 0).map(|i| UNAME[i]).collect();
            io.line(&parts, " ");
        }),
    ("exit", "", 0, 1, "exit [status]", "close this terminal", |s, a, _, io, _| {
        match a.first().map(|n| (n, n.parse::<u8>())) {
            Some((n, Err(_))) => io.err(&["exit: ", n, ": not a status (0 to 255)"]),
            Some((_, Ok(n))) => (s.status, s.quit) = (i32::from(n), true),
            None => s.quit = true,
        }
    }),
    ("help", "", 0, ANY, "", "", |s, _, _, io, _| help(io, s.cols.into())),
];
/// `help`'s sections: a title and the row of [`COMMANDS`] it starts at.
const SECTIONS: [(&str, usize); 3] = [("Files", 0), ("Apps", 9), ("Shell", 13)];

/// Writes the command list for a terminal `cols` wide: synopses in a column with descriptions
/// beside them, or below them, indented, when narrow.
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
            w += char_width(c);
        }
    }
    w
}

/// The columns `c` takes, as the shell guesses the terminal does: none for controls and
/// combining marks, two for East Asian wide characters and emoji, else one.
fn char_width(c: char) -> usize {
    match u32::from(c) {
        0..0x20 | 0x7F..0xA0 | 0x300..0x370 | 0x200B..0x2010 | 0xFE00..0xFE10 => 0,
        0x1100..0x1160 | 0x2E80..0xA4D0 | 0xAC00..0xD7A4 | 0xF900..0xFB00 | 0xFE30..0xFE50 => 2,
        0xFF00..0xFF61 | 0xFFE0..0xFFE7 | 0x1F300..0x1F650 | 0x1F900..0x1FA00 => 2,
        0x20000..0x3FFFE => 2,
        _ => 1,
    }
}

/// What a command does to each (absolute) path it names.
#[derive(Clone, Copy)]
enum Op<'a> {
    Cd,
    /// `cat`, numbering the lines (`-n`).
    Cat(bool),
    Touch,
    Write(&'a [u8], bool),
    Mkdir(u32),
    Rm(u32),
    Mv(&'a str),
    Open,
    Edit,
    Run(&'a str),
}

/// A key as the line editor reads it from a terminal's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Ctrl+C, Ctrl+D, Ctrl+L, Ctrl+U.
    Intr,
    Eof,
    Clear,
    Kill,
    None,
}

/// The key at the start of `b` (not empty), as a terminal sends it, and its length: arrows,
/// Home, End and Delete in their CSI and SS3 forms, Emacs's control keys, UTF-8. Anything else
/// is `Key::None`, a sequence cut short at the end of `b` all of it.
fn key(b: &[u8]) -> (Key, usize) {
    let one = |k| (k, 1);
    match *b {
        [b'\r', b'\n', ..] => (Key::Enter, 2),
        [b'\r' | b'\n', ..] => one(Key::Enter),
        [0x7F | 0x08, ..] => one(Key::Backspace),
        [3, ..] => one(Key::Intr),
        [4, ..] => one(Key::Eof),
        [0x0C, ..] => one(Key::Clear),
        [0x15, ..] => one(Key::Kill),
        [1, ..] => one(Key::Home),
        [5, ..] => one(Key::End),
        [2, ..] => one(Key::Left),
        [6, ..] => one(Key::Right),
        [0x10, ..] => one(Key::Up),
        [0x0E, ..] => one(Key::Down),
        [0x1B, b'[' | b'O', ref rest @ ..] => {
            let Some(n) = rest.iter().position(|c| (0x40..=0x7E).contains(c)) else {
                return (Key::None, b.len());
            };
            let k = match (&rest[..n], rest[n]) {
                (_, b'A') => Key::Up,
                (_, b'B') => Key::Down,
                (_, b'C') => Key::Right,
                (_, b'D') => Key::Left,
                (_, b'H') | (b"1" | b"7", b'~') => Key::Home,
                (_, b'F') | (b"4" | b"8", b'~') => Key::End,
                (b"3", b'~') => Key::Delete,
                _ => Key::None,
            };
            (k, 3 + n)
        }
        [0x1B, _, ..] => (Key::None, 2),
        [c, ..] if c < 0x20 => one(Key::None),
        [c, ..] => {
            let n = match c {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            let ch =
                b.get(..n).and_then(|s| std::str::from_utf8(s).ok()).and_then(|s| s.chars().next());
            (ch.map_or(Key::None, Key::Char), n.min(b.len()))
        }
        [] => (Key::None, 0),
    }
}

/// A shell: the working directory, the line being edited, the history.
#[derive(Debug)]
pub struct Shell {
    /// What to show next; the program takes it after each call.
    pub out: String,
    /// The terminal's width and height.
    pub cols: u16,
    pub rows: u16,
    /// `exit`, or Ctrl+D on an empty line: the shell is done.
    pub quit: bool,
    /// The last command line's status: its job's, a command's (1 when it reported an error, 2
    /// for a misuse), 127 for a program not found, 126 for a file that is none; `exit n`'s.
    pub status: i32,
    /// The working directory, absolute.
    pub cwd: String,
    line: Vec<char>,
    /// The caret, as an index into `line`.
    pos: usize,
    history: Vec<String>,
    /// The history entry shown while browsing, and the line typed before.
    browse: Option<usize>,
    draft: Vec<char>,
    /// The caret's row (below the prompt's first) and column, as last drawn.
    caret: (usize, usize),
    /// A job's nonzero status, which marks the prompt; whether a job ran since the last prompt
    /// (its output may have left the cursor anywhere on a row).
    mark: String,
    ran: bool,
}

impl Default for Shell {
    /// A shell in the guest's home, 80 columns by 24 rows.
    #[rustfmt::skip]
    fn default() -> Shell {
        let (out, line, history, draft, mark) = Default::default();
        let (cols, rows, cwd) = (80, 24, Vfs::HOME.into());
        let (quit, status, pos, browse, caret, ran) = (false, 0, 0, None, (0, 0), false);
        Shell { out, cols, rows, quit, status, cwd, line, pos, history, browse, draft, caret,
            mark, ran }
    }
}

/// A command's output: to the screen, or its stdout to a redirect's file (or a pipe).
struct Io<'a> {
    /// The command's name, which its errors start with.
    cmd: &'a str,
    screen: String,
    file: Option<String>,
    /// The command's status: 0, or 1 once it reported an error (2 for a misuse).
    failed: u8,
    /// The lines `cat -n` numbered so far.
    lines: u64,
}

impl<'a> Io<'a> {
    /// The output of `cmd`: to the screen, or (`to_file`) gathered for a file or a pipe.
    fn new(cmd: &'a str, to_file: bool) -> Io<'a> {
        Io { cmd, screen: String::new(), file: to_file.then(String::new), failed: 0, lines: 0 }
    }

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

    /// Writes `parts` and a line break to the screen: an error, which fails the command.
    fn err(&mut self, parts: &[&str]) {
        push_all(&mut self.screen, parts);
        self.screen.push('\n');
        self.failed = self.failed.max(1);
    }

    /// Asks the Terminal to `verb` `arg` (never with a control in `arg`, which would end the
    /// escape early).
    fn ask(&mut self, verb: &str, arg: &str) {
        if !arg.chars().any(char::is_control) {
            push_all(&mut self.screen, &[ASK, verb, ";", arg, "\x07"]);
        }
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

impl Shell {
    /// What `sh`'s arguments (its name left out) give it to run: none for no arguments (the
    /// console, or without one the script on its input), the line after `-c`, or the script in
    /// the file named. Else what to say, and the status to end with: 2 for a misuse, 127 for a
    /// script that cannot be read.
    pub fn script(
        &self,
        args: &[String],
        sys: &mut dyn Sys,
    ) -> Result<Option<String>, (String, u8)> {
        let say = |parts: &[&str], status| Err((parts.concat() + "\n", status));
        match args {
            [] => Ok(None),
            [c, line, ..] if c == "-c" => Ok(Some(line.clone())),
            [c] if c == "-c" => say(&["sh: -c: a command line must follow"], 2),
            [o, ..] if o.len() > 1 && o.starts_with('-') => say(&["sh: unknown option ", o], 2),
            [path, ..] => match self.abs(path).and_then(|a| sys.read(&a)) {
                Ok(text) => Ok(Some(String::from_utf8_lossy(&text).into_owned())),
                Err(e) => say(&["sh: ", path, ": ", e], 127),
            },
        }
    }

    /// The greeting, wrapped to the terminal, and the first prompt.
    pub fn greet(&mut self) {
        let mut text = String::new();
        wrap(GREETING, self.cols.into(), &mut text);
        self.out.push_str(&text);
        self.render();
    }

    /// Takes `bytes` from the console as a terminal sends them: printable text is typed at the
    /// caret, Enter runs the line, and the editing keys edit it (as `help` lists them). The line
    /// is drawn again once, after them all.
    pub fn feed(&mut self, mut bytes: &[u8], sys: &mut dyn Sys) {
        let mut edited = false;
        while !bytes.is_empty() && !self.quit {
            let (k, n) = key(bytes);
            bytes = bytes.get(n.max(1)..).unwrap_or_default();
            let len = self.line.len();
            match k {
                Key::Char(c) => {
                    self.line.insert(self.pos, c);
                    self.pos += 1;
                }
                Key::Enter => {
                    self.enter(sys);
                    edited = false;
                    continue;
                }
                Key::Intr => {
                    self.pos = len;
                    self.redraw();
                    self.out.push_str("^C\n");
                    (self.line, self.pos, self.browse) = (Vec::new(), 0, None);
                    self.render();
                    edited = false;
                    continue;
                }
                Key::Clear => {
                    self.out.push_str("\x1b[H\x1b[2J");
                    self.render();
                    edited = false;
                    continue;
                }
                Key::Eof if len == 0 => self.quit = true,
                Key::Home => self.pos = 0,
                Key::End => self.pos = len,
                Key::Left => self.pos = self.pos.saturating_sub(1),
                Key::Right => self.pos = (self.pos + 1).min(len),
                Key::Backspace if self.pos > 0 => {
                    self.pos -= 1;
                    self.line.remove(self.pos);
                }
                Key::Delete if self.pos < len => _ = self.line.remove(self.pos),
                Key::Kill => {
                    self.line.drain(..self.pos);
                    self.pos = 0;
                }
                Key::Up | Key::Down => self.recall(k == Key::Up),
                _ => continue,
            }
            edited = true;
        }
        if edited && !self.quit {
            self.redraw();
        }
    }

    /// Runs the line, then a prompt.
    fn enter(&mut self, sys: &mut dyn Sys) {
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
        self.mark.clear();
        let start = self.out.len();
        self.run(&line, sys);
        if self.quit {
            return;
        }
        let mut printed = self.out.get(start..).unwrap_or_default();
        // An ask of the Terminal shows nothing.
        while let Some(i) = printed.strip_suffix('\x07').and_then(|p| p.rfind(ASK)) {
            printed = &printed[..i];
        }
        if std::mem::take(&mut self.ran) {
            // A job's output may end anywhere on a row: a row of spaces wraps past it unless
            // the cursor was at its start, then the line goes, and the prompt starts a row.
            self.out.extend(std::iter::repeat_n(' ', self.cols.max(1).into()));
            self.out.push_str("\r\x1b[K");
        } else if !printed.is_empty() && !printed.ends_with('\n') && !printed.ends_with("\x1b[2J") {
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

    /// Moves back to where the prompt starts and erases down.
    fn erase(&mut self) {
        self.out.push('\r');
        csi(&mut self.out, self.caret.0, 'A');
        self.out.push_str("\x1b[J");
    }

    fn redraw(&mut self) {
        self.erase();
        self.render();
    }

    /// Draws the prompt and line from column 0, then moves to the caret, wrapping as the
    /// terminal does (a char that does not fit starts a row).
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
        let out = &mut self.out;
        push_all(out, &["\x1b[31m", &self.mark, GREEN, tilde, dir, "\x1b[m$ "]);
        out.extend(self.line.iter());
        let caret = prompt.iter().map(|s| s.chars().count()).sum::<usize>() + self.pos;
        let (mut i, mut end, mut at) = (0, (0, 0), None);
        let mut step = |c: char| {
            if i == caret {
                at = Some(end);
            }
            let w = char_width(c);
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

    /// Runs one command line. A command alone runs here, its stdout to the screen or a
    /// redirect's file; programs run as a job, the first of them may be a command (its output
    /// the job's input). As in POSIX, a redirect's file is made before anything runs: a path
    /// that cannot be written runs nothing.
    pub fn run(&mut self, line: &str, sys: &mut dyn Sys) {
        let Line { stages, input, output } = match parse(line) {
            Ok(parsed) => parsed,
            Err(e) => return self.fail(2, &["sh: ", e]),
        };
        let is = |w: &Vec<String>| COMMANDS.iter().position(|c| c.0 == w[0]);
        let first = stages.first().and_then(is);
        if let Some(name) = stages.iter().skip(1).find(|w| is(w).is_some()) {
            return self.fail(2, &["sh: ", &name[0], ": a command reads no pipe"]);
        }
        if let (Some(i), Some(_)) = (first, &input) {
            return self.fail(2, &["sh: ", COMMANDS[i].0, ": a command reads no file"]);
        }
        let alone = stages.len() == 1 && first.is_some();
        let mut io = Io::new("sh", output.is_some() || !alone);
        if let Some(i) = first {
            let (name, ok, min, max, synopsis, _, run) = COMMANDS[i];
            let args: Vec<&str> = stages[0].iter().skip(1).map(String::as_str).collect();
            io.cmd = name;
            if let Some((f, operands)) = flags(&args, ok, &mut io) {
                if (min..=max).contains(&operands.len()) {
                    run(self, operands, f, &mut io, sys);
                } else {
                    io.err(&["usage: ", synopsis]);
                    io.failed = 2;
                }
            }
            io.cmd = "sh";
        }
        if alone || stages.is_empty() {
            if let (Some((path, append)), Some(data)) = (&output, io.file.take()) {
                self.each(&[path], &mut io, sys, Op::Write(data.as_bytes(), *append));
            }
            // A blank line keeps the status; `exit` keeps the one it ends with.
            if !self.quit && (first.is_some() || output.is_some()) {
                self.status = i32::from(io.failed);
            }
            return self.out.push_str(&io.screen);
        }
        let data = io.file.take().unwrap_or_default();
        self.out.push_str(&io.screen);
        let programs = &stages[usize::from(first.is_some())..];
        self.exec(programs, (input, output), data.as_bytes(), first.is_some(), sys);
    }

    /// Runs `stages`, each a program's argv, as one job: its input `data` (when `piped`) or the
    /// redirect's file, its output to the other's; the status marks the prompt.
    fn exec(
        &mut self,
        stages: &[Vec<String>],
        (input, output): (In, Out),
        data: &[u8],
        piped: bool,
        sys: &mut dyn Sys,
    ) {
        let mut found = Vec::new();
        for argv in stages {
            match self.program(&argv[0], sys) {
                Some(path) => found.push((path, argv.clone())),
                None => return self.fail(127, &[&argv[0], ": command not found"]),
            }
        }
        let cwd = self.cwd.clone();
        let abs = |p: &str| Vfs::normalize(&cwd, p).unwrap_or_else(|_| p.into());
        let stdout = match &output {
            Some((p, append)) => {
                let path = abs(p);
                if let Err(e) = sys.write(&path, b"", *append) {
                    return self.fail(1, &["sh: ", p, ": ", e]);
                }
                (1 + u8::from(*append), path)
            }
            None => (0, String::new()),
        };
        let stdin = match &input {
            _ if piped => (2, String::new()),
            Some(path) => match sys.kind(&abs(path)) {
                Some(false) => (1, abs(path)),
                Some(true) => return self.fail(1, &["sh: ", path, ": ", IS_A_DIR]),
                None => return self.fail(1, &["sh: ", path, ": ", NOT_FOUND]),
            },
            None => (0, String::new()),
        };
        let name = &stages[0][0];
        let Some(bytes) = job(&self.cwd, (stdin.0, &stdin.1), (stdout.0, &stdout.1), &found, data)
        else {
            return self.fail(1, &["sh: ", name, ": ", TOO_LONG]);
        };
        match sys.run(&std::mem::take(&mut self.out), &bytes) {
            Ok(status) => {
                (self.ran, self.status) = (true, status);
                if status != 0 {
                    push_int(&mut self.mark, u64::from(status as u32), 0);
                    self.mark.push(' ');
                }
            }
            Err(errno) => {
                let (why, status) = refused(errno);
                self.fail(status, &["sh: ", name, ": ", why]);
            }
        }
    }

    /// Says the error `parts` on a line of its own; the command line's status is `status`.
    fn fail(&mut self, status: i32, parts: &[&str]) {
        push_all(&mut self.out, parts);
        self.out.push('\n');
        self.status = status;
    }

    /// The program the word `w` names: the file `w` if it has a `/`, else the first file of
    /// `/bin/w`, `/bin/w.wasm`, `~/.local/bin/w`, `~/.local/bin/w.wasm`. The kernel checks that
    /// it is one (wasm, or a `#!wasm` marker).
    fn program(&self, w: &str, sys: &mut dyn Sys) -> Option<String> {
        const BIN: [(&str, &str); 4] =
            [("/bin/", ""), ("/bin/", ".wasm"), ("~/.local/bin/", ""), ("~/.local/bin/", ".wasm")];
        let tries = if w.contains('/') { &[("", "")][..] } else { &BIN };
        let mut paths = tries.iter().filter_map(|(d, x)| self.abs(&[d, w, x].concat()).ok());
        paths.find(|p| sys.kind(p) == Some(false))
    }

    fn abs(&self, path: &str) -> Result<String, Why> {
        Vfs::normalize(&self.cwd, path).map_err(|_| "invalid path")
    }

    /// Does `op` to each path, reporting failures as `cmd: path: error`.
    fn each(&mut self, paths: &[&str], io: &mut Io<'_>, sys: &mut dyn Sys, op: Op<'_>) {
        for p in paths {
            if let Err(e) = self.abs(p).and_then(|a| self.apply(op, &a, io, sys)) {
                let cmd = io.cmd;
                io.err(&[cmd, ": ", p, ": ", e]);
            }
        }
    }

    /// Does `op` to the absolute path `a`.
    fn apply(
        &mut self,
        op: Op<'_>,
        a: &str,
        io: &mut Io<'_>,
        sys: &mut dyn Sys,
    ) -> Result<(), Why> {
        let kind = sys.kind(a);
        match op {
            Op::Cd if kind == Some(true) => self.cwd = a.to_string(),
            Op::Cd if kind.is_some() => return Err(NOT_A_DIR),
            Op::Cat(number) => {
                let mut text = String::from_utf8_lossy(&sys.read(a)?).into_owned();
                if number {
                    // `%6d\t` before each line, counting on from the files before; a file that
                    // ends mid-line goes on with the next one's first.
                    let shown = io.file.as_ref().unwrap_or(&io.screen);
                    let mut fresh = io.lines == 0 || shown.ends_with('\n');
                    let mut lines = String::new();
                    for line in text.split_inclusive('\n') {
                        if std::mem::replace(&mut fresh, true) {
                            io.lines += 1;
                            push_int(&mut lines, io.lines, 6);
                            lines.push('\t');
                        }
                        lines.push_str(line);
                    }
                    text = lines;
                }
                // On screen, a file never asks the Terminal for anything.
                let text = if io.file.is_none() { text.replace(ASK, DEFUSED) } else { text };
                io.out(&text);
            }
            Op::Touch => sys.write(a, b"", true)?,
            Op::Write(data, append) => sys.write(a, data, append)?,
            Op::Mkdir(f) => sys.mkdir(a, f != 0)?,
            Op::Rm(f) if f & 3 == 0 && kind == Some(true) => return Err(IS_A_DIR),
            Op::Rm(f) => match sys.remove(a, f & 3 != 0) {
                Err(NOT_FOUND) if f & 4 != 0 => {}
                r => r?,
            },
            Op::Mv(to) => {
                let mut dst = self.abs(to)?;
                if sys.kind(&dst) == Some(true) && a != dst {
                    dst.push_str(if dst.ends_with('/') { "" } else { "/" });
                    dst.push_str(a.get(a.rfind('/').map_or(0, |i| i + 1)..).unwrap_or_default());
                }
                sys.rename(a, &dst)?;
            }
            Op::Open | Op::Run(_) if kind == Some(false) && a.ends_with(".app") => {
                io.ask("open", a)
            }
            Op::Run(p) if kind == Some(false) => io.err(&["run: ", p, ": not an .app file"]),
            Op::Run(_) | Op::Edit if kind == Some(true) => return Err(IS_A_DIR),
            Op::Edit
                if sys.kind(a.get(..a.rfind('/').unwrap_or(0).max(1)).unwrap_or_default())
                    == Some(true) =>
            {
                let with = if a.ends_with(".app") { "studio:" } else { "editor:" };
                io.ask("open", &[with, a].concat());
            }
            Op::Cd | Op::Open | Op::Run(_) | Op::Edit => return Err(NOT_FOUND),
        }
        Ok(())
    }
}

/// `ls`'s flags, as bits of its row's letters `aAlh1`.
const DOTS: u32 = 3;
const LONG: u32 = 4;
const HUMAN: u32 = 8;
const ONE: u32 = 16;

/// `ls`: the files named, then each directory's entries (dot files only with -a or -A), under
/// its name when several are named, each sorted by name.
fn ls(s: &mut Shell, paths: &[&str], f: u32, io: &mut Io<'_>, sys: &mut dyn Sys) {
    let named = if paths.is_empty() { &["."][..] } else { paths };
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    for &p in named {
        match s.abs(p).map(|a| (sys.list(&a), a)) {
            Ok((Ok(list), _)) => dirs.push((p, list)),
            Ok((Err(e), a)) => match stat(sys, &a) {
                Some(file) if !file.is_dir => files.push(Entry { name: p.into(), ..file }),
                _ => io.err(&["ls: ", p, ": ", e]),
            },
            Err(e) => io.err(&["ls: ", p, ": ", e]),
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.sort_by(|a, b| a.0.cmp(b.0));
    let mut gap = !files.is_empty();
    entries(&files, f, io);
    for (p, mut list) in dirs {
        if named.len() > 1 {
            io.out(if gap { "\n" } else { "" });
            io.line(&[p, ":"], "");
            gap = true;
        }
        list.retain(|e| f & DOTS != 0 || !e.name.starts_with('.'));
        entries(&list, f, io);
    }
}

/// The entry of the absolute path `a`, from its directory's list.
fn stat(sys: &mut dyn Sys, a: &str) -> Option<Entry> {
    let (dir, name) = a.rsplit_once('/')?;
    let list = sys.list(if dir.is_empty() { "/" } else { dir }).ok()?;
    list.into_iter().find(|e| e.name == name)
}

/// Writes `list` as `ls` does with flags `f`: on screen the names share a line, directories in
/// blue; to a file or a pipe (or with -1) each has its own. With -l each has a line: its kind
/// (`d` a directory, `-` a file), its size in bytes (with -h as 1.5K, 12M), its name. The files
/// keep no dates, owners or modes, so neither does the line.
fn entries(list: &[Entry], f: u32, io: &mut Io<'_>) {
    const BLUE: [&str; 2] = ["\x1b[1;34m", "\x1b[m"];
    let color = io.file.is_none();
    let name = |e: &Entry| {
        let [open, close] = if e.is_dir && color { BLUE } else { ["", ""] };
        [open, &e.name, close].concat()
    };
    if f & LONG != 0 {
        let sizes: Vec<String> = list.iter().map(|e| size(e.size, f & HUMAN != 0)).collect();
        let width = sizes.iter().map(String::len).max().unwrap_or(0);
        for (e, n) in list.iter().zip(&sizes) {
            let pad = " ".repeat(width - n.len());
            io.line(&[if e.is_dir { "d" } else { "-" }, "  ", &pad, n, "  ", &name(e)], "");
        }
    } else if !list.is_empty() {
        let names: Vec<String> = list.iter().map(name).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        io.line(&names, if color && f & ONE == 0 { "  " } else { "\n" });
    }
}

/// `n` bytes as `ls -l` shows them: in full, or (`human`) from 1024 in K, M, G or T, rounded
/// up, with a tenth below 10 (1.5K, 12M).
fn size(n: u64, human: bool) -> String {
    let mut s = String::new();
    let unit = (1..5u32).take_while(|&u| human && n >= 1 << (10 * u)).last().unwrap_or(0);
    if unit == 0 {
        push_int(&mut s, n, 0);
        return s;
    }
    let scale = 1u64 << (10 * unit);
    let tenths = n.saturating_mul(10).div_ceil(scale);
    if tenths < 100 {
        push_int(&mut s, tenths / 10, 0);
        s.push('.');
        push_int(&mut s, tenths % 10, 0);
    } else {
        push_int(&mut s, n.div_ceil(scale), 0);
    }
    s.push(char::from(b"KMGT"[unit as usize - 1]));
    s
}

fn open(s: &mut Shell, args: &[&str], _: u32, io: &mut Io<'_>, sys: &mut dyn Sys) {
    match args[0] {
        app if APPS.contains(&app) => io.ask("open", app),
        app if app.ends_with(".app") => s.each(args, io, sys, Op::Open),
        app => io.err(&["open: ", app, ": no such app (see 'apps')"]),
    }
}

/// `theme`: lists the themes; `theme <name>` switches to one (any case).
fn theme(_: &mut Shell, args: &[&str], _: u32, io: &mut Io<'_>, _: &mut dyn Sys) {
    let Some(&want) = args.first() else {
        return THEMES.iter().for_each(|t| io.line(&[t], ""));
    };
    match THEMES.iter().find(|t| t.eq_ignore_ascii_case(want)) {
        Some(t) => io.ask("theme", t),
        None => io.err(&["theme: ", want, ": no such theme (see 'theme')"]),
    }
}

/// What a job too long for the kernel is told.
const TOO_LONG: &str = "too long to run (64 KB at most, piped input too)";

/// Why the kernel did not start a job, by its errno, and the command line's status.
fn refused(errno: u16) -> (&'static str, i32) {
    match errno {
        44 => ("command not found", 127),
        45 => ("cannot execute", 126),
        6 => ("too many programs are running", 1),
        1 => (TOO_LONG, 1),
        _ => ("cannot run", 1),
    }
}

/// Splits leading `-xyz` flags (chars of `ok`, as bits) from the operands, which `--` may
/// start; `None`, after reporting it (a misuse), for an unknown flag.
fn flags<'a, 'b>(args: &'a [&'b str], ok: &str, io: &mut Io<'_>) -> Option<(u32, &'a [&'b str])> {
    let mut set = 0;
    for (n, arg) in args.iter().enumerate() {
        if ok.is_empty() || arg.len() < 2 || !arg.starts_with('-') {
            return Some((set, &args[n..]));
        }
        if *arg == "--" {
            return Some((set, &args[n + 1..]));
        }
        for c in arg.chars().skip(1) {
            let Some(i) = ok.bytes().position(|o| char::from(o) == c) else {
                let cmd = io.cmd;
                io.err(&[cmd, ": unknown option -", c.encode_utf8(&mut [0; 4])]);
                io.failed = 2;
                return None;
            };
            set |= 1 << i;
        }
    }
    Some((set, &[]))
}

/// A `<` redirect's path; a `>` or `>>` redirect's path, and whether it appends.
type In = Option<String>;
type Out = Option<(String, bool)>;

/// A parsed command line: the argv of each stage of its pipe (none for a blank line), and its
/// redirects.
#[derive(Debug, Default, PartialEq, Eq)]
struct Line {
    stages: Vec<Vec<String>>,
    input: In,
    output: Out,
}

/// Splits a command line into stages of words, at `|`, and its redirects: `< path`, `> path`
/// and `>> path`, one of each. `'…'` is literal, `"…"` takes `\"` and `\\`, a bare `\` escapes.
/// A bare `~` alone or before a `/` starting a word is the home (for programs as for commands);
/// a bare `#` starting a word starts a comment, to the end of the line.
fn parse(line: &str) -> Result<Line, &'static str> {
    let mut parsed = Line::default();
    let (mut words, mut word, mut to): (Vec<String>, Option<String>, Option<u8>) =
        (Vec::new(), None, None);
    // Trailing spaces end the last word (two, so a final `\` escapes one).
    let mut chars = line.chars().chain([' ', ' ']).peekable();
    while let Some(c) = chars.next() {
        if matches!(c, ' ' | '\t' | '>' | '<' | '|') {
            match (word.take(), to.take()) {
                (Some(w), Some(b'<')) => parsed.input = Some(w),
                (Some(w), Some(r)) => parsed.output = Some((w, r == b'+')),
                (Some(w), None) => words.push(w),
                (None, pending) => to = pending,
            }
            match c {
                _ if to.is_some() && c != ' ' && c != '\t' => {
                    return Err(if to == Some(b'<') {
                        "expected a file name after <"
                    } else {
                        "expected a file name after >"
                    });
                }
                '>' if parsed.output.is_some() => return Err("only one > per line"),
                '<' if parsed.input.is_some() => return Err("only one < per line"),
                '>' => to = Some(if chars.next_if_eq(&'>').is_some() { b'+' } else { b'>' }),
                '<' => to = Some(b'<'),
                '|' if words.is_empty() => return Err("a | needs a program on each side"),
                '|' => parsed.stages.push(std::mem::take(&mut words)),
                _ => {}
            }
            continue;
        }
        if word.is_none() && c == '#' {
            break;
        }
        let ends = |c: Option<&char>| matches!(c, Some(' ' | '\t' | '/' | '|' | '<' | '>'));
        if word.is_none() && c == '~' && ends(chars.peek()) {
            word = Some(Vfs::HOME.into());
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
        Some(b'<') => Err("expected a file name after <"),
        Some(_) => Err("expected a file name after >"),
        None if words.is_empty() && !parsed.stages.is_empty() => {
            Err("a | needs a program on each side")
        }
        None => {
            if !words.is_empty() {
                parsed.stages.push(words);
            }
            Ok(parsed)
        }
    }
}

/// A job as the kernel reads it (its `wire::Job`): `str cwd | u8 stdin, str path | u8 stdout,
/// str path | u16 n | n stages (str path, u16 argc, str*) | the bytes of a piped stdin`, each
/// str a u16 length and UTF-8. Stdin is 0 the console, 1 a file, 2 a pipe; stdout 0 the console,
/// 1 a file, 2 a file's end, 3 a pipe, 4 nothing. `None` for a string or list too long.
pub fn job(
    cwd: &str,
    stdin: (u8, &str),
    stdout: (u8, &str),
    stages: &[(String, Vec<String>)],
    data: &[u8],
) -> Option<Vec<u8>> {
    fn n(b: &mut Vec<u8>, n: usize) -> Option<()> {
        b.extend(u16::try_from(n).ok()?.to_le_bytes());
        Some(())
    }
    fn s(b: &mut Vec<u8>, t: &str) -> Option<()> {
        n(b, t.len())?;
        b.extend(t.as_bytes());
        Some(())
    }
    let mut b = Vec::new();
    s(&mut b, cwd)?;
    for (kind, path) in [stdin, stdout] {
        b.push(kind);
        s(&mut b, path)?;
    }
    n(&mut b, stages.len())?;
    for (path, argv) in stages {
        s(&mut b, path)?;
        n(&mut b, argv.len())?;
        argv.iter().try_for_each(|a| s(&mut b, a))?;
    }
    b.extend_from_slice(data);
    Some(b)
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
