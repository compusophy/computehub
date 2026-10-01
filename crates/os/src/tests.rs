use super::*;
use platform::Effect as Fx;

/// Outcomes as (redraw, prevent_default).
const NOTHING: (bool, bool) = (false, false);
const SEMI: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

/// A key event holding the modifiers in `mods`: Shift Ctrl Alt Meta AltGraph
/// as `s c a m g`.
fn key(code: &str, key: &str, down: bool, mods: &str) -> Event {
    let [shift, ctrl, alt, meta, altgr] = ['s', 'c', 'a', 'm', 'g'].map(|m| mods.contains(m));
    let (code, key, repeat) = (code.into(), key.into(), false);
    Event::Key { code, key, down, repeat, shift, ctrl, alt, meta, altgr }
}

/// A key event from `code/key/mods` (key `Unidentified`, no mods if left out).
fn parse(e: &str, down: bool) -> Event {
    let mut f = e.split('/');
    let (code, k) = (f.next().unwrap_or(""), f.next().unwrap_or("Unidentified"));
    key(code, k, down, f.next().unwrap_or(""))
}

fn resize(w: f32, h: f32) -> Event {
    Event::Resize { w, h, dpr: 2.0 }
}

fn fetched(id: u32, result: Result<Vec<u8>, String>) -> Event {
    Event::Fetched { id, result }
}

/// What `desk` answers to `ev`, as (redraw, prevent_default), and what it
/// asked of the page.
fn send(desk: &mut Desktop, ev: Event) -> ((bool, bool), Vec<Fx>) {
    let mut ctl = Ctl::default();
    let h = desk.event(ev, &mut ctl);
    ((h.redraw, h.prevent_default), ctl.effects().to_vec())
}

fn prevented(desk: &mut Desktop, code: &str, k: &str, mods: &str) -> bool {
    send(desk, key(code, k, true, mods)).0.1
}

fn fresh() -> Desktop {
    Desktop::new().expect("the boot font loads")
}

/// A desktop at 1280 x 800, with a terminal open in front when `terminal`.
fn desktop(terminal: bool) -> Desktop {
    let mut desk = fresh();
    send(&mut desk, resize(1280.0, 800.0));
    if terminal {
        send(&mut desk, key("Enter", "Enter", true, "a"));
    }
    desk
}

/// A frame as [`App::frame`] draws it, without a renderer, and what it asked.
fn frame(desk: &mut Desktop) -> Vec<Fx> {
    let mut ctl = Ctl::default();
    let (_, animating) = desk.paint(1.0, &ctl);
    desk.drawn(animating, &mut ctl);
    ctl.effects().to_vec()
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
fn keys_map_by_code_or_by_meaning() {
    // By code; with no code (phone keyboards, remote desktops) by key; keypad
    // keys with NumLock off by the key they name; Backquote as a backquote;
    // with Ctrl, Alt or Meta an ASCII letter by meaning (AZERTY's Z at KeyW,
    // Dvorak's C at KeyI), anything else (Cyrillic, Option symbols, digits,
    // AltGr text) by position.
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
        let Some(Input::Key { key: k, .. }) = input_of(parse(ev, true)) else { panic!("{entry}") };
        assert_eq!(format!("{k:?}"), want, "{entry}");
    }
    assert_eq!(key_of("", " ", false), Key::Space);
}

#[test]
fn events_map_to_inputs() {
    // Modifiers as held=seen: Chrome and Edge on Windows send AltGr as "cag",
    // which is no Ctrl+Alt; left Ctrl+Alt and AltGraph without both stay.
    for entry in "sm=sm cag= scamg=sm ca=ca sca=sca ag=a cg=c g= =".split(' ') {
        let (held, seen) = entry.split_once('=').expect("held=seen");
        let [shift, ctrl, alt, meta] = ['s', 'c', 'a', 'm'].map(|c| seen.contains(c));
        let want = Input::Key { key: Key::Char('q'), mods: Mods { shift, ctrl, alt, meta } };
        assert_eq!(input_of(key("KeyQ", "q", true, held)), Some(want), "{held}");
    }
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
}

