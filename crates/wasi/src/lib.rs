//! The kernel's worker half, deterministic too: one [`Proc`] per process, in cpu.wasm. It holds
//! the fds, offsets, preopens, argv and env, and turns each WASI preview 1 call into local work or
//! a [`wire`] request to its [`Host`]; guest memory is reached only through [`Mem`]. A function's
//! id indexes [`NAMES`] (the import allowlist) and [`ARITY`] and has a constant.
//!
//! Preopens: fd 3 is `.` (the cwd), then each top-level directory (roots `["/"]`) or each root,
//! then /dev. wasi-libc names `.` as it would `/` (the empty prefix), so a program that started
//! outside `/` reaches `/x` from `.`, up (sh: `./../../x` from the home), or it would be the
//! cwd's `x`. Paths resolve lexically (`..` stops at "/"); main checks the roots. fds are paths:
//! each file call is one wire op (a WRITE 64 KiB at most). fds 0 and 1 are as the Start says:
//! the console, a file, a pipe (in a job: each read one PIPE_READ, each write one PIPE_WRITE) or,
//! for stdout, nothing; fd 2 is the console. /dev is local: `null`, `tty` (the console: a read
//! waits for its input), `consctl` (words written set its mode: `rawon`, `rawoff`, `echooff`,
//! `echoon`, each fd from cooked with echo), `winsize` (`"<cols> <rows>\n"`), `draw` (a write
//! is one uiwire frame), `events` (a read is one event) and `job` (a write is one
//! [`wire::Job`], started as the writer's children; a read waits for it to end and says its
//! status, `"<n>\n"`, then reads end of file until the next write). Not yet: poll_oneoff,
//! NONBLOCK.

#![forbid(unsafe_code)]

use kernel::snap::fnv64;
use kernel::wire::{self, Msg, Reader, Writer};
use kernel::wire::{E2BIG, EBADF, EEXIST, EFAULT, EILSEQ, EINVAL, EISDIR, ENOENT, ENOSYS};
use kernel::wire::{ENOTCAPABLE, ENOTDIR, ENOTSUP, ESPIPE, KIND_DIR, KIND_FILE, O_CREAT};
use kernel::wire::{MODE_NOECHO, MODE_RAW};
use vfs::Vfs;

/// Calls `$m! { ID name(types); .. }` on the 46 `wasi_snapshot_preview1` functions in witx order,
/// with their core argument types (u32 is i32, u64 is i64); a function's id is its index.
#[macro_export]
macro_rules! functions {
    ($m:ident) => { $m! {
    ARGS_GET args_get(u32 u32); ARGS_SIZES_GET args_sizes_get(u32 u32);
    ENVIRON_GET environ_get(u32 u32); ENVIRON_SIZES_GET environ_sizes_get(u32 u32);
    CLOCK_RES_GET clock_res_get(u32 u32); CLOCK_TIME_GET clock_time_get(u32 u64 u32);
    FD_ADVISE fd_advise(u32 u64 u64 u32); FD_ALLOCATE fd_allocate(u32 u64 u64);
    FD_CLOSE fd_close(u32); FD_DATASYNC fd_datasync(u32); FD_FDSTAT_GET fd_fdstat_get(u32 u32);
    FD_FDSTAT_SET_FLAGS fd_fdstat_set_flags(u32 u32);
    FD_FDSTAT_SET_RIGHTS fd_fdstat_set_rights(u32 u64 u64);
    FD_FILESTAT_GET fd_filestat_get(u32 u32); FD_FILESTAT_SET_SIZE fd_filestat_set_size(u32 u64);
    FD_FILESTAT_SET_TIMES fd_filestat_set_times(u32 u64 u64 u32);
    FD_PREAD fd_pread(u32 u32 u32 u64 u32); FD_PRESTAT_GET fd_prestat_get(u32 u32);
    FD_PRESTAT_DIR_NAME fd_prestat_dir_name(u32 u32 u32); FD_PWRITE fd_pwrite(u32 u32 u32 u64 u32);
    FD_READ fd_read(u32 u32 u32 u32); FD_READDIR fd_readdir(u32 u32 u32 u64 u32);
    FD_RENUMBER fd_renumber(u32 u32); FD_SEEK fd_seek(u32 u64 u32 u32); FD_SYNC fd_sync(u32);
    FD_TELL fd_tell(u32 u32); FD_WRITE fd_write(u32 u32 u32 u32);
    PATH_CREATE_DIRECTORY path_create_directory(u32 u32 u32);
    PATH_FILESTAT_GET path_filestat_get(u32 u32 u32 u32 u32);
    PATH_FILESTAT_SET_TIMES path_filestat_set_times(u32 u32 u32 u32 u64 u64 u32);
    PATH_LINK path_link(u32 u32 u32 u32 u32 u32 u32);
    PATH_OPEN path_open(u32 u32 u32 u32 u32 u64 u64 u32 u32);
    PATH_READLINK path_readlink(u32 u32 u32 u32 u32 u32);
    PATH_REMOVE_DIRECTORY path_remove_directory(u32 u32 u32);
    PATH_RENAME path_rename(u32 u32 u32 u32 u32 u32);
    PATH_SYMLINK path_symlink(u32 u32 u32 u32 u32); PATH_UNLINK_FILE path_unlink_file(u32 u32 u32);
    POLL_ONEOFF poll_oneoff(u32 u32 u32 u32); PROC_EXIT proc_exit(u32); PROC_RAISE proc_raise(u32);
    SCHED_YIELD sched_yield(); RANDOM_GET random_get(u32 u32); SOCK_ACCEPT sock_accept(u32 u32 u32);
    SOCK_RECV sock_recv(u32 u32 u32 u32 u32 u32); SOCK_SEND sock_send(u32 u32 u32 u32 u32);
    SOCK_SHUTDOWN sock_shutdown(u32 u32);
    } };
}

