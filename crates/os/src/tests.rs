use super::*;
pub(crate) use kernel::wire::{Msg, O_CREAT};
use platform::Effect as Fx;

const SEMI: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

/// A key event from `code/key/mods` (key `Unidentified` and no mods if left
/// out), the mods as `s c a m g`: Shift Ctrl Alt Meta AltGraph.
fn key(e: &str, down: bool) -> Event {
    let mut f = e.split('/');
    let (code, key) = (f.next().unwrap_or("").into(), f.next().unwrap_or("Unidentified").into());
    let mods = f.next().unwrap_or("");
    let [shift, ctrl, alt, meta, altgr] = ['s', 'c', 'a', 'm', 'g'].map(|m| mods.contains(m));
    Event::Key { code, key, down, repeat: false, shift, ctrl, alt, meta, altgr }
}

fn resize(w: f32, h: f32) -> Event {
    Event::Resize { w, h, dpr: 2.0 }
}

fn fetched(id: u32, result: Result<Vec<u8>, String>) -> Event {
    Event::Fetched { id, result }
}

/// What `desk` answers to `ev`, as (redraw, prevent_default), and asks of the page.
fn send(desk: &mut Desktop, ev: Event) -> ((bool, bool), Vec<Fx>) {
    let mut ctl = Ctl::default();
    let h = desk.event(ev, &mut ctl);
    ((h.redraw, h.prevent_default), ctl.effects().to_vec())
}

fn prevented(desk: &mut Desktop, e: &str) -> bool {
    send(desk, key(e, true)).0.1
}

fn fresh() -> Desktop {
    Desktop::new().expect("the boot font loads")
}

/// An isolated 1280 x 800 desktop (a reload), and a terminal if asked: program 2, its shell 3.
pub(crate) fn desktop(terminal: bool) -> Desktop {
    let (mut desk, mut ctl) = (fresh(), Ctl::default());
    ctl.session_set(logon::SESSION, Some("0"));
    desk.event(resize(1280.0, 800.0), &mut ctl);
    desk.shell.as_mut().expect("made").kernel_mut().set_isolated(true);
    if terminal {
        send(&mut desk, key("Enter/Enter/a", true));
        screen(&mut desk, vec![uiwire::Request::Tty { cols: 80, rows: 24 }]);
    }
    desk
}

/// What the Terminal's program (pid 2) asks of the page with a frame of its screen.
pub(crate) fn screen(desk: &mut Desktop, requests: Vec<uiwire::Request>) -> Vec<Fx> {
    let screen = uiwire::Node::Screen { id: 1, cols: 1, rows: 1, cursor: None, cells: vec![0; 13] };
    let f = uiwire::Frame { requests, nodes: vec![screen], ..Default::default() }.encode();
    send(desk, Event::Proc { pid: 2, msg: [&[kernel::wire::DRAW][..], &f].concat() }).1
}

/// What `desk` asks of the page, but replies, as the terminal's shell sends `msg`.
pub(crate) fn sh(desk: &mut Desktop, msg: Msg<'_>) -> Vec<Fx> {
    let fx = send(desk, Event::Proc { pid: 3, msg: msg.encode() }).1;
    fx.into_iter().filter(|f| !matches!(f, Fx::Reply { .. })).collect()
}

/// A frame as [`App::frame`] draws it, without a renderer, and what it asked.
fn frame(desk: &mut Desktop) -> Vec<Fx> {
    let mut ctl = Ctl::default();
    desk.paint(1.0, &ctl);
    desk.drawn(&mut ctl);
    ctl.effects().to_vec()
}

fn shell(desk: &Desktop) -> &Shell {
    desk.shell.as_ref().expect("created")
}

/// A bare spot in the focused window's content, just under its titlebar.
fn focused_middle(desk: &Desktop) -> (f32, f32) {
    let layout = shell(desk).wm().layout();
    let r = layout.iter().find(|p| p.focused).expect("focused").rect;
    ((r.x + r.w / 2) as f32, (r.y + wm::TITLE_H + 6) as f32)
}

