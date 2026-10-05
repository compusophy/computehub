use crate::wire::{self, Job, Msg, Start, Stdin, Stdout};
use crate::{Effect, Kernel, Load, Program, Spawn, program};
use vfs::Vfs;

/// A kernel for window 5 running `sh` on a 100 x 30 console under `/`, past its READY, with the
/// programs `/bin/a` and `/bin/b`, the wasm `/tmp/w.wasm` and the file `/tmp/f`.
struct Sh {
    k: Kernel,
    fs: Vfs,
    sh: u32,
}

impl Sh {
    fn new() -> Sh {
        let (mut k, mut fs) =
            (Kernel { isolated: true, owner: 5, ..Kernel::default() }, Vfs::new());
        fs.mkdir("/bin").and(crate::install(&mut fs, "a", "toolbox")).unwrap();
        fs.write("/bin/b", b"#!wasm /tmp/w.wasm").and(fs.write("/tmp/w.wasm", b"\0asm")).unwrap();
        fs.write("/tmp/f", b"text").unwrap();
        let (argv, program) = (vec!["sh".into()], Program::Url("bin/toolbox.wasm".into()));
        let (cwd, stdout, roots) = ("/".into(), Stdout::Console, vec!["/".into()]);
        let sh = k.spawn(Spawn { argv, program, cwd, tty: Some((80, 24)), stdout, roots }).unwrap();
        k.message(&mut fs, sh, &[wire::READY, wire::VERSION]);
        k.resize(sh, 100, 30);
        let _ = (k.take_effects(), k.take_woken());
        Sh { k, fs, sh }
    }
    /// `m` from `pid`: the effects.
    fn send(&mut self, pid: u32, m: Msg<'_>) -> Vec<Effect> {
        self.k.message(&mut self.fs, pid, &m.encode());
        self.k.take_effects()
    }
    /// The shell starts `job`: the reply's errno and data.
    fn spawn(&mut self, job: &Job<'_>) -> (u16, Vec<u8>) {
        match &self.send(self.sh, Msg::Spawn { job: &job.encode() })[..] {
            [.., Effect::Reply { errno, data, .. }] => (*errno, data.clone()),
            fx => panic!("{fx:?}"),
        }
    }
    /// Each of `pids` READY: their Starts and programs.
    fn ready(&mut self, pids: &[u32]) -> Vec<(Start, Load)> {
        let ready = [wire::READY, wire::VERSION];
        pids.iter().for_each(|&pid| self.k.message(&mut self.fs, pid, &ready));
        let starts = self.k.take_effects().into_iter().map(|e| match e {
            Effect::Start { msg, program, .. } => (Start::decode(&msg).unwrap(), program),
            e => panic!("{e:?}"),
        });
        starts.collect()
    }
}
fn job<'a>(stages: &[(&'a str, &str)], stdin: Stdin, stdout: Stdout, data: &'a [u8]) -> Job<'a> {
    let argv = |s: &str| s.split(' ').map(String::from).collect();
    let stages = stages.iter().map(|&(path, words)| (path, argv(words))).collect();
    Job { cwd: "/tmp", stdin, stdout, stages, data }
}
fn reply(pid: u32, data: &[u8]) -> Effect {
    Effect::Reply { pid, errno: 0, data: data.to_vec() }
}
fn err(pid: u32, errno: u16) -> Effect {
    Effect::Reply { pid, errno, data: vec![] }
}

