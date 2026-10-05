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
//! against the roots, and none writes under /bin (EROFS): only the desktop does. A GUI process
//! draws ([`Effect::Draw`]) and reads [`Kernel::post_event`]'s events.
//!
//! A process spawned with a tty holds a console: [`Kernel::input`] is its keys, as a terminal
//! sends them, which a CONS_READ waits for. Cooked (the default): a line at a time with
//! backspace, Enter (CR is LF), Ctrl+D (the line so far; on an empty line, end of file) and
//! Ctrl+C (the line dropped, the console's other processes ended with 130), echoed unless
//! NOECHO, controls as `^X`; output gets a CR before each LF. Raw: every byte as it comes, no
//! echo, nothing special, output as written. A CONS_MODE sets the mode until the process that
//! set it ends.

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
/// The most bytes of a command line [`Kernel::table`] tells: its words while they fit.
pub const MAX_CMD: usize = 160;

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
    /// Show `frame`, a uiwire frame as written (unchecked), for process `pid`.
    Draw { pid: u32, frame: Vec<u8> },
}

/// A process: its owner window, argv, Start and program until READY, valid roots, status once
/// ended, untaken output (its console's, if it holds one), queued events, the read it waits on
/// (its op and `max`; a WAIT's job), DRAWs taken; the pid holding its console (0: none) and, if
/// its own, the console; the process that spawned it (0: a window) and its job's first pid; the
/// bytes in its stdin pipe and the pid writing them (0: none, so they end in an end of file);
/// the pid whose stdin pipe its stdout is (0: none).
#[derive(Debug, Default)]
struct Process {
    pid: u32,
    owner: u32,
    argv: Vec<String>,
    start: Option<(Vec<u8>, Program)>,
    roots: Vec<String>,
    status: Option<i32>,
    out: Vec<u8>,
    events: Vec<Vec<u8>>,
    waiting: Option<(u8, u32)>,
    draws: u32,
    tty: u32,
    console: Option<Console>,
    parent: u32,
    job: u32,
    pipe: Vec<u8>,
    writer: u32,
    pout: u32,
}

/// A console: its size, its mode's bits (`wire::MODE_*`) and the pid that set them, the cooked
/// line being typed, the input reads take, an end of file pending.
#[derive(Debug, Default)]
struct Console {
    size: (u16, u16),
    bits: u8,
    setter: u32,
    line: Vec<u8>,
    input: Vec<u8>,
    eof: bool,
}

impl Process {
    /// argv\[0\], or nothing.
    fn name(&self) -> &str {
        self.argv.first().map_or("", String::as_str)
    }

    /// Whether it runs and waits on a read of `op`.
    fn reads(&self, op: u8) -> bool {
        self.status.is_none() && self.waiting.is_some_and(|w| w.0 == op)
    }
}

