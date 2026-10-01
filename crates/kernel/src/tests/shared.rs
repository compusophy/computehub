use super::Rng;
use crate::wire::{self, Msg, Writer};
use crate::{Effect, Kernel, Load, MAX_FRAME, NOT_ISOLATED, Program, Spawn};
use vfs::Vfs;

fn hello() -> Spawn {
    let (argv, program) = (vec!["hello".into()], Program::Url("bin/toolbox.wasm".into()));
    let (cwd, stdout, roots) = ("/tmp".into(), wire::Stdout::Console, vec!["/".into()]);
    Spawn { argv, program, cwd, tty: Some((80, 24)), stdout, roots }
}

/// An isolated kernel acting for window 5.
fn kernel() -> Kernel {
    Kernel { isolated: true, owner: 5, ..Kernel::default() }
}

const READY: [u8; 2] = [wire::READY, wire::VERSION];

/// Spawns `s` and answers its READY; the effects and wakes are drained.
fn run(k: &mut Kernel, fs: &mut Vfs, s: Spawn) -> u32 {
    let pid = k.spawn(s).unwrap();
    k.message(fs, pid, &READY);
    let _ = (k.take_effects(), k.take_woken());
    pid
}

/// A kernel running `hello` under `roots`, and its Vfs.
struct Sys {
    k: Kernel,
    fs: Vfs,
    pid: u32,
}

impl Sys {
    fn new(roots: &[&str]) -> Sys {
        let (mut k, mut fs, roots) = (kernel(), Vfs::new(), roots.iter().map(|r| r.to_string()));
        let pid = run(&mut k, &mut fs, Spawn { roots: roots.collect(), ..hello() });
        Sys { k, fs, pid }
    }

    /// Sends `msg` from the process: the effects it asked for.
    fn send(&mut self, msg: &[u8]) -> Vec<Effect> {
        self.k.message(&mut self.fs, self.pid, msg);
        self.k.take_effects()
    }

    /// Sends `m`: its one reply, `(errno, data)`.
    fn ask(&mut self, m: Msg<'_>) -> (u16, Vec<u8>) {
        let fx = self.send(&m.encode());
        let [Effect::Reply { errno, data, .. }] = &fx[..] else { panic!("{m:?}: {fx:?}") };
        (*errno, data.clone())
    }
}

#[test]
fn spawn_asks_for_a_worker_and_starts_it_at_ready() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let pid = k.spawn(hello()).unwrap();
    assert_eq!((pid, k.take_effects()), (2, vec![Effect::Spawn { pid, sab: true }]));
    let (Spawn { argv, cwd, tty, stdout, roots, .. }, role) = (hello(), wire::Role::Process);
    let msg = wire::Start { role, pid, tty, stdout, cwd, roots, argv, env: vec![] }.encode();
    k.message(&mut fs, pid, &READY);
    k.message(&mut fs, pid, &READY); // A second READY starts nothing.
    let program = Load::Url("bin/toolbox.wasm".into());
    assert_eq!(k.take_effects(), [Effect::Start { pid, msg, program }]);
    assert_eq!(k.procs(), [(2, "hello".into(), true)]);
    // What does not decode or goes the wrong way is EINVAL; console reads ENOSYS.
    let (exit, events, mode) = ([wire::EXIT, 0], [wire::EVENTS, 0], [wire::CONS_MODE, 0]);
    for m in [&exit[..], &[wire::SAVE], &[], &[wire::OPEN], &events, &mode] {
        k.message(&mut fs, pid, m);
        k.message(&mut fs, 9, m); // No such process: dropped.
    }
    let (inval, nosys) = (wire::EINVAL, wire::ENOSYS);
    let errnos = [inval, inval, inval, inval, inval, nosys];
    assert_eq!(k.take_effects(), errnos.map(|errno| Effect::Reply { pid, errno, data: vec![] }));
    assert!(k.take_woken().is_empty());
}

