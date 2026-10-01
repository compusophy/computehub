use super::*;
use platform::Effect as Fx;

/// Outcomes as (redraw, prevent_default).
const NOTHING: (bool, bool) = (false, false);
const SEMI: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

/// A key event with `KeyboardEvent.code` and `.key`, holding the modifiers
/// named in `mods`: Shift Ctrl Alt Meta AltGraph as `s c a m g`.
fn key(code: &str, key: &str, down: bool, mods: &str) -> Event {
    let [shift, ctrl, alt, meta, altgr] = ['s', 'c', 'a', 'm', 'g'].map(|m| mods.contains(m));
    let (code, key, repeat) = (code.into(), key.into(), false);
    Event::Key { code, key, down, repeat, shift, ctrl, alt, meta, altgr }
}

fn resize(w: f32, h: f32) -> Event {
    Event::Resize { w, h, dpr: 2.0 }
}

/// What `desk` answers to `ev` with `ctl`, as (redraw, prevent_default),
/// and what it asked of the page.
fn send_with(desk: &mut Desktop, ev: Event, mut ctl: Ctl) -> ((bool, bool), Vec<Fx>) {
    let h = desk.event(ev, &mut ctl);
    ((h.redraw, h.prevent_default), ctl.effects().to_vec())
}

fn send(desk: &mut Desktop, ev: Event) -> ((bool, bool), Vec<Fx>) {
    send_with(desk, ev, Ctl::new())
}

/// A desktop at 1280 x 800.
fn desktop() -> Desktop {
    let mut desk = Desktop::new().expect("the boot font loads");
    send(&mut desk, resize(1280.0, 800.0));
    desk
}

fn fetched(id: u32, result: Result<Vec<u8>, String>) -> Event {
    Event::Fetched { id, result }
}

/// A frame as [`App::frame`] draws it at `dpr`, without a renderer, and
/// what it asked of the page.
fn frame(desk: &mut Desktop, dpr: f32) -> Vec<Fx> {
    let mut ctl = Ctl::new();
    let (_, animating) = desk.paint(dpr, &ctl);
    desk.drawn(animating, &mut ctl);
    ctl.effects().to_vec()
}

/// Key-downs from `code/key/mods` entries split by single spaces, each
/// with its entry.
fn downs(table: &str) -> impl Iterator<Item = (&str, Event)> {
    table.split(' ').map(|e| {
        let f: Vec<&str> = e.split('/').collect();
        (e, key(f[0], f[1], true, f[2]))
    })
}

fn shell(desk: &Desktop) -> &Shell {
    desk.shell.as_ref().expect("created")
}

/// The middle of the focused window.
fn focused_middle(desk: &Desktop) -> (f32, f32) {
    let layout = shell(desk).wm().layout();
    let r = layout.iter().find(|p| p.focused).expect("focused").rect;
    ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32)
}

#[test]
fn key_downs_map_by_code() {
    let table = "KeyA=Char('a') KeyZ=Char('z') Digit0=Char('0') Digit9=Char('9') \
        Numpad7=Char('7') Enter=Enter NumpadEnter=Enter Escape=Escape Backspace=Backspace \
        Delete=Delete Tab=Tab Space=Space ArrowLeft=Left ArrowRight=Right ArrowUp=Up \
        ArrowDown=Down Home=Home End=End PageUp=PageUp PageDown=PageDown Insert=Insert \
        F1=F(1) F12=F(12) Backquote=Char('`') AltLeft=Other ShiftLeft=Other Minus=Other \
        keya=Other Key=Other =Other";
    for entry in table.split_whitespace() {
        let (code, want) = entry.split_once('=').expect("code=Key");
        let ev = key(code, "Unidentified", true, "");
        let Some(Input::Key { key: k, mods }) = input_of(ev) else {
            panic!("{code:?}");
        };
        assert_eq!((format!("{k:?}"), mods), (want.into(), Mods::default()), "{code:?}");
    }
}

