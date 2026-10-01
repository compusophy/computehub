//! The compusophyOS kernel. Pure and deterministic: no floats, no hash-ordered
//! collections, no clocks, no randomness.
//!
//! One fresh module Worker runs each process (cpu.wasm); the main thread is
//! its file server and console. [`Kernel`] is the shared half, on the main
//! thread next to the [`vfs::Vfs`]: the process table, consoles, roots, the
//! owner windows and homed's state. It talks to workers in [`wire`] messages
//! and asks the platform for what only it can do through [`Effect`]s.
//! [`wasi`] is the private half each worker runs; [`snap`] is the /home
//! snapshot format homed saves; [`module`] rewrites a guest's memory limit.
//!
//! A process: [`Kernel::spawn`] asks for a worker and sends its Start at the
//! worker's READY. Console output arrives as CONS_WRITE (the platform drains
//! the ring); EXIT, a kill or a failed worker ends it ([`Effect::Kill`]) and
//! leaves the status for [`Kernel::reap`]. Both wake its owner window. R2
//! step 1: file ops and CONS_READ answer ENOSYS; console input, modes and
//! homed do nothing yet.

#![forbid(unsafe_code)]

pub mod module;
pub mod snap;
pub mod wasi;
pub mod wire;

use vfs::Vfs;

/// What `spawn` says when the page is not cross-origin isolated.
pub const NOT_ISOLATED: &str = "programs need a cross-origin isolated page (COOP/COEP headers)";

/// Where a program's bytes come from: a VFS path (read when its worker is
/// ready), or a page-relative URL (`[A-Za-z0-9._/-]+`, no `..`, no scheme)
/// such as `bin/toolbox.wasm`, which the shell checks when it reads a
/// marker. The worker fetches `"../" + url`, a relative reference that
/// stays same-origin whatever the string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    Vfs(String),
    Url(String),
}

/// The program a Start carries: none (homed), its bytes, or a URL the worker
/// fetches as `"../" + url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    None,
    Bytes(Vec<u8>),
    Url(String),
}

/// A process to start: `argv[0]` names it; `tty` is `(cols, rows)` of the
/// console it runs on, `None` for none (an agent's run); `roots` are the
/// absolute directories its paths must stay under (`["/"]` for the shell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    pub argv: Vec<String>,
    pub program: Program,
    pub cwd: String,
    pub tty: Option<(u16, u16)>,
    pub stdout: wire::Stdout,
    pub roots: Vec<String>,
}

/// A console's mode: cooked with echo unless the program set raw or no-echo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub raw: bool,
    pub echo: bool,
}

/// What only the platform can do, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start a new Worker for `pid`, with a SAB unless it is homed.
    Spawn { pid: u32, sab: bool },
    /// Post `[sab | null, msg, program]` to it, after its READY.
    Start { pid: u32, msg: Vec<u8>, program: Load },
    /// Post a Uint8Array (REPLY, SAVE) to an async worker.
    Send { pid: u32, msg: Vec<u8> },
    /// Answer a blocked worker through its SAB, then notify it.
    Reply { pid: u32, errno: u16, data: Vec<u8> },
    /// `Atomics.store` a SAB word (COLS, ROWS, INPUT), then notify.
    Word { pid: u32, index: u32, value: i32 },
    /// Terminate the worker and drop its callbacks.
    Kill { pid: u32 },
    /// Arm the one-shot timer: [`Kernel::wake`] in `ms`.
    Wake { ms: u32 },
    /// Set `localStorage` `compusophy.home` to `"1"`.
    Saved,
}

/// A process: its owner window, the Start message and program until the
/// worker's READY, the status once it ended, console output not yet taken.
#[derive(Debug)]
struct Process {
    pid: u32,
    owner: u32,
    argv0: String,
    start: Option<(Vec<u8>, Program)>,
    status: Option<i32>,
    out: Vec<u8>,
}

/// The shared half of the kernel. Every call is a step of a deterministic
/// state machine; [`Kernel::take_effects`] hands out what it asked for.
/// Processes are kept in pid order; pids are never reused.
#[derive(Debug, Default)]
pub struct Kernel {
    isolated: bool,
    owner: u32,
    last_pid: u32,
    procs: Vec<Process>,
    woken: Vec<u32>,
    effects: Vec<Effect>,
}

