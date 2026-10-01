use crate::*;
use kernel::wire::{Role, Start};
use kernel::{Effect, Kernel, Program, Spawn};

/// Every test memory is 128 KiB: iovecs at 0, counts at 64, paths at [`PATH`], data at [`BUF`].
const MEM: u64 = 1 << 17;
const PATH: u64 = 8_192;
const BUF: u64 = 16_384;
/// path_open's oflags.
const CREAT: u64 = 1;
const DIR: u64 = 2;
const EXCL: u64 = 4;

/// Guest memory: out of bounds is EFAULT.
impl Mem for Vec<u8> {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16> {
        self.get(at as usize..at as usize + len as usize).map(<[u8]>::to_vec).ok_or(EFAULT)
    }
    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16> {
        self.get_mut(at as usize..at as usize + d.len()).ok_or(EFAULT)?.copy_from_slice(d);
        Ok(())
    }
}

/// A worker on a real Kernel and Vfs: a request is a message from `pid`, answered by its one Reply
/// (EAGAIN for none: an EVENTS read waiting). It logs ops, frames, output, clocks, random sizes.
#[derive(Default)]
struct Fake {
    k: Kernel,
    fs: Vfs,
    pid: u32,
    ops: Vec<u8>,
    frames: Vec<Vec<u8>>,
    out: Vec<Vec<u8>>,
    clocks: Vec<u32>,
    random: Vec<usize>,
}

impl Host for Fake {
    fn call(&mut self, req: &[u8]) -> (u16, Vec<u8>) {
        self.ops.push(req[0]);
        self.k.message(&mut self.fs, self.pid, req);
        let mut reply = None;
        for e in self.k.take_effects() {
            match e {
                Effect::Reply { errno, data, .. } if reply.is_none() => reply = Some((errno, data)),
                Effect::Draw { frame, .. } => self.frames.push(frame),
                e => panic!("unexpected {e:?}"),
            }
        }
        reply.unwrap_or((wire::EAGAIN, vec![]))
    }
    fn console(&mut self, bytes: &[u8]) {
        self.out.push(bytes.to_vec());
    }
    fn now_ns(&mut self, clock: u32) -> Option<u64> {
        self.clocks.push(clock);
        Some(1_234_567_891 + u64::from(clock))
    }
    fn random(&mut self, buf: &mut [u8]) {
        self.random.push(buf.len());
        buf.fill(0x5A);
    }
    fn wait(&mut self, _: Option<u64>, _: bool) -> bool {
        unreachable!("no waits before step 3")
    }
    fn winsize(&self) -> (u16, u16) {
        (80, 24)
    }
}

fn start(tty: Option<(u16, u16)>, env: &[&str]) -> Start {
    let strings = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
    let (stdout, cwd, roots) = (wire::Stdout::Console, "/tmp".into(), vec!["/".into()]);
    let (argv, env) = (strings(&["hello", "a", "b c"]), strings(env));
    Start { role: Role::Process, pid: 2, tty, stdout, cwd, roots, argv, env }
}

/// A process, its memory, and its worker.
struct T {
    p: Proc,
    m: Vec<u8>,
    h: Fake,
}