#[test]
fn keys_without_a_code_go_by_their_meaning() {
    // (code, key): phone keyboards, remote desktops and synthetic events
    // may report no code. A code, when there is one, wins, except on a
    // keypad with NumLock off, whose digits are arrows, Home, Delete...
    let table = "/Enter=Enter /Backspace=Backspace Unidentified/ArrowLeft=Left /F5=F(5) \
        /c=Char('c') /C=Char('c') /7=Char('7') /é=Other /!=Other /Unidentified=Other \
        /Process=Other /=Other KeyQ/a=Char('q') IntlBackslash/<=Other Numpad7/7=Char('7') \
        Numpad8/ArrowUp=Up Numpad7/Home=Home Numpad0/Insert=Insert NumpadDecimal/Delete=Delete \
        Numpad5/Clear=Char('5') NumpadDecimal/.=Other NumpadEnter/Enter=Enter \
        Backquote/²=Char('`') Backquote/Dead=Char('`') /`=Char('`') /~=Other";
    for entry in table.split(' ') {
        let (ev, want) = entry.split_once('=').expect("code/key=want");
        let (code, k) = ev.split_once('/').expect("code/key");
        assert_eq!(format!("{:?}", key_of(code, k, false)), want, "{entry}");
    }
    assert_eq!(key_of("", " ", false), Key::Space);
    // Ctrl+C with no code still reaches the terminal.
    let mut desk = desktop();
    send(&mut desk, key("Enter", "Enter", true, "a"));
    let ev = key("", "c", true, "c");
    let Some(Input::Key { key: k, mods }) = input_of(ev.clone()) else {
        panic!("a key");
    };
    assert_eq!((k, mods.ctrl), (Key::Char('c'), true));
    assert!(send(&mut desk, ev).0.1);
}

#[test]
fn shortcut_letters_follow_the_layout() {
    // code/key/mods=want: with Ctrl, Alt or Meta an ASCII letter goes by
    // meaning (AZERTY's Z is at KeyW, Dvorak's C at KeyI); anything else by
    // position (Cyrillic, macOS Option symbols, digits, AltGr text).
    let table = "KeyW/z/c KeyQ/a/c KeyA/q/a KeyI/c/c KeyV/K/cs Period/v/m KeyC/\u{441}/c \
        KeyQ/\u{153}/a Digit1/&/a KeyW/z/ KeyW/z/s KeyQ/@/cag Backquote/\u{b2}/a";
    for ((entry, ev), want) in downs(table).zip("zaqckvcq1wwq`".chars()) {
        let Some(Input::Key { key: got, .. }) = input_of(ev) else {
            panic!("{entry}");
        };
        assert_eq!(got, Key::Char(want), "{entry}");
    }
    // A terminal gets what the user pressed: Dvorak Ctrl+K (at KeyV) is no
    // paste, so it is delivered and prevented; Ctrl+V is the browser's.
    let mut desk = desktop();
    send(&mut desk, key("Enter", "Enter", true, "a"));
    assert!(send(&mut desk, key("KeyV", "k", true, "c")).0.1);
    assert!(!send(&mut desk, key("KeyV", "v", true, "c")).0.1);
}

#[test]
fn modifiers_pass_through_but_altgr_is_not_ctrl_alt() {
    // Held=seen, in `key`'s letters. Chrome and Edge on Windows send AltGr as
    // "cag"; left Ctrl+Alt ("ca") and AltGraph without both keep what they hold.
    for entry in "sm=sm cag= scamg=sm ca=ca sca=sca ag=a cg=c g= =".split(' ') {
        let (held, want) = entry.split_once('=').expect("held=seen");
        let ev = key("KeyQ", "q", true, held);
        let Some(Input::Key { key: k, mods: m }) = input_of(ev) else {
            panic!("{held}");
        };
        let seen = [(m.shift, 's'), (m.ctrl, 'c'), (m.alt, 'a'), (m.meta, 'm')];
        let seen: String = seen.iter().filter_map(|&(on, c)| on.then_some(c)).collect();
        assert_eq!((k, seen.as_str()), (Key::Char('q'), want), "{held}");
    }
}