macro_rules! names {
    ($($id:ident $name:ident($($t:ident)*);)+) => {
        /// The 46 `wasi_snapshot_preview1` functions; a function's id is its index.
        pub const NAMES: [&str; 46] = [$(stringify!($name)),+];
        /// How many core-wasm arguments each function takes, by id.
        pub const ARITY: [usize; 46] = [$(<[&str]>::len(&[$(stringify!($t)),*])),+];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        enum Id { $($id),+ }
        $(#[doc = stringify!($name)] pub const $id: usize = Id::$id as usize;)+
    };
}

functions!(names);

/// Clock resolution, and the floor of every time a guest sees: 100 µs.
const RES_NS: u64 = 100_000;
/// The most bytes one request, console or random copy moves; the most
/// iovecs a call takes (POSIX `IOV_MAX`, more is EINVAL).
const CHUNK: u32 = 65_536;
const IOV_MAX: u32 = 1_024;
/// A console's rights: FD_READ, FD_FDSTAT_SET_FLAGS, FD_WRITE, FD_FILESTAT_GET, POLL_FD_READWRITE;
/// never FD_SEEK or FD_TELL, so wasi-libc's `isatty()` holds for a character device.
const CONSOLE_RIGHTS: u64 = (1 << 1) | (1 << 3) | (1 << 6) | (1 << 21) | (1 << 27);
/// All 30 rights of preview 1, which files and directories report.
const ALL_RIGHTS: u64 = (1 << 30) - 1;

/// A guest's linear memory: bounds-checked copies only; out of bounds is [`wire::EFAULT`].
pub trait Mem {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16>;
    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16>;
}

/// What a [`Proc`] needs from its worker; `console` and `random` get 64 KiB at most a time.
pub trait Host {
    /// Sends a request and blocks for its reply: `(errno, data)`.
    fn call(&mut self, req: &[u8]) -> (u16, Vec<u8>);
    /// Console output into the ring; blocks while it is full.
    fn console(&mut self, bytes: &[u8]);
    /// WASI clock `clock` (Proc asks only 0, realtime, and 1, monotonic) in
    /// nanoseconds, floored to 100 µs; `None` for an unknown clock.
    fn now_ns(&mut self, clock: u32) -> Option<u64>;
    fn random(&mut self, buf: &mut [u8]);
    /// Sleeps until the timeout (`None`: forever) or, with `stdin`, console input: whether it came.
    fn wait(&mut self, timeout_ns: Option<u64>, stdin: bool) -> bool;
    /// `(cols, rows)` of the console, live.
    fn winsize(&self) -> (u16, u16);
}

/// The guest called `proc_exit` or `proc_raise`: its status in 0..=255 (the code & 0xFF,
/// or 128 + the signal), which the worker posts as EXIT before it stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit(pub u32);

/// What an fd is: the console, a VFS directory or file, or a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Console,
    Dir,
    File,
    Null,
    Winsize,
    Draw,
    Events,
    Consctl,
    Pipe,
    Job,
}

/// The devices of /dev, in listing order, and what each is.
const DEVS: [&str; 7] = ["consctl", "draw", "events", "job", "null", "tty", "winsize"];
#[rustfmt::skip]
const DEV_KINDS: [Kind; 7] =
    [Kind::Consctl, Kind::Draw, Kind::Events, Kind::Job, Kind::Null, Kind::Console, Kind::Winsize];
/// What /dev/consctl takes: words that set (even) or clear (odd) a mode bit.
const CTL: [(&str, u8); 4] =
    [("rawon", MODE_RAW), ("rawoff", MODE_RAW), ("echooff", MODE_NOECHO), ("echoon", MODE_NOECHO)];