#[test]
fn a_job_starts_its_stages_piped_on_the_console_and_its_wait_gets_the_last_status() {
    let mut s = Sh::new();
    let ab = job(&[("/bin/a", "a 1"), ("/bin/b", "b")], Stdin::Pipe, Stdout::Console, b"in");
    let (sh, j) = (s.sh, s.spawn(&ab));
    assert_eq!(j, (0, 3u32.to_le_bytes().to_vec()), "the job is its first pid");
    assert!(s.k.owns(sh) && s.k.owns(4), "the shell's window owns its job");
    s.k.set_owner(6);
    assert!(!s.k.owns(4) && !s.k.owns(9), "another window owns none of it");
    s.k.set_owner(5);
    let starts = s.ready(&[3, 4]);
    #[rustfmt::skip]
    let want = |pid, argv: &str, stdin, stdout| Start { role: wire::Role::Process, pid,
        tty: Some((100, 30)), stdin, stdout, cwd: "/tmp".into(), roots: vec!["/".into()],
        argv: argv.split(' ').map(String::from).collect(), env: vec![] };
    let (url, wasm) = (Load::Url("bin/toolbox.wasm".into()), Load::Bytes(b"\0asm".into()));
    assert_eq!(starts[0], (want(3, "a 1", Stdin::Pipe, Stdout::Pipe), url));
    assert_eq!(starts[1], (want(4, "b", Stdin::Pipe, Stdout::Console), wasm));
    // The first reads the job's bytes, then an end of file; the second waits for the first's.
    let reads = [3, 3, 4].map(|pid| s.send(pid, Msg::PipeRead { max: 64 }));
    assert_eq!(reads, [vec![reply(3, b"in")], vec![reply(3, b"")], vec![]]);
    assert_eq!(s.send(3, Msg::PipeWrite { data: b"out" }), [reply(4, b"out"), reply(3, b"")]);
    // Both write to the console; the shell's WAIT waits for both.
    s.send(3, Msg::ConsWrite { data: b"a:" });
    s.send(4, Msg::ConsWrite { data: b"b!" });
    assert_eq!((s.k.take_output(sh), s.k.take_woken()), (b"a:b!".to_vec(), vec![5]));
    assert!(s.send(sh, Msg::Wait { job: 3 }).is_empty());
    assert!(s.send(4, Msg::PipeRead { max: 9 }).is_empty());
    // The writer's end is its reader's end of file; the last's status is the job's, and both go.
    assert_eq!(s.send(3, Msg::Exit { status: 9 }), [Effect::Kill { pid: 3 }, reply(4, b"")]);
    let end = [Effect::Kill { pid: 4 }, reply(sh, &2i32.to_le_bytes())];
    assert_eq!(s.send(4, Msg::Exit { status: 2 }), end);
    assert_eq!(s.k.procs(), [(sh, "sh".into(), true)]);
    assert_eq!(s.send(sh, Msg::Wait { job: 3 }), [err(sh, wire::ECHILD)]);
}

#[test]
fn pipes_hold_back_a_writer_ahead_of_its_reader_and_end_with_either() {
    let mut s = Sh::new();
    let ab = job(&[("/bin/a", "a"), ("/bin/b", "b")], Stdin::Console, Stdout::Null, b"");
    s.spawn(&ab);
    let starts = s.ready(&[3, 4]);
    assert_eq!((&starts[0].0.stdin, &starts[1].0.stdout), (&Stdin::Console, &Stdout::Null));
    // Past MAX_PIPE bytes unread a write waits, until a read leaves no more than that.
    let big = vec![7; wire::MAX_PIPE];
    assert_eq!(s.send(3, Msg::PipeWrite { data: &big }), [reply(3, b"")]);
    assert!(s.send(3, Msg::PipeWrite { data: b"xy" }).is_empty());
    assert_eq!(s.send(4, Msg::PipeRead { max: 1 }), [reply(4, &[7])], "still past");
    assert_eq!(s.send(4, Msg::PipeRead { max: 1 }), [reply(3, b""), reply(4, &[7])]);
    // A reader that ends leaves its writer no one: EPIPE, waiting and then.
    assert!(s.send(3, Msg::PipeWrite { data: &big }).is_empty());
    assert_eq!(s.send(4, Msg::Exit { status: 0 }), [Effect::Kill { pid: 4 }, err(3, wire::EPIPE)]);
    assert_eq!(s.send(3, Msg::PipeWrite { data: b"z" }), [err(3, wire::EPIPE)]);
    // A process with no pipe reads an end of file.
    assert_eq!(s.send(s.sh, Msg::PipeRead { max: 9 }), [reply(s.sh, b"")]);
}

#[test]
fn ctrl_c_ends_the_job_on_the_console_and_a_mode_it_set() {
    let mut s = Sh::new();
    let sh = s.sh;
    s.spawn(&job(&[("/bin/a", "a"), ("/bin/a", "a")], Stdin::Console, Stdout::Console, b""));
    s.ready(&[3, 4]);
    // A stage reads the console in the mode it set, which goes with it.
    s.send(3, Msg::ConsMode { bits: wire::MODE_RAW });
    s.send(sh, Msg::Wait { job: 3 });
    assert!(s.send(4, Msg::ConsRead { max: 9 }).is_empty() && s.k.idle(sh));
    s.k.input(sh, b"k");
    assert_eq!(s.k.take_effects(), [reply(4, b"k")]);
    s.send(3, Msg::Exit { status: 0 });
    assert_eq!((s.k.mode(sh).raw, s.k.mode(sh).echo), (false, true));
    // Ctrl+C ends what runs on the console, not the shell holding it.
    s.k.input(sh, b"x\x03");
    let i = 130i32.to_le_bytes();
    assert_eq!(s.k.take_effects(), [Effect::Kill { pid: 4 }, reply(sh, &i)]);
    assert_eq!(s.k.take_output(sh), b"x^C");
    assert_eq!(s.k.procs(), [(sh, "sh".into(), true)]);
}