#[test]
fn events_map_to_inputs() {
    assert_eq!(input_of(key("KeyQ", "q", false, "a")), None);
    assert_eq!(input_of(fetched(1, Ok(vec![1]))), None);
    let (x, y, button, dy) = (3.0, 4.0, 2, -48.0);
    let time =
        platform::LocalTime { year: 2026, month: 9, day: 30, weekday: 3, hour: 14, minute: 7 };
    let shell_time = LocalTime { year: 2026, month: 9, day: 30, weekday: 3, hour: 14, minute: 7 };
    let pairs = [
        (Event::Text("é".into()), Input::Text("é".into())),
        (Event::PointerMove { x, y }, Input::PointerMove { x, y }),
        (Event::PointerDown { x, y, button }, Input::PointerDown { x, y, button }),
        (Event::PointerUp { x, y, button }, Input::PointerUp { x, y, button }),
        (Event::PointerLeave, Input::PointerLeave),
        (Event::Wheel { x, y, dy }, Input::Wheel { x, y, dy }),
        (resize(8.0, 6.0), Input::Resize { w: 8.0, h: 6.0 }),
        (Event::Tick { time }, Input::Tick { time: shell_time }),
    ];
    for (ev, want) in pairs {
        assert_eq!(input_of(ev.clone()), Some(want), "{ev:?}");
    }
    assert_eq!(shell_time.clock(), "14:07");
}

#[test]
fn cursors_map_one_to_one() {
    use platform::Cursor as P;
    let pairs = [
        (Cursor::Default, P::Default),
        (Cursor::Text, P::Text),
        (Cursor::Grab, P::Grab),
        (Cursor::Grabbing, P::Grabbing),
        (Cursor::EwResize, P::EwResize),
        (Cursor::NsResize, P::NsResize),
        (Cursor::NwseResize, P::NwseResize),
        (Cursor::NeswResize, P::NeswResize),
    ];
    for (from, to) in pairs {
        assert_eq!(cursor(from), to, "{from:?}");
    }
    // Over a titlebar the hand opens; over the bare desktop it is the arrow.
    let mut desk = desktop();
    let r = shell(&desk).wm().layout()[0].rect;
    let title = ((r.x + r.w / 2) as f32, (r.y + 12) as f32);
    let over = |d: &mut Desktop, (x, y)| send(d, Event::PointerMove { x, y }).1;
    assert_eq!(over(&mut desk, title), [Fx::Cursor(P::Grab)]);
    assert_eq!(over(&mut desk, title), []);
    assert_eq!(over(&mut desk, (8.0, 300.0)), [Fx::Cursor(P::Default)]);
}

#[test]
fn key_ups_prevent_only_modifiers() {
    for code in "AltLeft AltRight MetaLeft MetaRight".split(' ') {
        let h = key_up(code);
        assert!(h.prevent_default && !h.redraw, "{code}");
    }
    let mut desk = desktop();
    for code in "KeyQ Enter ControlLeft ShiftLeft OSLeft Digit1".split(' ') {
        assert_eq!(key_up(code), Handled::default(), "{code}");
        assert_eq!(send(&mut desk, key(code, "q", false, "a")), (NOTHING, vec![]));
    }
    let alt = key("AltLeft", "Alt", false, "");
    assert_eq!(send(&mut desk, alt), ((false, true), vec![]));
}

#[test]
fn typed_keys_are_left_to_the_textarea() {
    // (code, key, mods): text, dead keys, IME and phone keys; AltGr is text.
    let typing = [
        ("KeyA", "a", ""),
        ("KeyA", "A", "s"),
        ("Digit1", "!", "s"),
        ("Space", " ", ""),
        ("KeyE", "é", ""),
        ("KeyK", "क्ष", ""),
        ("KeyQ", "@", "cag"),
        ("Quote", "Dead", ""),
        ("KeyN", "Process", ""),
        ("", "Unidentified", ""),
    ];
    for (code, k, mods) in typing {
        assert!(types_text(&key(code, k, true, mods)), "{code} {k:?} {mods}");
        assert!(!types_text(&key(code, k, false, mods)), "{code} {k:?} {mods} up");
    }
    // Named keys, shortcuts, and keys that report nothing.
    let not = "Enter/Enter/ Tab/Tab/ Backspace/Backspace/ ArrowLeft/ArrowLeft/ F5/F5/ \
        ShiftLeft/Shift/s KeyC/c/c KeyB/b/a KeyV/v/m KeyQ/q/ca KeyA// Enter/\r/";
    for (entry, ev) in downs(not) {
        assert!(!types_text(&ev), "{entry:?}");
    }
    assert!(!types_text(&Event::Text("a".into())));

    // Without text input the shell's answer stands; with it, typed keys go
    // unprevented and everything else is still prevented.
    let mut desk = desktop();
    let prevented = |d: &mut Desktop, code, k, mods| send(d, key(code, k, true, mods)).0.1;
    assert!(prevented(&mut desk, "KeyA", "a", ""));
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    assert!(fx.contains(&Fx::TextInput(true)), "{fx:?}");
    assert!(!prevented(&mut desk, "KeyA", "a", ""));
    assert!(!prevented(&mut desk, "KeyQ", "@", "cag"));
    for (code, k, mods) in [("Tab", "Tab", ""), ("Enter", "Enter", ""), ("KeyC", "c", "c")] {
        assert!(prevented(&mut desk, code, k, mods), "{code}");
    }
}