impl T {
    fn new(tty: Option<(u16, u16)>, env: &[&str]) -> T {
        T::with(Vfs::new(), start(tty, env))
    }
    /// Process `s` on `fs`, the kernel's side past its READY.
    fn with(fs: Vfs, s: Start) -> T {
        let (argv, cwd, tty, stdout) = (s.argv.clone(), s.cwd.clone(), s.tty, s.stdout.clone());
        let (program, roots) = (Program::Url("bin/t.wasm".into()), s.roots.clone());
        let mut h = Fake { fs, ..Fake::default() };
        h.k.set_isolated(true);
        h.pid = h.k.spawn(Spawn { argv, program, cwd, tty, stdout, roots }).unwrap();
        h.k.message(&mut h.fs, h.pid, &[wire::READY, wire::VERSION]);
        h.k.take_effects();
        T { p: Proc::new(&s, &mut h).unwrap(), m: vec![0; MEM as usize], h }
    }
    /// The errno of `f(a)`; panics on an exit.
    fn ok(&mut self, f: usize, a: &[u64]) -> u16 {
        self.p.call(f, a, &mut self.m, &mut self.h).unwrap()
    }
    /// The little-endian number in the `n` bytes at `at`.
    fn num(&self, at: usize, n: usize) -> u64 {
        self.m[at..][..n].iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b))
    }
    fn put(&mut self, at: u64, b: &[u8]) {
        self.m[at as usize..][..b.len()].copy_from_slice(b);
    }
    /// The `n` bytes at [`BUF`].
    fn got(&self, n: usize) -> Vec<u8> {
        self.m[BUF as usize..][..n].to_vec()
    }
    /// path_open of `path` from directory fd `dir`: the new fd, or the errno.
    fn open(&mut self, dir: u64, path: &str, oflags: u64, fdflags: u64) -> Result<u64, u16> {
        self.put(PATH, path.as_bytes());
        let e = self.ok(PATH_OPEN, &[dir, 0, PATH, path.len() as u64, oflags, 0, 0, fdflags, 60]);
        if e == 0 { Ok(self.num(60, 4)) } else { Err(e) }
    }
    /// A call `f(dir, path, len)`: path_create_directory, _remove_directory or _unlink_file.
    fn at(&mut self, f: usize, dir: u64, path: &str) -> u16 {
        self.put(PATH, path.as_bytes());
        self.ok(f, &[dir, PATH, path.len() as u64])
    }
    /// fd_read/write (or pread/pwrite at `off`) of one iovec, `n` bytes at [`BUF`]: errno, count.
    fn io(&mut self, f: usize, fd: u64, n: u64, off: &[u64]) -> (u16, u64) {
        self.put(0, &[BUF as u32, n as u32].map(u32::to_le_bytes).concat());
        (self.ok(f, &[&[fd, 0, 1][..], off, &[64]].concat()), self.num(64, 4))
    }
    /// The dirents fd_readdir puts at [`BUF`] from `cookie`, in `len` bytes:
    /// `(d_next, d_ino, d_type, name)`, less a cut last one; and bufused.
    fn readdir(&mut self, fd: u64, len: u64, cookie: u64) -> (Vec<(u64, u64, u8, String)>, u64) {
        assert_eq!(self.ok(FD_READDIR, &[fd, BUF, len, cookie, 64]), 0);
        let (used, b) = (self.num(64, 4), self.got(len as usize));
        let mut r = Reader(&b[..used as usize]);
        let mut entry = || {
            let (next, ino, n, kind) = (r.u64()?, r.u64()?, r.u32()?, r.u32()?);
            Some((next, ino, kind as u8, String::from_utf8(r.take(n as usize)?.to_vec()).ok()?))
        };
        (core::iter::from_fn(&mut entry).collect(), used)
    }
    /// Checks that `sizes` and `get` copy out exactly `want`: count and
    /// size, the C strings back to back at 4096, their addresses at 64.
    fn strings(&mut self, sizes: usize, get: usize, want: &[&str]) {
        assert_eq!((self.ok(sizes, &[0, 4]), self.ok(get, &[64, 4096])), (0, 0));
        let blob: Vec<u8> = want.iter().flat_map(|s| [s.as_bytes(), b"\0"].concat()).collect();
        assert_eq!((self.num(0, 4), self.num(4, 4)), (want.len() as u64, blob.len() as u64));
        assert_eq!(self.m[4096..][..blob.len()], blob);
        let at = |i: usize| 4096 + want[..i].iter().map(|s| s.len() as u64 + 1).sum::<u64>();
        assert!((0..want.len()).all(|i| self.num(64 + 4 * i, 4) == at(i)), "addresses");
        assert_eq!(self.ok(get, &[64, MEM - 1]), EFAULT);
    }
}