/// An open fd: its kind, absolute path (`/dev/tty` for a console) and offset (/dev/consctl:
/// the mode's bits; /dev/job: the job started and not yet waited for, 0 for none).
#[derive(Debug)]
struct Fd {
    kind: Kind,
    path: String,
    /// A preopen's name (`.` or its path); empty for any other fd.
    pre: String,
    pos: u64,
    append: bool,
    /// /dev/events: the part of an event the guest has not read yet; /dev/job: of the status.
    buf: Vec<u8>,
    /// A directory's `(filetype, name)` entries as of fd_readdir at cookie 0, `.` and `..` first.
    list: Vec<(u8, String)>,
}

impl Fd {
    fn new(kind: Kind, path: &str) -> Fd {
        let (pre, buf, list) = (String::new(), Vec::new(), Vec::new());
        Fd { kind, path: path.into(), pre, pos: 0, append: false, buf, list }
    }
}

/// One process's private kernel state.
#[derive(Debug)]
pub struct Proc {
    /// argv and env as C strings back to back.
    args: Vec<u8>,
    env: Vec<u8>,
    fds: Vec<Option<Fd>>,
    tty: bool,
}

impl Proc {
    /// The process `start` describes. Env: `HOME`, `USER=guest`, `PWD`, `PATH` and, with a tty,
    /// `TERM=xterm-256color`, `COLUMNS`, `LINES`; each `K=V` of `start.env` replaces its key or is
    /// appended. fds 0 to 2 are the console, but stdin from a file (an OPEN: it must be one) or a
    /// pipe, and stdout to a file (OPEN CREAT, TRUNC unless appending), a pipe or nothing; then
    /// the preopens. EINVAL for an env entry without key or `=`, or a NUL in any string; E2BIG
    /// past [`wire::MAX_START`]; EISDIR for a stdin that is a directory; a failed OPEN's or
    /// LIST's errno.
    pub fn new(start: &wire::Start, h: &mut dyn Host) -> Result<Proc, u16> {
        let home = Vfs::HOME;
        let mut env = vec![["HOME=", home].concat(), "USER=guest".into()];
        env.extend([["PWD=", &start.cwd].concat(), ["PATH=/bin:", home, "/.local/bin"].concat()]);
        if let Some((cols, rows)) = start.tty {
            env.push("TERM=xterm-256color".into());
            env.push(["COLUMNS=", &cols.to_string()].concat());
            env.push(["LINES=", &rows.to_string()].concat());
        }
        for e in &start.env {
            let key = e.find('=').filter(|&i| i > 0).and_then(|i| e.get(..=i)).ok_or(EINVAL)?;
            match env.iter_mut().find(|d| d.starts_with(key)) {
                Some(d) => d.clone_from(e),
                None => env.push(e.clone()),
            }
        }
        let (args, env) = (cstrs(&start.argv)?, cstrs(&env)?);
        let mut fds: Vec<_> = (0..3).map(|_| Some(Fd::new(Kind::Console, "/dev/tty"))).collect();
        match &start.stdin {
            wire::Stdin::File(path) if open(h, 0, path)?.0 == 3 => return Err(EISDIR),
            wire::Stdin::File(path) => fds[0] = Some(Fd::new(Kind::File, path)),
            wire::Stdin::Pipe => fds[0] = Some(Fd::new(Kind::Pipe, "")),
            wire::Stdin::Console => {}
        }
        match &start.stdout {
            wire::Stdout::File { path, append } => {
                open(h, O_CREAT | if *append { 0 } else { wire::O_TRUNC }, path)?;
                fds[1] = Some(Fd { append: *append, ..Fd::new(Kind::File, path) });
            }
            wire::Stdout::Pipe => fds[1] = Some(Fd::new(Kind::Pipe, "")),
            wire::Stdout::Null => fds[1] = Some(Fd::new(Kind::Null, "/dev/null")),
            wire::Stdout::Console => {}
        }
        let roots = match &start.roots[..] {
            [root] if root == "/" => {
                let top = list(h, "/", 0)?.into_iter().filter(|(t, name)| *t == 3 && name != "dev");
                top.map(|(_, name)| ["/", &name].concat()).collect()
            }
            roots => roots.to_vec(),
        };
        let pre = [(start.cwd.clone(), ".".into())].into_iter();
        let pre = pre.chain(roots.into_iter().chain(["/dev".into()]).map(|r| (r.clone(), r)));
        fds.extend(pre.map(|(path, pre)| Some(Fd { pre, ..Fd::new(Kind::Dir, &path) })));
        Ok(Proc { args, env, fds, tty: start.tty.is_some() })
    }