#[test]
fn events_map_to_inputs_and_keys_by_code_or_by_meaning() {
    // By code, or with no code (phone keyboards, remote desktops) by key; NumLock-off keypad keys
    // by the key they name; with Ctrl, Alt or Meta an ASCII letter by meaning (AZERTY's Z at KeyW,
    // Dvorak's C at KeyI), anything else (Cyrillic, Option symbols, digits, AltGr text) by place.
    let table = "KeyA=Char('a') KeyZ=Char('z') Digit0=Char('0') Digit9=Char('9') Numpad7=Char('7') \
        Enter=Enter NumpadEnter=Enter Escape=Escape Backspace=Backspace Delete=Delete Tab=Tab \
        Space=Space ArrowLeft=Left ArrowRight=Right ArrowUp=Up ArrowDown=Down Home=Home End=End \
        PageUp=PageUp PageDown=PageDown Insert=Insert F1=F(1) F12=F(12) Backquote=Char('`') \
        AltLeft=Other ShiftLeft=Other Minus=Other keya=Other Key=Other =Other /Enter=Enter \
        /Backspace=Backspace Unidentified/ArrowLeft=Left /F5=F(5) /c=Char('c') /C=Char('c') \
        /7=Char('7') /é=Other /!=Other /Unidentified=Other /Process=Other /=Other KeyQ/a=Char('q') \
        IntlBackslash/<=Other Numpad7/7=Char('7') Numpad8/ArrowUp=Up Numpad7/Home=Home \
        Numpad0/Insert=Insert NumpadDecimal/Delete=Delete Numpad5/Clear=Char('5') \
        NumpadDecimal/.=Other NumpadEnter/Enter=Enter Backquote/²=Char('`') \
        Backquote/Dead=Char('`') /`=Char('`') /~=Other KeyW/z/c=Char('z') KeyQ/a/c=Char('a') \
        KeyA/q/a=Char('q') KeyI/c/c=Char('c') KeyV/K/cs=Char('k') Period/v/m=Char('v') \
        KeyC/\u{441}/c=Char('c') KeyQ/\u{153}/a=Char('q') Digit1/&/a=Char('1') KeyW/z/=Char('w') \
        KeyW/z/s=Char('w') KeyQ/@/cag=Char('q') Backquote/\u{b2}/a=Char('`')";
    for entry in table.split(' ') {
        let (ev, want) = entry.split_once('=').expect("ev=want");
        let Some(Input::Key { key: k, .. }) = input_of(key(ev, true)) else { panic!("{entry}") };
        assert_eq!(format!("{k:?}"), want, "{entry}");
    }
    assert_eq!(key_of("", " ", false), Key::Space);
    // Modifiers as held=seen: Chrome and Edge on Windows send AltGr as "cag",
    // which is no Ctrl+Alt; left Ctrl+Alt and AltGraph without both stay.
    for entry in "sm=sm cag= scamg=sm ca=ca sca=sca ag=a cg=c g= =".split(' ') {
        let (held, seen) = entry.split_once('=').expect("held=seen");
        let [shift, ctrl, alt, meta] = ['s', 'c', 'a', 'm'].map(|c| seen.contains(c));
        let want = Input::Key { key: Key::Char('q'), mods: Mods { shift, ctrl, alt, meta } };
        assert_eq!(input_of(key(&["KeyQ/q/", held].concat(), true)), Some(want), "{held}");
    }
    assert_eq!(input_of(key("KeyQ/q/a", false)), None);
    assert_eq!(input_of(fetched(1, Ok(vec![1]))), None);
    let (x, y, button, dy, touch) = (3.0, 4.0, 2, -48.0, true);
    let t = platform::LocalTime { year: 2026, month: 9, day: 30, weekday: 3, hour: 14, minute: 7 };
    let shell_time = LocalTime { year: 2026, month: 9, day: 30, weekday: 3, hour: 14, minute: 7 };
    #[rustfmt::skip]
    let pairs = [(Event::Text("é".into()), Input::Text("é".into())),
        (Event::PointerMove { x, y }, Input::PointerMove { x, y }), (Event::PointerLeave, Input::PointerLeave),
        (Event::PointerDown { x, y, button, touch }, Input::PointerDown { x, y, button, touch }),
        (Event::PointerUp { x, y, button }, Input::PointerUp { x, y, button }),
        (Event::Wheel { x, y, dy }, Input::Wheel { x, y, dy }), (resize(8.0, 6.0), Input::Resize { w: 8.0, h: 6.0 }),
        (Event::Tick { time: t }, Input::Tick { time: shell_time })];
    pairs.into_iter().for_each(|(ev, want)| assert_eq!(input_of(ev.clone()), Some(want), "{ev:?}"));
}