#[test]
fn every_function_links_with_its_arity_and_the_simple_ones_answer() {
    let sigs = [(PATH_OPEN, 9), (SCHED_YIELD, 0), (FD_WRITE, 4), (SOCK_RECV, 6), (PATH_LINK, 7)];
    assert!(sigs.iter().all(|&(f, n)| ARITY[f] == n) && ARITY.iter().sum::<usize>() == 152);
    let mut t = T::new(Some((80, 24)), &[]);
    let notsup = [SOCK_ACCEPT, SOCK_RECV, SOCK_SEND, SOCK_SHUTDOWN, PATH_LINK, PATH_SYMLINK];
    let others = [(PATH_READLINK, EINVAL), (POLL_ONEOFF, ENOSYS), (FD_FILESTAT_SET_TIMES, 0)];
    for (f, want) in notsup.map(|f| (f, ENOTSUP)).into_iter().chain(others) {
        assert_eq!(t.ok(f, &vec![1; ARITY[f]]), want, "{}", NAMES[f]);
    }
    for f in 0..NAMES.len() {
        assert_eq!(t.ok(f, &vec![1; ARITY[f] + 1]), EINVAL, "{} with one too many", NAMES[f]);
    }
    assert_eq!(t.ok(FD_FILESTAT_SET_TIMES, &[99, 0, 0, 0]), EBADF);
    assert_eq!((t.ok(NAMES.len(), &[]), t.ok(usize::MAX, &[])), (ENOSYS, ENOSYS));
    for id in 0..4 {
        t.m[8..24].fill(0xEE);
        assert_eq!((t.ok(CLOCK_RES_GET, &[id, 8]), t.num(8, 8)), (0, 100_000));
        assert_eq!((t.ok(CLOCK_TIME_GET, &[id, 1, 16]), t.num(16, 8)), (0, 1_234_500_000));
    }
    assert_eq!(t.h.clocks, [0, 1, 1, 1], "CPU time reads monotonic");
    assert_eq!((t.ok(CLOCK_RES_GET, &[4, 8]), t.ok(CLOCK_TIME_GET, &[4, 0, 8])), (EINVAL, EINVAL));
    assert_eq!(t.ok(CLOCK_TIME_GET, &[0, 0, MEM - 7]), EFAULT);
    assert_eq!((t.ok(RANDOM_GET, &[100, 70_000]), &t.h.random[..]), (0, &[65_536, 4_464][..]));
    assert!(t.m[100..70_100].iter().all(|&b| b == 0x5A) && t.m[70_100] == 0);
    let far = [t.ok(RANDOM_GET, &[MEM - 10, 11]), t.ok(RANDOM_GET, &[u32::MAX.into(), 9])];
    assert_eq!(far, [EFAULT, EFAULT]);
    let mut call = |f, a: &[u64]| t.p.call(f, a, &mut t.m, &mut t.h);
    assert_eq!((call(PROC_EXIT, &[0x1_02]), call(PROC_EXIT, &[0])), (Err(Exit(2)), Err(Exit(0))));
    assert_eq!((call(PROC_RAISE, &[9]), call(PROC_RAISE, &[0])), (Err(Exit(137)), Ok(0)));
    assert_eq!((call(PROC_RAISE, &[128]), call(SCHED_YIELD, &[])), (Ok(EINVAL), Ok(0)));
}

#[test]
fn start_copies_out_args_and_environ_or_is_refused() {
    let mut t = T::new(None, &[]);
    t.strings(ARGS_SIZES_GET, ARGS_GET, &["hello", "a", "b c"]);
    let home = ["HOME=", Vfs::HOME].concat();
    let path = ["PATH=/bin:", Vfs::HOME, "/.local/bin"].concat();
    let env = [home.as_str(), "USER=guest", "PWD=/tmp", path.as_str()];
    t.strings(ENVIRON_SIZES_GET, ENVIRON_GET, &env);
    let mut t = T::new(Some((100, 30)), &["PATH=/x", "LANG=C", "PATH=/y", "E="]);
    let tty = ["PATH=/y", "TERM=xterm-256color", "COLUMNS=100", "LINES=30", "LANG=C", "E="];
    t.strings(ENVIRON_SIZES_GET, ENVIRON_GET, &[&env[..3], &tty].concat());
    assert_eq!(t.ok(ENVIRON_SIZES_GET, &[0, MEM - 2]), EFAULT);
    let (mut h, s) = (t.h, start(None, &[]));
    let nul = Start { argv: vec!["a\0b".into()], ..s.clone() };
    let long = Start { argv: vec!["x".repeat(wire::MAX_START)], ..s.clone() };
    let stdout = wire::Stdout::File { path: "/tmp/no/out".into(), append: false };
    let envs = ["NOEQUALS", "=x", "A=\0"].map(|env| start(None, &[env]));
    let bad = envs.into_iter().chain([nul, long, Start { stdout, ..s }]);
    let errs: Vec<_> = bad.map(|s| Proc::new(&s, &mut h).err()).collect();
    assert_eq!(errs, [EINVAL, EINVAL, EINVAL, EINVAL, E2BIG, ENOENT].map(Some));
}