    /// Runs function `f` on its [`ARITY`]`[f]` arguments, each widened to u64 (i64s as bits):
    /// the errno (ENOSYS for an unknown `f`, EINVAL for a wrong count), or an [`Exit`].
    pub fn call(
        &mut self,
        f: usize,
        a: &[u64],
        m: &mut dyn Mem,
        h: &mut dyn Host,
    ) -> Result<u16, Exit> {
        match f {
            _ if ARITY.get(f) != Some(&a.len()) => Ok(if f < 46 { EINVAL } else { ENOSYS }),
            PROC_EXIT => Err(Exit(a[0] as u32 & 0xFF)),
            PROC_RAISE if (1..128).contains(&a[0]) => Err(Exit(128 + a[0] as u32)),
            _ => Ok(self.run(f, a, m, h).err().unwrap_or(0)),
        }
    }

    /// Function `f` on `a`, whose count [`Proc::call`] checked.
    fn run(&mut self, f: usize, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let p = |i: usize| a[i] as u32;
        let path = |s: &Proc, m: &dyn Mem, i: usize| s.path(p(i), p(i + 1), p(i + 2), m);
        match f {
            PROC_RAISE => (a[0] == 0).then_some(()).ok_or(EINVAL),
            ARGS_GET | ARGS_SIZES_GET => strs(&self.args, f == ARGS_GET, p(0), p(1), m),
            ENVIRON_GET | ENVIRON_SIZES_GET => strs(&self.env, f == ENVIRON_GET, p(0), p(1), m),
            CLOCK_RES_GET => clock(a[0]).and_then(|_| m.write(p(1), &RES_NS.to_le_bytes())),
            CLOCK_TIME_GET => clock(a[0])
                .and_then(|c| h.now_ns(c).ok_or(EINVAL))
                .and_then(|t| m.write(p(2), &(t / RES_NS * RES_NS).to_le_bytes())),
            RANDOM_GET => spans(p(0), p(1)).try_for_each(|(at, n)| {
                let mut b = vec![0; n as usize];
                h.random(&mut b);
                m.write(at, &b)
            }),
            SCHED_YIELD => Ok(()),
            FD_PRESTAT_GET => {
                let name = self.preopen(p(0))?;
                m.write(p(1), &Writer::default().u32(0).u32(name.len() as u32).done())
            }
            FD_PRESTAT_DIR_NAME => match self.preopen(p(0))? {
                name if (p(2) as usize) < name.len() => Err(EINVAL),
                name => m.write(p(1), name.as_bytes()),
            },
            FD_FDSTAT_GET => self.fdstat(p(0), p(1), m),
            FD_FDSTAT_SET_FLAGS => {
                self.fd_mut(p(0)).map(|f| f.append = f.kind == Kind::File && a[1] & 1 != 0)
            }
            FD_ADVISE | FD_DATASYNC | FD_SYNC | FD_FDSTAT_SET_RIGHTS | FD_FILESTAT_SET_TIMES => {
                self.fd(p(0)).map(drop)
            }
            FD_CLOSE => {
                self.fds.get_mut(p(0) as usize).and_then(Option::take).map(drop).ok_or(EBADF)
            }
            FD_RENUMBER => {
                self.fd(p(1))?;
                let fd = self.fds.get_mut(p(0) as usize).and_then(Option::take).ok_or(EBADF)?;
                self.fds[p(1) as usize] = Some(fd);
                Ok(())
            }
            FD_SEEK => {
                let f = self.file(p(0))?;
                let base = match a[2] {
                    0 => 0,
                    1 => f.pos,
                    2 => open(h, 0, &f.path)?.1,
                    _ => return Err(EINVAL),
                };
                f.pos = base.checked_add_signed(a[1] as i64).ok_or(EINVAL)?;
                m.write(p(3), &f.pos.to_le_bytes())
            }
            FD_TELL => self.file(p(0)).and_then(|f| m.write(p(1), &f.pos.to_le_bytes())),
            FD_READ | FD_PREAD => self.read(a, m, h),
            FD_WRITE | FD_PWRITE => self.write(a, m, h),
            FD_READDIR => self.readdir(a, m, h),
            FD_FILESTAT_GET => m.write(p(1), &filestat(h, &self.fd(p(0))?.path)?),
            FD_FILESTAT_SET_SIZE => req(h, Msg::SetLen { len: a[1], path: &self.file(p(0))?.path }),
            FD_ALLOCATE => {
                let (path, end) = (&self.file(p(0))?.path, a[1].checked_add(a[2]).ok_or(EINVAL)?);
                let grow = open(h, 0, path)?.1 < end;
                if grow { req(h, Msg::SetLen { len: end, path }) } else { Ok(()) }
            }
            PATH_OPEN => self.open(a, m, h),
            PATH_FILESTAT_GET => m.write(p(4), &filestat(h, &self.path(p(0), p(2), p(3), m)?)?),
            PATH_FILESTAT_SET_TIMES => self.path(p(0), p(2), p(3), m).map(drop),
            PATH_CREATE_DIRECTORY => req(h, Msg::Mkdir { path: &writable(path(self, m, 0))? }),
            PATH_REMOVE_DIRECTORY | PATH_UNLINK_FILE => {
                let kind = if f == PATH_UNLINK_FILE { KIND_FILE } else { KIND_DIR };
                req(h, Msg::Remove { kind, path: &writable(path(self, m, 0))? })
            }
            PATH_RENAME => {
                let (from, to) = (writable(path(self, m, 0))?, writable(path(self, m, 3))?);
                req(h, Msg::Rename { from: &from, to: &to })
            }
            PATH_LINK | PATH_SYMLINK | SOCK_ACCEPT..=SOCK_SHUTDOWN => Err(ENOTSUP),
            PATH_READLINK => Err(EINVAL),
            _ => Err(ENOSYS),
        }
    }

