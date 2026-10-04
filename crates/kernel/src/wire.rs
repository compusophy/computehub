//! The byte protocol between the main thread and a program's worker; the channel names the process,
//! so requests carry no pid. Little-endian; a `str` is a u16 length and UTF-8 (a guest's non-UTF-8
//! path is [`EILSEQ`] in the worker); `rest` is every remaining byte. A bad message decodes to
//! `None` (answered with [`EINVAL`]), never a panic.
//!
//! A process's SAB: 16 i32 words ([`STATE`] to [`PAGES`], the rest 0), the reply
//! payload at [`PAYLOAD_AT`], the console ring at [`RING_AT`]. A reply: payload,
//! LEN, ERRNO, STATE = 1 and notify; the worker stores STATE = 0 before its next
//! request. COLS, ROWS: the window; INPUT: [`INPUT_READY`], [`INPUT_EOF`]; SLEEP:
//! never written; HEAD, TAIL: console bytes, wrapping; BELL: 1 while a CONS_BELL flies.
//! The worker's meters, which main reads and never writes: RUN 1 from the program's start
//! but while it waits (each `Atomics.wait`); BUSY its ms running up to its last wait, wrapping;
//! SINCE when it last began to run (`timeOrigin + now()` ms, wrapping); PAGES its wasm memory
//! (the guest's and its own) in 64 KiB pages, as of its last wait.

use vfs::VfsError;

macro_rules! consts {
    ($($ty:ty: $($name:ident = $v:expr),+;)+) => { $($(pub const $name: $ty = $v;)+)+ };
}

consts! {
    u8: VERSION = 2;
    usize: MAX_PAYLOAD = 65_536, MAX_START = 65_536, MAX_PROCS = 8, MAX_PIPE = 65_536;
    u32: HOME_PID = 1, MEM_PAGES = 4_096;
    u32: SAB_BYTES = 64 + 65_536 + 65_536, PAYLOAD_AT = 64, RING_AT = 65_600, RING_BYTES = 65_536;
    u32: STATE = 0, ERRNO = 1, LEN = 2, COLS = 3, ROWS = 4, INPUT = 5, SLEEP = 6, HEAD = 7;
    u32: TAIL = 8, BELL = 9, RUN = 10, BUSY = 11, SINCE = 12, PAGES = 13;
    i32: INPUT_READY = 1, INPUT_EOF = 2;
    u8: READY = 0x00, OPEN = 0x01, READ = 0x02, WRITE = 0x03, LIST = 0x04, MKDIR = 0x05;
    u8: REMOVE = 0x06, RENAME = 0x07, SETLEN = 0x08, CONS_BELL = 0x10, CONS_WRITE = 0x11;
    u8: CONS_READ = 0x12, CONS_MODE = 0x13, SPAWN = 0x14, WAIT = 0x15, PIPE_READ = 0x16;
    u8: PIPE_WRITE = 0x17, HOME_STATE = 0x18, EXIT = 0x1F, DRAW = 0x20;
    u8: EVENTS = 0x21, REPLY = 0x80, SAVE = 0x81;
    u8: O_CREAT = 1, O_DIRECTORY = 2, O_EXCL = 4, O_TRUNC = 8, KIND_FILE = 1, KIND_DIR = 2;
    u8: MODE_RAW = 1, MODE_NOECHO = 2;
    u8: HOME_WRITER = 1, HOME_READONLY = 2, HOME_SAVED = 3, HOME_FAILED = 4;
    u64: APPEND = u64::MAX;
    u16: E2BIG = 1, EAGAIN = 6, EBADF = 8, ECHILD = 12, EEXIST = 20, EFAULT = 21, EILSEQ = 25;
    u16: EINVAL = 28, EISDIR = 31, ENOENT = 44, ENOEXEC = 45, ENOSPC = 51, ENOSYS = 52;
    u16: ENOTDIR = 54, ENOTEMPTY = 55, ENOTSUP = 58, ENOTTY = 59, EPIPE = 64, EROFS = 69;
    u16: ESPIPE = 70, ENOTCAPABLE = 76;
    i32: CANNOT_EXECUTE = 126, NOT_FOUND = 127, INTERRUPTED = 130, TRAPPED = 134, KILLED = 137;
}