#[test]
fn typed_keys_are_left_to_the_textarea_and_key_ups_prevent_only_modifiers() {
    // Text, dead keys, IME and phone keys type; AltGr is text.
    let typing = "KeyA/a/ KeyA/A/s Digit1/!/s KeyE/é/ KeyK/क्ष/ KeyQ/@/cag Quote/Dead/ \
        KeyN/Process/ /Unidentified/";
    for down in [true, false] {
        for e in typing.split(' ') {
            assert_eq!(types_text(&key(e, down)), down, "{e} {down}");
        }
        assert_eq!(types_text(&key("Space/ ", down)), down);
    }
    // Named keys, shortcuts, and keys that report nothing.
    let not = "Enter/Enter/ Tab/Tab/ Backspace/Backspace/ ArrowLeft/ArrowLeft/ F5/F5/ \
        ShiftLeft/Shift/s KeyC/c/c KeyB/b/a KeyV/v/m KeyQ/q/ca KeyA// Enter/\r/";
    not.split(' ').for_each(|e| assert!(!types_text(&key(e, true)), "{e:?}"));
    assert!(!types_text(&Event::Text("a".into())));
    // Key-ups draw nothing; only releasing Alt or Meta is prevented.
    let mut desk = desktop(false);
    let table = "AltLeft=1 AltRight=1 MetaLeft=1 MetaRight=1 KeyQ=0 Enter=0 ControlLeft=0 \
        ShiftLeft=0 OSLeft=0 Digit1=0";
    for (code, prevent) in table.split(' ').filter_map(|e| e.split_once('=')) {
        let (prevent, up) = (prevent == "1", key(&[code, "/q/a"].concat(), false));
        assert_eq!(key_up(code), Handled { redraw: false, prevent_default: prevent }, "{code}");
        assert_eq!(send(&mut desk, up), ((false, prevent), vec![]));
    }
    // Without text input the shell's answer stands; with it (in a terminal),
    // typed keys go unprevented, the rest still prevented: Ctrl+C with no code
    // reaches the terminal; Dvorak's Ctrl+K (at KeyV) is no paste, Ctrl+V is.
    assert!(prevented(&mut desk, "KeyA/a"));
    let (_, fx) = send(&mut desk, key("Enter/Enter/a", true));
    assert!(fx.contains(&Fx::TextInput(true)), "{fx:?}");
    assert!(["KeyA/a", "KeyQ/@/cag", "KeyV/v/c"].iter().all(|e| !prevented(&mut desk, e)));
    let shortcuts = ["Tab/Tab", "Enter/Enter", "KeyC/c/c", "/c/c", "KeyV/k/c"];
    shortcuts.iter().for_each(|e| assert!(prevented(&mut desk, e), "{e}"));
}

#[test]
fn the_welcome_then_the_desktop_start_at_the_first_usable_sizes() {
    let mut desk = fresh();
    let (time, short) = (platform::LocalTime::EPOCH, shell::BAR_H + shell::DOCK_CLEAR + 0.5);
    let early = [key("Enter/Enter/a", true), Event::PointerLeave, Event::Tick { time }];
    let small = [(0.0, 0.0), (0.5, 600.0), (f32::NAN, 600.0)];
    for ev in early.into_iter().chain(small.map(|(w, h)| resize(w, h))) {
        assert_eq!(send(&mut desk, ev.clone()), ((false, false), vec![]), "{ev:?}");
        assert!(desk.shell.is_none() && desk.logon.is_none(), "{ev:?}");
    }
    assert!(desk.missed_tick);
    assert_eq!(desk.paint(1.0, &Ctl::default()), ui::theme("").base);
    // A new tab's welcome shows at any size; Enter signs in to guest: no Welcome window opens.
    assert_eq!(send(&mut desk, resize(800.0, short)), ((true, false), vec![]));
    send(&mut desk, resize(1280.0, 800.0));
    let store = |k: &str, v: &str| Fx::Store { key: k.into(), value: v.into() };
    let s = Fx::Session { key: logon::SESSION.into(), value: Some("0".into()) };
    let fx = [store(logon::SEEN, "1"), store("compusophy.last", "0"), s, Fx::TextInput(false)];
    assert_eq!(send(&mut desk, key("Enter/Enter", true)), ((true, true), fx.to_vec()));
    assert!(desk.logon.as_ref().is_some_and(Logon::leaving));
    assert!(shell(&desk).wm().layout().is_empty() && desk.parts.is_none() && desk.saved == "Mono");
    assert_eq!(shell(&desk).vfs().read(remote::STUDIO), Ok(&b"#!wasm bin/studio.wasm\n"[..]));
}

