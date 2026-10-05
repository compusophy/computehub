//! `sh` (see the library) on its console: raw while it edits a line, cooked while a job runs,
//! files through `std::fs` (which WASI serves from the desktop's VFS), jobs through /dev/job.
//! It starts in its job's working directory. `sh -c line` runs the line, `sh file` each line of
//! the file, and with no console (`sh < script`, a pipe) each line of its input is a command;
//! then nothing is edited, and the exit status is the last line's (or `exit n`'s). On its
//! console it is `exit n`'s, else 0 (at `exit`, Ctrl+D on an empty line or the end of its
//! input), so its Terminal closes unless `exit` gives another.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::process::ExitCode;

use sh::{Entry, Shell, Sys, Why};

/// The program's world: its console's mode, if it has a console.
struct Os {
    ctl: Option<File>,
}

impl Os {
    /// Sets the console raw (`on`) or cooked; whether it took.
    fn raw(&mut self, on: bool) -> bool {
        let word: &[u8] = if on { b"rawon" } else { b"rawoff" };
        self.ctl.as_mut().is_some_and(|f| f.write_all(word).is_ok())
    }

    /// The console's columns and rows now.
    fn size() -> Option<(u16, u16)> {
        let text = fs::read_to_string("/dev/winsize").ok()?;
        let mut n = text.split_whitespace().map(|w| w.parse().ok());
        Some((n.next()??, n.next()??))
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
        for e in fs::read_dir(path).map_err(why)? {
            let e = e.map_err(why)?;
            let meta = e.metadata().map_err(why)?;
            let name = e.file_name().to_string_lossy().into_owned();
            out.push(Entry { name, is_dir: meta.is_dir(), size: meta.len() });
        }
        Ok(out)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, Why> {
        fs::read(path).map_err(why)
    }

    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), Why> {
        let mut o = OpenOptions::new();
        let f = o.create(true).write(!append).append(append).truncate(!append).open(path);
        f.and_then(|mut f| f.write_all(data)).map_err(why)
    }

    fn mkdir(&mut self, path: &str, parents: bool) -> Result<(), Why> {
        if parents { fs::create_dir_all(path) } else { fs::create_dir(path) }.map_err(why)
    }

    fn remove(&mut self, path: &str, all: bool) -> Result<(), Why> {
        match fs::metadata(path).map_err(why)?.is_dir() {
            true if all => fs::remove_dir_all(path),
            true => fs::remove_dir(path),
            false => fs::remove_file(path),
        }
        .map_err(why)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Why> {
        fs::rename(from, to).map_err(why)
    }

    fn kind(&mut self, path: &str) -> Option<bool> {
        fs::metadata(path).ok().map(|m| m.is_dir())
    }

    /// The job goes to /dev/job in one write, cooked: its programs read lines and Ctrl+C ends
    /// them, not the shell. A read waits for its status.
    fn run(&mut self, shown: &str, job: &[u8]) -> Result<i32, u16> {
        show(shown, self.ctl.is_some());
        let tty = self.raw(false);
        let status =
            OpenOptions::new().read(true).write(true).open("/dev/job").and_then(|mut f| {
                (f.write(job)? == job.len()).then_some(()).ok_or(ErrorKind::WriteZero)?;
                let mut text = String::new();
                f.read_to_string(&mut text)?;
                Ok(text.trim().parse().unwrap_or(1))
            });
        if tty {
            self.raw(true);
        }
        status.map_err(|e| e.raw_os_error().map_or(0, |n| n as u16))
    }
}

/// Writes `s` to stdout, on a raw console (`tty`) a CR before each LF.
fn show(s: &str, tty: bool) {
    if s.is_empty() {
        return;
    }
    let text = if tty { s.replace('\n', "\r\n") } else { s.to_string() };
    let mut out = io::stdout().lock();
    // A console that takes no more leaves nothing to report it on.
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ctl = OpenOptions::new().write(true).open("/dev/consctl").ok();
    let mut os = Os { ctl };
    let mut sh = Shell::default();
    if let Some(pwd) = std::env::var("PWD").ok().filter(|p| p.starts_with('/')) {
        sh.cwd = pwd;
    }
    if let Some((cols, rows)) = Os::size() {
        (sh.cols, sh.rows) = (cols, rows);
    }
    let script = match sh.script(&args, &mut os) {
        // The console, raw while a line is edited.
        Ok(None) if os.raw(true) => None,
        Ok(None) => {
            let mut text = String::new();
            let _ = io::stdin().read_to_string(&mut text);
            Some(text)
        }
        Ok(script) => script,
        Err((why, status)) => {
            show(&why, false);
            return ExitCode::from(status);
        }
    };
    if let Some(text) = script {
        // A script: a line a command, on a console left as it was.
        os.ctl = None;
        for line in text.lines() {
            sh.run(line, &mut os);
            show(&std::mem::take(&mut sh.out), false);
            if sh.quit {
                break;
            }
        }
        return ExitCode::from(sh.status as u8);
    }
    sh.greet();
    show(&std::mem::take(&mut sh.out), true);
    let mut buf = vec![0; 4096];
    while !sh.quit {
        let n = match io::stdin().read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if let Some((cols, rows)) = Os::size() {
            (sh.cols, sh.rows) = (cols, rows);
        }
        sh.feed(&buf[..n], &mut os);
        show(&std::mem::take(&mut sh.out), true);
    }
    ExitCode::from(sh.exited.unwrap_or(0))
}