    fn fd(&self, fd: u32) -> Result<&Fd, u16> {
        self.fds.get(fd as usize).and_then(Option::as_ref).ok_or(EBADF)
    }

    fn fd_mut(&mut self, fd: u32) -> Result<&mut Fd, u16> {
        self.fds.get_mut(fd as usize).and_then(Option::as_mut).ok_or(EBADF)
    }

    /// A file fd: EISDIR for a directory, ESPIPE for the console or a device.
    fn file(&mut self, fd: u32) -> Result<&mut Fd, u16> {
        let f = self.fd_mut(fd)?;
        match f.kind {
            Kind::File => Ok(f),
            Kind::Dir => Err(EISDIR),
            _ => Err(ESPIPE),
        }
    }

    /// The name of preopen `fd`; EBADF for any other fd.
    fn preopen(&self, fd: u32) -> Result<&str, u16> {
        self.fd(fd).ok().map(|f| f.pre.as_str()).filter(|n| !n.is_empty()).ok_or(EBADF)
    }

    /// The absolute path the `len` bytes at `at` name from directory `fd`, resolved lexically;
    /// ENOTDIR for another fd, EILSEQ for non-UTF-8, ENOTCAPABLE for an absolute path.
    fn path(&self, fd: u32, at: u32, len: u32, m: &dyn Mem) -> Result<String, u16> {
        let dir = Some(self.fd(fd)?).filter(|d| d.kind == Kind::Dir).ok_or(ENOTDIR)?;
        let b = m.read(at, len)?;
        let rel = core::str::from_utf8(&b).map_err(|_| EILSEQ)?;
        (!rel.starts_with('/')).then_some(()).ok_or(ENOTCAPABLE)?;
        Vfs::normalize("/", &[&dir.path, "/", rel].concat()).map_err(wire::errno)
    }

    /// The 24-byte fdstat: u8 filetype, u16 flags at 2, u64 rights at 8 and 16. A console is
    /// a character device (2) with a tty, else UNKNOWN (0); files and directories have all rights.
    fn fdstat(&self, fd: u32, at: u32, m: &mut dyn Mem) -> Result<(), u16> {
        let f = self.fd(fd)?;
        let (kind, base, inheriting) = match f.kind {
            Kind::Dir => (3, ALL_RIGHTS, ALL_RIGHTS),
            Kind::File => (4, ALL_RIGHTS, 0),
            Kind::Console if !self.tty => (0, CONSOLE_RIGHTS, 0),
            Kind::Pipe => (0, CONSOLE_RIGHTS, 0),
            _ => (2, CONSOLE_RIGHTS, 0),
        };
        let w = Writer::default().u8(kind).u8(0).u16(f.append.into()).u32(0).u64(base);
        m.write(at, &w.u64(inheriting).done())
    }

    /// path_open `(fd, dirflags, path, len, oflags, base, inheriting, fdflags, opened)` on the
    /// lowest free fd: a device here, else an OPEN. Rights are not enforced; of fdflags, APPEND.
    fn open(&mut self, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let path = self.path(a[0] as u32, a[2] as u32, a[3] as u32, m)?;
        let o = (a[4] & 0xF) as u8;
        let slot = self.fds.iter().position(Option::is_none).unwrap_or(self.fds.len());
        m.write(a[8] as u32, &(slot as u32).to_le_bytes())?;
        let kind = match dev(&path) {
            Some(Ok(_)) if o & (O_CREAT | wire::O_EXCL) == O_CREAT | wire::O_EXCL => Err(EEXIST),
            Some(Ok(k)) if o & wire::O_DIRECTORY != 0 && k != Kind::Dir => Err(ENOTDIR),
            Some(kind) => kind,
            None => open(h, o, &path).map(|(t, _)| if t == 3 { Kind::Dir } else { Kind::File }),
        }?;
        self.fds.resize_with(self.fds.len().max(slot + 1), || None);
        self.fds[slot] =
            Some(Fd { append: kind == Kind::File && a[7] & 1 != 0, ..Fd::new(kind, &path) });
        Ok(())
    }