#[test]
fn the_theme_comes_from_storage_and_goes_back_when_it_changes() {
    let stored = |name: &str| {
        let (mut ctl, mut desk) = (Ctl::default(), fresh());
        ctl.storage_set(THEME_KEY, name);
        ctl.session_set(logon::SESSION, Some("0"));
        desk.event(resize(1280.0, 800.0), &mut ctl);
        (desk, ctl.effects().iter().filter(|f| matches!(f, Fx::Store { .. })).count())
    };
    // Names match in any case, an unknown one is the default; none is written back (the
    // other write is the first visit's mark).
    let (mut desk, writes) = stored("dawn");
    assert_eq!((shell(&desk).theme_name(), writes), ("Dawn", 2));
    assert_eq!(shell(&stored("Solarized").0).theme_name(), "Mono");
    // A new theme (Settings sets it) is stored once, after the event that set it.
    let store = Fx::Store { key: THEME_KEY.into(), value: "Mono".into() };
    assert!(desk.shell.as_mut().expect("created").set_theme("mono"));
    let fx = send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 }).1;
    let stores = fx.iter().filter(|f| **f == store).count();
    assert_eq!((shell(&desk).theme_name(), stores), ("Mono", 1));
    for fx in [frame(&mut desk), send(&mut desk, Event::PointerMove { x: 9.0, y: 200.0 }).1] {
        assert!(fx.iter().all(|f| !matches!(f, Fx::Store { .. })), "{fx:?}");
    }
}

#[test]
fn fonts_load_in_groups_and_frames_come_only_while_something_moves() {
    let (bold, mono) = (u32::MAX - 1, u32::MAX);
    let has =
        |d: &mut Desktop, id| text_of(&mut d.shell, &mut d.parts).is_some_and(|t| t.has_font(id));
    let mut desk = fresh();
    assert!(!has(&mut desk, FontId::SansBold) && !has(&mut desk, FontId::Mono));
    // Before the deferred fonts are asked for, their ids are nobody's.
    assert_eq!(send(&mut desk, fetched(mono, Ok(MONO.to_vec()))), ((false, false), vec![]));
    assert!(!has(&mut desk, FontId::Mono));
    // The first frame asks for both, even before the shell exists (asking
    // for no next frame: nothing moves yet); later frames ask for nothing.
    let fetch = |id, f: &str| Fx::Fetch { id, url: ["fonts/deferred/", f].concat() };
    let fetches = [fetch(bold, "Inter-SemiBold.ttf"), fetch(mono, "JetBrainsMono-Regular.ttf")];
    assert_eq!(frame(&mut desk), fetches);
    assert_eq!(frame(&mut desk), []);
    // A font that arrives before the shell exists is the shell's later.
    assert_eq!(send(&mut desk, fetched(mono, Ok(MONO.to_vec()))), ((true, false), vec![]));
    send(&mut desk, resize(1280.0, 800.0));
    assert!(has(&mut desk, FontId::Mono) && !has(&mut desk, FontId::SansBold));
    // A failed fetch leaves the slot empty, quietly; then its id is the shell's, which knows none.
    let quiet = [Err("HTTP 404".into()), Ok(SEMI.to_vec())];
    quiet.into_iter().for_each(|got| assert_eq!(send(&mut desk, fetched(bold, got)).1, vec![]));
    assert!(!has(&mut desk, FontId::SansBold));
    // With the shell up: a bad font is skipped, a good one fills the slot. The
    // welcome window fades in and the page clock reads 0 here, so the fade
    // never ends: every frame asks for the next, and any event redraws.
    for (got, want) in [(vec![1, 2, 3], false), (SEMI.to_vec(), true)] {
        let mut desk = desktop(false);
        assert_eq!(frame(&mut desk), [&[Fx::RequestFrame][..], &fetches].concat());
        assert_eq!(frame(&mut desk), [Fx::RequestFrame]);
        assert!(send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 }).0.0);
        let mut ctl = Ctl::default();
        desk.shell.as_mut().expect("made").set_now(1000.0);
        desk.drawn(&mut ctl);
        assert_eq!(ctl.effects(), [Fx::FrameIn(125)]); // faded in, the living grain's, by timer
        let (h, fx) = send(&mut desk, fetched(bold, Ok(got)));
        assert_eq!((h.1, fx, has(&mut desk, FontId::SansBold)), (false, vec![], want));
    }
    // The lazy fonts: the first terminal's console asks for them, under other ids.
    assert_eq!(send(&mut fresh(), fetched(1, Ok(vec![]))), ((false, false), vec![]));
    let mut desk = desktop(false);
    send(&mut desk, key("Enter/Enter/a", true));
    let fx = screen(&mut desk, vec![uiwire::Request::Tty { cols: 80, rows: 24 }]);
    let [Fx::Fetch { id: a, url: ref ua }, Fx::Fetch { id: b, url: ref ub }, ..] = fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!([ua, ub], ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"]);
    let deferred = frame(&mut desk).into_iter().filter(|f| matches!(f, Fx::Fetch { .. }));
    assert!(deferred.map(|f| f != fx[0] && f != fx[1]).eq([true, true]));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/lazy/");
    let font = std::fs::read(format!("{dir}symbols-a.ttf")).expect("lazy font");
    // The second failed first: nothing is added until the first arrives.
    let fallbacks = |d: &mut Desktop| d.shell.as_mut().expect("made").text_mut().fallback_count();
    assert_eq!(send(&mut desk, fetched(b, Err("HTTP 404".into()))).1, vec![]);
    assert_eq!(fallbacks(&mut desk), 0);
    assert_eq!(send(&mut desk, fetched(a, Ok(font))), ((true, false), vec![]));
    assert_eq!(fallbacks(&mut desk), 1);
}

