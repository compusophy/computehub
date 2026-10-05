//! The `agent` program (see the library) on a terminal's console: `agent [-y] [-m model]
//! [--no-learn] [--] [task...]`, its options before the task (a `-y` among the task's words is a
//! word). With a task it works on it and exits; without, it reads tasks a line
//! at a time (the console cooked: it edits the line) until Ctrl+D or /exit, with commands of its
//! own (/help). Files through `std::fs`, which WASI serves from the desktop's VFS (each path
//! from the directory it started in, as sh opens them: [`sh::from_start`]); jobs through
//! /dev/job, their output caught in a file; the AI through /dev/draw and /dev/events. Ctrl+C
//! ends it, as the console ends any job. Exit status: 0, or 1 when the task given did not end
//! with an answer, 2 for an option it does not know.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::process::ExitCode;

use agent::{Agent, Answer, BOLD, DIM, Heard, PLAIN, World, YELLOW, learn};
use sh::{Entry, Sys, Why};
use uiwire::client::Client;
use uiwire::{Event, Frame, Request};

/// Where a job's stdout is caught while a shell command runs: the first of `-1`, `-2`, ... after
/// this that the agent could make new ([`Os::claim`]).
const CAUGHT: &str = "/tmp/.agent-out";
/// Why the shell's commands here read and write no device.
const DEVICE: Why = "a device, which the agent's shell reads and writes none of";
const GREETING: &str = "\x1b[1magent\x1b[m \u{2014} a coding agent on the free AI, in your files. \
Type a task, or /help. Ctrl+C stops it.\n";
const PROMPT: &str = "\n\x1b[1;35m\u{203a}\x1b[m ";
const HELP: &str = "Type a task in plain words, such as: make a pomodoro timer app.\n\
It reads freely; it asks before it writes a file or runs a command, and each time before\n\
it replaces a file or runs what may remove, move or overwrite (rm, mv, >, a program but\n\
wc, rev, hello).\n\
  /clear    start a fresh conversation\n  /yes      write and run without asking (again: ask)\n  \
/lessons  what it learned from past failures (~/.agent/lessons.md)\n  /forget   forget them\n  \
/learn    stop learning (again: learn)\n  /exit     leave (Ctrl+D too)\n";
const USAGE: &str = "usage: agent [-y] [-m model] [--no-learn] [--] [task...]\n  -y          write \
and run without asking\n  -m model    the model to ask (by default, the one chosen in Settings)\n  \
--no-learn  keep no lessons from failures\nOptions go before the task. With a task it works on \
it and exits; without, it asks for tasks.\n";

/// The program's world: the AI's channel once opened, the request in flight, what the last
/// shell command's jobs wrote, and how many names down the directory it started in is (each
/// file's path goes from there).
#[derive(Default)]
struct Os {
    ui: Option<Client<File, File>>,
    id: u32,
    ran: String,
    depth: usize,
}

impl Os {
    /// The path `std::fs` opens for the absolute `path`.
    fn at(&self, path: &str) -> String {
        sh::from_start(self.depth, path)
    }

    /// A file of this agent's alone to catch a job's output in, made new: the first of
    /// [`CAUGHT`]`-1`, `-2`, ... not there already (another agent's in another Terminal, or one
    /// a Ctrl+C left), so two agents never write or remove each other's.
    fn claim(&self) -> Option<String> {
        for n in 1..1000u32 {
            let path = [CAUGHT, "-", &n.to_string()].concat();
            match OpenOptions::new().write(true).create_new(true).open(self.at(&path)) {
                Ok(_) => return Some(path),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    }
}

/// An I/O error as the VFS words it.
fn why(e: io::Error) -> Why {
    match e.kind() {
        ErrorKind::NotFound => sh::NOT_FOUND,
        ErrorKind::NotADirectory => sh::NOT_A_DIR,
        ErrorKind::IsADirectory => sh::IS_A_DIR,
        ErrorKind::AlreadyExists => "file exists",
        ErrorKind::DirectoryNotEmpty => "directory not empty",
        ErrorKind::StorageFull => "no space left on device",
        ErrorKind::ReadOnlyFilesystem => "read-only file system",
        ErrorKind::PermissionDenied => "permission denied",
        _ => "invalid path",
    }
}

impl Sys for Os {
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, Why> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.at(path)).map_err(why)? {
            let e = e.map_err(why)?;
            let meta = e.metadata().map_err(why)?;
            let name = e.file_name().to_string_lossy().into_owned();
            out.push(Entry { name, is_dir: meta.is_dir(), size: meta.len() });
        }
        Ok(out)
    }