#[test]
fn cursors_are_css_keywords() {
    use Cursor::*;
    let all = [Default, Text, Grab, Grabbing, EwResize, NsResize, NwseResize, NeswResize];
    let css = "default text grab grabbing ew-resize ns-resize nwse-resize nesw-resize";
    assert_eq!(all.map(cursor).join(" "), css);
    // Over a titlebar the hand opens, once; over the bare desktop the arrow.
    let mut desk = desktop(false);
    let r = shell(&desk).wm().layout()[0].rect;
    let title = ((r.x + r.w / 2) as f32, (r.y + 12) as f32);
    let over = |d: &mut Desktop, (x, y)| send(d, Event::PointerMove { x, y }).1;
    assert_eq!(over(&mut desk, title), [Fx::Cursor("grab")]);
    assert_eq!(over(&mut desk, title), []);
    assert_eq!(over(&mut desk, (8.0, 300.0)), [Fx::Cursor("default")]);
}

#[test]
fn key_ups_prevent_only_modifiers() {
    let mut desk = desktop(false);
    let table = "AltLeft=1 AltRight=1 MetaLeft=1 MetaRight=1 KeyQ=0 Enter=0 ControlLeft=0 \
        ShiftLeft=0 OSLeft=0 Digit1=0";
    for (code, prevent) in table.split(' ').filter_map(|e| e.split_once('=')) {
        let prevent = prevent == "1";
        assert_eq!(key_up(code), Handled { redraw: false, prevent_default: prevent }, "{code}");
        assert_eq!(send(&mut desk, key(code, "q", false, "a")), ((false, prevent), vec![]));
    }
}

#[test]
fn typed_keys_are_left_to_the_textarea() {
    // Text, dead keys, IME and phone keys type; AltGr is text.
    let typing = "KeyA/a/ KeyA/A/s Digit1/!/s KeyE/é/ KeyK/क्ष/ KeyQ/@/cag Quote/Dead/ \
        KeyN/Process/ /Unidentified/";
    for down in [true, false] {
        for e in typing.split(' ') {
            assert_eq!(types_text(&parse(e, down)), down, "{e} {down}");
        }
        assert_eq!(types_text(&key("Space", " ", down, "")), down);
    }
    // Named keys, shortcuts, and keys that report nothing.
    let not = "Enter/Enter/ Tab/Tab/ Backspace/Backspace/ ArrowLeft/ArrowLeft/ F5/F5/ \
        ShiftLeft/Shift/s KeyC/c/c KeyB/b/a KeyV/v/m KeyQ/q/ca KeyA// Enter/\r/";
    for e in not.split(' ') {
        assert!(!types_text(&parse(e, true)), "{e:?}");
    }
    assert!(!types_text(&Event::Text("a".into())));

    // Without text input the shell's answer stands; with it (in a terminal),
    // typed keys go unprevented and everything else is still prevented: Ctrl+C
    // with no code reaches the terminal, and Dvorak's Ctrl+K (at KeyV) is no
    // paste, while Ctrl+V is the browser's.
    let mut desk = desktop(false);
    assert!(prevented(&mut desk, "KeyA", "a", ""));
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    assert!(fx.contains(&Fx::TextInput(true)), "{fx:?}");
    assert!(!prevented(&mut desk, "KeyA", "a", "") && !prevented(&mut desk, "KeyQ", "@", "cag"));
    assert!(!prevented(&mut desk, "KeyV", "v", "c"));
    let keys = [("Tab", "Tab", ""), ("Enter", "Enter", ""), ("KeyC", "c", "c"), ("", "c", "c")];
    for (code, k, mods) in keys.into_iter().chain([("KeyV", "k", "c")]) {
        assert!(prevented(&mut desk, code, k, mods), "{code} {k}");
    }
}

#[test]
fn shell_starts_at_the_first_usable_size() {
    let mut desk = fresh();
    let time = platform::LocalTime::EPOCH;
    let early = [key("Enter", "Enter", true, "a"), Event::PointerLeave, Event::Tick { time }];
    for ev in early {
        assert_eq!(send(&mut desk, ev), (NOTHING, vec![]));
    }
    // A frame before the shell clears to the default theme's base.
    assert_eq!(desk.paint(1.0, &Ctl::default()), (ui::THEMES[0].base, false));
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
    let (text, fs) = fresh().parts.expect("unused");
    let fresh = Shell::new(1280.0, 800.0, text, fs, registry(), "");
    let got = shell(&desk);
    let wm = |s: &Shell| (s.wm().state_hash(), s.wm().layout());
    assert_eq!(wm(got), wm(&fresh));
    assert_eq!(got.wm().layout().len(), 1);
    assert!(got.vfs().is_file(studio::DEFAULT_FILE));
    assert_eq!((got.theme_name(), desk.saved), ("Midnight", "Midnight"));
    assert!(desk.parts.is_none());
}