    /// fd_read `(fd, iovs, n, nread)` or fd_pread `(.., off, nread)`: one request (64 KiB) at most.
    fn read(&mut self, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let list = iovs(m, a[1] as u32, a[2] as u32)?;
        let max = list.iter().fold(0u32, |n, v| n.saturating_add(v.1)).min(CHUNK);
        let (f, at) = (self.fd_mut(a[0] as u32)?, (a.len() == 5).then(|| a[3]));
        let data = match (f.kind, at) {
            (Kind::Dir, _) => return Err(EISDIR),
            (Kind::Draw, _) => return Err(EBADF),
            _ if max == 0 => Vec::new(),
            (Kind::File, _) => {
                let off = at.unwrap_or(f.pos);
                call(h, &Msg::Read { off, max, path: &f.path }.encode())?
            }
            (_, Some(_)) => return Err(ESPIPE),
            (Kind::Console, _) => call(h, &Msg::ConsRead { max }.encode())?,
            (Kind::Pipe, _) => call(h, &Msg::PipeRead { max }.encode())?,
            (Kind::Null | Kind::Consctl, _) => Vec::new(),
            // The status, once the job ended, as text; what is not read waits for the next read.
            (Kind::Job, _) => {
                if f.buf.is_empty() && f.pos != 0 {
                    let status = call(h, &Msg::Wait { job: f.pos as u32 }.encode())?;
                    let status = Reader(&status).u32().ok_or(EINVAL)? as i32;
                    let text = [status.to_string().as_str(), "\n"].concat();
                    (f.pos, f.buf) = (0, text.into_bytes());
                }
                let n = scatter(m, &list, &f.buf)?;
                f.buf.drain(..n as usize);
                return m.write(a[a.len() - 1] as u32, &n.to_le_bytes());
            }
            (Kind::Winsize, _) => {
                let ((cols, rows), at) = (h.winsize(), f.pos.min(16) as usize);
                format!("{cols} {rows}\n").into_bytes().get(at..).unwrap_or_default().to_vec()
            }
            // What the guest does not take of an event waits for its next read.
            (Kind::Events, _) => {
                if f.buf.is_empty() {
                    f.buf = call(h, &Writer::new(wire::EVENTS).u32(CHUNK).done())?;
                }
                let n = scatter(m, &list, &f.buf)?;
                f.buf.drain(..n as usize);
                return m.write(a[a.len() - 1] as u32, &n.to_le_bytes());
            }
        };
        let n = scatter(m, &list, &data)?;
        f.pos += if at.is_none() { u64::from(n) } else { 0 };
        m.write(a[a.len() - 1] as u32, &n.to_le_bytes())
    }

    /// fd_write `(fd, iovs, n, nwritten)` or fd_pwrite `(.., off, nwritten)`: a file takes 64 KiB
    /// at most (one WRITE), /dev/draw a whole frame (one DRAW, E2BIG past [`kernel::MAX_FRAME`]).
    fn write(&mut self, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let list = iovs(m, a[1] as u32, a[2] as u32)?;
        let total = list.iter().fold(0u32, |n, v| n.saturating_add(v.1));
        let (f, at) = (self.fd_mut(a[0] as u32)?, (a.len() == 5).then(|| a[3]));
        let n = match (f.kind, at) {
            (Kind::File, _) => {
                let mut data = Vec::new();
                let n = gather(m, &list, CHUNK, &mut |b| data.extend_from_slice(b))?;
                let off = at.unwrap_or(if f.append { wire::APPEND } else { f.pos });
                let size = call(h, &Msg::Write { off, path: &f.path, data: &data }.encode())?;
                let size = Reader(&size).u64().ok_or(EINVAL)?;
                if at.is_none() {
                    f.pos = if f.append { size } else { f.pos + u64::from(n) };
                }
                n
            }
            (_, Some(_)) => return Err(ESPIPE),
            (Kind::Console, _) => gather(m, &list, u32::MAX, &mut |b| h.console(b))?,
            (Kind::Null, _) => total,
            (Kind::Pipe, _) => {
                let mut data = Vec::new();
                let n = gather(m, &list, CHUNK, &mut |b| data.extend_from_slice(b))?;
                req(h, Msg::PipeWrite { data: &data })?;
                n
            }
            // A whole job in one write; a read waits for it by its first pid.
            (Kind::Job, _) if total as usize > wire::MAX_PAYLOAD => return Err(E2BIG),
            (Kind::Job, _) => {
                let mut job = Vec::new();
                let n = gather(m, &list, total, &mut |b| job.extend_from_slice(b))?;
                (n == total).then_some(()).ok_or(EFAULT)?;
                let first = call(h, &Msg::Spawn { job: &job }.encode())?;
                (f.pos, f.buf) = (u64::from(Reader(&first).u32().ok_or(EINVAL)?), Vec::new());
                n
            }
            (Kind::Consctl, _) => {
                let mut text = Vec::new();
                let n = gather(m, &list, CHUNK, &mut |b| text.extend_from_slice(b))?;
                for w in text.split(u8::is_ascii_whitespace).filter(|w| !w.is_empty()) {
                    let i = CTL.iter().position(|c| c.0.as_bytes() == w).ok_or(EINVAL)?;
                    let bit = u64::from(CTL[i].1);
                    f.pos = if i % 2 == 0 { f.pos | bit } else { f.pos & !bit };
                }
                req(h, Msg::ConsMode { bits: f.pos as u8 })?;
                n
            }
            (Kind::Draw, _) if total as usize > kernel::MAX_FRAME => return Err(E2BIG),
            (Kind::Draw, _) => {
                let mut frame = vec![wire::DRAW];
                let n = gather(m, &list, total, &mut |b| frame.extend_from_slice(b))?;
                (n == total).then_some(()).ok_or(EFAULT).and_then(|()| call(h, &frame))?;
                total
            }
            _ => return Err(EBADF),
        };
        m.write(a[a.len() - 1] as u32, &n.to_le_bytes())
    }