#[test]
fn the_console_is_a_tty_only_with_one_and_takes_writes_unless_stdout_is_a_file() {
    for (tty, kind) in [(Some((80, 24)), 2), (None, 0)] {
        let mut t = T::new(tty, &[]);
        for fd in 0..3 {
            t.m[..24].fill(0xAA);
            assert_eq!(t.ok(FD_FDSTAT_GET, &[fd, 0]), 0);
            let rights = [0x4A, 0, 0x20, 0x08, 0, 0, 0, 0];
            assert_eq!(t.m[..24], [&[kind, 0, 0, 0, 0, 0, 0, 0][..], &rights, &[0; 8]].concat());
        }
        assert_eq!(t.num(8, 8) & ((1 << 2) | (1 << 5)), 0, "no FD_SEEK or FD_TELL");
        let bad = [t.ok(FD_FDSTAT_GET, &[99, 0]), t.ok(FD_FDSTAT_GET, &[1, MEM - 23])];
        assert_eq!(bad, [EBADF, EFAULT]);
    }
    let mut t = T::new(Some((80, 24)), &[]);
    let _ = (t.put(1000, b"hello"), t.put(2000, b" 42"));
    t.put(0, &[1000, 5, u32::MAX, 0, 2000, 3].map(u32::to_le_bytes).concat());
    for fd in [1, 2, 0] {
        assert_eq!((t.ok(FD_WRITE, &[fd, 0, 3, 64]), t.num(64, 4)), (0, 8));
    }
    t.put(0, &[1000, 5, MEM as u32 - 2, 5].map(u32::to_le_bytes).concat());
    assert_eq!((t.ok(FD_WRITE, &[1, 0, 2, 64]), t.num(64, 4)), (0, 5), "a short write");
    t.put(0, &[MEM as u32 - 2, 5, 1000, 5].map(u32::to_le_bytes).concat());
    assert_eq!(t.ok(FD_WRITE, &[1, 0, 2, 64]), EFAULT);
    assert_eq!(t.h.out.concat(), b"hello 42hello 42hello 42hello");
    t.h.out.clear();
    t.put(0, &[0, 70_000].map(u32::to_le_bytes).concat());
    assert_eq!((t.ok(FD_WRITE, &[1, 0, 1, 70_000]), t.num(70_000, 4)), (0, 70_000));
    assert_eq!(t.h.out.iter().map(Vec::len).collect::<Vec<_>>(), [65_536, 4_464]);
    let bad = [[99, 0, 1, 64], [1, MEM - 4, 1, 64], [1, 0, 1_025, 64], [1, 0, 1, MEM - 3]];
    assert_eq!(bad.map(|a| t.ok(FD_WRITE, &a)), [EBADF, EFAULT, EINVAL, EFAULT]);
    assert_eq!(t.ok(FD_WRITE, &[3, 0, 1, 64]), EBADF, "a directory");
    for (append, want) in [(false, &b"hi"[..]), (true, b"oldhi")] {
        let mut fs = Vfs::new();
        fs.write("/tmp/out", b"old").unwrap();
        let stdout = wire::Stdout::File { path: "/tmp/out".into(), append };
        let mut t = T::with(fs, Start { stdout, ..start(None, &[]) });
        t.put(BUF, b"hi");
        assert_eq!(t.io(FD_WRITE, 1, 2, &[]), (0, 2));
        assert_eq!((t.h.fs.read("/tmp/out"), t.h.out.len()), (Ok(want), 0));
        assert_eq!((t.ok(FD_FDSTAT_GET, &[1, 0]), t.num(0, 1), t.num(2, 2)), (0, 4, append.into()));
    }
}

#[test]
fn preopens_are_the_cwd_each_top_level_directory_and_dev() {
    let mut fs = Vfs::new();
    fs.mkdir("/bin").and(fs.mkdir("/dev")).and(fs.write("/file", b"")).unwrap();
    let s = start(None, &[]);
    let mut t = T::with(fs, s.clone());
    assert_eq!(t.h.ops, [wire::LIST], "LIST / for the preopens");
    // The name of preopen `fd`, a directory.
    let name = |t: &mut T, fd| {
        assert_eq!((t.ok(FD_PRESTAT_GET, &[fd, 0]), t.num(0, 4)), (0, 0));
        assert_eq!(t.ok(FD_PRESTAT_DIR_NAME, &[fd, 8, t.num(4, 4)]), 0);
        String::from_utf8(t.m[8..][..t.num(4, 4) as usize].to_vec()).unwrap()
    };
    let names: Vec<_> = (3..9).map(|fd| name(&mut t, fd)).collect();
    assert_eq!(names, [".", "/apps", "/bin", "/home", "/tmp", "/dev"]);
    assert_eq!([0, 1, 2, 9].map(|fd| t.ok(FD_PRESTAT_GET, &[fd, 0])), [EBADF; 4]);
    assert_eq!(t.ok(FD_PRESTAT_DIR_NAME, &[4, 8, 4]), EINVAL, "a name longer than the buffer");
    assert_eq!(t.ok(FD_FDSTAT_GET, &[3, 0]), 0);
    let all = (1 << 30) - 1;
    assert_eq!([0, 8, 16].map(|at| t.num(at, 8)), [3, all, all], "a directory has every right");
    let mut t = T::with(Vfs::new(), Start { roots: vec![Vfs::HOME.into(), "/tmp".into()], ..s });
    let names: Vec<_> = (4..7).map(|fd| name(&mut t, fd)).collect();
    assert_eq!(names, [Vfs::HOME, "/tmp", "/dev"]);
    assert!(t.h.ops.is_empty() && t.ok(FD_PRESTAT_GET, &[7, 0]) == EBADF, "no LIST");
    // The kernel holds a process to its roots, whatever path it opens.
    assert_eq!((t.open(3, "../home", DIR, 0), t.open(4, "x", CREAT, 0)), (Err(ENOTCAPABLE), Ok(7)));
}