#[test]
fn file_ops_serve_the_vfs() {
    let mut s = Sys::new(&["/"]);
    let (open, size) =
        (|oflags, path| Msg::Open { oflags, path }, |n: u64| n.to_le_bytes().to_vec());
    let stat = |kind: u8, n: u64| (0, Writer::default().u8(kind).u64(n).done());
    let (creat, dir, excl, trunc) = (wire::O_CREAT, wire::O_DIRECTORY, wire::O_EXCL, wire::O_TRUNC);
    let write = |off, data: &'static [u8]| Msg::Write { off, path: "/tmp/f", data };
    assert_eq!(s.ask(open(creat, "/tmp/f")), stat(1, 0));
    assert_eq!([s.ask(write(2, b"hi")).1, s.ask(write(wire::APPEND, b"!!")).1], [size(4), size(6)]);
    for (off, data) in [(1, &b"\0hi"[..]), (5, &b"!"[..]), (u64::MAX, &[][..])] {
        assert_eq!(s.ask(Msg::Read { off, max: 3, path: "/tmp/f" }), (0, data.to_vec()));
    }
    assert_eq!([s.ask(open(0, "/tmp/f")), s.ask(open(dir, "/tmp"))], [stat(1, 6), stat(2, 0)]);
    assert_eq!(s.ask(open(creat | trunc, "/tmp/f")), stat(1, 0));
    let (mv, len) = (Msg::Rename { from: "/tmp/f", to: "/tmp/d/g" }, 2);
    for m in [Msg::Mkdir { path: "/tmp/d" }, mv, Msg::SetLen { len, path: "/tmp/d/g" }] {
        assert_eq!(s.ask(m), (0, vec![]));
    }
    assert_eq!(s.fs.read("/tmp/d/g"), Ok(&[0, 0][..]));
    // LIST: `u8 kind, u64 size, str name` per entry, from `skip` on.
    s.fs.write("/tmp/e", b"abc").unwrap();
    let list = |skip| Msg::List { skip, path: "/tmp" };
    let d = Writer::default().u8(2).u64(0).str("d").done();
    let e = Writer::default().u8(1).u64(3).str("e").done();
    let all = [s.ask(list(0)).1, s.ask(list(1)).1, s.ask(list(2)).1];
    assert_eq!(all, [[&d[..], &e].concat(), e, vec![]]);
    let rm = |kind, path| Msg::Remove { kind, path };
    assert_eq!([s.ask(rm(1, "/tmp/d/g")), s.ask(rm(2, "/tmp/d"))], [(0, vec![]), (0, vec![])]);
    // Failures are WASI errnos and change nothing.
    let rename = Msg::Rename { from: "/x", to: "/y" };
    #[rustfmt::skip]
    let errnos = [(open(creat | excl, "/tmp/e"), wire::EEXIST), (open(dir, "/tmp/e"), wire::ENOTDIR),
        (open(creat, "/tmp"), wire::EISDIR), (open(trunc, "/tmp"), wire::EISDIR),
        (open(0, "/tmp/x"), wire::ENOENT), (open(creat | dir, "/tmp/x"), wire::ENOENT),
        (open(creat, "/tmp/e/x"), wire::ENOTDIR), (open(16, "/tmp/e"), wire::EINVAL),
        (Msg::Read { off: 0, max: 1, path: "/tmp" }, wire::EISDIR),
        (Msg::Write { off: 0, path: "/tmp/x", data: b"" }, wire::ENOENT),
        (Msg::List { skip: 0, path: "/tmp/e" }, wire::ENOTDIR), (Msg::Mkdir { path: "/tmp" }, wire::EEXIST),
        (rm(1, "/tmp"), wire::EISDIR), (rm(2, "/tmp/e"), wire::ENOTDIR), (rm(2, "/home"), wire::ENOTEMPTY),
        (rm(3, "/tmp/e"), wire::EINVAL), (rename, wire::ENOENT),
        (Msg::SetLen { len: 1 << 30, path: "/tmp/e" }, wire::ENOSPC)];
    for (m, errno) in errnos {
        assert_eq!(s.ask(m), (errno, vec![]), "{m:?}");
    }
    assert_eq!((s.fs.read("/tmp/e"), s.fs.list("/tmp").map(|l| l.len())), (Ok(&b"abc"[..]), Ok(1)));
    // A reply holds at most 64 KiB: 251 entries of 261 bytes, then the other 49.
    s.fs.write("/tmp/r", &[7; 70_000]).unwrap();
    let read = Msg::Read { off: 1, max: u32::MAX, path: "/tmp/r" };
    assert_eq!(s.ask(read), (0, vec![7; wire::MAX_PAYLOAD]));
    s.fs.mkdir("/tmp/big").unwrap();
    (0..300).for_each(|i| s.fs.write(&format!("/tmp/big/{i:0>250}"), b"").unwrap());
    let mut page = |skip| s.ask(Msg::List { skip, path: "/tmp/big" }).1.len();
    assert_eq!([page(0), page(251), page(300)], [251 * 261, 49 * 261, 0]);
}

