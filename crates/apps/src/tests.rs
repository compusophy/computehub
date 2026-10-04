use super::*;
use ui::kernel::wire::{self, Msg};
use ui::kernel::{Effect, Kernel, Load};
use ui::{AppEvent, Cx, Request};

/// A console with the VFS and kernel its [`Cx`]s are made from (kept as the host keeps them).
struct Sim {
    console: Console,
    fs: Vfs,
    kernel: Kernel,
}

impl Sim {
    /// A console on a kernel (isolated if asked) whose /bin holds the shell.
    fn new(isolated: bool) -> Sim {
        let (mut fs, mut kernel) = (Vfs::new(), Kernel::new());
        kernel.set_isolated(isolated);
        fs.mkdir("/bin").and(fs.write(SHELL, b"#!wasm bin/sh.wasm\n")).unwrap();
        Sim { console: Console::default(), fs, kernel }
    }
    /// Runs `f` with a [`Cx`]; what it gives and the requests it made.
    fn cx<T>(&mut self, f: impl FnOnce(&mut Console, &mut Cx<'_>) -> T) -> (T, Vec<Request>) {
        let mut cx = Cx::new(&mut self.fs, &mut self.kernel, 0.0);
        let out = f(&mut self.console, &mut cx);
        (out, cx.take_requests())
    }
    /// The shell, started at 80 x 24, past its READY, its console set `raw`; its pid.
    fn shell(raw: bool) -> (Sim, u32) {
        let mut s = Sim::new(true);
        s.cx(|c, cx| c.start(cx, 80, 24));
        let pid = s.console.pid.expect("the shell runs");
        s.kernel.message(&mut s.fs, pid, &[wire::READY, wire::VERSION]);
        s.kernel.message(&mut s.fs, pid, &Msg::ConsMode { bits: u8::from(raw) }.encode());
        s.kernel.take_effects();
        (s, pid)
    }
    /// What the shell writes, as its worker sends it; what the Terminal then hears.
    fn wrote(&mut self, pid: u32, data: &[u8]) -> Vec<Event> {
        self.kernel.message(&mut self.fs, pid, &Msg::ConsWrite { data }.encode());
        self.cx(|c, cx| c.heard(cx)).0
    }
    /// What the shell's next read of its console gets.
    fn read(&mut self, pid: u32) -> Vec<u8> {
        self.kernel.message(&mut self.fs, pid, &Msg::ConsRead { max: 4096 }.encode());
        let reply = self.kernel.take_effects().into_iter().find_map(|e| match e {
            Effect::Reply { data, .. } => Some(data),
            _ => None,
        });
        reply.expect("input waiting")
    }
}

#[test]
fn the_shell_starts_once_on_a_console_the_size_asked_and_resizes_with_it() {
    let mut s = Sim::new(true);
    // The first ask starts it (and loads the symbol fonts), in the guest's home on the whole tree.
    let (told, asked) = s.cx(|c, cx| c.start(cx, 80, 24));
    assert!(told.is_empty() && asked == [Request::LoadFallbackFonts]);
    assert_eq!(s.kernel.procs(), [(2, "sh".into(), true)]);
    s.kernel.message(&mut s.fs, 2, &[wire::READY, wire::VERSION]);
    let start = s.kernel.take_effects().into_iter().find_map(|e| match e {
        Effect::Start { msg, program: Load::Url(url), .. } => {
            Some((wire::Start::decode(&msg), url))
        }
        _ => None,
    });
    let (start, url) = start.expect("a Start");
    let start = start.unwrap();
    assert_eq!(
        (url.as_str(), start.argv, start.tty),
        ("bin/sh.wasm", vec!["sh".into()], Some((80, 24)))
    );
    assert_eq!((start.cwd.as_str(), start.roots), (Vfs::HOME, vec!["/".into()]));
    // One shell, whatever sizes follow; a new size reaches its console.
    let (told, asked) = s.cx(|c, cx| c.start(cx, 70, 24));
    assert!(told.is_empty() && asked.is_empty() && s.kernel.procs().len() == 1);
    let words = s.kernel.take_effects();
    assert!(words.contains(&Effect::Word { pid: 2, index: wire::COLS, value: 70 }), "{words:?}");
    // A shell that cannot start says why, and ends (127); it is not tried again.
    let mut s = Sim::new(false);
    let why = b"sh: programs need a cross-origin isolated page (COOP/COEP headers)\r\n".to_vec();
    let failed = vec![Event::Output { data: why }, Event::Ended { status: 127 }];
    assert_eq!(s.cx(|c, cx| c.start(cx, 80, 24)).0, failed);
    assert!(s.cx(|c, cx| c.start(cx, 80, 24)).0.is_empty() && s.console.pid.is_none());
    let mut s = Sim::new(true);
    s.fs.remove(SHELL, false).unwrap();
    let missing = s.cx(|c, cx| c.start(cx, 80, 24)).0;
    assert_eq!(missing[0], Event::Output { data: b"sh: not found\r\n".to_vec() });
}

#[test]
fn the_terminal_hears_what_the_shell_wrote_and_its_end_and_types_into_it() {
    // Cooked, the kernel puts a CR before each LF; raw, output is as written.
    let (mut s, pid) = Sim::shell(false);
    assert_eq!(s.wrote(pid, b"one\ntwo"), [Event::Output { data: b"one\r\ntwo".to_vec() }]);
    let (mut s, pid) = Sim::shell(true);
    assert_eq!(s.wrote(pid, b"x\ny"), [Event::Output { data: b"x\ny".to_vec() }]);
    assert!(s.cx(|c, cx| c.heard(cx)).0.is_empty(), "nothing new");
    // What it types reaches the console; nothing reaches none.
    s.cx(|c, cx| c.input(cx, b"ls\r"));
    assert_eq!(s.read(pid), b"ls\r");
    // Its end, with what it wrote last; then nothing more.
    s.kernel.message(&mut s.fs, pid, &Msg::ConsWrite { data: b"bye" }.encode());
    s.kernel.message(&mut s.fs, pid, &Msg::Exit { status: 3 }.encode());
    let end = [Event::Output { data: b"bye".to_vec() }, Event::Ended { status: 3 }];
    assert_eq!(s.cx(|c, cx| c.heard(cx)).0, end);
    assert!(s.cx(|c, cx| c.heard(cx)).0.is_empty() && s.console.pid.is_none());
    s.cx(|c, cx| c.input(cx, b"gone"));
}

#[test]
fn the_window_keys_text_and_wheel_go_as_xterm_needs_them() {
    let (no, ctrl) = (Mods::default(), Mods { ctrl: true, ..Mods::default() });
    let key = |key, mods| input(&AppEvent::Key { key, mods });
    let ev = |key, mods: u8, ch| Some(Event::Key { id: 0, key, mods, ch });
    // Printable text comes as text, so a character key only with Ctrl or Alt.
    assert_eq!(key(Key::Char('x'), no), None);
    assert_eq!(key(Key::Space, no), None);
    assert_eq!(key(Key::Char('c'), ctrl), ev(uiwire::Key::Char, 2, 'c'));
    assert_eq!(key(Key::Up, no), ev(uiwire::Key::Up, 0, '\0'));
    assert_eq!(key(Key::F(5), no), ev(uiwire::Key::F, 0, '\u{5}'));
    let shift = Mods { shift: true, ..Mods::default() };
    assert_eq!(key(Key::Home, shift), ev(uiwire::Key::Home, 1, '\0'));
    assert_eq!(key(Key::Other, no), None);
    assert_eq!(input(&AppEvent::Text("é".into())), Some(Event::Text { text: "é".into() }));
    let wheel = |dy| input(&AppEvent::Wheel { x: 0.0, y: 0.0, dy });
    assert_eq!(
        [wheel(-51.0), wheel(f32::NAN)],
        [Some(Event::Wheel { dy: -51 }), Some(Event::Wheel { dy: 0 })]
    );
    assert_eq!(input(&AppEvent::Io), None);
    // Every key the wire has, and the modifiers' bits.
    let all = [Key::Enter, Key::Escape, Key::Tab, Key::Backspace, Key::Delete, Key::Insert];
    assert!(all.iter().all(|&k| wire_key(k).is_some_and(|w| w.0 as u8 > 0)));
    let every = Mods { shift: true, ctrl: true, alt: true, meta: true };
    assert_eq!(wire_mods(every), uiwire::mods::ALL);
}