    /// Not a device's (/dev), whose read may wait for ever (the console, the agent's own events).
    fn read(&mut self, path: &str) -> Result<Vec<u8>, Why> {
        if agent::tools::device(path) {
            return Err(DEVICE);
        }
        fs::read(self.at(path)).map_err(why)
    }

    /// Not a device's (/dev), where a write may start a job or draw: the shell's own commands
    /// write files alone.
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), Why> {
        if agent::tools::device(path) {
            return Err(DEVICE);
        }
        let mut o = OpenOptions::new();
        let f = o.create(true).write(!append).append(append).truncate(!append).open(self.at(path));
        f.and_then(|mut f| f.write_all(data)).map_err(why)
    }

    fn mkdir(&mut self, path: &str, parents: bool) -> Result<(), Why> {
        let path = self.at(path);
        if parents { fs::create_dir_all(path) } else { fs::create_dir(path) }.map_err(why)
    }

    fn remove(&mut self, path: &str, all: bool) -> Result<(), Why> {
        let path = self.at(path);
        match fs::metadata(&path).map_err(why)?.is_dir() {
            true if all => fs::remove_dir_all(path),
            true => fs::remove_dir(path),
            false => fs::remove_file(path),
        }
        .map_err(why)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Why> {
        fs::rename(self.at(from), self.at(to)).map_err(why)
    }

    fn kind(&mut self, path: &str) -> Option<bool> {
        fs::metadata(self.at(path)).ok().map(|m| m.is_dir())
    }

    /// The job goes to /dev/job in one write, its stdout caught in a file of its own
    /// ([`Os::claim`]) unless it goes elsewhere already; what it wrote, and a status not 0, go to
    /// what the shell command ran. With no file to catch it in, it does not run.
    fn run(&mut self, shown: &str, job: &[u8]) -> Result<i32, u16> {
        self.ran += shown;
        let file = match agent::capture(job, "") {
            Some(_) => Some(self.claim().ok_or(0u16)?),
            None => None,
        };
        let caught = file.as_ref().and_then(|f| agent::capture(job, f));
        let bytes = caught.as_deref().unwrap_or(job);
        let status =
            OpenOptions::new().read(true).write(true).open("/dev/job").and_then(|mut f| {
                (f.write(bytes)? == bytes.len()).then_some(()).ok_or(ErrorKind::WriteZero)?;
                let mut text = String::new();
                f.read_to_string(&mut text)?;
                Ok(text.trim().parse().unwrap_or(1))
            });
        if let Some(f) = file {
            self.ran += &String::from_utf8_lossy(&fs::read(self.at(&f)).unwrap_or_default());
            let _ = fs::remove_file(self.at(&f));
        }
        if let Some(n) = status.as_ref().ok().filter(|&&n| n != 0) {
            self.ran += "[exit status ";
            self.ran += &n.to_string();
            self.ran += "]\n";
        }
        status.map_err(|e| e.raw_os_error().map_or(0, |n| n as u16))
    }
}

impl World for Os {
    fn ask(&mut self, body: &str) {
        self.id += 1;
        if self.ui.is_none() {
            self.ui = uiwire::client::open().ok();
        }
        let requests = vec![Request::Ai { id: self.id, body: body.into() }];
        let frame = Frame { title: "agent".into(), requests, ..Frame::default() };
        if self.ui.as_mut().is_some_and(|ui| ui.show(&frame).is_err()) {
            self.ui = None;
        }
    }