#[test]
fn desktop_routes_events_through_the_shell() {
    use shell::Cursor::*;
    let all = [Default, Text, Grab, Grabbing, EwResize, NsResize, NwseResize, NeswResize];
    let css = "default text grab grabbing ew-resize ns-resize nwse-resize nesw-resize";
    assert_eq!(all.map(|c| CURSORS[c as usize]).join(" "), css);
    // Over a titlebar the hand opens, once; over the bare desktop the arrow.
    let mut desk = desktop(false);
    let r = shell(&desk).wm().layout()[0].rect;
    let title = ((r.x + r.w / 2) as f32, (r.y + 12) as f32);
    let over = |d: &mut Desktop, (x, y)| send(d, Event::PointerMove { x, y }).1;
    assert_eq!(over(&mut desk, title), [Fx::Cursor("grab")]);
    assert_eq!(over(&mut desk, title), []);
    assert_eq!(over(&mut desk, (8.0, 300.0)), [Fx::Cursor("default")]);
    // A release on the focused window asks for the keyboard again, only
    // without text input (the welcome window wants none).
    let down =
        |d: &mut Desktop, (x, y)| send(d, Event::PointerDown { x, y, button: 0, touch: false });
    let up = |d: &mut Desktop, (x, y), button| send(d, Event::PointerUp { x, y, button }).1;
    let at = focused_middle(&desk);
    down(&mut desk, at);
    assert_eq!(up(&mut desk, at, 0), []);
    let state = |d: &Desktop| shell(d).wm().state_hash();
    let count = |d: &Desktop| shell(d).wm().layout().len();
    assert_eq!(send(&mut desk, key("Enter/Enter/a", true)).0, (true, true));
    let opened = state(&desk);
    assert_eq!(count(&desk), 2);
    // Key-ups never reach the shell: releasing Alt+Q closes nothing. AltGr+Up
    // and AltGr+Backquote are no bindings: AltGr is not Alt.
    assert_eq!(send(&mut desk, key("KeyQ/q/a", false)).0, (false, false));
    assert_eq!(state(&desk), opened);
    for ev in ["ArrowUp/ArrowUp/cag", "Backquote/`/cag"] {
        send(&mut desk, key(ev, true));
        assert_eq!(state(&desk), opened, "{ev}");
    }
    // A terminal typing (no screen yet): on the focused window, button 0 only; not off it.
    let at = focused_middle(&desk);
    assert_eq!(down(&mut desk, at), ((true, true), vec![Fx::Cursor("default")]));
    assert_eq!(up(&mut desk, at, 0), [Fx::TextInput(true)]);
    assert_eq!(up(&mut desk, at, 2), []);
    // A finger's scroll there is no tap: its lift asks for nothing.
    send(&mut desk, Event::PointerDown { x: at.0, y: at.1, button: 0, touch: true });
    send(&mut desk, Event::PointerMove { x: at.0, y: at.1 + 40.0 });
    assert_eq!(up(&mut desk, (at.0, at.1 + 40.0), 0), []);
    let bare = (4.0, 796.0);
    let r = |p: &wm::Placement| gfx::RectF::from_i32(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
    assert!(shell(&desk).wm().layout().iter().all(|p| !r(p).contains(bare.0, bare.1)));
    down(&mut desk, bare);
    assert_eq!(up(&mut desk, bare, 0), []);
    // Alt+Backquote moves focus to the other window, Alt+Shift+Backquote back; Alt+Up maximizes.
    let focus = |d: &Desktop| shell(d).wm().focused();
    let terminal = focus(&desk);
    assert!(prevented(&mut desk, "Backquote/`/a"));
    assert_ne!(focus(&desk), terminal);
    send(&mut desk, key("Backquote/~/as", true));
    assert_eq!(focus(&desk), terminal);
    assert_eq!(send(&mut desk, key("ArrowUp/ArrowUp/a", true)).0, (true, true));
    let top = shell(&desk).wm().layout().pop().expect("a window");
    assert_eq!((top.state, top.rect), (wm::State::Maximized, shell(&desk).wm().area()));
    // Alt+Q closes the focused window.
    assert!(prevented(&mut desk, "KeyQ/q/a"));
    assert_eq!(count(&desk), 1);
    // Pointer events are consumed; resizes redraw but are not.
    assert!(send(&mut desk, Event::PointerMove { x: 640.0, y: 400.0 }).0.1);
    assert_eq!(send(&mut desk, resize(1024.0, 768.0)).0, (true, false));
    assert!(!send(&mut desk, Event::PointerLeave).0.1);
}

#[test]
fn programs_reach_the_kernel_and_its_effects_the_page() {
    // /bin holds each applet's marker; kernel events before the shell are dropped.
    let mut desk = fresh();
    let vfs = &desk.parts.as_ref().expect("unused").1;
    assert_eq!(vfs.list("/bin").map(|l| l.len()), Ok(20));
    assert_eq!(vfs.read("/bin/sh").unwrap(), b"#!wasm bin/sh.wasm\n");
    assert_eq!(vfs.read("/bin/selftest").unwrap(), b"#!wasm bin/toolbox.wasm\n");
    assert_eq!(vfs.read("/bin/assistant").unwrap(), b"#!wasm bin/assistant.wasm\n");
    assert_eq!(vfs.read("/bin/settings").unwrap(), b"#!wasm bin/system.wasm\n");
    assert_eq!(send(&mut desk, Event::Hidden), ((false, false), vec![]));
    // Each kernel effect is its platform call.
    let mut ctl = Ctl::default();
    #[rustfmt::skip]
    let fx = [K::Spawn { pid: 2, sab: true }, K::Send { pid: 1, msg: vec![0x81] },
        K::Start { pid: 2, msg: vec![1], program: kernel::Load::Bytes(vec![0]) },
        K::Reply { pid: 2, errno: 44, data: vec![7] }, K::Word { pid: 2, index: 5, value: 1 },
        K::Kill { pid: 2 }, K::Wake { ms: 1000 }];
    fx.into_iter().for_each(|k| effect(Effect::Kernel(k), &mut ctl, &desk.ai));
    #[rustfmt::skip]
    let want = [Fx::Spawn { pid: 2, sab: true }, Fx::Send { pid: 1, msg: vec![0x81] },
        Fx::Start { pid: 2, msg: vec![1], program: platform::Load::Bytes(vec![0]) },
        Fx::Reply { pid: 2, errno: 44, data: vec![7] }, Fx::Word { pid: 2, index: 5, value: 1 },
        Fx::Kill(2), Fx::Wake(1000)];
    assert_eq!(ctl.effects(), want);
    // The AI model loads with the shell (one not on offer is the default), as do the theme,
    // the dock's favorites and the first visit's mark (after which no Welcome opens).
    ctl.storage_set(ai::MODEL, "openai/gpt-x");
    ctl.storage_set("compusophy.dock", "terminal");
    ctl.storage_set("compusophy.seen", "1");
    let want = shell::Prefs { dock: Some("terminal".into()), seen: true, ..Default::default() };
    assert_eq!(prefs(&ctl), want);
    ctl.session_set(logon::SESSION, Some("0"));
    desk.event(resize(1280.0, 800.0), &mut ctl);
    assert_eq!(desk.ai.status().model, ai::DEFAULT_MODEL);
    assert!(shell(&desk).wm().layout().is_empty());
    // Preferences are kept as compusophy.<key> (the AI model through the AI hub, the face in the
    // list); other keys are dropped, as are streams nobody asked for.
    let pref = |key: &str, value: &str| Effect::Pref { key: key.into(), value: value.into() };
    let mut ctl = Ctl::default();
    #[rustfmt::skip]
    let asked = [("ai.model", "zai/glm-5.3-flash"), ("ai.nope", "x"), ("dock", "studio,files"),
        ("seen", "1"), ("reports", "off"), ("theme", "x"), ("face", "3"), ("face", "10")];
    asked.into_iter().for_each(|(k, v)| effect(pref(k, v), &mut ctl, &desk.ai));
    let store = |k: &str, v: &str| Fx::Store { key: k.into(), value: v.into() };
    #[rustfmt::skip]
    let stored = [store(ai::MODEL, "zai/glm-5.3-flash"), store("compusophy.dock", "studio,files"),
        store("compusophy.seen", "1"), store("compusophy.reports", "off"),
        store(LIST, "CSPR 1 1\n0 00000003 - guest")];
    assert_eq!((ctl.effects(), desk.ai.status().model.as_str()), (&stored[..], ai::MODELS[1]));
    let end = Event::StreamEnd { id: 1, status: 0, error: "".into() };
    for ev in [Event::Chunk { id: 1, data: vec![1] }, end] {
        assert_eq!((send(&mut desk, ev.clone()), input_of(ev)), (((false, false), vec![]), None));
    }
}

#[test]
fn home_and_the_meters_keep_to_the_one_shot_timer() {
    // At once, then by the one-shot timer, armed again if it fires early (here every wake does:
    // the page clock reads 0); hiding keeps it at once. Once another tab kept its own: never.
    let mut desk = desktop(true);
    let touch =
        |d: &mut Desktop, f| sh(d, Msg::Open { oflags: O_CREAT, path: &[Vfs::HOME, f].concat() });
    assert_eq!([touch(&mut desk, "/a"), touch(&mut desk, "/b")], [vec![], vec![Fx::Wake(1001)]]);
    assert_eq!(send(&mut desk, Event::Wake).1, [Fx::Wake(1001)]);
    assert_eq!([send(&mut desk, Event::Hidden).1, send(&mut desk, Event::Wake).1], [[], []]);
    let mut ctl = Ctl::default();
    ctl.storage_set(home::MARK, "another tab's");
    touch(&mut desk, "/c");
    desk.event(Event::Hidden, &mut ctl);
    assert!(desk.home.unkept && touch(&mut desk, "/d").is_empty());
    // A watcher (Activity's process; the shell's here) hears the meters at once, in its read of
    // its events, and the timer is (already) armed for a look a second on; nothing more is due.
    (desk.ai.0.borrow_mut().watch, desk.ai.0.borrow_mut().fresh) = (Some(3), true);
    let read = |d: &mut Desktop| send(d, Event::Proc { pid: 3, msg: vec![0x21, 0, 0, 1, 0] }).1;
    let (fx, stats) = (read(&mut desk), |d: &Vec<u8>| d.get(5..).and_then(stat::Stats::decode));
    let s = fx.iter().find_map(|f| if let Fx::Reply { data, .. } = f { stats(data) } else { None });
    assert!(s.is_some_and(|s| s.loud.len() == stat::LOUD && s.quiet.len() == stat::QUIET));
    assert_eq!((desk.pace.wait(0), desk.wake), (Some(1000), 1001.0));
    assert!(read(&mut desk).iter().all(|f| !matches!(f, Fx::Reply { .. } | Fx::Wake(_))));
}
