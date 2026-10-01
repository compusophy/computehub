use super::*;
use platform::{Effect as Fx, WsEvent as Ws};

/// Outcomes as (redraw, prevent_default).
const NOTHING: (bool, bool) = (false, false);
const TOKEN: [u8; 32] = [0xab; 32];
const NODE: &str = "ws://127.0.0.1:8123/";
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

fn ws(id: u32, ev: Ws) -> Event {
    Event::Ws { id, ev }
}

/// What `desk` answers to `ev`, as (redraw, prevent_default), and what it
/// asked of the page.
fn send(desk: &mut Desktop, ev: Event) -> ((bool, bool), Vec<Fx>) {
    let mut ctl = Ctl::new();
    let h = desk.event(ev, &mut ctl);
    ((h.redraw, h.prevent_default), ctl.effects().to_vec())
}

/// A desktop at 1280 x 800, booted from a hash that pairs it with a node on
/// port 8123 if `paired`.
fn desktop(paired: bool) -> Desktop {
    let hash = format!("node=8123&token={}", "ab".repeat(32));
    let mut desk = Desktop::boot(if paired { &hash } else { "" }).expect("the boot font loads");
    send(&mut desk, resize(1280.0, 800.0));
    desk
}

/// The sockets `fx` opens, as (id, url).
fn opened(fx: &[Fx]) -> Vec<(u32, &str)> {
    let mut out = Vec::new();
    for f in fx {
        if let Fx::WsOpen { id, url } = f {
            out.push((*id, url.as_str()));
        }
    }
    out
}

fn fetched(id: u32, result: Result<Vec<u8>, String>) -> Event {
    Event::Fetched { id, result }
}