#[test]
fn shell_starts_at_the_first_usable_size() {
    let mut desk = Desktop::new().expect("the boot font loads");
    assert_eq!(send(&mut desk, key("Enter", "Enter", true, "a")), (NOTHING, vec![]));
    assert_eq!(send(&mut desk, Event::PointerLeave), (NOTHING, vec![]));
    let time = platform::LocalTime::EPOCH;
    assert_eq!(send(&mut desk, Event::Tick { time }), (NOTHING, vec![]));
    // A frame before the shell clears to the default theme's base.
    assert_eq!(desk.paint(1.0, &Ctl::new()), (ui::THEMES[0].base, false));
    let short = shell::BAR_H + shell::DOCK_CLEAR + 0.5;
    for (w, h) in [(0.0, 0.0), (800.0, short), (0.5, 600.0), (f32::NAN, 600.0)] {
        assert_eq!(send(&mut desk, resize(w, h)), (NOTHING, vec![]), "{w}x{h}");
        assert!(desk.shell.is_none(), "{w}x{h}");
    }
    assert!(desk.missed_tick);
    // Welcome opens, focused, and wants no text input.
    let (h, fx) = send(&mut desk, resize(1280.0, 800.0));
    assert_eq!((h, fx), ((true, false), vec![Fx::TextInput(false)]));
    // The startup window sits as it would at that size from the start.
    let (text, fs) = Desktop::new().expect("fonts").parts.expect("unused");
    let fresh = Shell::new(1280.0, 800.0, text, fs, registry(), "");
    let got = shell(&desk);
    assert_eq!(got.wm().state_hash(), fresh.wm().state_hash());
    assert_eq!(got.wm().layout(), fresh.wm().layout());
    assert_eq!(got.wm().layout().len(), 1);
    assert!(got.vfs().is_file(studio::DEFAULT_FILE));
    assert_eq!((got.theme_name(), desk.saved), ("Midnight", "Midnight"));
    assert!(desk.parts.is_none());
}

#[test]
fn the_theme_comes_from_storage_and_goes_back_when_it_changes() {
    let stored = |name: &str| {
        let mut ctl = Ctl::new();
        ctl.storage_set(THEME_KEY, name);
        let mut desk = Desktop::new().expect("the boot font loads");
        let (_, fx) = send_with(&mut desk, resize(1280.0, 800.0), ctl);
        let writes = fx.iter().filter(|f| matches!(f, Fx::Store { .. })).count();
        (desk, writes)
    };
    // Names match regardless of case; an unknown name is the default. None
    // is written back at start: only the test's own write is queued.
    let (mut desk, writes) = stored("dawn");
    assert_eq!((shell(&desk).theme_name(), writes), ("Dawn", 1));
    let (other, _) = stored("Solarized");
    assert_eq!(shell(&other).theme_name(), "Midnight");
    // The top bar's theme button (at the right) moves to the next theme,
    // which is stored once.
    let at = (1280.0 - 12.0 - 7.0, shell::BAR_H / 2.0);
    let store = Fx::Store { key: THEME_KEY.into(), value: "Mono".into() };
    send(&mut desk, Event::PointerMove { x: at.0, y: at.1 });
    send(&mut desk, Event::PointerDown { x: at.0, y: at.1, button: 0 });
    let (_, fx) = send(&mut desk, Event::PointerUp { x: at.0, y: at.1, button: 0 });
    assert_eq!(shell(&desk).theme_name(), "Mono");
    assert_eq!(fx.iter().filter(|f| **f == store).count(), 1, "{fx:?}");
    assert!(frame(&mut desk, 1.0).iter().all(|f| !matches!(f, Fx::Store { .. })));
    let (_, fx) = send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 });
    assert!(fx.iter().all(|f| !matches!(f, Fx::Store { .. })), "{fx:?}");
}