    /// fd_readdir `(fd, buf, len, cookie, bufused)`: from entry `cookie` of the listing taken at
    /// cookie 0 (removals never skip one), 24-byte dirents (u64 next, u64 ino: FNV-1a-64 of the
    /// path, u32 namlen, u8 type) and names, the last cut at `len`.
    fn readdir(&mut self, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let f = Some(self.fd_mut(a[0] as u32)?).filter(|f| f.kind == Kind::Dir).ok_or(ENOTDIR)?;
        if a[3] == 0 || f.list.is_empty() {
            f.list = vec![(3, ".".into()), (3, "..".into())];
            while let Some(page) =
                Some(list(h, &f.path, f.list.len() - 2)?).filter(|p| !p.is_empty())
            {
                f.list.extend(page);
            }
        }
        let (len, skip, mut out) = (a[2] as usize, a[3].try_into(), Writer::default());
        for (i, (kind, name)) in f.list.iter().enumerate().skip(skip.unwrap_or(usize::MAX)) {
            if out.0.len() >= len {
                break;
            }
            let full = Vfs::normalize("/", &[&f.path, "/", name].concat()).unwrap_or_default();
            let w = out.u64(i as u64 + 1).u64(fnv64(full.as_bytes())).u32(name.len() as u32);
            out = w.u32((*kind).into()).bytes(name.as_bytes());
        }
        out.0.truncate(len);
        m.write(a[1] as u32, &out.0)?;
        m.write(a[4] as u32, &(out.0.len() as u32).to_le_bytes())
    }
}

/// Sends `req` and waits: the reply's data, or its errno.
fn call(h: &mut dyn Host, req: &[u8]) -> Result<Vec<u8>, u16> {
    let (errno, data) = h.call(req);
    if errno == 0 { Ok(data) } else { Err(errno) }
}

/// Sends a request whose reply carries nothing.
fn req(h: &mut dyn Host, msg: Msg) -> Result<(), u16> {
    call(h, &msg.encode()).map(drop)
}

/// OPEN `path` with `oflags` (0 is a stat): its filetype (3 or 4) and size.
fn open(h: &mut dyn Host, oflags: u8, path: &str) -> Result<(u8, u64), u16> {
    let reply = call(h, &Msg::Open { oflags, path }.encode())?;
    let mut r = Reader(&reply);
    match (r.u8(), r.u64()) {
        (Some(KIND_DIR), Some(size)) => Ok((3, size)),
        (Some(KIND_FILE), Some(size)) => Ok((4, size)),
        _ => Err(EINVAL),
    }
}

/// The `(filetype, name)` entries of directory `path` from `skip`: one LIST page, or /dev's.
fn list(h: &mut dyn Host, path: &str, skip: usize) -> Result<Vec<(u8, String)>, u16> {
    if path == "/dev" {
        return Ok(DEVS.iter().skip(skip).map(|d| (2, d.to_string())).collect());
    }
    let reply = call(h, &Msg::List { skip: skip as u32, path }.encode())?;
    let mut r = Reader(&reply);
    let entry = || Some((if r.u8()? == KIND_DIR { 3 } else { 4 }, r.u64()?, r.str()?.into()));
    Ok(core::iter::from_fn(entry).map(|(kind, _, name)| (kind, name)).collect())
}

