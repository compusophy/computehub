use crate::wire::{self, Msg};
use crate::{Effect, Kernel, Mode, Program, Spawn};
use vfs::Vfs;

/// An isolated kernel for window 5 running `sh` with a tty past its READY, and its Vfs.
fn sys() -> (Kernel, Vfs, u32) {
    let (mut k, mut fs) = (Kernel { isolated: true, owner: 5, ..Kernel::default() }, Vfs::new());
    let pid = k.spawn(spawn(Some((80, 24)))).unwrap();
    k.message(&mut fs, pid, &[wire::READY, wire::VERSION]);
    let _ = (k.take_effects(), k.take_woken());
    (k, fs, pid)
}
fn spawn(tty: Option<(u16, u16)>) -> Spawn {
    let (argv, program) = (vec!["sh".into()], Program::Url("bin/toolbox.wasm".into()));
    let (cwd, stdout, roots) = ("/tmp".into(), wire::Stdout::Console, vec!["/".into()]);
    Spawn { argv, program, cwd, tty, stdout, roots }
}
/// `m` from `pid`: the effects.
fn send(k: &mut Kernel, fs: &mut Vfs, pid: u32, m: Msg<'_>) -> Vec<Effect> {
    k.message(fs, pid, &m.encode());
    k.take_effects()
}
fn read(max: u32) -> Msg<'static> {
    Msg::ConsRead { max }
}
fn reply(pid: u32, data: &[u8]) -> Vec<Effect> {
    vec![Effect::Reply { pid, errno: 0, data: data.to_vec() }]
}
fn mode(bits: u8) -> Msg<'static> {
    Msg::ConsMode { bits }
}

#[test]
fn cooked_keys_become_lines_with_editing_echo_and_end_of_file() {
    let (mut k, mut fs, pid) = sys();
    // A read waits for a line and wakes the owner: the console waits for keys.
    assert!(send(&mut k, &mut fs, pid, read(64)).is_empty());
    assert_eq!(
        (k.take_woken(), k.idle(pid), k.mode(pid)),
        (vec![5], true, Mode { raw: false, echo: true })
    );
    // Backspace takes a whole char (é is two bytes); controls echo as ^X; a line is still typed.
    k.input(pid, b"ab\x7fc\xc3\xa9\x7f!\x1b[A");
    assert!(k.take_effects().is_empty() && k.idle(pid));
    assert_eq!(k.take_output(pid), b"ab\x08 \x08c\xc3\xa9\x08 \x08!^[[A");
    // Enter (CR is LF) ends it: the read gets the line, at most `max` bytes at a time.
    k.input(pid, b"\r");
    assert_eq!((k.take_effects(), k.idle(pid)), (reply(pid, b"ac!\x1b[A\n"), false));
    assert_eq!(k.take_output(pid), b"\n");
    k.input(pid, b"abcdef\n");
    let reads = [3, 9].map(|max| send(&mut k, &mut fs, pid, read(max)));
    assert_eq!(reads, [reply(pid, b"abc"), reply(pid, b"def\n")]);
    // Ctrl+D sends the line so far; on an empty line it is an end of file, read once after the
    // input before it.
    k.input(pid, b"x\x04");
    assert_eq!(send(&mut k, &mut fs, pid, read(9)), reply(pid, b"x"));
    k.input(pid, b"y\r\x04");
    let reads = [0; 3].map(|_| send(&mut k, &mut fs, pid, read(9)));
    assert_eq!(reads, [reply(pid, b"y\n"), reply(pid, b""), vec![]]);
    // Ctrl+C drops the line and what was queued, and ends nothing it holds.
    k.input(pid, b"z\rgone\x03");
    assert_eq!((k.take_effects(), k.procs()), (vec![], vec![(pid, "sh".into(), true)]));
    assert_eq!(k.take_output(pid), b"abcdef\nxy\nz\ngone^C");
    // Without echo, lines are still cooked.
    assert_eq!(send(&mut k, &mut fs, pid, mode(wire::MODE_NOECHO)), reply(pid, b""));
    k.input(pid, b"pw\x7fW\r");
    assert_eq!((k.take_effects(), k.take_output(pid)), (reply(pid, b"pW\n"), vec![]));
}

#[test]
fn raw_keys_pass_through_and_modes_need_a_console() {
    let (mut k, mut fs, pid) = sys();
    // Raw takes the line typed so far as it is, then every byte: no echo, nothing special.
    k.input(pid, b"ls -");
    assert_eq!(
        send(&mut k, &mut fs, pid, mode(wire::MODE_RAW | wire::MODE_NOECHO)),
        reply(pid, b"")
    );
    assert_eq!(k.mode(pid), Mode { raw: true, echo: false });
    k.input(pid, b"\x7f\x03\x04\r\x1b[A");
    assert_eq!(send(&mut k, &mut fs, pid, read(64)), reply(pid, b"ls -\x7f\x03\x04\r\x1b[A"));
    assert_eq!((k.take_output(pid), k.procs().len()), (b"ls -".to_vec(), 1));
    // A read waiting gets the next keys at once.
    assert!(send(&mut k, &mut fs, pid, read(64)).is_empty());
    k.input(pid, b"q");
    assert_eq!(k.take_effects(), reply(pid, b"q"));
    let bad = Effect::Reply { pid, errno: wire::EINVAL, data: vec![] };
    assert_eq!(send(&mut k, &mut fs, pid, mode(4)), [bad]);
    // A process with no console reads an end of file at once and takes no mode; a resize
    // reaches each process running on the console.
    let gui = k.spawn(spawn(None)).unwrap();
    k.message(&mut fs, gui, &[wire::READY, wire::VERSION]);
    k.take_effects();
    assert_eq!(send(&mut k, &mut fs, gui, read(64)), reply(gui, b""));
    let notty = Effect::Reply { pid: gui, errno: wire::ENOTTY, data: vec![] };
    assert_eq!(send(&mut k, &mut fs, gui, mode(0)), [notty]);
    k.input(gui, b"x");
    let _ = (k.resize(gui, 9, 9), k.resize(pid, 100, 30));
    let word = |index, value| Effect::Word { pid, index, value };
    assert_eq!(k.take_effects(), [word(wire::COLS, 100), word(wire::ROWS, 30)]);
    assert!(!k.idle(gui) && k.take_output(gui).is_empty());
}