#[test]
fn files_open_read_write_seek_and_stat_through_the_kernel() {
    let mut t = T::new(None, &[]);
    let fd = t.open(3, "a", CREAT | EXCL, 0).unwrap();
    assert_eq!(fd, 8, "the lowest free fd, after the preopens 3 to 7");
    let again = [t.open(3, "a", CREAT | EXCL, 0), t.open(3, "b", 0, 0), t.open(3, "a", DIR, 0)];
    assert_eq!(again, [Err(EEXIST), Err(ENOENT), Err(ENOTDIR)]);
    t.put(BUF, b"hello world");
    assert_eq!(t.io(FD_WRITE, fd, 11, &[]), (0, 11));
    let seek = |t: &mut T, off: i64, w| (t.ok(FD_SEEK, &[fd, off as u64, w, 64]), t.num(64, 8));
    let seeks = [seek(&mut t, 6, 0), seek(&mut t, -1, 2), seek(&mut t, -4, 1)];
    assert_eq!(seeks, [(0, 6), (0, 10), (0, 6)]);
    assert_eq!((t.io(FD_READ, fd, 100, &[]), t.got(5)), ((0, 5), b"world".to_vec()));
    assert_eq!(seek(&mut t, 0, 1), (0, 11), "the read moved the offset");
    assert_eq!((seek(&mut t, -12, 1).0, seek(&mut t, 0, 3).0), (EINVAL, EINVAL));
    // pread and pwrite leave the offset as it is.
    t.put(BUF, b"J");
    assert_eq!(t.io(FD_PWRITE, fd, 1, &[0]), (0, 1));
    assert_eq!((t.io(FD_PREAD, fd, 5, &[0]), t.got(5)), ((0, 5), b"Jello".to_vec()));
    assert_eq!((t.ok(FD_TELL, &[fd, 64]), t.num(64, 8)), (0, 11));
    // One WRITE takes at most 64 KiB: a short write, which libc loops on.
    assert_eq!(t.io(FD_WRITE, fd, 70_000, &[]), (0, 65_536));
    let stat = |t: &mut T, f, a: &[u64]| {
        (t.ok(f, a), [0, 8, 16, 24, 32, 40, 48, 56].map(|at| t.num(256 + at, 8)))
    };
    let a = (0, [0, fnv64(b"/tmp/a"), 4, 1, 65_547, 0, 0, 0]);
    assert_eq!(stat(&mut t, FD_FILESTAT_GET, &[fd, 256]), a);
    t.put(PATH, b"../tmp/./a");
    assert_eq!(stat(&mut t, PATH_FILESTAT_GET, &[3, 0, PATH, 10, 256]), a);
    let root = (0, [0, fnv64(b"/"), 3, 1, 0, 0, 0, 0]);
    assert_eq!(stat(&mut t, PATH_FILESTAT_GET, &[3, 0, PATH, 2, 256]), root, "..: the root");
    assert_eq!(t.ok(PATH_FILESTAT_GET, &[3, 0, PATH + 3, 4, 256]), ENOENT, "/tmp/tmp");
    // Sizes: set_size cuts, fd_allocate only grows.
    assert_eq!(t.ok(FD_FILESTAT_SET_SIZE, &[fd, 3]), 0);
    assert_eq!((t.ok(FD_ALLOCATE, &[fd, 0, 2]), t.ok(FD_ALLOCATE, &[fd, 4, 2])), (0, 0));
    assert_eq!(t.h.fs.read("/tmp/a"), Ok(&b"Jel\0\0\0"[..]));
    // APPEND writes at the end, wherever the offset was; it can be cleared.
    let ap = t.open(3, "a", 0, 1).unwrap();
    assert_eq!((t.ok(FD_FDSTAT_GET, &[ap, 0]), t.num(0, 1), t.num(2, 2)), (0, 4, 1));
    t.put(BUF, b"!");
    assert_eq!(t.io(FD_WRITE, ap, 1, &[]), (0, 1));
    assert_eq!((t.ok(FD_TELL, &[ap, 64]), t.num(64, 8)), (0, 7));
    assert_eq!((t.ok(FD_FDSTAT_SET_FLAGS, &[ap, 0]), t.ok(FD_SEEK, &[ap, 0, 0, 64])), (0, 0));
    assert_eq!(t.io(FD_WRITE, ap, 1, &[]), (0, 1));
    assert_eq!(t.h.fs.read("/tmp/a"), Ok(&b"!el\0\0\0!"[..]));
    assert_eq!(t.open(3, "a", 8, 0), Ok(10), "TRUNC");
    assert_eq!(t.h.fs.read("/tmp/a"), Ok(&b""[..]));
    // close, the lowest free fd again, renumber.
    let closes = [t.ok(FD_CLOSE, &[fd]), t.ok(FD_CLOSE, &[fd]), t.ok(FD_CLOSE, &[99])];
    assert_eq!((closes, t.open(3, "a", 0, 0)), ([0, EBADF, EBADF], Ok(fd)));
    assert_eq!((t.ok(FD_RENUMBER, &[ap, 99]), t.ok(FD_RENUMBER, &[ap, fd])), (EBADF, 0));
    assert_eq!((t.ok(FD_FDSTAT_GET, &[ap, 0]), t.ok(FD_FDSTAT_GET, &[fd, 0])), (EBADF, 0));
    // What a directory, the console or a bad pointer gets.
    let dir = [t.io(FD_READ, 3, 1, &[]).0, t.ok(FD_TELL, &[3, 64]), t.ok(FD_SEEK, &[1, 0, 0, 64])];
    assert_eq!(dir, [EISDIR, EISDIR, ESPIPE]);
    let console = (t.io(FD_PREAD, 1, 1, &[0]).0, t.ok(FD_FILESTAT_SET_SIZE, &[0, 1]));
    assert_eq!(console, (ESPIPE, ESPIPE));
    let tty = (0, [0, fnv64(b"/dev/tty"), 2, 1, 0, 0, 0, 0]);
    assert_eq!(stat(&mut t, FD_FILESTAT_GET, &[1, 256]), tty);
    t.put(PATH, b"c");
    assert_eq!(t.ok(PATH_OPEN, &[3, 0, PATH, 1, CREAT, 0, 0, 0, MEM - 2]), EFAULT);
    assert!(!t.h.fs.exists("/tmp/c"), "nothing is made when the fd cannot be stored");
}