#[test]
fn the_theme_comes_from_storage_and_goes_back_when_it_changes() {
    let stored = |name: &str| {
        let mut ctl = Ctl::default();
        ctl.storage_set(THEME_KEY, name);
        let mut desk = fresh();
        desk.event(resize(1280.0, 800.0), &mut ctl);
        let writes = ctl.effects().iter().filter(|f| matches!(f, Fx::Store { .. })).count();
        (desk, writes)
    };
    // Names match regardless of case; an unknown name is the default. None
    // is written back at start: only the test's own write is queued.
    let (mut desk, writes) = stored("dawn");
    assert_eq!((shell(&desk).theme_name(), writes), ("Dawn", 1));
    assert_eq!(shell(&stored("Solarized").0).theme_name(), "Midnight");
    // The top bar's theme button (at the right) moves to the next theme,
    // which is stored once.
    let (x, y) = (1280.0 - 12.0 - 7.0, shell::BAR_H / 2.0);
    let store = Fx::Store { key: THEME_KEY.into(), value: "Mono".into() };
    send(&mut desk, Event::PointerMove { x, y });
    send(&mut desk, Event::PointerDown { x, y, button: 0 });
    let (_, fx) = send(&mut desk, Event::PointerUp { x, y, button: 0 });
    assert_eq!(shell(&desk).theme_name(), "Mono");
    assert_eq!(fx.iter().filter(|f| **f == store).count(), 1, "{fx:?}");
    for fx in [frame(&mut desk), send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 }).1] {
        assert!(fx.iter().all(|f| !matches!(f, Fx::Store { .. })), "{fx:?}");
    }
}

#[test]
fn frames_keep_coming_only_while_something_moves() {
    // Nothing moves before the shell exists (see the deferred fonts). The
    // welcome window fades in, and the page clock reads 0 here, so the fade
    // never ends: every frame asks for the next, and so does any event.
    let mut desk = desktop(false);
    frame(&mut desk);
    assert_eq!(frame(&mut desk), [Fx::RequestFrame]);
    assert!(send(&mut desk, Event::PointerMove { x: 9.0, y: 300.0 }).0.0);
    // A frame with nothing moving asks for nothing.
    let mut ctl = Ctl::default();
    desk.drawn(false, &mut ctl);
    assert_eq!(ctl.effects(), []);
}

#[test]
fn deferred_fonts_are_fetched_after_the_first_frame() {
    let (bold, mono) = (u32::MAX - 1, u32::MAX);
    let has =
        |d: &mut Desktop, id| text_of(&mut d.shell, &mut d.parts).is_some_and(|t| t.has_font(id));
    let mut desk = fresh();
    assert!(!has(&mut desk, FontId::SansBold) && !has(&mut desk, FontId::Mono));
    // Before they are asked for, their ids are nobody's.
    assert_eq!(send(&mut desk, fetched(mono, Ok(MONO.to_vec()))), (NOTHING, vec![]));
    assert!(!has(&mut desk, FontId::Mono));
    // The first frame asks for both, even before the shell exists (asking
    // for no next frame: nothing moves yet); later frames ask for nothing.
    let url = |f: &str| ["fonts/deferred/", f].concat();
    let fetches = [
        Fx::Fetch { id: bold, url: url("Inter-SemiBold.ttf") },
        Fx::Fetch { id: mono, url: url("JetBrainsMono-Regular.ttf") },
    ];
    assert_eq!(frame(&mut desk), fetches);
    assert_eq!(frame(&mut desk), []);
    // A font that arrives before the shell exists is the shell's later.
    assert_eq!(send(&mut desk, fetched(mono, Ok(MONO.to_vec()))), ((true, false), vec![]));
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
        let mut desk = desktop(false);
        let [Fx::RequestFrame, ref rest @ ..] = frame(&mut desk)[..] else {
            panic!("the welcome window fades in");
        };
        assert_eq!(rest, fetches);
        let (h, fx) = send(&mut desk, fetched(bold, Ok(got)));
        assert_eq!((h.1, fx, has(&mut desk, FontId::SansBold)), (false, vec![], want));
    }
}

