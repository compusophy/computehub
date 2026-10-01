//! The byte protocol between the main thread and a program's worker; the
//! channel names the process, so requests carry no pid. Little-endian; a `str`
//! is a u16 length and UTF-8 (a guest's non-UTF-8 path is [`EILSEQ`] in the
//! worker); `rest` is every remaining byte. A bad message decodes to `None`
//! (answered with [`EINVAL`]), never a panic.
//!
//! A process's SAB: 16 i32 words ([`STATE`] to [`BELL`], the rest 0), the reply
//! payload at [`PAYLOAD_AT`], the console ring at [`RING_AT`]. A reply: payload,
//! LEN, ERRNO, STATE = 1 and notify; the worker stores STATE = 0 before its next
//! request. COLS, ROWS: the window; INPUT: [`INPUT_READY`], [`INPUT_EOF`]; SLEEP:
//! never written; HEAD, TAIL: console bytes, wrapping; BELL: 1 while a CONS_BELL flies.

use vfs::VfsError;

macro_rules! consts {
    ($($ty:ty: $($name:ident = $v:expr),+;)+) => { $($(pub const $name: $ty = $v;)+)+ };
}

consts! {
    u8: VERSION = 1;
    usize: MAX_PAYLOAD = 65_536, MAX_START = 65_536, MAX_PROCS = 8;
    u32: HOME_PID = 1, MEM_PAGES = 4_096;
    u32: SAB_BYTES = 64 + 65_536 + 65_536, PAYLOAD_AT = 64, RING_AT = 65_600, RING_BYTES = 65_536;
    u32: STATE = 0, ERRNO = 1, LEN = 2, COLS = 3, ROWS = 4, INPUT = 5, SLEEP = 6, HEAD = 7;
    u32: TAIL = 8, BELL = 9;
    i32: INPUT_READY = 1, INPUT_EOF = 2;
    u8: READY = 0x00, OPEN = 0x01, READ = 0x02, WRITE = 0x03, LIST = 0x04, MKDIR = 0x05;
    u8: REMOVE = 0x06, RENAME = 0x07, SETLEN = 0x08, CONS_BELL = 0x10, CONS_WRITE = 0x11;
    u8: CONS_READ = 0x12, CONS_MODE = 0x13, HOME_STATE = 0x18, EXIT = 0x1F, DRAW = 0x20;
    u8: EVENTS = 0x21, REPLY = 0x80, SAVE = 0x81;
    u8: O_CREAT = 1, O_DIRECTORY = 2, O_EXCL = 4, O_TRUNC = 8, KIND_FILE = 1, KIND_DIR = 2;
    u8: MODE_RAW = 1, MODE_NOECHO = 2;
    u8: HOME_WRITER = 1, HOME_READONLY = 2, HOME_SAVED = 3, HOME_FAILED = 4;
    u64: APPEND = u64::MAX;
    u16: E2BIG = 1, EAGAIN = 6, EBADF = 8, EEXIST = 20, EFAULT = 21, EILSEQ = 25, EINVAL = 28;
    u16: EISDIR = 31, ENOENT = 44, ENOSPC = 51, ENOSYS = 52, ENOTDIR = 54, ENOTEMPTY = 55;
    u16: ENOTSUP = 58, ESPIPE = 70, ENOTCAPABLE = 76;
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

/// One message, borrowed: worker to main (READY to EXIT; the platform makes CONS_WRITE from
/// the ring) or main to an async worker (REPLY, SAVE). Replies carry: OPEN `u8 kind, u64 size`;
/// READ, CONS_READ the bytes; WRITE `u64 size`; LIST `{u8 kind, u64 size, str name}*`; others none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg<'a> {
    Ready { version: u8 },
    Open { oflags: u8, path: &'a str },
    Read { off: u64, max: u32, path: &'a str },
    Write { off: u64, path: &'a str, data: &'a [u8] },
    List { skip: u32, path: &'a str },
    Mkdir { path: &'a str },
    Remove { kind: u8, path: &'a str },
    Rename { from: &'a str, to: &'a str },
    SetLen { len: u64, path: &'a str },
    ConsBell,
    ConsWrite { data: &'a [u8] },
    ConsRead { max: u32 },
    ConsMode { bits: u8 },
    HomeState { state: u8, note: &'a str },
    Exit { status: i32 },
    Reply { errno: u16, data: &'a [u8] },
    Save,
}

impl<'a> Msg<'a> {
    /// `None` for an unknown or reserved op, a short or long body, non-UTF-8
    /// text, or a WRITE body over [`MAX_PAYLOAD`].
    pub fn decode(b: &'a [u8]) -> Option<Msg<'a>> {
        let mut r = Reader(b);
        let msg = match r.u8()? {
            READY => Msg::Ready { version: r.u8()? },
            OPEN => Msg::Open { oflags: r.u8()?, path: r.str()? },
            READ => Msg::Read { off: r.u64()?, max: r.u32()?, path: r.str()? },
            WRITE => {
                let (off, path, data) = (r.u64()?, r.str()?, r.rest());
                (data.len() <= MAX_PAYLOAD).then_some(Msg::Write { off, path, data })?
            }
            LIST => Msg::List { skip: r.u32()?, path: r.str()? },
            MKDIR => Msg::Mkdir { path: r.str()? },
            REMOVE => Msg::Remove { kind: r.u8()?, path: r.str()? },
            RENAME => Msg::Rename { from: r.str()?, to: r.str()? },
            SETLEN => Msg::SetLen { len: r.u64()?, path: r.str()? },
            CONS_BELL => Msg::ConsBell,
            CONS_WRITE => Msg::ConsWrite { data: r.rest() },
            CONS_READ => Msg::ConsRead { max: r.u32()? },
            CONS_MODE => Msg::ConsMode { bits: r.u8()? },
            HOME_STATE => Msg::HomeState { state: r.u8()?, note: r.str()? },
            EXIT => Msg::Exit { status: r.i32()? },
            REPLY => Msg::Reply { errno: r.u16()?, data: r.rest() },
            SAVE => Msg::Save,
            _ => return None,
        };
        r.end().map(|()| msg)
    }

    pub fn encode(&self) -> Vec<u8> {
        match *self {
            Msg::Ready { version } => Writer::new(READY).u8(version),
            Msg::Open { oflags, path } => Writer::new(OPEN).u8(oflags).str(path),
            Msg::Read { off, max, path } => Writer::new(READ).u64(off).u32(max).str(path),
            Msg::Write { off, path, data } => Writer::new(WRITE).u64(off).str(path).bytes(data),
            Msg::List { skip, path } => Writer::new(LIST).u32(skip).str(path),
            Msg::Mkdir { path } => Writer::new(MKDIR).str(path),
            Msg::Remove { kind, path } => Writer::new(REMOVE).u8(kind).str(path),
            Msg::Rename { from, to } => Writer::new(RENAME).str(from).str(to),
            Msg::SetLen { len, path } => Writer::new(SETLEN).u64(len).str(path),
            Msg::ConsBell => Writer::new(CONS_BELL),
            Msg::ConsWrite { data } => Writer::new(CONS_WRITE).bytes(data),
            Msg::ConsRead { max } => Writer::new(CONS_READ).u32(max),
            Msg::ConsMode { bits } => Writer::new(CONS_MODE).u8(bits),
            Msg::HomeState { state, note } => Writer::new(HOME_STATE).u8(state).str(note),
            Msg::Exit { status } => Writer::new(EXIT).i32(status),
            Msg::Reply { errno, data } => Writer::new(REPLY).u16(errno).bytes(data),
            Msg::Save => Writer::new(SAVE),
        }
        .done()
    }
}

/// What a worker runs as: a program, or homed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Process,
    Home,
}

/// Where fd 1 goes: the console, or a file (`prog > file`; `>>` appends).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stdout {
    Console,
    File { path: String, append: bool },
}

/// The first message to a worker, after its READY: `u8 version | u8 role | u32 pid | u8 flags
/// (bit0 tty) | u16 cols | u16 rows | u8 stdout (0 console, 1 file, 2 append) | str stdout_path
/// | str cwd | u16 nroots, str* | u16 argc, str* | u16 envc, str*`; `env`: `K=V` overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub role: Role,
    pub pid: u32,
    pub tty: Option<(u16, u16)>,
    pub stdout: Stdout,
    pub cwd: String,
    pub roots: Vec<String>,
    pub argv: Vec<String>,
    pub env: Vec<String>,
}

impl Start {
    /// The bytes; longer than [`MAX_START`] (E2BIG) whenever a string or a
    /// list was too long to encode whole.
    pub fn encode(&self) -> Vec<u8> {
        let role = u8::from(self.role == Role::Home);
        let (flags, (cols, rows)) = (u8::from(self.tty.is_some()), self.tty.unwrap_or((0, 0)));
        let (kind, path) = match &self.stdout {
            Stdout::Console => (0, ""),
            Stdout::File { path, append } => (1 + u8::from(*append), path.as_str()),
        };
        let w = Writer::new(VERSION).u8(role).u32(self.pid).u8(flags).u16(cols).u16(rows);
        let mut w = w.u8(kind).str(path).str(&self.cwd);
        for list in [&self.roots, &self.argv, &self.env] {
            let n = list.len().min(usize::from(u16::MAX));
            w = list[..n].iter().fold(w.u16(n as u16), |w, s| w.str(s));
        }
        w.done()
    }

    /// `None` for another version, an unknown role, flag or stdout kind, a
    /// path with console stdout, or a short or long message.
    pub fn decode(b: &[u8]) -> Option<Start> {
        let mut r = Reader(b);
        (r.u8()? == VERSION).then_some(())?;
        let role = match r.u8()? {
            0 => Role::Process,
            1 => Role::Home,
            _ => return None,
        };
        let (pid, flags, size) = (r.u32()?, r.u8()?, (r.u16()?, r.u16()?));
        (flags < 2).then_some(())?;
        let tty = (flags == 1).then_some(size);
        let (kind, path) = (r.u8()?, r.str()?);
        let stdout = match kind {
            0 if path.is_empty() => Stdout::Console,
            1 | 2 => Stdout::File { path: path.into(), append: kind == 2 },
            _ => return None,
        };
        let cwd = r.str()?.into();
        let mut list = || -> Option<Vec<String>> {
            (0..r.u16()?).map(|_| r.str().map(String::from)).collect()
        };
        let (roots, argv, env) = (list()?, list()?, list()?);
        r.end()?;
        Some(Start { role, pid, tty, stdout, cwd, roots, argv, env })
    }
}