    fn heard(&mut self) -> Heard {
        let Some(ui) = &mut self.ui else {
            return Heard::End(0, "the desktop's AI cannot be reached from here".into());
        };
        loop {
            match ui.next_event() {
                Ok(Event::AiData { id, data }) if id == self.id => return Heard::Data(data),
                Ok(Event::AiEnd { id, status, error }) if id == self.id => {
                    return Heard::End(status, error);
                }
                Ok(_) => {}
                Err(_) => {
                    self.ui = None;
                    return Heard::End(0, "the desktop's AI stopped answering".into());
                }
            }
        }
    }

    fn say(&mut self, text: &str) {
        let mut out = io::stdout().lock();
        // A console that takes no more leaves nothing to report it on.
        let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    }

    fn confirm(&mut self, question: &str, always: bool) -> Answer {
        let ask = [YELLOW, "? ", PLAIN, BOLD, "Allow: ", PLAIN, question, "? "];
        let keys = if always { "[y]es [n]o [a]lways " } else { "[y]es [n]o (asked each time) " };
        self.say(&[&ask[..], &[DIM, keys, PLAIN]].concat().concat());
        let mut line = String::new();
        if io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            self.say("\n");
            return Answer::No;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Answer::Yes,
            "a" | "always" => Answer::Always,
            _ => Answer::No,
        }
    }

    fn ran(&mut self) -> String {
        mem::take(&mut self.ran)
    }
}

fn main() -> ExitCode {
    let pwd = std::env::var("PWD").ok().filter(|p| p.starts_with('/'));
    // Without its PWD, from as far down as a path goes.
    let depth = pwd
        .as_ref()
        .map_or(vfs::Vfs::MAX_DEPTH, |p| p.split('/').filter(|n| !n.is_empty()).count());
    let mut os = Os { depth, ..Os::default() };
    let cwd = pwd.unwrap_or_else(|| vfs::Vfs::HOME.into());
    let mut a = Agent::new(&cwd, &mut os);
    let words = match agent::options(&mut a, std::env::args().skip(1).collect()) {
        Ok(words) => words,
        Err(None) => {
            os.say(USAGE);
            return ExitCode::SUCCESS;
        }
        Err(Some(o)) => {
            os.say(&["agent: unknown option ", &o, "\n", USAGE].concat());
            return ExitCode::from(2);
        }
    };
    if !words.is_empty() {
        let done = a.task(&words.join(" "), &mut os);
        return if done { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    os.say(GREETING);
    loop {
        os.say(PROMPT);
        let mut line = String::new();
        if io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            os.say("\n");
            return ExitCode::SUCCESS;
        }
        let said = |on: bool, yes: &str, no: &str| [if on { yes } else { no }, "\n"].concat();
        match line.trim() {
            "" => {}
            "/exit" | "/quit" | "exit" => return ExitCode::SUCCESS,
            "/help" | "help" => os.say(HELP),
            "/clear" => {
                a.clear();
                os.say("A fresh conversation.\n");
            }
            "/yes" => {
                a.auto = !a.auto;
                let on = "It writes and runs without asking.";
                os.say(&said(a.auto, on, "It asks before it writes or runs."));
            }
            "/learn" => {
                a.learn = !a.learn;
                os.say(&said(a.learn, "It learns from failures.", "It learns nothing new."));
            }
            "/lessons" if a.lessons.trim().is_empty() => os.say("No lessons yet.\n"),
            "/lessons" => os.say(&agent::safe(&a.lessons)),
            "/forget" => {
                learn::forget(&mut os);
                a.lessons.clear();
                os.say("Its lessons are forgotten.\n");
            }
            cmd if cmd.starts_with('/') => {
                os.say(&[cmd, ": no such command (see /help)\n"].concat())
            }
            task => _ = a.task(task, &mut os),
        }
    }
}