/// A frame as [`App::frame`] draws it at `dpr`, without a renderer, and
/// what it asked of the page.
fn frame(desk: &mut Desktop, dpr: f32) -> Vec<Fx> {
    let mut ctl = Ctl::new();
    desk.paint(dpr, &ctl);
    desk.drawn(&mut ctl);
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

/// A DATA message from the node, or to it.
fn data(bytes: &[u8]) -> Vec<u8> {
    [&[3][..], bytes].concat()
}

#[test]
fn key_downs_map_by_code() {
    let table = "KeyA=Char('a') KeyZ=Char('z') Digit0=Char('0') Digit9=Char('9') \
        Numpad7=Char('7') Enter=Enter NumpadEnter=Enter Escape=Escape Backspace=Backspace \
        Delete=Delete Tab=Tab Space=Space ArrowLeft=Left ArrowRight=Right ArrowUp=Up \
        ArrowDown=Down Home=Home End=End PageUp=PageUp PageDown=PageDown Insert=Insert \
        F1=F(1) F12=F(12) AltLeft=Other ShiftLeft=Other keya=Other Key=Other =Other";
    for entry in table.split_whitespace() {
        let (code, want) = entry.split_once('=').expect("code=Key");
        let ev = key(code, "Unidentified", true, "");
        let Some(Input::Key { key: k, mods }) = input_of(ev, &Ctl::new()) else {
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
        Numpad5/Clear=Char('5') NumpadDecimal/.=Other NumpadEnter/Enter=Enter";
    for entry in table.split(' ') {
        let (ev, want) = entry.split_once('=').expect("code/key=want");
        let (code, k) = ev.split_once('/').expect("code/key");
        assert_eq!(format!("{:?}", key_of(code, k, false)), want, "{entry}");
    }
    assert_eq!(key_of("", " ", false), Key::Space);
    // Ctrl+C with no code still interrupts the program.
    let mut desk = desktop(false);
    send(&mut desk, key("Enter", "Enter", true, "a"));
    let ev = key("", "c", true, "c");
    let Some(Input::Key { key: k, mods }) = input_of(ev.clone(), &Ctl::new()) else {
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
        KeyQ/\u{153}/a Digit1/&/a KeyW/z/ KeyW/z/s KeyQ/@/cag";
    for ((entry, ev), want) in downs(table).zip("zaqckvcq1wwq".chars()) {
        let Some(Input::Key { key: got, .. }) = input_of(ev, &Ctl::new()) else {
            panic!("{entry}");
        };
        assert_eq!(got, Key::Char(want), "{entry}");
    }
    // A live terminal gets what the user pressed: AZERTY Ctrl+Z suspends
    // (0x1a, not 0x17, delete-word), Dvorak Ctrl+C interrupts, and Dvorak
    // Ctrl+K (at KeyV) is no paste: it is delivered and prevented.
    let mut desk = desktop(true);
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    let [(id, _)] = opened(&fx)[..] else {
        panic!("{fx:?}");
    };
    send(&mut desk, ws(id, Ws::Open));
    send(&mut desk, ws(id, Ws::Data(vec![2, 1, 80, 0, 24, 0, b'x'])));
    for (code, k, byte) in [("KeyW", "z", 26), ("KeyI", "c", 3), ("KeyV", "k", 11)] {
        let (h, fx) = send(&mut desk, key(code, k, true, "c"));
        let bytes = data(&[byte]);
        assert_eq!((h.1, fx), (true, vec![Fx::WsSend { id, bytes }]), "{code}");
    }
}

#[test]
fn modifiers_pass_through_but_altgr_is_not_ctrl_alt() {
    // Held=seen, in `key`'s letters. Chrome and Edge on Windows send AltGr as
    // "cag"; left Ctrl+Alt ("ca") and AltGraph without both keep what they hold.
    for entry in "sm=sm cag= scamg=sm ca=ca sca=sca ag=a cg=c g= =".split(' ') {
        let (held, want) = entry.split_once('=').expect("held=seen");
        let ev = key("KeyQ", "q", true, held);
        let Some(Input::Key { key: k, mods: m }) = input_of(ev, &Ctl::new()) else {
            panic!("{held}");
        };
        let seen = [(m.shift, 's'), (m.ctrl, 'c'), (m.alt, 'a'), (m.meta, 'm')];
        let seen: String = seen.iter().filter_map(|&(on, c)| on.then_some(c)).collect();
        assert_eq!((k, seen.as_str()), (Key::Char('q'), want), "{held}");
    }
}

#[test]
fn events_map_to_inputs() {
    let ctl = Ctl::new();
    let of = |ev| input_of(ev, &ctl);
    assert_eq!(of(key("KeyQ", "q", false, "a")), None);
    assert_eq!(of(fetched(1, Ok(vec![1]))), None);
    let (x, y, button, dy, minutes, now_ms) = (3.0, 4.0, 2, -48.0, 61, 0.0);
    let pairs = [
        (Event::Text("é".into()), Input::Text("é".into())),
        (Event::PointerMove { x, y }, Input::PointerMove { x, y }),
        (Event::PointerDown { x, y, button }, Input::PointerDown { x, y, button }),
        (Event::PointerUp { x, y, button }, Input::PointerUp { x, y, button }),
        (Event::PointerLeave, Input::PointerLeave),
        (Event::Wheel { x, y, dy }, Input::Wheel { x, y, dy }),
        (resize(8.0, 6.0), Input::Resize { w: 8.0, h: 6.0 }),
        // Outside the browser the page clock reads 0.
        (Event::Tick { minutes }, Input::Tick { minutes, now_ms }),
    ];
    for (ev, want) in pairs {
        assert_eq!(of(ev.clone()), Some(want), "{ev:?}");
    }
    let (code, reason) = (4401, "bye".to_string());
    let socket = [
        (Ws::Open, WsEvent::Open),
        (Ws::Data(vec![1, 2]), WsEvent::Data(vec![1, 2])),
        (Ws::Closed { code, reason: reason.clone() }, WsEvent::Closed { code, reason }),
        (Ws::Error, WsEvent::Error),
    ];
    for (ev, want) in socket {
        assert_eq!(of(ws(9, ev)), Some(Input::Ws { id: 9, ev: want }));
    }
}

#[test]
fn key_ups_prevent_only_modifiers() {
    for code in "AltLeft AltRight MetaLeft MetaRight".split(' ') {
        let h = key_up(code);
        assert!(h.prevent_default && !h.redraw, "{code}");
    }
    let mut desk = desktop(false);
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
    let mut desk = desktop(false);
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
    assert_eq!(send(&mut desk, Event::Tick { minutes: 5 }), (NOTHING, vec![]));
    let short = theme::PANEL_H + 0.5;
    for (w, h) in [(0.0, 0.0), (800.0, short), (0.5, 600.0), (f32::NAN, 600.0)] {
        assert_eq!(send(&mut desk, resize(w, h)), (NOTHING, vec![]), "{w}x{h}");
        assert!(desk.shell.is_none(), "{w}x{h}");
    }
    assert!(desk.missed_tick);
    // The terminal asks for the lazy fonts; welcome has the focus and wants
    // no text input.
    let (h, fx) = send(&mut desk, resize(1280.0, 800.0));
    assert_eq!(h, (true, false));
    let [Fx::Fetch { url: a, .. }, Fx::Fetch { url: b, .. }, Fx::TextInput(false)] = &fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!([a, b], ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"]);
    // The startup windows split as they would at that size from the start.
    let (text, fs) = Desktop::new().expect("fonts").parts.expect("unused");
    let fresh = Shell::new(1280.0, 800.0, text, fs, registry(), None);
    let got = desk.shell.as_ref().expect("created");
    assert_eq!(got.wm().state_hash(), fresh.wm().state_hash());
    assert_eq!(got.wm().layout(), fresh.wm().layout());
    assert_eq!(got.wm().layout().len(), 3);
    assert!(got.vfs().is_file(studio::DEFAULT_FILE));
    assert!(desk.parts.is_none());
}

#[test]
fn the_pairing_comes_from_the_hash_which_is_cleared() {
    let mut desk = Desktop::new().expect("the boot font loads");
    let mut ctl = Ctl::new();
    desk.pair("", &mut ctl);
    assert_eq!((desk.pairing, ctl.effects()), (None, &[][..]));
    let hash = format!("#token={}&node=8123", "AB".repeat(32));
    desk.pair(&hash, &mut ctl);
    assert_eq!(desk.pairing.map(|p| (p.port, p.token)), Some((8123, TOKEN)));
    assert_eq!(ctl.effects(), [Fx::ClearHash]);
    // Any other hash is cleared too, and pairs with nothing: the pairing
    // read before stays.
    let mut ctl = Ctl::new();
    assert_eq!(desk.pair("#token=12&node=8123", &mut ctl), None);
    assert_eq!(desk.pairing.map(|p| p.port), Some(8123));
    assert_eq!(ctl.effects(), [Fx::ClearHash]);
    // `start` takes (and clears) the hash before anything that can fail and
    // boots from it: no event or frame clears it again, and the startup
    // terminal connects with its pairing.
    let mut desk = Desktop::boot(&hash[1..]).expect("the boot font loads");
    assert_eq!(send(&mut desk, Event::PointerLeave), (NOTHING, vec![]));
    assert!(!frame(&mut desk, 1.0).contains(&Fx::ClearHash));
    let (_, fx) = send(&mut desk, resize(1280.0, 800.0));
    assert!(!fx.contains(&Fx::ClearHash), "{fx:?}");
    assert_eq!(opened(&fx).iter().map(|o| o.1).collect::<Vec<_>>(), [NODE]);
    for h in ["", "x=1", "#node=8123&token=12"] {
        assert_eq!(Desktop::boot(h).expect("boots").pairing, None, "{h}");
    }
    assert_eq!(platform::take_location_hash(), "");
}

#[test]
fn leaving_asks_first_while_a_node_socket_is_open() {
    let guards = |(_, fx): ((bool, bool), Vec<Fx>)| -> Vec<Fx> {
        let guard = |f: &Fx| matches!(f, Fx::GuardUnload(_));
        fx.into_iter().filter(guard).collect()
    };
    let hash = format!("node=8123&token={}", "ab".repeat(32));
    let mut desk = Desktop::boot(&hash).expect("the boot font loads");
    let (_, fx) = send(&mut desk, resize(1280.0, 800.0));
    let [(a, _)] = opened(&fx)[..] else {
        panic!("{fx:?}");
    };
    // Connecting is not yet a shell: the guard comes with Open, once.
    let (_, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    let [(b, _)] = opened(&fx)[..] else {
        panic!("{fx:?}");
    };
    assert!(!desk.guarded && !fx.contains(&Fx::GuardUnload(true)));
    assert_eq!(guards(send(&mut desk, ws(a, Ws::Open))), [Fx::GuardUnload(true)]);
    assert_eq!(guards(send(&mut desk, ws(b, Ws::Open))), []);
    // The node drops the startup terminal's connection: the new one's
    // shell still lives. Alt+Q closes the new terminal and its socket: no
    // shell is left, so leaving no longer asks.
    let (code, reason) = (1006, String::new());
    assert_eq!(guards(send(&mut desk, ws(a, Ws::Closed { code, reason }))), []);
    let (_, fx) = send(&mut desk, key("KeyQ", "q", true, "a"));
    assert!(fx.contains(&Fx::WsClose { id: b }), "{fx:?}");
    assert_eq!(guards((NOTHING, fx)), [Fx::GuardUnload(false)]);
    assert!(desk.open.is_empty() && !desk.guarded);
}

#[test]
fn a_hash_change_pairs_a_new_terminal() {
    let hash = format!("node=8123&token={}", "ab".repeat(32));
    let windows = |d: &Desktop| d.shell.as_ref().map(|s| s.wm().layout().len());
    let mut desk = desktop(false);
    assert_eq!(windows(&desk), Some(3));
    // An empty hash is nothing; any other is cleared, and may pair.
    let change = |h: &str| Event::HashChange(h.into());
    assert_eq!(send(&mut desk, change("")), (NOTHING, vec![]));
    assert_eq!(send(&mut desk, change("x=1")), (NOTHING, vec![Fx::ClearHash]));
    assert_eq!(windows(&desk), Some(3));
    // A pairing opens a terminal, focused and typing, which connects.
    let (h, fx) = send(&mut desk, change(&hash));
    let [Fx::ClearHash, Fx::WsOpen { id, url }, Fx::TextInput(true)] = &fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!((h, url.as_str(), windows(&desk)), ((true, false), NODE, Some(4)));
    let (id, bytes) = (*id, [&[1, 1][..], &TOKEN].concat());
    let (_, fx) = send(&mut desk, ws(id, Ws::Open));
    assert_eq!(fx, [Fx::WsSend { id, bytes }, Fx::GuardUnload(true)]);
    // Before the shell exists, the pairing waits for it: the startup
    // terminal connects with it, and no window is added.
    let mut desk = Desktop::new().expect("the boot font loads");
    assert_eq!(send(&mut desk, change(&hash)), (NOTHING, vec![Fx::ClearHash]));
    let (_, fx) = send(&mut desk, resize(1280.0, 800.0));
    assert_eq!(opened(&fx).iter().map(|o| o.1).collect::<Vec<_>>(), [NODE]);
    assert_eq!(windows(&desk), Some(3));
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
        assert_eq!(send(&mut desk, fetched(bold, got)), (NOTHING, vec![]));
    }
    assert!(!has(&mut desk, FontId::SansBold));
    // With the shell up: a bad font is skipped, a good one fills the slot.
    for (got, want) in [(vec![1, 2, 3], false), (SEMI.to_vec(), true)] {
        let mut desk = desktop(false);
        assert_eq!(frame(&mut desk, 1.0), fetches);
        let (h, fx) = send(&mut desk, fetched(bold, Ok(got)));
        assert_eq!((h, fx, has(&mut desk, FontId::SansBold)), ((want, false), vec![], want));
    }
}

#[test]
fn what_the_shell_queues_while_drawing_goes_out_after_the_frame() {
    let hash = format!("node=8123&token={}", "ab".repeat(32));
    let mut desk = Desktop::boot(&hash).expect("the boot font loads");
    let (_, fx) = send(&mut desk, resize(1280.0, 800.0));
    let [(id, _)] = opened(&fx)[..] else {
        panic!("{fx:?}");
    };
    send(&mut desk, ws(id, Ws::Open));
    let ready = [&[2, 1, 80, 0, 24, 0][..], b"windows/powershell"].concat();
    let (_, fx) = send(&mut desk, ws(id, Ws::Data(ready)));
    let [Fx::WsSend { bytes: before, .. }] = &fx[..] else {
        panic!("{fx:?}");
    };
    // At 3 device pixels per CSS pixel the cell is narrower (23 px): the
    // frame changes the grid, and the RESIZE the terminal sends then goes
    // out after the frame, with the deferred fetches.
    let fx = frame(&mut desk, 3.0);
    let [Fx::Fetch { .. }, Fx::Fetch { .. }, Fx::WsSend { id: to, bytes }] = &fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!((*to, bytes[0], bytes.len()), (id, 4, 5));
    assert_ne!(bytes, before);
    assert_eq!(frame(&mut desk, 3.0), []);
}

#[test]
fn a_paired_terminal_reaches_the_node() {
    let mut desk = desktop(true);
    // The startup terminal connected at once; Alt+Enter opens another,
    // focused and typing, which connects too.
    let (h, fx) = send(&mut desk, key("Enter", "Enter", true, "a"));
    assert_eq!(h, (true, true));
    assert!(fx.contains(&Fx::TextInput(true)), "{fx:?}");
    let [(id, url)] = opened(&fx)[..] else {
        panic!("{fx:?}");
    };
    assert_eq!(url, NODE);
    // Open: HELLO with the token, and leaving now asks first. READY:
    // RESIZE to the window's grid.
    let sent = |b: &[u8]| vec![Fx::WsSend { id, bytes: b.to_vec() }];
    let (_, fx) = send(&mut desk, ws(id, Ws::Open));
    let hello = [&[1, 1][..], &TOKEN].concat();
    assert_eq!(fx, [sent(&hello), vec![Fx::GuardUnload(true)]].concat());
    let ready = [&[2, 1, 80, 0, 24, 0][..], b"windows/powershell"].concat();
    let (h, fx) = send(&mut desk, ws(id, Ws::Data(ready)));
    let [Fx::WsSend { id: to, bytes }] = &fx[..] else {
        panic!("{fx:?}");
    };
    assert_eq!((h.0, *to, bytes[0], bytes.len()), (true, id, 4, 5));
    // A typed key is left to the textarea; its text goes out as DATA, one
    // char as typed and several as a paste.
    let (h, fx) = send(&mut desk, key("KeyC", "c", true, ""));
    assert_eq!((h.1, fx), (false, vec![]));
    for text in ["c", "laude --help"] {
        let (_, fx) = send(&mut desk, Event::Text(text.into()));
        assert_eq!(fx, sent(&data(text.as_bytes())));
    }
    // Enter and Ctrl+C are keys, prevented and encoded.
    let (h, fx) = send(&mut desk, key("Enter", "Enter", true, ""));
    assert_eq!((h.1, fx), (true, sent(&data(b"\r"))));
    let (h, fx) = send(&mut desk, key("KeyC", "c", true, "c"));
    assert_eq!((h.1, fx), (true, sent(&data(b"\x03"))));
    // Output draws; an answer to a query goes back at once.
    let (h, fx) = send(&mut desk, ws(id, Ws::Data(data(b"hi \x1b[6n"))));
    assert_eq!((h.0, fx), (true, sent(&data(b"\x1b[1;4R"))));
    // EXIT closes the socket; later events for it go nowhere. The startup
    // terminal's socket never opened, so leaving no longer asks.
    let (_, fx) = send(&mut desk, ws(id, Ws::Data(vec![5, 0, 0, 0, 0])));
    assert_eq!(fx, [Fx::WsClose { id }, Fx::GuardUnload(false)]);
    assert_eq!(send(&mut desk, ws(id, Ws::Data(data(b"late")))), (NOTHING, vec![]));
}

#[test]
fn lazy_fonts_are_fetched_and_added_in_order() {
    let mut desk = Desktop::new().expect("the boot font loads");
    assert_eq!(send(&mut desk, fetched(1, Ok(vec![]))), (NOTHING, vec![]));
    let (_, fx) = send(&mut desk, resize(1280.0, 800.0));
    let [Fx::Fetch { id: a, .. }, Fx::Fetch { id: b, .. }, ..] = fx[..] else {
        panic!("{fx:?}");
    };
    // The deferred fonts are on their way too, under other ids.
    assert_eq!(frame(&mut desk, 1.0).len(), 2);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/lazy/");
    let font = std::fs::read(format!("{dir}symbols-a.ttf")).expect("lazy font");
    // The second failed first: nothing is added until the first arrives.
    let failed = fetched(b, Err("HTTP 404".into()));
    assert_eq!(send(&mut desk, failed), (NOTHING, vec![]));
    let got = send(&mut desk, fetched(a, Ok(font)));
    assert_eq!(got, ((true, false), vec![]));
    let shell = desk.shell.as_mut().expect("created");
    assert_eq!(shell.text_mut().fallback_count(), 1);
}

#[test]
fn a_release_on_the_focused_window_asks_for_the_keyboard_again() {
    let mut desk = desktop(false);
    send(&mut desk, key("Enter", "Enter", true, "a"));
    let layout = desk.shell.as_ref().expect("created").wm().layout();
    let r = layout.iter().find(|p| p.focused).expect("focused").rect;
    let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
    let (h, fx) = send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!((h.1, fx), (true, vec![]));
    let up = |d: &mut Desktop, x, y, button| send(d, Event::PointerUp { x, y, button }).1;
    assert_eq!(up(&mut desk, x, y, 0), [Fx::TextInput(true)]);
    // Not for another button, nor off the focused window (the outer gap).
    assert_eq!(up(&mut desk, x, y, 2), []);
    let (x, y) = (1.0, theme::PANEL_H + 1.0);
    send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!(up(&mut desk, x, y, 0), []);
    // Nor without text input: the welcome window wants none.
    let mut desk = desktop(false);
    let layout = desk.shell.as_ref().expect("created").wm().layout();
    let r = layout.iter().find(|p| p.focused).expect("focused").rect;
    let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
    send(&mut desk, Event::PointerDown { x, y, button: 0 });
    assert_eq!(up(&mut desk, x, y, 0), []);
}

#[test]
fn desktop_routes_events_through_the_shell() {
    let mut desk = desktop(false);
    let state = |d: &Desktop| d.shell.as_ref().map(|s| s.wm().state_hash());
    assert_eq!(send(&mut desk, key("Enter", "Enter", true, "a")).0, (true, true));
    let opened = state(&desk);
    assert_eq!(desk.shell.as_ref().map(|s| s.wm().layout().len()), Some(4));
    // Key-ups never reach the shell: releasing Alt+Q closes nothing.
    assert_eq!(send(&mut desk, key("KeyQ", "q", false, "a")).0, NOTHING);
    assert_eq!(state(&desk), opened);
    // AltGr+H/J/K/L types letters on some layouts: it goes to the new
    // terminal as text, and never moves the focus. Left Ctrl+Alt+L still
    // resizes.
    for code in "KeyH KeyJ KeyK KeyL".split(' ') {
        assert!(!send(&mut desk, key(code, "ł", true, "cag")).0.1, "{code}");
    }
    assert_eq!(state(&desk), opened);
    send(&mut desk, key("ArrowLeft", "ArrowLeft", true, "a"));
    let focused = state(&desk);
    assert_eq!(send(&mut desk, key("KeyL", "l", true, "ca")).0, (true, true));
    assert_ne!(state(&desk), focused);
    // Alt+Q closes the focused window.
    assert!(send(&mut desk, key("KeyQ", "q", true, "a")).0.1);
    assert_eq!(desk.shell.as_ref().map(|s| s.wm().layout().len()), Some(3));
    // Pointer events are consumed; resizes redraw but are not.
    let (x, y) = (640.0, 400.0);
    assert!(send(&mut desk, Event::PointerMove { x, y }).0.1);
    assert_eq!(send(&mut desk, resize(1024.0, 768.0)).0, (true, false));
    assert!(!send(&mut desk, Event::PointerLeave).0.1);
}