#[test]
fn lazy_fonts_are_fetched_and_added_in_order() {
    let mut desk = fresh();
    assert_eq!(send(&mut desk, fetched(1, Ok(vec![]))), (NOTHING, vec![]));
    send(&mut desk, resize(1280.0, 800.0));
    // The first terminal asks for them.
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    let [Fx::Fetch { id: a, url: ref ua }, Fx::Fetch { id: b, url: ref ub }, ..] = fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!([ua, ub], ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"]);
    // The deferred fonts are on their way too, under other ids.
    let deferred = frame(&mut desk).into_iter().filter(|f| matches!(f, Fx::Fetch { .. }));
    assert!(deferred.map(|f| f != fx[0] && f != fx[1]).eq([true, true]));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/lazy/");
    let font = std::fs::read(format!("{dir}symbols-a.ttf")).expect("lazy font");
    // The second failed first: nothing is added until the first arrives.
    let fallbacks =
        |d: &mut Desktop| d.shell.as_mut().expect("created").text_mut().fallback_count();
    assert_eq!(send(&mut desk, fetched(b, Err("HTTP 404".into()))).1, vec![]);
    assert_eq!(fallbacks(&mut desk), 0);
    assert_eq!(send(&mut desk, fetched(a, Ok(font))), ((true, false), vec![]));
    assert_eq!(fallbacks(&mut desk), 1);
}

#[test]
fn a_release_on_the_focused_window_asks_for_the_keyboard_again() {
    let down = |d: &mut Desktop, (x, y)| send(d, Event::PointerDown { x, y, button: 0 });
    let up = |d: &mut Desktop, (x, y), button| send(d, Event::PointerUp { x, y, button }).1;
    let mut desk = desktop(true);
    let at = focused_middle(&desk);
    let (h, fx) = down(&mut desk, at);
    assert_eq!((h.1, fx), (true, vec![]));
    assert_eq!(up(&mut desk, at, 0), [Fx::TextInput(true)]);
    // Not for another button, nor off the focused window (the bare desktop).
    assert_eq!(up(&mut desk, at, 2), []);
    let bare = (4.0, shell::BAR_H + 4.0);
    let r = |p: &wm::Placement| RectF::from_i32(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
    assert!(shell(&desk).wm().layout().iter().all(|p| !r(p).contains(bare.0, bare.1)));
    down(&mut desk, bare);
    assert_eq!(up(&mut desk, bare, 0), []);
    // Nor without text input: the welcome window wants none.
    let mut desk = desktop(false);
    let at = focused_middle(&desk);
    down(&mut desk, at);
    assert_eq!(up(&mut desk, at, 0), []);
}

#[test]
fn desktop_routes_events_through_the_shell() {
    let mut desk = desktop(false);
    let state = |d: &Desktop| shell(d).wm().state_hash();
    let count = |d: &Desktop| shell(d).wm().layout().len();
    assert_eq!(send(&mut desk, key("Enter", "Enter", true, "a")).0, (true, true));
    let opened = state(&desk);
    assert_eq!(count(&desk), 2);
    // Key-ups never reach the shell: releasing Alt+Q closes nothing. AltGr+Up
    // and AltGr+Backquote are no bindings: AltGr is not Alt.
    assert_eq!(send(&mut desk, key("KeyQ", "q", false, "a")).0, NOTHING);
    assert_eq!(state(&desk), opened);
    for ev in ["ArrowUp/ArrowUp/cag", "Backquote/`/cag"] {
        send(&mut desk, parse(ev, true));
        assert_eq!(state(&desk), opened, "{ev}");
    }
    // Alt+Backquote moves the focus to the other window, Alt+Shift+Backquote
    // back; Alt+Up maximizes.
    let focus = |d: &Desktop| shell(d).wm().focused();
    let terminal = focus(&desk);
    assert!(prevented(&mut desk, "Backquote", "`", "a"));
    assert_ne!(focus(&desk), terminal);
    send(&mut desk, key("Backquote", "~", true, "as"));
    assert_eq!(focus(&desk), terminal);
    assert_eq!(send(&mut desk, key("ArrowUp", "ArrowUp", true, "a")).0, (true, true));
    let top = shell(&desk).wm().layout().pop().expect("a window");
    assert_eq!((top.state, top.rect), (wm::State::Maximized, shell(&desk).wm().area()));
    // Alt+Q closes the focused window.
    assert!(prevented(&mut desk, "KeyQ", "q", "a"));
    assert_eq!(count(&desk), 1);
    // Pointer events are consumed; resizes redraw but are not.
    assert!(send(&mut desk, Event::PointerMove { x: 640.0, y: 400.0 }).0.1);
    assert_eq!(send(&mut desk, resize(1024.0, 768.0)).0, (true, false));
    assert!(!send(&mut desk, Event::PointerLeave).0.1);
}