/// The errno (WASI numbering) for a filesystem error.
pub fn errno(e: VfsError) -> u16 {
    match e {
        VfsError::NotFound => ENOENT,
        VfsError::NotADir => ENOTDIR,
        VfsError::IsADir => EISDIR,
        VfsError::Exists => EEXIST,
        VfsError::NotEmpty => ENOTEMPTY,
        VfsError::InvalidPath => EINVAL,
        VfsError::NoSpace => ENOSPC,
    }
}

/// A checked cursor over a message: every read is `None` past the end.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a>(pub &'a [u8]);

/// A message under construction: `Writer::new(op).u32(..).str(..).done()`;
/// [`Writer::default`] starts with no op byte, for reply payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Writer(pub Vec<u8>);

macro_rules! numbers {
    ($($t:ident)+) => {
        impl Reader<'_> {
            $(pub fn $t(&mut self) -> Option<$t> { Some($t::from_le_bytes(self.array()?)) })+
        }
        impl Writer {
            $(pub fn $t(self, v: $t) -> Writer { self.bytes(&v.to_le_bytes()) })+
        }
    };
}
numbers!(u8 u16 u32 u64 i32);

impl<'a> Reader<'a> {
    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    /// A u16 length and that many bytes of UTF-8.
    pub fn str(&mut self) -> Option<&'a str> {
        let n = self.u16()?;
        core::str::from_utf8(self.take(usize::from(n))?).ok()
    }

    /// Every remaining byte, leaving none.
    pub fn rest(&mut self) -> &'a [u8] {
        core::mem::take(&mut self.0)
    }

    /// `Some(())` when every byte was read: decoders end with it.
    pub fn end(&self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

impl Writer {
    pub fn new(op: u8) -> Writer {
        Writer(vec![op])
    }

    /// A u16 length and the bytes, cut to 65,535 on a char boundary: callers bound
    /// theirs first (paths by the Vfs limits, [`Start`] by [`MAX_START`]).
    pub fn str(self, s: &str) -> Writer {
        let mut n = s.len().min(usize::from(u16::MAX));
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.u16(n as u16).bytes(s.as_bytes().get(..n).unwrap_or_default())
    }

    /// Raw bytes with no length: a `rest`.
    pub fn bytes(mut self, b: &[u8]) -> Writer {
        self.0.extend_from_slice(b);
        self
    }

    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// Declares [`Msg`] and its codec, an `OP Name { field: type }` per op; a type is a number, `str`,
/// `bytes` (a `rest`) or `payload` (a `rest` of at most [`MAX_PAYLOAD`] bytes).
macro_rules! msgs {
    ($($op:ident $name:ident $({ $($f:ident: $t:ident),+ })?;)+) => {
        /// One message, borrowed: worker to main (READY to EXIT; the platform makes CONS_WRITE
        /// from the ring) or main to an async worker (REPLY, SAVE). Replies carry: OPEN `u8 kind,
        /// u64 size`; READ, CONS_READ, PIPE_READ the bytes; WRITE `u64 size`; LIST `{u8 kind, u64
        /// size, str name}*`; SPAWN (a [`Job`]) `u32 job`, its first pid; WAIT `i32 status`, its
        /// last stage's; others none.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Msg<'a> {
            $($name $({ $($f: msgs!(@ty $t)),+ })?,)+
        }

        impl<'a> Msg<'a> {
            /// `None` for an unknown or reserved op, a short or long body, non-UTF-8
            /// text, or a WRITE body over [`MAX_PAYLOAD`].
            pub fn decode(b: &'a [u8]) -> Option<Msg<'a>> {
                let mut r = Reader(b);
                let msg = match r.u8()? {
                    $($op => Msg::$name $({ $($f: msgs!(@get r $t)),+ })?,)+
                    _ => return None,
                };
                r.end().map(|()| msg)
            }

            pub fn encode(&self) -> Vec<u8> {
                match *self {
                    $(Msg::$name $({ $($f),+ })? => msgs!(@put Writer::new($op), $($($t $f)+)?),)+
                }
                .done()
            }
        }
    };
    (@ty str) => { &'a str };
    (@ty bytes) => { &'a [u8] };
    (@ty payload) => { &'a [u8] };
    (@ty $t:ident) => { $t };
    (@get $r:ident bytes) => { $r.rest() };
    (@get $r:ident payload) => { Some($r.rest()).filter(|d| d.len() <= MAX_PAYLOAD)? };
    (@get $r:ident $t:ident) => { $r.$t()? };
    (@put $w:expr,) => { $w };
    (@put $w:expr, payload $f:ident $($more:tt)*) => { msgs!(@put $w.bytes($f), $($more)*) };
    (@put $w:expr, $t:ident $f:ident $($more:tt)*) => { msgs!(@put $w.$t($f), $($more)*) };
}

msgs! {
    READY Ready { version: u8 }; OPEN Open { oflags: u8, path: str };
    READ Read { off: u64, max: u32, path: str }; WRITE Write { off: u64, path: str, data: payload };
    LIST List { skip: u32, path: str }; MKDIR Mkdir { path: str };
    REMOVE Remove { kind: u8, path: str }; RENAME Rename { from: str, to: str };
    SETLEN SetLen { len: u64, path: str }; CONS_BELL ConsBell; CONS_WRITE ConsWrite { data: bytes };
    CONS_READ ConsRead { max: u32 }; CONS_MODE ConsMode { bits: u8 }; SPAWN Spawn { job: payload };
    WAIT Wait { job: u32 }; PIPE_READ PipeRead { max: u32 }; PIPE_WRITE PipeWrite { data: payload };
    HOME_STATE HomeState { state: u8, note: str }; EXIT Exit { status: i32 };
    REPLY Reply { errno: u16, data: bytes }; SAVE Save;
}

/// What a worker runs as: a program, or homed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Process,
    Home,
}

/// Where fd 0 comes from: the console, a file (`prog < file`), or a pipe (from the stage before
/// in a [`Job`], or the job's bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stdin {
    Console,
    File(String),
    Pipe,
}

/// Where fd 1 goes: the console, a file (`prog > file`; `>>` appends), a pipe (to the next
/// stage of a [`Job`]) or nowhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stdout {
    Console,
    File { path: String, append: bool },
    Pipe,
    Null,
}

/// A stdin or stdout, `u8 kind | str path`, the path empty but for a file (kinds `files`):
/// `None` for a kind past `last`, or a path with one that is no file.
fn io<T>(
    r: &mut Reader<'_>,
    last: u8,
    files: [u8; 2],
    make: impl FnOnce(u8, String) -> T,
) -> Option<T> {
    let (kind, path) = (r.u8()?, r.str()?);
    (kind <= last && (files.contains(&kind) || path.is_empty())).then(|| make(kind, path.into()))
}

impl Stdin {
    /// Its `u8 kind` (0 console, 1 file, 2 pipe) and path.
    fn put(&self, w: Writer) -> Writer {
        let (kind, path) = match self {
            Stdin::Console => (0, ""),
            Stdin::File(path) => (1, path.as_str()),
            Stdin::Pipe => (2, ""),
        };
        w.u8(kind).str(path)
    }

    fn get(r: &mut Reader<'_>) -> Option<Stdin> {
        io(r, 2, [1, 1], |kind, path| match kind {
            0 => Stdin::Console,
            1 => Stdin::File(path),
            _ => Stdin::Pipe,
        })
    }
}

impl Stdout {
    /// Its `u8 kind` (0 console, 1 file, 2 append, 3 pipe, 4 null) and path.
    fn put(&self, w: Writer) -> Writer {
        let (kind, path) = match self {
            Stdout::Console => (0, ""),
            Stdout::File { path, append } => (1 + u8::from(*append), path.as_str()),
            Stdout::Pipe => (3, ""),
            Stdout::Null => (4, ""),
        };
        w.u8(kind).str(path)
    }

    fn get(r: &mut Reader<'_>) -> Option<Stdout> {
        io(r, 4, [1, 2], |kind, path| match kind {
            0 => Stdout::Console,
            3 => Stdout::Pipe,
            4 => Stdout::Null,
            _ => Stdout::File { path, append: kind == 2 },
        })
    }
}

/// The first message to a worker, after its READY: `u8 version | u8 role | u32 pid | u8 flags
/// (bit0 tty) | u16 cols | u16 rows | stdin | stdout | str cwd | u16 nroots, str* | u16 argc,
/// str* | u16 envc, str*`; `env`: `K=V` overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub role: Role,
    pub pid: u32,
    pub tty: Option<(u16, u16)>,
    pub stdin: Stdin,
    pub stdout: Stdout,
    pub cwd: String,
    pub roots: Vec<String>,
    pub argv: Vec<String>,
    pub env: Vec<String>,
}

/// `n` strings, as a u16 count and each.
fn strs(w: Writer, list: &[String]) -> Writer {
    let n = list.len().min(usize::from(u16::MAX));
    list[..n].iter().fold(w.u16(n as u16), |w, s| w.str(s))
}

/// A u16 count and that many strings.
fn list(r: &mut Reader<'_>) -> Option<Vec<String>> {
    (0..r.u16()?).map(|_| r.str().map(String::from)).collect()
}

impl Start {
    /// The bytes; longer than [`MAX_START`] (E2BIG) whenever a string or a
    /// list was too long to encode whole.
    pub fn encode(&self) -> Vec<u8> {
        let role = u8::from(self.role == Role::Home);
        let (flags, (cols, rows)) = (u8::from(self.tty.is_some()), self.tty.unwrap_or((0, 0)));
        let w = Writer::new(VERSION).u8(role).u32(self.pid).u8(flags).u16(cols).u16(rows);
        let w = self.stdout.put(self.stdin.put(w)).str(&self.cwd);
        strs(strs(strs(w, &self.roots), &self.argv), &self.env).done()
    }

    /// `None` for another version, an unknown role, flag or stdin or stdout kind, a path with
    /// one that has none, or a short or long message.
    pub fn decode(b: &[u8]) -> Option<Start> {
        let mut r = Reader(b);
        (r.u8()? == VERSION).then_some(())?;
        let role = [Role::Process, Role::Home].get(usize::from(r.u8()?)).copied()?;
        let (pid, flags, size) = (r.u32()?, r.u8()?, (r.u16()?, r.u16()?));
        (flags < 2).then_some(())?;
        let tty = (flags == 1).then_some(size);
        let (stdin, stdout, cwd) = (Stdin::get(&mut r)?, Stdout::get(&mut r)?, r.str()?.into());
        let (roots, argv, env) = (list(&mut r)?, list(&mut r)?, list(&mut r)?);
        r.end()?;
        Some(Start { role, pid, tty, stdin, stdout, cwd, roots, argv, env })
    }
}

/// The programs a SPAWN starts, a job: `str cwd | stdin | stdout | u16 n | n stages (str path,
/// u16 argc, str*) | rest`. Each stage runs the program at `path` (wasm, or a /bin marker) with
/// its argv, in `cwd`, a pipe from each to the next; stdin is the first's (a pipe holds the rest
/// of the bytes, then an end of file) and stdout the last's, each as a [`Start`] says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job<'a> {
    pub cwd: &'a str,
    pub stdin: Stdin,
    pub stdout: Stdout,
    pub stages: Vec<(&'a str, Vec<String>)>,
    pub data: &'a [u8],
}

impl<'a> Job<'a> {
    pub fn encode(&self) -> Vec<u8> {
        let w = self.stdout.put(self.stdin.put(Writer::default().str(self.cwd)));
        let w = w.u16(self.stages.len().min(usize::from(u16::MAX)) as u16);
        let w = self.stages.iter().fold(w, |w, (path, argv)| strs(w.str(path), argv));
        w.bytes(self.data).done()
    }

    /// `None` for a short message, a bad stdin or stdout, or non-UTF-8 text.
    pub fn decode(b: &'a [u8]) -> Option<Job<'a>> {
        let mut r = Reader(b);
        let (cwd, stdin, stdout) = (r.str()?, Stdin::get(&mut r)?, Stdout::get(&mut r)?);
        let stages =
            (0..r.u16()?).map(|_| Some((r.str()?, list(&mut r)?))).collect::<Option<_>>()?;
        Some(Job { cwd, stdin, stdout, stages, data: r.rest() })
    }
}
