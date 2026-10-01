use crate::wasi::*;
use crate::wire::{self, EBADF, EFAULT, EINVAL, ENOSYS, ENOTDIR, ENOTSUP, Role, Start};
use vfs::Vfs;

/// The size of every test memory: 128 KiB.
const MEM: u64 = 1 << 17;

/// Guest memory in a Vec: out of bounds is EFAULT.
struct VecMem(Vec<u8>);

impl Mem for VecMem {
    fn read(&self, at: u32, len: u32) -> Result<Vec<u8>, u16> {
        self.0.get(at as usize..at as usize + len as usize).map(<[u8]>::to_vec).ok_or(EFAULT)
    }
    fn write(&mut self, at: u32, d: &[u8]) -> Result<(), u16> {
        self.0.get_mut(at as usize..at as usize + d.len()).ok_or(EFAULT)?.copy_from_slice(d);
        Ok(())
    }
}

/// A worker that records what the process asked of it. Step 1 never talks
/// to the kernel or waits (the worker, not Proc, posts EXIT).
#[derive(Default)]
struct Fake {
    out: Vec<Vec<u8>>,
    clocks: Vec<u32>,
    random: Vec<usize>,
}

impl Host for Fake {
    fn call(&mut self, _: &[u8]) -> (u16, Vec<u8>) {
        unreachable!("no requests in step 1")
    }
    fn post(&mut self, _: &[u8]) {
        unreachable!("no posts in step 1")
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
        unreachable!("no waits in step 1")
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

/// A process, 128 KiB of its memory, and its worker.
struct T {
    p: Proc,
    m: VecMem,
    h: Fake,
}

impl T {
    fn new(tty: Option<(u16, u16)>, env: &[&str]) -> T {
        let mut h = Fake::default();
        T { p: Proc::new(&start(tty, env), &mut h).unwrap(), m: VecMem(vec![0; MEM as usize]), h }
    }

    /// The errno of `f(a)`; panics on an exit.
    fn ok(&mut self, f: usize, a: &[u64]) -> u16 {
        self.p.call(f, a, &mut self.m, &mut self.h).unwrap()
    }

    /// The little-endian number in the `n` bytes at `at`.
    fn num(&self, at: usize, n: usize) -> u64 {
        self.m.0[at..][..n].iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b))
    }

    /// Checks that `sizes` and `get` copy out exactly `want`: count and
    /// size, the C strings back to back at 4096, their addresses at 64.
    fn strings(&mut self, sizes: usize, get: usize, want: &[&str]) {
        assert_eq!((self.ok(sizes, &[0, 4]), self.ok(get, &[64, 4096])), (0, 0));
        let blob: Vec<u8> = want.iter().flat_map(|s| [s.as_bytes(), b"\0"].concat()).collect();
        assert_eq!((self.num(0, 4), self.num(4, 4)), (want.len() as u64, blob.len() as u64));
        assert_eq!(self.m.0[4096..][..blob.len()], blob);
        let mut at = 4096;
        for (i, s) in want.iter().enumerate() {
            assert_eq!(self.num(64 + 4 * i, 4), at);
            at += s.len() as u64 + 1;
        }
        assert_eq!(self.ok(get, &[64, MEM - 1]), EFAULT);
    }
}

#[test]
fn every_function_links_with_its_arity_and_answers_for_now() {
    let sigs = [(PATH_OPEN, 9), (SCHED_YIELD, 0), (FD_WRITE, 4), (SOCK_RECV, 6), (PATH_LINK, 7)];
    assert!(sigs.iter().all(|&(f, n)| ARITY[f] == n) && ARITY.iter().sum::<usize>() == 152);
    let mut t = T::new(Some((80, 24)), &[]);
    let local = [ARGS_GET, ARGS_SIZES_GET, ENVIRON_GET, ENVIRON_SIZES_GET, CLOCK_RES_GET];
    let local = [&local[..], &[CLOCK_TIME_GET, RANDOM_GET, FD_FDSTAT_GET, FD_WRITE, SCHED_YIELD]];
    let mut stubs = [SOCK_ACCEPT, SOCK_RECV, SOCK_SEND, SOCK_SHUTDOWN, PATH_LINK, PATH_SYMLINK]
        .map(|f| (f, ENOTSUP))
        .to_vec();
    stubs.extend([(FD_FILESTAT_SET_TIMES, 0), (PATH_FILESTAT_SET_TIMES, ENOTDIR)]);
    stubs.extend([(PATH_READLINK, EINVAL), (FD_PRESTAT_GET, EBADF)]);
    for f in 0..NAMES.len() {
        let got = t.p.call(f, &vec![1; ARITY[f]], &mut t.m, &mut t.h);
        let now = if local.concat().contains(&f) { 0 } else { ENOSYS };
        if f != PROC_EXIT && f != PROC_RAISE {
            let want = stubs.iter().find(|s| s.0 == f).map_or(now, |s| s.1);
            assert_eq!(got, Ok(want), "{}", NAMES[f]);
        }
        assert_eq!(t.ok(f, &vec![1; ARITY[f] + 1]), EINVAL, "{} with one too many", NAMES[f]);
    }
    assert_eq!(t.ok(FD_FILESTAT_SET_TIMES, &[3, 0, 0, 0]), EBADF);
    assert_eq!((t.ok(NAMES.len(), &[]), t.ok(usize::MAX, &[])), (ENOSYS, ENOSYS));
}

#[test]
fn args_and_environ_copy_out_as_c_strings() {
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
}