#[test]
fn a_job_starts_whole_or_not_at_all() {
    let mut s = Sh::new();
    s.fs.write("/tmp/text", b"#!/bin/sh").unwrap();
    let one = |path| job(&[("/bin/a", "a"), (path, "x")], Stdin::Console, Stdout::Console, b"");
    let mut fails: Vec<_> = ["/bin/nope", "/tmp/text", "/tmp", "bin/a", "/bin/../bin/a"]
        .map(|p| s.spawn(&one(p)).0)
        .into();
    fails.push(s.spawn(&job(&[], Stdin::Console, Stdout::Console, b"")).0);
    fails.push(s.spawn(&Job { cwd: "tmp", ..one("/bin/a") }).0);
    // A job that fits in a SPAWN, but not its Start.
    let long = ["x"; 1].map(|x| x.repeat(65_510));
    fails.push(s.spawn(&Job { stages: vec![("/bin/a", long.to_vec())], ..one("/bin/a") }).0);
    assert!(s.send(s.sh, Msg::Spawn { job: b"\0" }).iter().any(|e| *e == err(s.sh, wire::EINVAL)));
    use wire::{E2BIG, EINVAL, ENOENT, ENOEXEC};
    assert_eq!(fails, [ENOENT, ENOEXEC, ENOENT, EINVAL, EINVAL, EINVAL, EINVAL, E2BIG]);
    assert_eq!((s.k.procs().len(), s.k.take_effects()), (1, vec![]));
    // Eight run at most: the shell and six, then two more are one too many.
    let seven: Vec<_> = vec![("/bin/a", "a"); 7];
    assert_eq!(s.spawn(&job(&seven[..6], Stdin::Console, Stdout::Console, b"")).0, 0);
    assert_eq!(s.spawn(&job(&seven[..2], Stdin::Console, Stdout::Console, b"")).0, wire::EAGAIN);
    // Under narrower roots, only what is under them.
    let mut s = Sh::new();
    let (argv, program) = (vec!["sh".into()], Program::Url("bin/toolbox.wasm".into()));
    let (cwd, stdout, roots) = ("/tmp".into(), Stdout::Console, vec!["/tmp".into()]);
    let sh = s.k.spawn(Spawn { argv, program, cwd, tty: None, stdout, roots }).unwrap();
    s.k.message(&mut s.fs, sh, &[wire::READY, wire::VERSION]);
    s.k.take_effects();
    let ask = |s: &mut Sh, path| {
        let one = job(&[(path, "w")], Stdin::Console, Stdout::Console, b"").encode();
        s.send(sh, Msg::Spawn { job: &one })
    };
    assert_eq!(ask(&mut s, "/bin/a"), [err(sh, wire::ENOTCAPABLE)]);
    let started = ask(&mut s, "/tmp/w.wasm");
    assert_eq!(started.last(), Some(&reply(sh, &(sh + 1).to_le_bytes())));
    assert_eq!(s.ready(&[sh + 1])[0].0.tty, None, "no console to give");
}

#[test]
fn programs_are_wasm_or_markers_naming_wasm() {
    let mut fs = Vfs::new();
    #[rustfmt::skip]
    let files = [("/a", "#!wasm bin/toolbox.wasm\n"), ("/b.wasm", "\0asm"), ("/c", "#!wasm  /tmp/c 12ab\n"),
        ("/d", "#!wasm x/../y"), ("/e", "#!wasm https://x"), ("/f", "#!wasm\tf"), ("/g", "#!wasm \u{e9}")];
    for (path, data) in files {
        fs.write(path, data.as_bytes()).unwrap();
    }
    let vfs = |p: &str| Ok(Program::Vfs(p.into()));
    #[rustfmt::skip]
    let want = [Ok(Program::Url("bin/toolbox.wasm".into())), vfs("/b.wasm"), vfs("/tmp/c"),
        Err(false), Err(false), Err(false), Err(false), Err(true), Err(true)];
    for (p, want) in
        ["/a", "/b.wasm", "/c", "/d", "/e", "/f", "/g", "/h", "/"].into_iter().zip(want)
    {
        assert_eq!(program(&fs, p), want, "{p}");
    }
}