#[test]
fn paths_must_be_normalized_and_under_a_root() {
    // "" and "rel" are no roots: they grant nothing.
    let mut s = Sys::new(&["/tmp", Vfs::HOME, "", "rel"]);
    s.fs.write("/apps/x", b"x").unwrap();
    let creat = |path| Msg::Open { oflags: wire::O_CREAT, path };
    for path in ["tmp/f", "/tmp/", "/tmp//f", "/tmp/./f", "/tmp/../tmp/f", "", "~/f", "/tmp/\0"] {
        assert_eq!(s.ask(creat(path)).0, wire::EINVAL, "{path:?}");
    }
    for path in ["/", "/apps/x", "/home", "/tmpx", "/tmpx/f", "/rel", "/x"] {
        assert_eq!(s.ask(creat(path)).0, wire::ENOTCAPABLE, "{path:?}");
    }
    let mut mv = |from, to| s.ask(Msg::Rename { from, to }).0;
    assert_eq!([mv("/apps/x", "/tmp/x"), mv("/tmp", "/apps/t")], [wire::ENOTCAPABLE; 2]);
    assert_eq!([s.ask(creat(&[Vfs::HOME, "/f"].concat())).0, s.ask(creat("/tmp/f")).0], [0, 0]);
    assert!(s.fs.exists("/apps/x") && !s.fs.exists("/tmp/x") && !s.fs.exists("/apps/t"));
}

#[test]
fn frames_become_draws_and_events_wait_for_their_reader() {
    let mut s = Sys::new(&["/"]);
    let pid = s.pid;
    let reply = |data: &[u8]| Effect::Reply { pid, errno: 0, data: data.to_vec() };
    assert_eq!(s.send(&[wire::DRAW, 1, 2]), [Effect::Draw { pid, frame: vec![1, 2] }, reply(&[])]);
    let too_big = s.send(&vec![wire::DRAW; MAX_FRAME + 2]);
    assert_eq!(too_big, [Effect::Reply { pid, errno: wire::E2BIG, data: vec![] }]);
    // One event per read, or what is left of it; a read waits for one.
    let events = |max: u32| [&[wire::EVENTS][..], &max.to_le_bytes()].concat();
    let posts = [(pid, &[5][..]), (pid, &[1, 7, 0, 0, 0]), (pid, &[]), (9, &[6])];
    posts.iter().for_each(|&(to, event)| s.k.post_event(to, event));
    let reads = [events(64), events(3), events(64)].map(|m| s.send(&m));
    assert_eq!(reads, [[reply(&[5])], [reply(&[1, 7, 0])], [reply(&[0, 0])]]);
    assert!(s.send(&events(64)).is_empty());
    s.k.post_event(pid, &[4, 1, 0, 2, 0]);
    assert_eq!(s.k.take_effects(), [reply(&[4, 1, 0, 2, 0])]);
    // At most 1 MiB is queued: the third half is dropped.
    (0..3).for_each(|_| s.k.post_event(pid, &vec![2; MAX_FRAME / 2]));
    let all: Vec<_> = (0..16).flat_map(|_| s.send(&events(u32::MAX))).collect();
    assert_eq!((all.len(), &all[15], s.send(&events(1))), (16, &reply(&[2; 65_536]), vec![]));
    s.k.kill(pid, wire::KILLED);
    s.k.post_event(pid, &[5]); // An ended process gets no events.
    assert_eq!(s.k.take_effects(), [Effect::Kill { pid }]);
}