#[test]
fn a_bad_start_is_refused() {
    let mut h = Fake::default();
    for env in ["NOEQUALS", "=x", "A=\0"] {
        assert_eq!(Proc::new(&start(None, &[env]), &mut h).err(), Some(EINVAL), "{env}");
    }
    let mut s = start(None, &[]);
    s.argv.push("a\0b".into());
    assert_eq!(Proc::new(&s, &mut h).err(), Some(EINVAL));
    s.argv = vec!["x".repeat(wire::MAX_START)];
    assert_eq!(Proc::new(&s, &mut h).err(), Some(wire::E2BIG));
    s.stdout = wire::Stdout::File { path: "/tmp/x".into(), append: false };
    assert_eq!(Proc::new(&s, &mut h).err(), Some(ENOSYS), "files come with step 2");
}

#[test]
fn clocks_random_exit_raise_and_yield() {
    let mut t = T::new(None, &[]);
    for id in 0..4 {
        t.m.0[8..24].fill(0xEE);
        assert_eq!((t.ok(CLOCK_RES_GET, &[id, 8]), t.num(8, 8)), (0, 100_000));
        assert_eq!((t.ok(CLOCK_TIME_GET, &[id, 1, 16]), t.num(16, 8)), (0, 1_234_500_000));
    }
    assert_eq!(t.h.clocks, [0, 1, 1, 1], "CPU time reads monotonic");
    assert_eq!((t.ok(CLOCK_RES_GET, &[4, 8]), t.ok(CLOCK_TIME_GET, &[4, 0, 8])), (EINVAL, EINVAL));
    assert_eq!(t.ok(CLOCK_TIME_GET, &[0, 0, MEM - 7]), EFAULT);

    assert_eq!(t.ok(RANDOM_GET, &[100, 70_000]), 0);
    assert_eq!(t.h.random, [65_536, 4_464]);
    assert!(t.m.0[100..70_100].iter().all(|&b| b == 0x5A) && t.m.0[70_100] == 0);
    let far = [t.ok(RANDOM_GET, &[MEM - 10, 11]), t.ok(RANDOM_GET, &[u32::MAX.into(), 9])];
    assert_eq!(far, [EFAULT, EFAULT]);

    let mut call = |f, a: &[u64]| t.p.call(f, a, &mut t.m, &mut t.h);
    assert_eq!((call(PROC_EXIT, &[0x1_02]), call(PROC_EXIT, &[0])), (Err(Exit(2)), Err(Exit(0))));
    assert_eq!((call(PROC_RAISE, &[9]), call(PROC_RAISE, &[0])), (Err(Exit(137)), Ok(0)));
    assert_eq!((call(PROC_RAISE, &[128]), call(SCHED_YIELD, &[])), (Ok(EINVAL), Ok(0)));
}

#[test]
fn console_fdstat_makes_isatty_true_only_with_a_tty() {
    for (tty, kind) in [(Some((80, 24)), 2), (None, 0)] {
        let mut t = T::new(tty, &[]);
        for fd in 0..3 {
            t.m.0[..24].fill(0xAA);
            assert_eq!(t.ok(FD_FDSTAT_GET, &[fd, 0]), 0);
            let rights = [0x4A, 0, 0x20, 0x08, 0, 0, 0, 0];
            assert_eq!(t.m.0[..24], [&[kind, 0, 0, 0, 0, 0, 0, 0][..], &rights, &[0; 8]].concat());
        }
        assert_eq!(t.num(8, 8) & ((1 << 2) | (1 << 5)), 0, "no FD_SEEK or FD_TELL");
        let bad = [t.ok(FD_FDSTAT_GET, &[3, 0]), t.ok(FD_FDSTAT_GET, &[1, MEM - 23])];
        assert_eq!(bad, [EBADF, EFAULT]);
    }
}

#[test]
fn fd_write_puts_console_bytes_in_the_ring() {
    let mut t = T::new(Some((80, 24)), &[]);
    let iovs = |t: &mut T, list: &[(u32, u32)]| {
        let b: Vec<u8> =
            list.iter().flat_map(|&(a, n)| [a, n]).flat_map(u32::to_le_bytes).collect();
        t.m.0[..b.len()].copy_from_slice(&b);
    };
    t.m.0[1000..1005].copy_from_slice(b"hello");
    t.m.0[2000..2003].copy_from_slice(b" 42");
    iovs(&mut t, &[(1000, 5), (u32::MAX, 0), (2000, 3)]);
    for fd in [1, 2, 0] {
        assert_eq!((t.ok(FD_WRITE, &[fd, 0, 3, 64]), t.num(64, 4)), (0, 8));
    }
    iovs(&mut t, &[(1000, 5), (MEM as u32 - 2, 5)]);
    assert_eq!((t.ok(FD_WRITE, &[1, 0, 2, 64]), t.num(64, 4)), (0, 5), "a short write");
    iovs(&mut t, &[(MEM as u32 - 2, 5), (1000, 5)]);
    assert_eq!(t.ok(FD_WRITE, &[1, 0, 2, 64]), EFAULT);
    assert_eq!(t.h.out.concat(), b"hello 42hello 42hello 42hello");

    t.h.out.clear();
    iovs(&mut t, &[(0, 70_000)]);
    assert_eq!((t.ok(FD_WRITE, &[1, 0, 1, 70_000]), t.num(70_000, 4)), (0, 70_000));
    assert_eq!(t.h.out.iter().map(Vec::len).collect::<Vec<_>>(), [65_536, 4_464]);
    let bad = [[3, 0, 1, 64], [1, MEM - 4, 1, 64], [1, 0, 1_025, 64], [1, 0, 1, MEM - 3]];
    for (a, e) in bad.iter().zip([EBADF, EFAULT, EINVAL, EFAULT]) {
        assert_eq!(t.ok(FD_WRITE, a), e, "{a:?}");
    }
}
