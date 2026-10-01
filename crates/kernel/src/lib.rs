//! The compusophyOS kernel's main-thread half, pure and deterministic (no floats, hash-ordered
//! collections, clocks or randomness). Each process runs in its own module Worker (cpu.wasm,
//! whose half is the `wasi` crate); [`Kernel`], next to the [`vfs::Vfs`], holds the process
//! table, consoles, roots and owner windows, talks to workers in [`wire`] messages and asks the
//! platform for the rest through [`Effect`]s. [`snap`]: the /home snapshot format; [`module`]:
//! a guest's memory cap.
//!
//! [`Kernel::spawn`] asks for a worker and sends its Start at READY. Output arrives as
//! CONS_WRITE; EXIT, a kill or a failed worker ends a process, leaving its status for
//! [`Kernel::reap`]; both wake its owner window. File ops are served at once, each path checked
//! against the roots; a GUI process draws ([`Effect::Draw`]) and reads [`Kernel::post_event`]'s
//! events. Not yet: console reads, modes, homed.

#![forbid(unsafe_code)]

pub mod module;
pub mod snap;
pub mod wire;

use vfs::{Vfs, VfsError};
use wire::Msg;

/// What `spawn` says when the page is not cross-origin isolated.
pub const NOT_ISOLATED: &str = "programs need a cross-origin isolated page (COOP/COEP headers)";
/// The most bytes in one DRAW frame, and in the events queued for a process.
pub const MAX_FRAME: usize = 1 << 20;

/// A program's bytes: a VFS path (read at READY), or a page-relative URL
/// (`[A-Za-z0-9._/-]+`, no `..`; the shell checks) fetched as `"../" + url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    Vfs(String),
    Url(String),
}

/// The program a Start carries: none (homed), its bytes, or a URL (fetched as `"../" + url`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    None,
    Bytes(Vec<u8>),
    Url(String),
}

/// A process to start: `tty` is its console's `(cols, rows)`, if any; its
/// paths stay under `roots`, normalized absolute directories (others grant nothing).
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
    /// Show `frame`, a uiwire frame as written (unchecked), for process `pid`.
    Draw { pid: u32, frame: Vec<u8> },
}

/// A process: its owner window, Start and program until READY, valid roots,
/// status once ended, untaken output, queued events, a waiting read's `max`.
#[derive(Debug, Default)]
struct Process {
    pid: u32,
    owner: u32,
    argv0: String,
    start: Option<(Vec<u8>, Program)>,
    roots: Vec<String>,
    status: Option<i32>,
    out: Vec<u8>,
    events: Vec<Vec<u8>>,
    waiting: Option<u32>,
}