#[test]
fn output_exits_kills_and_failures_wake_the_owner_and_reap_gives_each_status_once() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let pid = run(&mut k, &mut fs, hello());
    k.message(&mut fs, pid, &Msg::ConsWrite { data: b"hi\n" }.encode());
    k.message(&mut fs, pid, &Msg::ConsWrite { data: b"there\n" }.encode());
    assert_eq!((k.take_woken(), k.take_woken()), (vec![5], vec![]));
    assert_eq!((k.reap(pid), k.take_output(pid)), (None, b"hi\nthere\n".to_vec()));
    k.message(&mut fs, pid, &Msg::Exit { status: 3 }.encode());
    assert_eq!((k.take_effects(), k.take_woken()), (vec![Effect::Kill { pid }], vec![5]));
    // An ended process takes no more output and no kill.
    k.message(&mut fs, pid, &Msg::ConsWrite { data: b"late" }.encode());
    k.kill(pid, wire::INTERRUPTED);
    assert!(k.take_effects().is_empty() && k.take_output(pid).is_empty());
    assert_eq!(k.procs(), [(pid, "hello".into(), false)]);
    assert_eq!((k.reap(pid), k.reap(pid), k.procs()), (Some(3), None, vec![]));
    // Kills, failures, version mismatches and missing programs end with their statuses.
    let (mut k, mut fs) = (kernel(), Vfs::new());
    fs.write("/tmp/p.wasm", b"\0asm").unwrap();
    let at = |path: &str| Spawn { program: Program::Vfs(path.into()), tty: None, ..hello() };
    let a = k.spawn(hello()).unwrap(); // Never READY: still killable.
    let b = run(&mut k, &mut fs, hello());
    let [c, d, e] = [hello(), at("/tmp/gone"), at("/tmp/p.wasm")].map(|s| k.spawn(s).unwrap());
    let _ = (k.take_effects(), k.kill(a, wire::INTERRUPTED), k.failed(b));
    k.message(&mut fs, c, &[wire::READY, wire::VERSION + 1]);
    [d, e].into_iter().for_each(|pid| k.message(&mut fs, pid, &READY));
    let (fx, kills) = (k.take_effects(), [a, b, c, d].map(|pid| Effect::Kill { pid }));
    assert_eq!((&fx[..4], fx.len(), k.take_woken()), (&kills[..], 5, vec![5]));
    assert!(matches!(&fx[4], Effect::Start { program: Load::Bytes(p), .. } if p == b"\0asm"));
    assert_eq!(k.take_output(c), b"the OS was updated; reload the page\n");
    assert_eq!(k.take_output(d), b"hello: not found\n");
    assert_eq!([a, b, c, d].map(|pid| k.reap(pid)), [Some(130), Some(126), Some(126), Some(127)]);
}