/// The first `max` bytes of `v` (and [`wire::MAX_PAYLOAD`]), taken out of it.
fn take(v: &mut Vec<u8>, max: u32) -> Vec<u8> {
    let n = v.len().min(max as usize).min(wire::MAX_PAYLOAD);
    let data = v[..n].to_vec();
    v.copy_within(n.., 0);
    v.truncate(v.len() - n);
    data
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
        let pid = self.last_pid.max(wire::HOME_PID) + 1;
        let (role, stdin, env) = (wire::Role::Process, wire::Stdin::Console, vec![]);
        let st = wire::Start { role, pid, tty, stdin, stdout, cwd, roots, argv, env };
        let msg = st.encode();
        if msg.len() > wire::MAX_START {
            return Err("argument list too long");
        }
        let (owner, start, wire::Start { roots, argv, .. }) =
            (self.owner, Some((msg, program)), st);
        let console = tty.map(|size| Console { size, ..Console::default() });
        let tty = if tty.is_some() { pid } else { 0 };
        let p = Process {
            pid,
            owner,
            argv,
            start,
            roots,
            tty,
            console,
            job: pid,
            ..Process::default()
        };
        self.procs.push(p);
        self.last_pid = pid;
        // No COLS/ROWS words yet: until step 3's live /dev/winsize the worker
        // reads its size from the Start, and the words cost boot bytes.
        self.effects.push(Effect::Spawn { pid, sab: true });
        Ok(pid)
    }

    /// A message from the worker of `pid`, dropped unless it runs. Each request gets one
    /// [`Effect::Reply`]: EINVAL if it does not decode or goes the wrong way. Raw until [`Msg`]
    /// has them: DRAW (`rest frame`) is an [`Effect::Draw`] (E2BIG past [`MAX_FRAME`]); EVENTS
    /// (`u32 max`) waits for an event and gets up to `max` bytes (and [`wire::MAX_PAYLOAD`]), the
    /// rest left for next time. CONS_READ waits for console input as EVENTS does for events (no
    /// console: end of file at once) and wakes the owner; CONS_MODE takes bits below 4 (ENOTTY
    /// with no console). SPAWN starts a [`wire::Job`] whole or not at all; WAIT waits for each
    /// process of one of the asker's jobs to end, gets the last's status and forgets them
    /// (ECHILD: no such job). PIPE_READ waits for bytes in its stdin pipe, or an end of file
    /// once its writer ended; PIPE_WRITE adds to its reader's (EPIPE once that ended), and waits
    /// while more than [`wire::MAX_PIPE`] bytes are unread.
    pub fn message(&mut self, vfs: &mut Vfs, pid: u32, msg: &[u8]) {
        let Some(i) = self.find(pid, true) else { return };
        let (tty, owner) = (self.procs[i].tty, self.procs[i].owner);
        let reply = match *msg {
            [wire::DRAW, ref frame @ ..] if frame.len() <= MAX_FRAME => {
                self.procs[i].draws = self.procs[i].draws.wrapping_add(1);
                self.effects.push(Effect::Draw { pid, frame: frame.to_vec() });
                Ok(Vec::new())
            }
            [wire::DRAW, ..] => Err(wire::E2BIG),
            [wire::EVENTS, a, b, c, d] => {
                return self.wait(i, wire::EVENTS, u32::from_le_bytes([a, b, c, d]));
            }
            _ => match Msg::decode(msg) {
                Some(Msg::Ready { version }) => return self.ready(vfs, i, version),
                Some(Msg::ConsWrite { data }) => return self.show(tty, i, data),
                Some(Msg::Exit { status }) => return self.end(i, status),
                Some(Msg::ConsBell | Msg::HomeState { .. }) => return,
                Some(Msg::ConsRead { max }) => {
                    self.touch(owner);
                    return self.wait(i, wire::CONS_READ, max);
                }
                Some(Msg::ConsMode { bits }) => match self.console(tty) {
                    Some(c) if bits < 4 => {
                        // Raw takes the line typed so far as it is.
                        if bits & wire::MODE_RAW != 0 {
                            c.input.append(&mut c.line);
                        }
                        (c.bits, c.setter) = (bits, pid);
                        Ok(Vec::new())
                    }
                    Some(_) => Err(wire::EINVAL),
                    None => Err(wire::ENOTTY),
                },
                Some(Msg::Spawn { job }) => self.jobs(vfs, i, job).map(|j| j.to_le_bytes().into()),
                Some(Msg::Wait { job }) => return self.wait(i, wire::WAIT, job),
                Some(Msg::PipeRead { max }) => return self.wait(i, wire::PIPE_READ, max),
                Some(Msg::PipeWrite { data }) => match self.find(self.procs[i].pout, true) {
                    Some(r) => {
                        self.procs[r].pipe.extend_from_slice(data);
                        self.serve(r);
                        // Past MAX_PIPE bytes unread, the writer waits for room.
                        if self.procs[r].pipe.len() > wire::MAX_PIPE {
                            return self.procs[i].waiting = Some((wire::PIPE_WRITE, 0));
                        }
                        Ok(Vec::new())
                    }
                    None => Err(wire::EPIPE),
                },
                Some(op) => file(vfs, &self.procs[i].roots, op),
                None => Err(wire::EINVAL),
            },
        };
        self.reply(pid, reply);
    }

    /// The worker of `pid` failed to load or run: status 126.
    pub fn failed(&mut self, pid: u32) {
        self.kill(pid, wire::CANNOT_EXECUTE);
    }

    /// Keys for the console `pid` holds, as a terminal sends them (and its replies), taken in its
    /// mode (module docs); then its newest waiting reader is served.
    pub fn input(&mut self, pid: u32, bytes: &[u8]) {
        let Some(c) = self.console(pid) else { return };
        let (mut shown, mut intr) = (Vec::new(), false);
        for &b in bytes {
            if c.bits & wire::MODE_RAW != 0 {
                c.input.push(b);
                continue;
            }
            match b {
                3 => {
                    (intr, c.eof) = (true, false);
                    c.line.clear();
                    c.input.clear();
                    shown.extend_from_slice(b"^C");
                }
                4 if c.line.is_empty() => c.eof = true,
                4 => c.input.append(&mut c.line),
                b'\r' | b'\n' => {
                    c.line.push(b'\n');
                    c.input.append(&mut c.line);
                    shown.push(b'\n');
                }
                // Backspace takes a whole UTF-8 char.
                8 | 0x7F => {
                    while let Some(x) = c.line.pop() {
                        if x & 0xC0 != 0x80 {
                            shown.extend_from_slice(b"\x08 \x08");
                            break;
                        }
                    }
                }
                b'\t' | 0x20.. => {
                    c.line.push(b);
                    shown.push(b);
                }
                _ => {
                    c.line.push(b);
                    shown.extend_from_slice(&[b'^', b + 64]);
                }
            }
        }
        if c.bits & wire::MODE_NOECHO != 0 {
            shown.clear();
        }
        if let Some(i) = self.find(pid, true) {
            self.show(pid, i, &shown);
        }
        // Ctrl+C: what runs on the console, but the process holding it, ends.
        let job = self.procs.iter().filter(|p| intr && p.tty == pid && p.pid != pid);
        for q in job.map(|p| p.pid).collect::<Vec<u32>>() {
            self.kill(q, wire::INTERRUPTED);
        }
        if let Some(i) = self.procs.iter().rposition(|p| p.tty == pid && p.reads(wire::CONS_READ)) {
            self.serve(i);
        }
    }

    /// Whether a process waits to read the console `pid` holds and no input is queued for it:
    /// the console waits for keys.
    pub fn idle(&self, pid: u32) -> bool {
        let reads = self.procs.iter().any(|p| p.tty == pid && p.reads(wire::CONS_READ));
        let c = self.procs.iter().find(|p| p.pid == pid).and_then(|p| p.console.as_ref());
        reads && c.is_some_and(|c| c.input.is_empty() && !c.eof)
    }

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

    /// The console `pid` holds is now `cols` x `rows`: stored in the SAB of each process running
    /// on it, and given to those it starts.
    pub fn resize(&mut self, pid: u32, cols: u16, rows: u16) {
        if let Some(c) = self.console(pid) {
            c.size = (cols, rows);
        }
        for p in self.procs.iter().filter(|p| p.tty == pid && p.status.is_none()) {
            let words = [(wire::COLS, cols), (wire::ROWS, rows)];
            let pid = p.pid;
            self.effects.extend(words.map(|(index, n)| Effect::Word {
                pid,
                index,
                value: n.into(),
            }));
        }
    }

    /// The console output of `pid` so far, leaving none.
    pub fn take_output(&mut self, pid: u32) -> Vec<u8> {
        self.find(pid, false).map(|i| core::mem::take(&mut self.procs[i].out)).unwrap_or_default()
    }

    /// The mode of the console `pid` holds (cooked with echo for none).
    pub fn mode(&self, pid: u32) -> Mode {
        let c = self.procs.iter().find(|p| p.pid == pid).and_then(|p| p.console.as_ref());
        let bits = c.map_or(0, |c| c.bits);
        Mode { raw: bits & wire::MODE_RAW != 0, echo: bits & wire::MODE_NOECHO == 0 }
    }

    /// Whether `pid` runs.
    pub fn runs(&self, pid: u32) -> bool {
        self.find(pid, true).is_some()
    }

    /// Whether `pid` runs for the window the calls act for ([`Kernel::set_owner`]): a program
    /// that window's shell started, or its own.
    pub fn owns(&self, pid: u32) -> bool {
        self.find(pid, true).is_some_and(|i| self.procs[i].owner == self.owner)
    }

    /// The exit status of `pid` once it ended, once; then the pid is gone.
    pub fn reap(&mut self, pid: u32) -> Option<i32> {
        let i = self.find(pid, false).filter(|&i| self.procs[i].status.is_some())?;
        self.procs.remove(i).status
    }

    /// Every process not yet reaped, in pid order: pid, argv\[0\], whether it still runs.
    pub fn procs(&self) -> Vec<(u32, String, bool)> {
        self.procs.iter().map(|p| (p.pid, p.name().into(), p.status.is_none())).collect()
    }

    /// Appends the process table as a watcher reads it (`uiwire::stat`), the watcher `except`
    /// left out: a u16 count, then per process in pid order its pid, owner window, state (0 waits
    /// for an event, 1 runs, 2 ended), argv's words while their bytes fit in [`MAX_CMD`] (a u16
    /// count, each a u32 length and UTF-8) and counts (one: its DRAWs).
    pub fn table(&self, except: u32, out: &mut Vec<u8>) {
        let rows = || self.procs.iter().filter(|p| p.pid != except);
        out.extend_from_slice(&(rows().count() as u16).to_le_bytes());
        for p in rows() {
            let state = if p.status.is_some() { 2 } else { u8::from(p.waiting.is_none()) };
            let (mut room, mut k) = (MAX_CMD, 0);
            while let Some(n) = p.argv.get(k).map(String::len).filter(|&n| n <= room) {
                (room, k) = (room - n, k + 1);
            }
            out.extend_from_slice(&p.pid.to_le_bytes());
            out.extend_from_slice(&p.owner.to_le_bytes());
            out.extend_from_slice(&[state, k as u8, (k >> 8) as u8]);
            for a in p.argv.iter().take(k) {
                out.extend_from_slice(&(a.len() as u32).to_le_bytes());
                out.extend_from_slice(a.as_bytes());
            }
            out.extend_from_slice(&[1, 0]);
            out.extend_from_slice(&p.draws.to_le_bytes());
        }
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

    /// `Vfs::generation` moved: nothing yet (the desktop keeps /home, see `os::home`).
    pub fn vfs_changed(&mut self) {}

    /// The one-shot timer fired, or the page was hidden: nothing yet.
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

    /// Ends process `i` with `status`: its worker is terminated, a console mode it set goes back
    /// to cooked with echo, the writer of its stdin pipe finds no reader, and its stdout pipe's
    /// reader and its parent may be done waiting.
    fn end(&mut self, i: usize, status: i32) {
        let p = &mut self.procs[i];
        (p.status, p.start, p.waiting) = (Some(status), None, None);
        p.pipe = Vec::new();
        let (pid, owner, tty, writer, pout, parent) =
            (p.pid, p.owner, p.tty, p.writer, p.pout, p.parent);
        self.effects.push(Effect::Kill { pid });
        self.touch(owner);
        if let Some(c) = self.console(tty).filter(|c| c.setter == pid) {
            (c.bits, c.setter) = (0, 0);
        }
        if let Some(w) = self.find(writer, true).filter(|&w| self.procs[w].reads(wire::PIPE_WRITE))
        {
            self.procs[w].waiting = None;
            self.reply(writer, Err(wire::EPIPE));
        }
        for q in [pout, parent] {
            if let Some(j) = self.find(q, true) {
                self.serve(j);
            }
        }
    }

    /// Answers `pid`'s request.
    fn reply(&mut self, pid: u32, r: Result<Vec<u8>, u16>) {
        let (errno, data) = r.map_or_else(|e| (e, Vec::new()), |data| (0, data));
        self.effects.push(Effect::Reply { pid, errno, data });
    }

    /// Process `i` waits on `op` (with its `max`, or a WAIT's job): answered now if it can be.
    fn wait(&mut self, i: usize, op: u8, arg: u32) {
        self.procs[i].waiting = Some((op, arg));
        self.serve(i);
    }

    /// SPAWN from process `i` ([`wire::Job`]): its stages start as children of `i` for its
    /// window, under its roots, on its console, their pipes joined; its first pid. Nothing starts
    /// unless all can: EINVAL for a job that does not decode or has no stages, a cwd or path not
    /// normalized (or ENOTCAPABLE outside the roots), ENOENT for a program not found, ENOEXEC for
    /// a file that is no program, EAGAIN past [`wire::MAX_PROCS`] running, E2BIG for a Start past
    /// [`wire::MAX_START`].
    fn jobs(&mut self, vfs: &Vfs, i: usize, body: &[u8]) -> Result<u32, u16> {
        let job = wire::Job::decode(body).ok_or(wire::EINVAL)?;
        let p = &self.procs[i];
        let (parent, owner, tty, roots) = (p.pid, p.owner, p.tty, &p.roots);
        let (n, first) = (job.stages.len(), self.last_pid.max(wire::HOME_PID) + 1);
        let running = self.procs.iter().filter(|p| p.status.is_none()).count();
        checked(roots, job.cwd)?;
        let size = self.procs.iter().find(|p| p.pid == tty).and_then(|p| p.console.as_ref());
        let size = size.map(|c| c.size);
        let mut made = Vec::new();
        for (k, (path, argv)) in job.stages.into_iter().enumerate() {
            let program = program(vfs, checked(roots, path)?)
                .map_err(|missing| if missing { wire::ENOENT } else { wire::ENOEXEC })?;
            let (pid, last) = (first + k as u32, k + 1 == n);
            let stdin = if k == 0 { job.stdin.clone() } else { wire::Stdin::Pipe };
            let stdout = if last { job.stdout.clone() } else { wire::Stdout::Pipe };
            let (role, cwd, env) = (wire::Role::Process, job.cwd.into(), vec![]);
            let roots = roots.clone();
            let st = wire::Start { role, pid, tty: size, stdin, stdout, cwd, roots, argv, env };
            let msg = st.encode();
            if msg.len() > wire::MAX_START {
                return Err(wire::E2BIG);
            }
            let (start, wire::Start { roots, argv, .. }) = (Some((msg, program)), st);
            let pipe = if k == 0 { job.data.to_vec() } else { Vec::new() };
            let (writer, pout) = (if k > 0 { pid - 1 } else { 0 }, if last { 0 } else { pid + 1 });
            made.push(Process {
                pid,
                owner,
                argv,
                start,
                roots,
                tty,
                parent,
                job: first,
                pipe,
                writer,
                pout,
                ..Process::default()
            });
        }
        match n {
            0 => return Err(wire::EINVAL),
            _ if running + n > wire::MAX_PROCS => return Err(wire::EAGAIN),
            _ => {}
        }
        self.effects.extend(made.iter().map(|p| Effect::Spawn { pid: p.pid, sab: true }));
        self.procs.extend(made);
        self.last_pid = first + n as u32 - 1;
        Ok(first)
    }

    /// The worker of process `i` is up: send its Start. A version mismatch ends it with 126, a
    /// VFS program that is gone with 127, saying so on its console.
    fn ready(&mut self, vfs: &Vfs, i: usize, version: u8) {
        let p = &mut self.procs[i];
        let Some((msg, program)) = p.start.take() else { return };
        let (pid, tty) = (p.pid, p.tty);
        let (why, status) = match program {
            _ if version != wire::VERSION => {
                (b"the OS was updated; reload the page\n".to_vec(), wire::CANNOT_EXECUTE)
            }
            Program::Url(url) => {
                return self.effects.push(Effect::Start { pid, msg, program: Load::Url(url) });
            }
            Program::Vfs(path) => match vfs.read(&path) {
                Ok(bytes) => {
                    let program = Load::Bytes(bytes.to_vec());
                    return self.effects.push(Effect::Start { pid, msg, program });
                }
                Err(_) => ([p.name().as_bytes(), b": not found\n"].concat(), wire::NOT_FOUND),
            },
        };
        self.show(tty, i, &why);
        self.end(i, status);
    }

    /// The console `pid` holds, while it runs.
    fn console(&mut self, pid: u32) -> Option<&mut Console> {
        let i = self.find(pid, true)?;
        self.procs[i].console.as_mut()
    }

    /// Console output `data` from process `i`: onto the console `tty` holds (0: its own), each LF
    /// after a CR while that console is cooked (ONLCR, as the mode was when it was written);
    /// wakes its owner.
    fn show(&mut self, tty: u32, i: usize, data: &[u8]) {
        if !data.is_empty() {
            let at = self.find(tty, false).unwrap_or(i);
            let p = &mut self.procs[at];
            let cooked = p.console.as_ref().is_some_and(|c| c.bits & wire::MODE_RAW == 0);
            for &b in data {
                if b == b'\n' && cooked {
                    p.out.push(b'\r');
                }
                p.out.push(b);
            }
            self.touch(self.procs[i].owner);
        }
    }

    /// Answers what process `i` waits on, if it can be: EVENTS from its oldest event, CONS_READ
    /// from its console's input (or its end of file; no console, at once), PIPE_READ from its
    /// pipe (once its writer ended, an end of file; a writer waiting for room goes on), WAIT
    /// once its job ended. A PIPE_WRITE is answered by its reader's reads.
    fn serve(&mut self, i: usize) {
        let p = &self.procs[i];
        let (pid, tty, writer) = (p.pid, p.tty, p.writer);
        let Some((op, max)) = p.waiting else { return };
        let reply = match op {
            wire::EVENTS => {
                let p = &mut self.procs[i];
                let Some(head) = p.events.first_mut() else { return };
                let data = take(head, max);
                if head.is_empty() {
                    p.events.remove(0);
                }
                Ok(data)
            }
            wire::CONS_READ => match self.console(tty) {
                Some(c) if c.input.is_empty() && !c.eof => return,
                Some(c) => {
                    // An end of file is read once, after the input before it.
                    c.eof &= !c.input.is_empty();
                    Ok(take(&mut c.input, max))
                }
                None => Ok(Vec::new()),
            },
            wire::PIPE_READ => {
                let w = self.find(writer, true);
                let p = &mut self.procs[i];
                if p.pipe.is_empty() && w.is_some() {
                    return;
                }
                let (data, left) = (take(&mut p.pipe, max), p.pipe.len());
                let w =
                    w.filter(|&w| left <= wire::MAX_PIPE && self.procs[w].reads(wire::PIPE_WRITE));
                if let Some(w) = w {
                    self.procs[w].waiting = None;
                    self.reply(writer, Ok(Vec::new()));
                }
                Ok(data)
            }
            wire::WAIT => {
                let kids = |p: &Process| p.parent == pid && p.job == max;
                if self.procs.iter().any(|p| kids(p) && p.status.is_none()) {
                    return;
                }
                // Children come after their parent, in pid order: `i` stays.
                let last = self.procs.iter().rev().find(|p| kids(p)).and_then(|p| p.status);
                self.procs.retain(|p| !kids(p));
                last.map(|s| s.to_le_bytes().into()).ok_or(wire::ECHILD)
            }
            _ => return,
        };
        self.procs[i].waiting = None;
        self.reply(pid, reply);
    }
}