/// The shared half: a deterministic state machine whose asks
/// [`Kernel::take_effects`] hands out. Processes in pid order; pids never reused.
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

    /// Starts a process for the owner and asks for its worker: its pid (2 up), or why
    /// not ([`NOT_ISOLATED`], [`wire::MAX_PROCS`] running, a Start over [`wire::MAX_START`]).
    pub fn spawn(&mut self, s: Spawn) -> Result<u32, &'static str> {
        if !self.isolated {
            return Err(NOT_ISOLATED);
        }
        if self.procs.iter().filter(|p| p.status.is_none()).count() >= wire::MAX_PROCS {
            return Err("too many programs are running");
        }
        let Spawn { argv, program, cwd, tty, stdout, roots } = s;
        let (pid, argv0) = (self.last_pid.max(wire::HOME_PID) + 1, argv.first().cloned());
        let (role, env) = (wire::Role::Process, vec![]);
        let st = wire::Start { role, pid, tty, stdout, cwd, roots, argv, env };
        let msg = st.encode();
        if msg.len() > wire::MAX_START {
            return Err("argument list too long");
        }
        let (owner, argv0, start) = (self.owner, argv0.unwrap_or_default(), Some((msg, program)));
        let roots = st.roots;
        self.procs.push(Process { pid, owner, argv0, start, roots, ..Process::default() });
        self.last_pid = pid;
        // No COLS/ROWS words yet: until step 3's live /dev/winsize the worker
        // reads its size from the Start, and the words cost boot bytes.
        self.effects.push(Effect::Spawn { pid, sab: true });
        Ok(pid)
    }

    /// A message from the worker of `pid`, dropped unless it runs. Each request gets one
    /// [`Effect::Reply`]: EINVAL if it does not decode or goes the wrong way, ENOSYS for
    /// CONS_READ and CONS_MODE. Raw until [`Msg`] has them: DRAW (`rest frame`) is an
    /// [`Effect::Draw`] (E2BIG past [`MAX_FRAME`]); EVENTS (`u32 max`) waits for an event
    /// and gets up to `max` bytes (and [`wire::MAX_PAYLOAD`]), the rest left for next time.
    pub fn message(&mut self, vfs: &mut Vfs, pid: u32, msg: &[u8]) {
        let Some(i) = self.find(pid, true) else { return };
        let reply = match *msg {
            [wire::DRAW, ref frame @ ..] if frame.len() <= MAX_FRAME => {
                self.effects.push(Effect::Draw { pid, frame: frame.to_vec() });
                Ok(Vec::new())
            }
            [wire::DRAW, ..] => Err(wire::E2BIG),
            [wire::EVENTS, a, b, c, d] => {
                self.procs[i].waiting = Some(u32::from_le_bytes([a, b, c, d]));
                return self.serve(i);
            }
            _ => match Msg::decode(msg) {
                Some(Msg::Ready { version }) => return self.ready(vfs, i, version),
                Some(Msg::ConsWrite { data }) => {
                    self.procs[i].out.extend_from_slice(data);
                    return self.touch(self.procs[i].owner);
                }
                Some(Msg::Exit { status }) => return self.end(i, status),
                Some(Msg::ConsBell | Msg::HomeState { .. }) => return,
                Some(Msg::ConsRead { .. } | Msg::ConsMode { .. }) => Err(wire::ENOSYS),
                Some(op) => file(vfs, &self.procs[i].roots, op),
                None => Err(wire::EINVAL),
            },
        };
        let (errno, data) = reply.map_or_else(|e| (e, Vec::new()), |data| (0, data));
        self.effects.push(Effect::Reply { pid, errno, data });
    }

    /// The worker of `pid` failed to load or run: status 126.
    pub fn failed(&mut self, pid: u32) {
        self.kill(pid, wire::CANNOT_EXECUTE);
    }

    /// Console input for `pid`: cooked lines, raw bytes or terminal replies. Not yet.
    pub fn input(&mut self, _pid: u32, _bytes: &[u8]) {}

    /// Queues `event` (one uiwire event) for the /dev/events reads of `pid` if
    /// it runs; dropped if empty or past [`MAX_FRAME`] bytes queued.
    pub fn post_event(&mut self, pid: u32, event: &[u8]) {
        let Some(i) = self.find(pid, true) else { return };
        let q = &mut self.procs[i].events;
        if !event.is_empty() && q.iter().map(Vec::len).sum::<usize>() + event.len() <= MAX_FRAME {
            q.push(event.to_vec());
            self.serve(i);
        }
    }

    /// Ends `pid` with `status` (130 for Ctrl+C, 137 for kill) if it runs; wakes its owner.
    pub fn kill(&mut self, pid: u32, status: i32) {
        if let Some(i) = self.find(pid, true) {
            self.end(i, status);
        }
    }

    /// The console of `pid` is now `cols` x `rows`: stored in its SAB.
    pub fn resize(&mut self, pid: u32, cols: u16, rows: u16) {
        let runs = self.find(pid, true).is_some();
        let words = [(wire::COLS, cols), (wire::ROWS, rows)].into_iter().filter(|_| runs);
        self.effects.extend(words.map(|(index, n)| Effect::Word { pid, index, value: n.into() }));
    }

    /// The console output of `pid` so far, leaving none.
    pub fn take_output(&mut self, pid: u32) -> Vec<u8> {
        self.find(pid, false).map(|i| core::mem::take(&mut self.procs[i].out)).unwrap_or_default()
    }

    /// The console mode of `pid`: cooked with echo until modes are served.
    pub fn mode(&self, _pid: u32) -> Mode {
        Mode { raw: false, echo: true }
    }

    /// The exit status of `pid` once it ended, once; then the pid is gone.
    pub fn reap(&mut self, pid: u32) -> Option<i32> {
        let i = self.find(pid, false).filter(|&i| self.procs[i].status.is_some())?;
        self.procs.remove(i).status
    }

    /// Every process not yet reaped, in pid order: pid, argv\[0\], whether it still runs.
    pub fn procs(&self) -> Vec<(u32, String, bool)> {
        self.procs.iter().map(|p| (p.pid, p.argv0.clone(), p.status.is_none())).collect()
    }

    /// The window `owner` closed: its processes end (137, never reaped) and are forgotten.
    pub fn kill_owned(&mut self, owner: u32) {
        while let Some(i) = self.procs.iter().position(|p| p.owner == owner) {
            let p = self.procs.remove(i);
            if p.status.is_none() {
                self.effects.push(Effect::Kill { pid: p.pid });
            }
        }
    }

    /// The owners woken (by output or an exit) since the last call, in order;
    /// `u32::MAX` is every window.
    pub fn take_woken(&mut self) -> Vec<u32> {
        core::mem::take(&mut self.woken)
    }

    /// `Vfs::generation` moved: /home may need saving (homed, not yet).
    pub fn vfs_changed(&mut self) {}

    /// The one-shot timer fired, or the page was hidden (homed, not yet).
    pub fn wake(&mut self) {}

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

    /// Answers a waiting EVENTS read of process `i` from its oldest event.
    fn serve(&mut self, i: usize) {
        let p = &mut self.procs[i];
        let (Some(max), Some(head)) = (p.waiting, p.events.first_mut()) else { return };
        let n = head.len().min(max as usize).min(wire::MAX_PAYLOAD);
        let data = head[..n].to_vec();
        head.copy_within(n.., 0);
        head.truncate(head.len() - n);
        if head.is_empty() {
            p.events.remove(0);
        }
        p.waiting = None;
        self.effects.push(Effect::Reply { pid: p.pid, errno: 0, data });
    }
}