#[test]
fn paths_resolve_lexically_from_their_directory_and_change_the_tree() {
    let mut t = T::new(None, &[]);
    let made = ["d", "d/../../../tmp/./e/"].map(|p| t.at(PATH_CREATE_DIRECTORY, 3, p));
    assert_eq!(made, [0, 0], ".. stops at /");
    assert!(t.h.fs.is_dir("/tmp/d") && t.h.fs.is_dir("/tmp/e"));
    t.h.fs.write("/tmp/d/f", b"f").unwrap();
    // The path at PATH from fd 3 (/tmp), to the one at PATH + 64 from fd 5 (/home).
    let rename = |t: &mut T, from: &str, to: &str| {
        let _ = (t.put(PATH, from.as_bytes()), t.put(PATH + 64, to.as_bytes()));
        t.ok(PATH_RENAME, &[3, PATH, from.len() as u64, 5, PATH + 64, to.len() as u64])
    };
    assert_eq!((rename(&mut t, "d", "guest/d2"), rename(&mut t, "d", "x")), (0, ENOENT));
    assert_eq!(t.h.fs.read(&[Vfs::HOME, "/d2/f"].concat()), Ok(&b"f"[..]));
    assert_eq!(rename(&mut t, "e", "../dev/e"), ENOTSUP, "nothing changes in /dev");
    // Unlink sends a file's REMOVE, rmdir a directory's (the kernel tests the mismatches).
    let (rm, rmdir) = (PATH_UNLINK_FILE, PATH_REMOVE_DIRECTORY);
    let gone = [t.at(rm, 5, "guest/d2/f"), t.at(rmdir, 5, "guest/d2"), t.at(rm, 5, "guest/d2")];
    assert_eq!(gone, [0, 0, ENOENT]);
    assert_eq!([t.at(rm, 7, "null"), t.at(PATH_CREATE_DIRECTORY, 7, "x")], [ENOTSUP, ENOTSUP]);
    // Bad paths and bad directory fds.
    let bad = [t.at(rmdir, 3, "/tmp/e"), t.at(rmdir, 3, "e\0"), t.at(rmdir, 1, "e")];
    assert_eq!((bad, t.at(rmdir, 99, "e")), ([ENOTCAPABLE, EINVAL, ENOTDIR], EBADF));
    t.put(PATH, &[0xFF]);
    assert_eq!((t.ok(rmdir, &[3, PATH, 1]), t.ok(rmdir, &[3, MEM - 1, 2])), (EILSEQ, EFAULT));
    assert_eq!(t.ok(PATH_FILESTAT_SET_TIMES, &[3, 0, PATH + 1, 0, 0, 0, 0]), 0, "touch works");
    assert!(t.h.fs.is_dir("/tmp/e"));
}