#[test]
fn frames_keep_coming_only_while_something_moves() {
    let mut desk = Desktop::new().expect("the boot font loads");
    // Nothing moves before the shell exists.
    assert!(!frame(&mut desk, 1.0).contains(&Fx::RequestFrame));
    send(&mut desk, resize(1280.0, 800.0));
    // The welcome window fades in: its first frame asks for the next. (The
    // page clock reads 0 here, so the fade never ends.)
    assert_eq!(frame(&mut desk, 1.0), [Fx::RequestFrame]);
    assert_eq!(frame(&mut desk, 1.0), [Fx::RequestFrame]);
    let mut ctl = Ctl::new();
    desk.drawn(false, &mut ctl);
    assert_eq!(ctl.effects(), []);
    // An event while an animation runs asks for a frame too.
    let (h, _) = send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 });
    assert!(h.0);
}

#[test]
fn deferred_fonts_are_fetched_after_the_first_frame() {
    let (bold, mono) = (u32::MAX - 1, u32::MAX);
    let has =
        |d: &mut Desktop, id| text_of(&mut d.shell, &mut d.parts).is_some_and(|t| t.has_font(id));
    let mut desk = Desktop::new().expect("the boot font loads");
    assert!(!has(&mut desk, FontId::SansBold) && !has(&mut desk, FontId::Mono));
    // Before they are asked for, their ids are nobody's.
    assert_eq!(send(&mut desk, fetched(mono, Ok(MONO.to_vec()))), (NOTHING, vec![]));
    assert!(!has(&mut desk, FontId::Mono));
    // The first frame asks for both, even before the shell exists; later
    // frames do not.
    let url = |f: &str| ["fonts/deferred/", f].concat();
    let fetches = [
        Fx::Fetch { id: bold, url: url("Inter-SemiBold.ttf") },
        Fx::Fetch { id: mono, url: url("JetBrainsMono-Regular.ttf") },
    ];
    assert_eq!(frame(&mut desk, 1.0), fetches);
    assert_eq!(frame(&mut desk, 1.0), []);
    // A font that arrives before the shell exists is the shell's later.
    let got = send(&mut desk, fetched(mono, Ok(MONO.to_vec())));
    assert_eq!(got, ((true, false), vec![]));
    send(&mut desk, resize(1280.0, 800.0));
    assert!(has(&mut desk, FontId::Mono) && !has(&mut desk, FontId::SansBold));
    // A failed fetch leaves the slot empty, quietly; after that its id
    // goes to the shell, which knows no such fetch.
    for got in [Err("HTTP 404".into()), Ok(SEMI.to_vec())] {
        assert_eq!(send(&mut desk, fetched(bold, got)).1, vec![]);
    }
    assert!(!has(&mut desk, FontId::SansBold));
    // With the shell up: a bad font is skipped, a good one fills the slot.
    for (got, want) in [(vec![1, 2, 3], false), (SEMI.to_vec(), true)] {
        let mut desk = desktop();
        let [Fx::RequestFrame, ref rest @ ..] = frame(&mut desk, 1.0)[..] else {
            panic!("the welcome window fades in");
        };
        assert_eq!(rest, fetches);
        let (h, fx) = send(&mut desk, fetched(bold, Ok(got)));
        assert_eq!((h.1, fx, has(&mut desk, FontId::SansBold)), (false, vec![], want));
    }
}

