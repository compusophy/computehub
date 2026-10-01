//! The worker half of the kernel: one [`Proc`] per process, linked only into
//! cpu.wasm. It holds the fd table, offsets, preopens, argv and env, and
//! turns each WASI preview 1 call into local work or a [`wire`] request
//! through its [`Host`]. Guest memory is reached only through [`Mem`].
//! Function ids index [`NAMES`] (the import allowlist) and [`ARITY`]; each
//! has a constant (`FD_WRITE` is 26).
//!
//! Step 1: args, environ, clocks, random, fdstat, console writes on fds 0 to
//! 2, exit, raise and yield, plus the spec's lasting stubs (time setters 0,
//! links and sockets ENOTSUP, readlink EINVAL). `fd_prestat_get` is EBADF:
//! no preopens yet, which wasi-libc accepts (any other errno exits 71).
//! Every other call is ENOSYS until its step lands.

use crate::wire::{self, E2BIG, EBADF, EINVAL, ENOSYS, ENOTDIR, ENOTSUP};

macro_rules! names {
    ($($id:ident $name:literal $n:literal)+) => {
        /// The 46 `wasi_snapshot_preview1` functions; a function's id is its index.
        pub const NAMES: [&str; 46] = [$($name),+];
        /// How many core-wasm arguments each function takes, by id.
        pub const ARITY: [usize; 46] = [$($n),+];
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        enum Id { $($id),+ }
        $(#[doc = $name] pub const $id: usize = Id::$id as usize;)+
    };
}

names! {
    ARGS_GET "args_get" 2 ARGS_SIZES_GET "args_sizes_get" 2 ENVIRON_GET "environ_get" 2
    ENVIRON_SIZES_GET "environ_sizes_get" 2 CLOCK_RES_GET "clock_res_get" 2
    CLOCK_TIME_GET "clock_time_get" 3 FD_ADVISE "fd_advise" 4 FD_ALLOCATE "fd_allocate" 3
    FD_CLOSE "fd_close" 1 FD_DATASYNC "fd_datasync" 1 FD_FDSTAT_GET "fd_fdstat_get" 2
    FD_FDSTAT_SET_FLAGS "fd_fdstat_set_flags" 2 FD_FDSTAT_SET_RIGHTS "fd_fdstat_set_rights" 3
    FD_FILESTAT_GET "fd_filestat_get" 2 FD_FILESTAT_SET_SIZE "fd_filestat_set_size" 2
    FD_FILESTAT_SET_TIMES "fd_filestat_set_times" 4 FD_PREAD "fd_pread" 5
    FD_PRESTAT_GET "fd_prestat_get" 2 FD_PRESTAT_DIR_NAME "fd_prestat_dir_name" 3
    FD_PWRITE "fd_pwrite" 5 FD_READ "fd_read" 4 FD_READDIR "fd_readdir" 5
    FD_RENUMBER "fd_renumber" 2 FD_SEEK "fd_seek" 4 FD_SYNC "fd_sync" 1 FD_TELL "fd_tell" 2
    FD_WRITE "fd_write" 4 PATH_CREATE_DIRECTORY "path_create_directory" 3
    PATH_FILESTAT_GET "path_filestat_get" 5 PATH_FILESTAT_SET_TIMES "path_filestat_set_times" 7
    PATH_LINK "path_link" 7 PATH_OPEN "path_open" 9 PATH_READLINK "path_readlink" 6
    PATH_REMOVE_DIRECTORY "path_remove_directory" 3 PATH_RENAME "path_rename" 6
    PATH_SYMLINK "path_symlink" 5 PATH_UNLINK_FILE "path_unlink_file" 3
    POLL_ONEOFF "poll_oneoff" 4 PROC_EXIT "proc_exit" 1 PROC_RAISE "proc_raise" 1
    SCHED_YIELD "sched_yield" 0 RANDOM_GET "random_get" 2 SOCK_ACCEPT "sock_accept" 3
    SOCK_RECV "sock_recv" 6 SOCK_SEND "sock_send" 5 SOCK_SHUTDOWN "sock_shutdown" 2
}

/// Clock resolution, and the floor of every time a guest sees: 100 µs.
const RES_NS: u64 = 100_000;
/// The most bytes one console or random copy moves; the most iovecs a call
/// takes (POSIX `IOV_MAX`, more is EINVAL).
const CHUNK: u32 = 65_536;
const IOV_MAX: u32 = 1_024;
/// A console's rights: FD_READ, FD_FDSTAT_SET_FLAGS, FD_WRITE,
/// FD_FILESTAT_GET, POLL_FD_READWRITE. Never FD_SEEK or FD_TELL, so
/// wasi-libc's `isatty()` is true when the filetype is a character device.
const CONSOLE_RIGHTS: u64 = (1 << 1) | (1 << 3) | (1 << 6) | (1 << 21) | (1 << 27);

/// A guest's linear memory: bounds-checked copies only; out of bounds is
/// [`wire::EFAULT`].
pub trait Mem {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16>;
    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16>;
}

/// What a [`Proc`] needs from its worker. Proc hands `console` and `random`
/// at most 64 KiB at a time.
pub trait Host {
    /// Sends a request and blocks for its reply: `(errno, data)`.
    fn call(&mut self, req: &[u8]) -> (u16, Vec<u8>);
    /// Posts a message that has no reply (EXIT, HOME_STATE).
    fn post(&mut self, msg: &[u8]);
    /// Console output into the ring; blocks while it is full.
    fn console(&mut self, bytes: &[u8]);
    /// WASI clock `clock` in nanoseconds, floored to 100 µs; `None` for an
    /// unknown clock. Proc asks only for 0 (realtime) and 1 (monotonic):
    /// it reads the CPU-time clocks as monotonic, and floors again.
    fn now_ns(&mut self, clock: u32) -> Option<u64>;
    fn random(&mut self, buf: &mut [u8]);
    /// Sleeps until the timeout (`None`: forever) or, with `stdin`, until
    /// console input is ready; whether it is.
    fn wait(&mut self, timeout_ns: Option<u64>, stdin: bool) -> bool;
    /// `(cols, rows)` of the console, live.
    fn winsize(&self) -> (u16, u16);
}

/// The guest called `proc_exit` or `proc_raise`: its status, already in
/// 0..=255 (the code & 0xFF, or 128 + the signal). Proc posts nothing; the
/// worker posts EXIT with this status and stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit(pub u32);

/// What an fd is. Step 1 has only the console, on fds 0 to 2.
#[derive(Debug)]
enum Fd {
    Console,
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
    /// The process `start` describes. Its environment: `HOME`, `USER=guest`,
    /// `PWD` (the cwd), `PATH=/bin:$HOME/.local/bin` and, with a tty,
    /// `TERM=xterm-256color`, `COLUMNS` and `LINES`; each `K=V` of
    /// `start.env` then replaces its key in place or is appended. EINVAL for
    /// an env entry with no key or `=`, or a NUL in any string; E2BIG past
    /// [`wire::MAX_START`]. Step 1: fds 0 to 2 are the console, there are no
    /// preopens, and a redirected stdout is ENOSYS.
    pub fn new(start: &wire::Start, _h: &mut dyn Host) -> Result<Proc, u16> {
        if start.stdout != wire::Stdout::Console {
            return Err(ENOSYS);
        }
        let home = vfs::Vfs::HOME;
        let mut env = vec![["HOME=", home].concat(), "USER=guest".into()];
        env.extend([["PWD=", &start.cwd].concat(), ["PATH=/bin:", home, "/.local/bin"].concat()]);
        if let Some((cols, rows)) = start.tty {
            env.push("TERM=xterm-256color".into());
            env.push(["COLUMNS=", &cols.to_string()].concat());
            env.push(["LINES=", &rows.to_string()].concat());
        }
        for e in &start.env {
            let key = e.find('=').filter(|&i| i > 0).map(|i| &e[..=i]).ok_or(EINVAL)?;
            match env.iter_mut().find(|d| d.starts_with(key)) {
                Some(d) => d.clone_from(e),
                None => env.push(e.clone()),
            }
        }
        let fds = vec![Some(Fd::Console), Some(Fd::Console), Some(Fd::Console)];
        Ok(Proc { args: cstrs(&start.argv)?, env: cstrs(&env)?, fds, tty: start.tty.is_some() })
    }

    /// Runs function `f` (an index into [`NAMES`]) on its arguments `a`:
    /// exactly [`ARITY`]`[f]` of them, each widened to u64 (u32s and
    /// pointers zero-extended, i64s as their bits). The errno to return (0
    /// is success), or the [`Exit`] of `proc_exit` and `proc_raise`. An
    /// unknown `f` is ENOSYS and a wrong argument count EINVAL.
    pub fn call(
        &mut self,
        f: usize,
        a: &[u64],
        m: &mut dyn Mem,
        h: &mut dyn Host,
    ) -> Result<u16, Exit> {
        if ARITY.get(f) != Some(&a.len()) {
            return Ok(if f < NAMES.len() { EINVAL } else { ENOSYS });
        }
        let p = |i: usize| a[i] as u32;
        let done = match f {
            PROC_EXIT => return Err(Exit(p(0) & 0xFF)),
            PROC_RAISE if (1..128).contains(&a[0]) => return Err(Exit(128 + p(0))),
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
            FD_FDSTAT_GET => self.fdstat(p(0), p(1), m),
            FD_WRITE => self.write(a, m, h),
            FD_PRESTAT_GET => Err(EBADF),
            FD_FILESTAT_SET_TIMES => self.fd(p(0)).map(drop),
            PATH_FILESTAT_SET_TIMES => self.fd(p(0)).and(Err(ENOTDIR)),
            PATH_LINK | PATH_SYMLINK | SOCK_ACCEPT | SOCK_RECV | SOCK_SEND | SOCK_SHUTDOWN => {
                Err(ENOTSUP)
            }
            PATH_READLINK => Err(EINVAL),
            _ => Err(ENOSYS),
        };
        Ok(done.err().unwrap_or(0))
    }

    fn fd(&self, fd: u32) -> Result<&Fd, u16> {
        self.fds.get(fd as usize).and_then(Option::as_ref).ok_or(EBADF)
    }

    /// The 24-byte fdstat: u8 filetype, u16 flags at 2, u64 rights base at
    /// 8 and inheriting at 16. A console is a character device (2) with a
    /// tty, UNKNOWN (0) without.
    fn fdstat(&self, fd: u32, at: u32, m: &mut dyn Mem) -> Result<(), u16> {
        let Fd::Console = self.fd(fd)?;
        let w = wire::Writer::default().u8(if self.tty { 2 } else { 0 }).u8(0).u16(0).u32(0);
        m.write(at, &w.u64(CONSOLE_RIGHTS).u64(0).done())
    }

    /// fd_write `(fd, iovs, n, out)`: gathers the iovecs into the console
    /// ring and stores the count at `out`. A bad iovec after some bytes went
    /// out ends a short write, as POSIX `writev` does; before any, EFAULT.
    fn write(&self, a: &[u64], m: &mut dyn Mem, h: &mut dyn Host) -> Result<(), u16> {
        let [fd, iovs, n, out] = [a[0], a[1], a[2], a[3]].map(|x| x as u32);
        let Fd::Console = self.fd(fd)?;
        if n > IOV_MAX {
            return Err(EINVAL);
        }
        let b = m.read(iovs, n * 8)?;
        let mut r = wire::Reader(&b);
        let iovs: Vec<_> = (0..n).map_while(|_| Some((r.u32()?, r.u32()?))).collect();
        let mut done = 0u32;
        for (at, k) in iovs.into_iter().flat_map(|(at, len)| spans(at, len)) {
            let Some(sum) = done.checked_add(k) else { break };
            match m.read(at, k) {
                Ok(b) => h.console(&b),
                Err(e) if done == 0 => return Err(e),
                Err(_) => break,
            }
            done = sum;
        }
        m.write(out, &done.to_le_bytes())
    }
}

/// C strings back to back: EINVAL for a NUL inside one, E2BIG past
/// [`wire::MAX_START`].
fn cstrs(list: &[String]) -> Result<Vec<u8>, u16> {
    let mut blob = Vec::new();
    for s in list {
        if s.contains('\0') {
            return Err(EINVAL);
        }
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
    let addrs = ends.scan(buf, |next, s| {
        let this = *next;
        *next = this.wrapping_add(s.len() as u32);
        Some(this)
    });
    m.write(at, &addrs.flat_map(u32::to_le_bytes).collect::<Vec<u8>>())
}

/// A WASI clock as the [`Host`] reads it: the CPU-time clocks (2, 3) read as
/// monotonic (1); past 3 is EINVAL.
fn clock(id: u64) -> Result<u32, u16> {
    (id < 4).then_some(id.min(1) as u32).ok_or(EINVAL)
}

/// `len` bytes from `at`, in pieces of at most [`CHUNK`]. A piece past the
/// end of the address space saturates, so [`Mem`] refuses it.
fn spans(at: u32, len: u32) -> impl Iterator<Item = (u32, u32)> {
    let piece = move |i: u32| (at.saturating_add(i * CHUNK), (len - i * CHUNK).min(CHUNK));
    (0..len.div_ceil(CHUNK)).map(piece)
}