#[test]
fn spawn_refuses_past_its_limits_and_a_closed_window_ends_its_processes() {
    assert_eq!(Kernel::new().spawn(hello()), Err(NOT_ISOLATED));
    let mut k = kernel();
    let long = Spawn { argv: vec!["x".repeat(wire::MAX_START)], ..hello() };
    assert_eq!(k.spawn(long), Err("argument list too long"));
    assert!(k.take_effects().is_empty() && k.procs().is_empty());
    let pids: Vec<u32> = (0..wire::MAX_PROCS).map(|_| k.spawn(hello()).unwrap()).collect();
    assert_eq!((pids, k.spawn(hello())), ((2..10).collect(), Err("too many programs are running")));
    k.kill(2, wire::KILLED);
    assert_eq!(k.spawn(hello()), Ok(10)); // An ended process frees its slot, not its pid.
    // Closing a window kills and forgets its processes.
    let mut k = kernel();
    let [a, b, c] = [5, 6, 5].map(|owner| (k.set_owner(owner), k.spawn(hello()).unwrap()).1);
    let _ = (k.kill(c, 0), k.take_effects(), k.take_woken());
    k.kill_owned(5);
    assert_eq!((k.take_effects(), k.take_woken()), (vec![Effect::Kill { pid: a }], vec![]));
    assert_eq!((k.procs(), k.reap(c)), (vec![(b, "hello".into(), true)], None));
    k.resize(a, 1, 1);
    k.resize(b, 100, 30);
    let word = |index, value| Effect::Word { pid: b, index, value };
    assert_eq!(k.take_effects(), [word(wire::COLS, 100), word(wire::ROWS, 30)]);
}

/// 10,000 messages, well formed or not, from a process under /tmp: none
/// panics, every reply fits the SAB, and nothing outside /tmp changes.
#[test]
fn random_messages_never_panic_or_leave_the_roots() {
    let (mut s, mut rng) = (Sys::new(&["/tmp"]), Rng(0x2545_F491_4F6C_DD1D));
    let paths = ["/", "/tmp", "/tmp/a", "/tmp/a/b", "/tmp/b", "/apps/x", "/apps/y", "/tmp/.."];
    let dirs = ["/", "/apps", "/home", Vfs::HOME];
    let outside = |fs: &Vfs| (dirs.map(|d| fs.list(d)), fs.read(paths[5]).map(<[u8]>::to_vec));
    s.fs.write(paths[5], b"keep").unwrap();
    let before = outside(&s.fs);
    for _ in 0..10_000 {
        let (r, n) = (rng.next(), rng.next());
        let (path, to) = (paths[r as usize % 8], paths[(r >> 8) as usize % 8]);
        let (data, off) =
            (rng.bytes((r >> 16) as usize % 8), [n % 9, wire::APPEND][(r >> 63) as usize]);
        let mut msg = match (r >> 24) % 12 {
            0 => Msg::Open { oflags: n as u8 % 32, path }.encode(),
            1 => Msg::Read { off, max: n as u32 % 9, path }.encode(),
            2 => Msg::Write { off, path, data: &data }.encode(),
            3 => Msg::List { skip: n as u32 % 4, path }.encode(),
            4 => Msg::Mkdir { path }.encode(),
            5 => Msg::Remove { kind: n as u8 % 4, path }.encode(),
            6 => Msg::Rename { from: path, to }.encode(),
            7 => Msg::SetLen { len: n % 9, path }.encode(),
            8 => [wire::EVENTS, n as u8 % 4, 0, 0, 0].to_vec(),
            9 => [&[wire::DRAW][..], &data].concat(),
            _ => [&[n as u8 % 0x22][..], &data].concat(), // Mostly real ops.
        };
        msg.truncate(if n % 3 == 0 { r as usize % (msg.len() + 1) } else { msg.len() });
        s.k.post_event(s.pid, &data);
        let big = |fx: &Effect| matches!(fx, Effect::Reply { data, .. } if data.len() > 65_536);
        assert!(!s.send(&msg).iter().any(big));
        if s.k.reap(s.pid).is_some() {
            s.pid = run(&mut s.k, &mut s.fs, Spawn { roots: vec!["/tmp".into()], ..hello() });
        }
        s.k.take_output(s.pid);
    }
    assert_eq!(outside(&s.fs), before);
}