#[test]
fn lazy_fonts_are_fetched_and_added_in_order() {
    let mut desk = Desktop::new().expect("the boot font loads");
    assert_eq!(send(&mut desk, fetched(1, Ok(vec![]))), (NOTHING, vec![]));
    send(&mut desk, resize(1280.0, 800.0));
    // The first terminal asks for them.
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    let [Fx::Fetch { id: a, url: ref ua }, Fx::Fetch { id: b, url: ref ub }, ..] = fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!([ua, ub], ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"]);
    // The deferred fonts are on their way too, under other ids.
    let deferred = frame(&mut desk, 1.0).into_iter().filter(|f| matches!(f, Fx::Fetch { .. }));
    assert!(deferred.map(|f| f != fx[0] && f != fx[1]).eq([true, true]));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/lazy/");
    let font = std::fs::read(format!("{dir}symbols-a.ttf")).expect("lazy font");
    // The second failed first: nothing is added until the first arrives.
    let failed = fetched(b, Err("HTTP 404".into()));
    assert_eq!(send(&mut desk, failed).1, vec![]);
    assert_eq!(desk.shell.as_mut().expect("created").text_mut().fallback_count(), 0);
    let got = send(&mut desk, fetched(a, Ok(font)));
    assert_eq!(got, ((true, false), vec![]));
    assert_eq!(desk.shell.as_mut().expect("created").text_mut().fallback_count(), 1);
}

#[test]
fn a_release_on_the_focused_window_asks_for_the_keyboard_again() {
    let mut desk = desktop();
    send(&mut desk, key("Enter", "Enter", true, "a"));
    let (x, y) = focused_middle(&desk);
    let (h, fx) = send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!((h.1, fx), (true, vec![]));
    let up = |d: &mut Desktop, x, y, button| send(d, Event::PointerUp { x, y, button }).1;
    assert_eq!(up(&mut desk, x, y, 0), [Fx::TextInput(true)]);
    // Not for another button, nor off the focused window (the bare desktop).
    assert_eq!(up(&mut desk, x, y, 2), []);
    let (x, y) = (4.0, shell::BAR_H + 4.0);
    let layout = shell(&desk).wm().layout();
    assert!(
        layout
            .iter()
            .all(|p| !RectF::from_i32(p.rect.x, p.rect.y, p.rect.w, p.rect.h).contains(x, y))
    );
    send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!(up(&mut desk, x, y, 0), []);
    // Nor without text input: the welcome window wants none.
    let mut desk = desktop();
    let (x, y) = focused_middle(&desk);
    send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!(up(&mut desk, x, y, 0), []);
}

#[test]
fn desktop_routes_events_through_the_shell() {
    let mut desk = desktop();
    let state = |d: &Desktop| shell(d).wm().state_hash();
    let count = |d: &Desktop| shell(d).wm().layout().len();
    assert_eq!(send(&mut desk, key("Enter", "Enter", true, "a")).0, (true, true));
    let opened = state(&desk);
    assert_eq!(count(&desk), 2);
    // Key-ups never reach the shell: releasing Alt+Q closes nothing.
    assert_eq!(send(&mut desk, key("KeyQ", "q", false, "a")).0, NOTHING);
    assert_eq!(state(&desk), opened);
    // AltGr+Up and AltGr+Backquote are no bindings: AltGr is not Alt.
    for (code, k) in [("ArrowUp", "ArrowUp"), ("Backquote", "`")] {
        send(&mut desk, key(code, k, true, "cag"));
        assert_eq!(state(&desk), opened, "{code}");
    }
    // Alt+Backquote moves the focus to the other window, Alt+Shift+Backquote
    // back; Alt+Up maximizes.
    let focus = |d: &Desktop| shell(d).wm().focused();
    let terminal = focus(&desk);
    assert!(send(&mut desk, key("Backquote", "`", true, "a")).0.1);
    assert_ne!(focus(&desk), terminal);
    send(&mut desk, key("Backquote", "~", true, "as"));
    assert_eq!(focus(&desk), terminal);
    assert_eq!(send(&mut desk, key("ArrowUp", "ArrowUp", true, "a")).0, (true, true));
    let top = shell(&desk).wm().layout().pop().expect("a window");
    assert_eq!((top.state, top.rect), (wm::State::Maximized, shell(&desk).wm().area()));
    // Alt+Q closes the focused window.
    assert!(send(&mut desk, key("KeyQ", "q", true, "a")).0.1);
    assert_eq!(count(&desk), 1);
    // Pointer events are consumed; resizes redraw but are not.
    let (x, y) = (640.0, 400.0);
    assert!(send(&mut desk, Event::PointerMove { x, y }).0.1);
    assert_eq!(send(&mut desk, resize(1024.0, 768.0)).0, (true, false));
    assert!(!send(&mut desk, Event::PointerLeave).0.1);
}