#[test]
fn readdir_reads_every_page_cuts_at_the_buffer_and_never_skips_entries_removed_meanwhile() {
    let mut fs = Vfs::new();
    fs.mkdir("/tmp/d").and(fs.mkdir("/tmp/d/s")).and(fs.mkdir("/tmp/big")).unwrap();
    for name in ["x1", "x2", "x3", "x4", "x5"] {
        fs.write(&["/tmp/d/", name].concat(), name.as_bytes()).unwrap();
    }
    let long: Vec<_> = (100..400).map(|i| format!("{i}{}", "x".repeat(240))).collect();
    long.iter().for_each(|name| fs.write(&["/tmp/big/", name].concat(), b"").unwrap());
    let mut t = T::with(fs, start(None, &[]));
    // 254 bytes an entry: 258 fit in the first 64 KiB LIST, then 42, then none.
    let big = t.open(3, "big", DIR, 0).unwrap();
    t.h.ops.clear();
    let (all, used) = t.readdir(big, 100_000, 0);
    assert_eq!((all.len(), used, &t.h.ops[..]), (302, 25 + 26 + 300 * 267, &[wire::LIST; 3][..]));
    assert!(all[2..].iter().zip(&long).all(|(e, name)| e.3 == *name && e.2 == 4));
    let d = t.open(3, "d", DIR, 0).unwrap();
    let (all, used) = t.readdir(d, 1_000, 0);
    let names: Vec<_> = all.iter().map(|e| e.3.as_str()).collect();
    assert_eq!(names, [".", "..", "s", "x1", "x2", "x3", "x4", "x5"]);
    assert_eq!(used, 25 + 26 + 25 + 5 * 26);
    let ino = |p: &str| fnv64(p.as_bytes());
    let want = [(1, ino("/tmp/d"), 3), (2, ino("/tmp"), 3), (3, ino("/tmp/d/s"), 3)];
    assert_eq!(all[..3].iter().map(|e| (e.0, e.1, e.2)).collect::<Vec<_>>(), want);
    assert_eq!((all[3].0, all[3].1, all[3].2), (4, ino("/tmp/d/x1"), 4));
    // A buffer that cuts an entry is full; the next call starts at its cookie.
    assert_eq!(t.readdir(d, 30, 0), (all[..1].to_vec(), 30));
    assert_eq!(t.readdir(d, 1_000, 6).0, all[6..]);
    assert_eq!((t.readdir(d, 1_000, 8), t.readdir(d, 1_000, u64::MAX)), ((vec![], 0), (vec![], 0)));
    // One entry per call, unlinking each file once read: none is skipped.
    let (mut cookie, mut seen) = (0, vec![]);
    while let Some((next, _, kind, name)) = t.readdir(d, 40, cookie).0.first().cloned() {
        assert!(kind != 4 || t.at(PATH_UNLINK_FILE, d, &name) == 0);
        (cookie, seen) = (next, [seen, vec![name]].concat());
    }
    assert_eq!(seen, all.iter().map(|e| e.3.clone()).collect::<Vec<_>>());
    assert_eq!(t.readdir(d, 1_000, 0).0.len(), 3, "cookie 0 lists again: ., .. and s");
    assert_eq!(t.ok(FD_READDIR, &[1, BUF, 100, 0, 64]), ENOTDIR);
    assert_eq!(t.ok(FD_READDIR, &[d, MEM - 10, 100, 0, 64]), EFAULT);
}