/// The kind of `path` in /dev, made here (ENOENT for no such device); `None` outside /dev.
fn dev(path: &str) -> Option<Result<Kind, u16>> {
    let name = path.strip_prefix("/dev")?;
    if name.is_empty() {
        return Some(Ok(Kind::Dir));
    }
    let name = name.strip_prefix('/')?;
    Some(DEVS.iter().position(|d| *d == name).map(|i| DEV_KINDS[i]).ok_or(ENOENT))
}

/// `path` for an op that changes the VFS: ENOTSUP inside /dev.
fn writable(path: Result<String, u16>) -> Result<String, u16> {
    path.and_then(|p| if dev(&p).is_some() { Err(ENOTSUP) } else { Ok(p) })
}

/// The 64-byte filestat of `path`: dev 0, ino its FNV-1a-64, filetype, nlink 1, size, times 0.
fn filestat(h: &mut dyn Host, path: &str) -> Result<Vec<u8>, u16> {
    let (kind, size) = match dev(path) {
        Some(kind) => (if kind? == Kind::Dir { 3 } else { 2 }, 0),
        None => open(h, 0, path)?,
    };
    let w = Writer::default().u64(0).u64(fnv64(path.as_bytes())).u64(kind.into()).u64(1);
    Ok(w.u64(size).u64(0).u64(0).u64(0).done())
}

/// The `n` iovecs `(buf, len)` at `at`; EINVAL past [`IOV_MAX`].
fn iovs(m: &dyn Mem, at: u32, n: u32) -> Result<Vec<(u32, u32)>, u16> {
    let b = m.read(at, (n <= IOV_MAX).then(|| n * 8).ok_or(EINVAL)?)?;
    let mut r = Reader(&b);
    Ok((0..n).map_while(|_| Some((r.u32()?, r.u32()?))).collect())
}

/// Hands `f` the bytes in iovecs `v`, at most `cap`, 64 KiB at a time: how many. A bad iovec
/// after some bytes ends a short write (as POSIX `writev`); before any, it is EFAULT.
fn gather(m: &dyn Mem, v: &[(u32, u32)], cap: u32, f: &mut dyn FnMut(&[u8])) -> Result<u32, u16> {
    let mut done = 0;
    for (at, k) in v.iter().flat_map(|&(at, len)| spans(at, len)) {
        let k = k.min(cap - done);
        if k == 0 {
            break;
        }
        match m.read(at, k) {
            Ok(b) => f(&b),
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
        done += k;
    }
    Ok(done)
}

/// Copies `data` into iovecs `list`: how many bytes; a bad iovec ends a short read (first: EFAULT).
fn scatter(m: &mut dyn Mem, list: &[(u32, u32)], mut data: &[u8]) -> Result<u32, u16> {
    let mut done = 0;
    for &(at, len) in list {
        let k = data.len().min(len as usize);
        if k == 0 {
            continue;
        }
        match m.write(at, &data[..k]) {
            Ok(()) => (done, data) = (done + k, &data[k..]),
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done as u32)
}

/// C strings back to back: EINVAL for a NUL inside one, E2BIG past [`wire::MAX_START`].
fn cstrs(list: &[String]) -> Result<Vec<u8>, u16> {
    let mut blob = Vec::new();
    for s in list {
        (!s.contains('\0')).then_some(()).ok_or(EINVAL)?;
        blob.extend_from_slice(s.as_bytes());
        blob.push(0);
    }
    (blob.len() <= wire::MAX_START).then_some(blob).ok_or(E2BIG)
}

/// Copies out the C strings in `blob`: with `get`, the strings at `buf` and
/// their addresses at `at`; without, their count at `at` and size at `buf`.
fn strs(blob: &[u8], get: bool, at: u32, buf: u32, m: &mut dyn Mem) -> Result<(), u16> {
    let ends = blob.split_inclusive(|&b| b == 0);
    if !get {
        m.write(at, &(ends.count() as u32).to_le_bytes())?;
        return m.write(buf, &(blob.len() as u32).to_le_bytes());
    }
    m.write(buf, blob)?;
    let addrs = ends.scan(buf, |p, s| Some(core::mem::replace(p, p.wrapping_add(s.len() as u32))));
    m.write(at, &addrs.flat_map(u32::to_le_bytes).collect::<Vec<u8>>())
}

/// A WASI clock as the [`Host`] reads it (CPU time, 2 and 3, as monotonic); EINVAL past 3.
fn clock(id: u64) -> Result<u32, u16> {
    (id < 4).then_some(id.min(1) as u32).ok_or(EINVAL)
}

/// `len` bytes from `at` in pieces of at most [`CHUNK`]; one past 4 GiB saturates: [`Mem`] refuses.
fn spans(at: u32, len: u32) -> impl Iterator<Item = (u32, u32)> {
    let piece = move |i: u32| (at.saturating_add(i * CHUNK), (len - i * CHUNK).min(CHUNK));
    (0..len.div_ceil(CHUNK)).map(piece)
}

#[cfg(test)]
mod tests;