/// Makes `/bin/<name>` the marker [`program`] reads as the URL `bin/<wasm>.wasm`, which the page
/// fetches when the program first runs.
pub fn install(vfs: &mut Vfs, name: &str, wasm: &str) -> Result<(), VfsError> {
    vfs.write(&["/bin/", name].concat(), ["#!wasm bin/", wasm, ".wasm\n"].concat().as_bytes())
}

/// The program the file at `path` (absolute) is: itself if it holds wasm, else what its marker
/// `#!wasm <target> [<sha256>]` names, an absolute VFS path or a URL `[A-Za-z0-9._/-]+` without
/// `..`. Err: whether the file is missing (else it is no program).
pub fn program(vfs: &Vfs, path: &str) -> Result<Program, bool> {
    let data = vfs.read(path).map_err(|_| true)?;
    if data.starts_with(b"\0asm") {
        return Ok(Program::Vfs(path.into()));
    }
    let line = data.strip_prefix(b"#!wasm ").ok_or(false)?.split(|&b| b == b'\n').next();
    let t = line.unwrap_or_default().split(u8::is_ascii_whitespace).find(|t| !t.is_empty());
    // Lossy: `str::from_utf8` would add 0.3 KB of boot wasm. Bad UTF-8 fails the URL check,
    // and a VFS path with it is not found.
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

/// Serves file op `m` for a process under `roots`: the reply, or the errno. What writes (OPEN
/// with CREAT or TRUNC, WRITE, MKDIR, REMOVE, RENAME, SETLEN) is EROFS under /bin, which only
/// the desktop writes: what a `/bin` marker names runs as that app, the overlay's too.
fn file<'a>(vfs: &mut Vfs, roots: &[String], m: Msg<'a>) -> Result<Vec<u8>, u16> {
    let at = |path: &'a str| checked(roots, path);
    let bin = |p: &str| p.strip_prefix("/bin").is_some_and(|t| t.is_empty() || t.starts_with('/'));
    let to = |path: &'a str| at(path).and_then(|p| if bin(p) { Err(wire::EROFS) } else { Ok(p) });
    let none = |r: Result<(), VfsError>| r.map(|()| Vec::new()).map_err(wire::errno);
    match m {
        Msg::Open { oflags, path } if oflags & (wire::O_CREAT | wire::O_TRUNC) != 0 => {
            open(vfs, to(path)?, oflags)
        }
        Msg::Open { oflags, path } => open(vfs, at(path)?, oflags),
        Msg::Read { off, max, path } => {
            let data = vfs.read(at(path)?).map_err(wire::errno)?;
            let from = off.min(data.len() as u64) as usize;
            let n = (data.len() - from).min(max as usize).min(wire::MAX_PAYLOAD);
            Ok(data[from..from + n].to_vec())
        }
        Msg::Write { off, path, data } => {
            Ok(vfs.write_at(to(path)?, off, data).map_err(wire::errno)?.to_le_bytes().into())
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
        Msg::Mkdir { path } => none(vfs.mkdir(to(path)?)),
        Msg::Remove { kind, path } => match (kind, to(path)?) {
            (wire::KIND_FILE, path) if vfs.is_dir(path) => Err(wire::EISDIR),
            (wire::KIND_DIR, path) if vfs.is_file(path) => Err(wire::ENOTDIR),
            (wire::KIND_FILE | wire::KIND_DIR, path) => none(vfs.remove(path, false)),
            _ => Err(wire::EINVAL),
        },
        Msg::Rename { from, to: dest } => none(vfs.rename(to(from)?, to(dest)?)),
        Msg::SetLen { len, path } => none(vfs.set_len(to(path)?, len)),
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