/// Serves file op `m` for a process under `roots`: the reply, or the errno.
fn file<'a>(vfs: &mut Vfs, roots: &[String], m: Msg<'a>) -> Result<Vec<u8>, u16> {
    let at = |path: &'a str| checked(roots, path);
    let none = |r: Result<(), VfsError>| r.map(|()| Vec::new()).map_err(wire::errno);
    match m {
        Msg::Open { oflags, path } => open(vfs, at(path)?, oflags),
        Msg::Read { off, max, path } => {
            let data = vfs.read(at(path)?).map_err(wire::errno)?;
            let from = off.min(data.len() as u64) as usize;
            let n = (data.len() - from).min(max as usize).min(wire::MAX_PAYLOAD);
            Ok(data[from..from + n].to_vec())
        }
        Msg::Write { off, path, data } => {
            Ok(vfs.write_at(at(path)?, off, data).map_err(wire::errno)?.to_le_bytes().into())
        }
        Msg::List { skip, path } => {
            let mut w = wire::Writer::default();
            for e in vfs.list(at(path)?).map_err(wire::errno)?.iter().skip(skip as usize) {
                if w.0.len() + 11 + e.name.len() > wire::MAX_PAYLOAD {
                    break;
                }
                w = w.u8(wire::KIND_FILE + u8::from(e.is_dir)).u64(e.size).str(&e.name);
            }
            Ok(w.done())
        }
        Msg::Mkdir { path } => none(vfs.mkdir(at(path)?)),
        Msg::Remove { kind, path } => match (kind, at(path)?) {
            (wire::KIND_FILE, path) if vfs.is_dir(path) => Err(wire::EISDIR),
            (wire::KIND_DIR, path) if vfs.is_file(path) => Err(wire::ENOTDIR),
            (wire::KIND_FILE | wire::KIND_DIR, path) => none(vfs.remove(path, false)),
            _ => Err(wire::EINVAL),
        },
        Msg::Rename { from, to } => none(vfs.rename(at(from)?, at(to)?)),
        Msg::SetLen { len, path } => none(vfs.set_len(at(path)?, len)),
        _ => Err(wire::EINVAL),
    }
}

/// OPEN: `u8 kind, u64 size`; CREAT or TRUNC on a directory is EISDIR.
fn open(vfs: &mut Vfs, path: &str, oflags: u8) -> Result<Vec<u8>, u16> {
    let has = |flag: u8| oflags & flag != 0;
    match (vfs.exists(path), vfs.is_dir(path)) {
        _ if oflags > 15 => Err(VfsError::InvalidPath),
        (true, _) if has(wire::O_CREAT) && has(wire::O_EXCL) => Err(VfsError::Exists),
        (_, true) if has(wire::O_CREAT | wire::O_TRUNC) => Err(VfsError::IsADir),
        (true, false) if has(wire::O_DIRECTORY) => Err(VfsError::NotADir),
        (true, _) if has(wire::O_TRUNC) => vfs.set_len(path, 0),
        (false, _) if has(wire::O_CREAT) && !has(wire::O_DIRECTORY) => vfs.write(path, b""),
        _ => Ok(()),
    }
    .map_err(wire::errno)?;
    let (kind, size) = match vfs.read(path).map(<[u8]>::len) {
        Err(VfsError::IsADir) => (wire::KIND_DIR, 0),
        n => (wire::KIND_FILE, n.map_err(wire::errno)? as u64),
    };
    Ok(wire::Writer::default().u8(kind).u64(size).done())
}

/// `path` if [`Vfs::normalize`] keeps it (else EINVAL) and under a root (else ENOTCAPABLE).
fn checked<'a>(roots: &[String], path: &'a str) -> Result<&'a str, u16> {
    let normal = |p: &str| Vfs::normalize("/", p).as_deref() == Ok(p);
    if !normal(path) {
        return Err(wire::EINVAL);
    }
    let sub = |t: &str| t.is_empty() || t.starts_with('/');
    let under =
        |r: &String| normal(r) && (r == "/" || path.strip_prefix(r.as_str()).is_some_and(sub));
    roots.iter().any(under).then_some(path).ok_or(wire::ENOTCAPABLE)
}

#[cfg(test)]
mod tests;