impl Kernel {
    pub fn new() -> Kernel {
        Kernel::default()
    }

    /// Whether the page is cross-origin isolated (SAB works); set at start.
    pub fn set_isolated(&mut self, on: bool) {
        self.isolated = on;
    }

    /// The window the next calls act for: spawns are owned by it.
    pub fn set_owner(&mut self, owner: u32) {
        self.owner = owner;
    }

    /// Starts a process for the owner: its pid (2 up), or why not:
    /// [`NOT_ISOLATED`], [`wire::MAX_PROCS`] already running (EAGAIN), or a
    /// Start over [`wire::MAX_START`] (E2BIG). Asks for a worker with a SAB.
    pub fn spawn(&mut self, s: Spawn) -> Result<u32, &'static str> {
        if !self.isolated {
            return Err(NOT_ISOLATED);
        }
        if self.procs.iter().filter(|p| p.status.is_none()).count() >= wire::MAX_PROCS {
            return Err("too many programs are running");
        }
        let (pid, argv0) = (self.last_pid.max(wire::HOME_PID) + 1, s.argv.first().cloned());
        let (tty, stdout, cwd, roots, argv) = (s.tty, s.stdout, s.cwd, s.roots, s.argv);
        let (role, env) = (wire::Role::Process, vec![]);
        let msg = wire::Start { role, pid, tty, stdout, cwd, roots, argv, env }.encode();
        if msg.len() > wire::MAX_START {
            return Err("argument list too long");
        }
        let (owner, argv0, start) = (self.owner, argv0.unwrap_or_default(), Some((msg, s.program)));
        self.procs.push(Process { pid, owner, argv0, start, status: None, out: Vec::new() });
        self.last_pid = pid;
        // No COLS/ROWS words yet: until step 3's live /dev/winsize the worker
        // reads its size from the Start, and the words cost boot bytes.
        self.effects.push(Effect::Spawn { pid, sab: true });
        Ok(pid)
    }

    /// A message from the worker of `pid`; never panics on any bytes, and
    /// drops it unless `pid` runs. The ops not served yet (the file ops,
    /// CONS_READ, CONS_MODE) are answered ENOSYS whatever their body; any
    /// other request that does not decode, or goes the wrong way, EINVAL.
    /// Matched as byte patterns, not by [`Msg::decode`](wire::Msg::decode):
    /// step 1 needs only the fixed-size ops, and the full decoder costs boot
    /// bytes until the file ops arrive.
    pub fn message(&mut self, vfs: &mut Vfs, pid: u32, msg: &[u8]) {
        let Some(i) = self.find(pid, true) else { return };
        let errno = match *msg {
            [wire::READY, version] => return self.ready(vfs, i, version),
            [wire::CONS_WRITE, ref data @ ..] => {
                self.procs[i].out.extend_from_slice(data);
                return self.touch(self.procs[i].owner);
            }
            [wire::EXIT, a, b, c, d] => return self.end(i, i32::from_le_bytes([a, b, c, d])),
            [wire::CONS_BELL] | [wire::HOME_STATE, ..] => return,
            [op, ..] if op <= wire::SETLEN && op != wire::READY => wire::ENOSYS,
            [wire::CONS_READ | wire::CONS_MODE, ..] => wire::ENOSYS,
            _ => wire::EINVAL,
        };
        self.effects.push(Effect::Reply { pid, errno, data: Vec::new() });
    }

    /// The worker of `pid` failed to load or run: status 126.
    pub fn failed(&mut self, pid: u32) {
        self.kill(pid, wire::CANNOT_EXECUTE);
    }

    /// Console input for `pid`: cooked lines, raw bytes or terminal replies
    /// (at most 1 MiB queued).
    pub fn input(&mut self, pid: u32, bytes: &[u8]) {
        let _ = (pid, bytes);
    }

    /// End of file on the console of `pid`: its next read gets 0 bytes.
    pub fn eof(&mut self, pid: u32) {
        let _ = pid;
    }

    /// Ends `pid` with `status` (130 for Ctrl+C, 137 for kill) if it runs:
    /// its worker is terminated and its owner woken.
    pub fn kill(&mut self, pid: u32, status: i32) {
        if let Some(i) = self.find(pid, true) {
            self.end(i, status);
        }
    }

    /// The console of `pid` is now `cols` x `rows`: stored in its SAB.
    pub fn resize(&mut self, pid: u32, cols: u16, rows: u16) {
        if self.find(pid, true).is_some() {
            for (index, n) in [(wire::COLS, cols), (wire::ROWS, rows)] {
                self.effects.push(Effect::Word { pid, index, value: n.into() });
            }
        }
    }

    /// The console output of `pid` so far, leaving none.
    pub fn take_output(&mut self, pid: u32) -> Vec<u8> {
        let i = self.find(pid, false);
        i.map(|i| core::mem::take(&mut self.procs[i].out)).unwrap_or_default()
    }

    /// The console mode of `pid`.
    pub fn mode(&self, pid: u32) -> Mode {
        let _ = pid;
        Mode { raw: false, echo: true }
    }

    /// The exit status of `pid` once it ended, once; then the pid is gone.
    pub fn reap(&mut self, pid: u32) -> Option<i32> {
        let i = self.find(pid, false)?;
        let status = self.procs[i].status?;
        self.procs.remove(i);
        Some(status)
    }

    /// Every process not yet reaped, in pid order: pid, argv\[0\], whether
    /// it still runs.
    pub fn procs(&self) -> Vec<(u32, String, bool)> {
        self.procs.iter().map(|p| (p.pid, p.argv0.clone(), p.status.is_none())).collect()
    }

    /// The window `owner` closed: its processes end (status 137, which no
    /// one reaps) and are forgotten.
    pub fn kill_owned(&mut self, owner: u32) {
        while let Some(i) = self.procs.iter().position(|p| p.owner == owner) {
            let p = self.procs.remove(i);
            if p.status.is_none() {
                self.effects.push(Effect::Kill { pid: p.pid });
            }
        }
    }

    /// The generation and text of homed's newest note (0 and "" for none).
    pub fn note(&self) -> (u32, &str) {
        (0, "")
    }

    /// The owners with output, an exit or a mode change since the last call,
    /// in the order they were woken (`u32::MAX`: every window).
    pub fn take_woken(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.woken)
    }

    /// `Vfs::generation` moved: /home may need saving.
    pub fn vfs_changed(&mut self) {}

    /// The one-shot timer fired, or the page was hidden.
    pub fn wake(&mut self) {}

    /// After the first frame: start homed now if /home was `saved_before`,
    /// else at the first VFS change.
    pub fn boot_home(&mut self, saved_before: bool) {
        let _ = saved_before;
    }

    /// The effects asked for so far, oldest first, leaving none.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        core::mem::take(&mut self.effects)
    }

    /// The table index of `pid`; with `running`, only while it runs.
    fn find(&self, pid: u32, running: bool) -> Option<usize> {
        self.procs.iter().position(|p| p.pid == pid && !(running && p.status.is_some()))
    }

    /// Wakes window `owner`, once until [`Kernel::take_woken`].
    fn touch(&mut self, owner: u32) {
        if !self.woken.contains(&owner) {
            self.woken.push(owner);
        }
    }

    /// Ends process `i` with `status`: its worker is terminated.
    fn end(&mut self, i: usize, status: i32) {
        let p = &mut self.procs[i];
        (p.status, p.start) = (Some(status), None);
        let (pid, owner) = (p.pid, p.owner);
        self.effects.push(Effect::Kill { pid });
        self.touch(owner);
    }

    /// The worker of process `i` is up: send its Start. A version mismatch
    /// ends it with 126, a VFS program that is gone with 127.
    fn ready(&mut self, vfs: &Vfs, i: usize, version: u8) {
        let p = &mut self.procs[i];
        let Some((msg, program)) = p.start.take() else { return };
        let program = match program {
            _ if version != wire::VERSION => {
                p.out.extend_from_slice(b"the OS was updated; reload the page\n");
                return self.end(i, wire::CANNOT_EXECUTE);
            }
            Program::Url(url) => Load::Url(url),
            Program::Vfs(path) => match vfs.read(&path) {
                Ok(bytes) => Load::Bytes(bytes.to_vec()),
                Err(_) => {
                    p.out.extend_from_slice(p.argv0.as_bytes());
                    p.out.extend_from_slice(b": not found\n");
                    return self.end(i, wire::NOT_FOUND);
                }
            },
        };
        self.effects.push(Effect::Start { pid: p.pid, msg, program });
    }
}

#[cfg(test)]
mod tests;