#[test]
fn dev_holds_null_tty_winsize_draw_and_events() {
    let mut t = T::new(Some((80, 24)), &[]);
    let devs: Vec<_> = t.readdir(7, 1_000, 0).0.into_iter().map(|e| (e.2, e.3)).collect();
    let names = ["draw", "events", "null", "tty", "winsize"].map(|n| (2, n.to_string()));
    assert_eq!(devs, [vec![(3, ".".into()), (3, "..".into())], names.to_vec()].concat());
    let bad = [("nope", 0), ("null", CREAT | EXCL), ("tty", DIR)].map(|(d, o)| t.open(7, d, o, 0));
    assert_eq!(bad, [Err(ENOENT), Err(EEXIST), Err(ENOTDIR)]);
    assert_eq!((t.open(3, "../dev/.", DIR, 0), t.open(3, "../dev", 0, 0)), (Ok(8), Ok(9)));
    let [null, tty, size, draw, events] =
        ["null", "tty", "winsize", "draw", "events"].map(|d| t.open(7, d, 0, 0).unwrap());
    assert_eq!(t.h.ops, [wire::LIST], "/dev is local: only the preopens' LIST /");
    assert_eq!((t.io(FD_WRITE, null, 100, &[]), t.io(FD_READ, null, 100, &[])), ((0, 100), (0, 0)));
    t.put(BUF, b"hi");
    assert_eq!((t.io(FD_WRITE, tty, 2, &[]), t.h.out.concat()), ((0, 2), b"hi".to_vec()));
    assert_eq!((t.ok(FD_FDSTAT_GET, &[tty, 0]), t.num(0, 1)), (0, 2));
    assert_eq!((t.io(FD_READ, size, 3, &[]), t.got(3)), ((0, 3), b"80 ".to_vec()));
    assert_eq!((t.io(FD_READ, size, 9, &[]), t.got(3)), ((0, 3), b"24\n".to_vec()));
    assert_eq!(t.io(FD_READ, size, 9, &[]), (0, 0));
    assert_eq!((t.ok(FD_FILESTAT_GET, &[size, 256]), t.num(272, 1), t.num(288, 8)), (0, 2, 0));
    // A write to /dev/draw is one DRAW of the whole frame, at most 1 MiB.
    let _ = (t.put(BUF, b"fr"), t.put(BUF + 100, b"ame"));
    t.put(0, &[BUF as u32, 2, BUF as u32 + 100, 3].map(u32::to_le_bytes).concat());
    assert_eq!((t.ok(FD_WRITE, &[draw, 0, 2, 64]), t.num(64, 4)), (0, 5));
    assert_eq!(t.h.frames, [b"frame".to_vec()]);
    t.put(0, &[0, (1 << 20) + 1].map(u32::to_le_bytes).concat());
    assert_eq!(t.ok(FD_WRITE, &[draw, 0, 1, 64]), E2BIG);
    assert_eq!([t.io(FD_READ, draw, 1, &[]).0, t.io(FD_PWRITE, draw, 1, &[0]).0], [EBADF, ESPIPE]);
    // A read takes one event; a part the guest did not take waits in the worker.
    let _ = (t.h.k.post_event(t.h.pid, b"abcdef"), t.h.k.post_event(t.h.pid, b"gh"));
    t.h.ops.clear();
    assert_eq!((t.io(FD_READ, events, 4, &[]), t.got(4)), ((0, 4), b"abcd".to_vec()));
    assert_eq!((t.io(FD_READ, events, 0, &[]), t.io(FD_WRITE, events, 1, &[]).0), ((0, 0), EBADF));
    assert_eq!((t.io(FD_READ, events, 9, &[]), t.got(2)), ((0, 2), b"ef".to_vec()));
    assert_eq!((t.io(FD_READ, events, 9, &[]), t.got(2)), ((0, 2), b"gh".to_vec()));
    assert_eq!(t.h.ops, [wire::EVENTS; 2]);
}

#[test]
fn random_calls_never_panic() {
    let mut t = T::new(Some((80, 24)), &[]);
    for (dir, name) in [(3, "f"), (3, "g"), (7, "draw"), (7, "events")] {
        t.open(dir, name, CREAT, 0).unwrap(); // fds 8 to 11
    }
    // xorshift64: deterministic, never 0 from a nonzero seed.
    let mut x = 0x9E37_79B9_7F4A_7C15_u64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    t.m = (0..MEM).map(|_| b"anull.events"[usize::from(rng() as u8 % 12)]).collect();
    let often = [PATH_OPEN, FD_READ, FD_WRITE, FD_PREAD, FD_PWRITE, FD_READDIR, FD_SEEK, FD_TELL];
    for _ in 0..10_000 {
        let f = [(rng() % 47) as usize, often[(rng() % 8) as usize]][(rng() % 2) as usize];
        let mut arg = || [rng() % 12, rng() % MEM, rng() % 300, rng()][(rng() % 4) as usize];
        let a: Vec<u64> = (0..ARITY.get(f).copied().unwrap_or(1)).map(|_| arg()).collect();
        if f == FD_CLOSE && a[0] < 12 {
            continue; // Keep the console, the preopens and the fds above.
        }
        if let Err(Exit(status)) = t.p.call(f, &a, &mut t.m, &mut t.h) {
            assert!(status < 256);
        }
        (t.h.out, t.h.frames) = (vec![], vec![]);
    }
}
