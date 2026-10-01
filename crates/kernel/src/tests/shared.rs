use super::Rng;
use crate::wire::{self, Msg};
use crate::{Effect, Kernel, Load, NOT_ISOLATED, Program, Spawn};
use vfs::Vfs;

fn hello() -> Spawn {
    Spawn {
        argv: vec!["hello".into()],
        program: Program::Url("bin/toolbox.wasm".into()),
        cwd: "/tmp".into(),
        tty: Some((80, 24)),
        stdout: wire::Stdout::Console,
        roots: vec!["/".into()],
    }
}

/// An isolated kernel acting for window 5.
fn kernel() -> Kernel {
    let mut k = Kernel::new();
    k.set_isolated(true);
    k.set_owner(5);
    k
}

const READY: [u8; 2] = [wire::READY, wire::VERSION];

/// Spawns `s` and answers its READY; the effects and wakes are drained.
fn run(k: &mut Kernel, fs: &mut Vfs, s: Spawn) -> u32 {
    let pid = k.spawn(s).unwrap();
    k.message(fs, pid, &READY);
    k.take_effects();
    k.take_woken();
    pid
}

#[test]
fn spawn_asks_for_a_worker_and_starts_it_at_ready() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let pid = k.spawn(hello()).unwrap();
    assert_eq!((pid, k.take_effects()), (2, vec![Effect::Spawn { pid, sab: true }]));
    let (role, tty, stdout) = (wire::Role::Process, Some((80, 24)), wire::Stdout::Console);
    let (cwd, roots, argv) = ("/tmp".into(), vec!["/".into()], vec!["hello".into()]);
    let msg = wire::Start { role, pid, tty, stdout, cwd, roots, argv, env: vec![] }.encode();
    k.message(&mut fs, pid, &READY);
    k.message(&mut fs, pid, &READY); // A second READY starts nothing.
    let program = Load::Url("bin/toolbox.wasm".into());
    assert_eq!(k.take_effects(), [Effect::Start { pid, msg, program }]);
    assert_eq!(k.procs(), [(2, "hello".into(), true)]);
    // Bytes that do not decode are EINVAL; ops not served yet are ENOSYS.
    let mkdir = Msg::Mkdir { path: "/tmp/d" }.encode();
    let (exit, mode) = ([wire::EXIT, 0], [wire::CONS_MODE]);
    for m in [&exit[..], &[wire::SAVE], &[], &[wire::DRAW], &mkdir, &[wire::OPEN], &mode] {
        k.message(&mut fs, pid, m);
        k.message(&mut fs, 9, m); // No such process: dropped.
    }
    let (inval, nosys) = (wire::EINVAL, wire::ENOSYS);
    let errnos = [inval, inval, inval, inval, nosys, nosys, nosys];
    assert_eq!(k.take_effects(), errnos.map(|errno| Effect::Reply { pid, errno, data: vec![] }));
    assert!(k.take_woken().is_empty());
}

#[test]
fn output_and_exit_wake_the_owner_and_reap_gives_the_status_once() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let pid = run(&mut k, &mut fs, hello());
    for out in ["hi\n", "there\n"] {
        k.message(&mut fs, pid, &Msg::ConsWrite { data: out.as_bytes() }.encode());
    }
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
}

#[test]
fn kills_failures_and_mismatches_end_with_their_statuses() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let a = k.spawn(hello()).unwrap(); // Never READY: still killable.
    let b = run(&mut k, &mut fs, hello());
    let c = k.spawn(Spawn { tty: None, ..hello() }).unwrap();
    k.take_effects();
    k.kill(a, wire::INTERRUPTED);
    k.failed(b);
    k.message(&mut fs, c, &[wire::READY, wire::VERSION + 1]);
    let kills = [a, b, c].map(|pid| Effect::Kill { pid });
    assert_eq!((k.take_effects(), k.take_woken()), (kills.to_vec(), vec![5]));
    assert_eq!(k.take_output(c), b"the OS was updated; reload the page\n");
    let statuses = [a, b, c].map(|pid| k.reap(pid));
    assert_eq!(statuses, [Some(130), Some(126), Some(126)]);
}

#[test]
fn a_vfs_program_is_read_when_its_worker_is_ready() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    fs.write("/tmp/p.wasm", b"\0asm").unwrap();
    let at = |path: &str| Spawn { program: Program::Vfs(path.into()), tty: None, ..hello() };
    let (a, b) = (k.spawn(at("/tmp/p.wasm")).unwrap(), k.spawn(at("/tmp/gone")).unwrap());
    k.take_effects();
    for pid in [a, b] {
        k.message(&mut fs, pid, &READY);
    }
    let fx = k.take_effects();
    assert!(matches!(&fx[..], [Effect::Start { program: Load::Bytes(p), .. }, Effect::Kill { .. }]
        if p == b"\0asm"));
    assert_eq!((k.take_output(b), k.reap(b)), (b"hello: not found\n".to_vec(), Some(127)));
}

#[test]
fn spawn_refuses_without_isolation_past_eight_and_long_arguments() {
    assert_eq!(Kernel::new().spawn(hello()), Err(NOT_ISOLATED));
    let mut k = kernel();
    let long = Spawn { argv: vec!["x".repeat(wire::MAX_START)], ..hello() };
    assert_eq!(k.spawn(long), Err("argument list too long"));
    assert!(k.take_effects().is_empty() && k.procs().is_empty());
    let pids: Vec<u32> = (0..wire::MAX_PROCS).map(|_| k.spawn(hello()).unwrap()).collect();
    assert_eq!((pids, k.spawn(hello())), ((2..10).collect(), Err("too many programs are running")));
    k.kill(2, wire::KILLED);
    assert_eq!(k.spawn(hello()), Ok(10)); // An ended process frees its slot, not its pid.
}

#[test]
fn closing_a_window_kills_and_forgets_its_processes() {
    let mut k = kernel();
    let a = k.spawn(hello()).unwrap();
    k.set_owner(6);
    let b = k.spawn(hello()).unwrap();
    k.set_owner(5);
    let c = k.spawn(hello()).unwrap();
    k.kill(c, 0);
    k.take_effects();
    k.take_woken();
    k.kill_owned(5);
    assert_eq!((k.take_effects(), k.take_woken()), (vec![Effect::Kill { pid: a }], vec![]));
    assert_eq!((k.procs(), k.reap(c)), (vec![(b, "hello".into(), true)], None));
    k.resize(a, 1, 1);
    k.resize(b, 100, 30);
    let word = |index, value| Effect::Word { pid: b, index, value };
    assert_eq!(k.take_effects(), [word(wire::COLS, 100), word(wire::ROWS, 30)]);
}

#[test]
fn random_messages_never_panic() {
    let (mut k, mut fs) = (kernel(), Vfs::new());
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut pid = run(&mut k, &mut fs, hello());
    for _ in 0..10_000 {
        let len = (rng.next() % 40) as usize;
        let mut b = rng.bytes(len);
        if let Some(op) = b.first_mut() {
            *op %= 0x22; // Mostly real ops.
        }
        k.message(&mut fs, pid, &b);
        if k.reap(pid).is_some() {
            pid = run(&mut k, &mut fs, hello());
        }
        let _ = (k.take_effects(), k.take_output(pid));
    }
}
