use super::*;

/// Outcomes as (redraw, prevent_default).
const NOTHING: (bool, bool) = (false, false);
const REDRAW: (bool, bool) = (true, false);

/// A key event holding the modifiers named in `mods`: Shift Ctrl Alt Meta
/// AltGraph as `s c a m g`. Every key-down is an auto-repeat, a press too.
fn key(code: &str, down: bool, mods: &str) -> Event {
    let [shift, ctrl, alt, meta, altgr] = ['s', 'c', 'a', 'm', 'g'].map(|m| mods.contains(m));
    let (code, repeat) = (code.into(), down);
    Event::Key {
        code,
        down,
        repeat,
        shift,
        ctrl,
        alt,
        meta,
        altgr,
    }
}

fn resize(w: f32, h: f32) -> Event {
    Event::Resize { w, h, dpr: 2.0 }
}

/// What `desk` answers to `ev`, as (redraw, prevent_default).
fn send(desk: &mut Desktop, ev: Event) -> (bool, bool) {
    let h = desk.event(ev);
    (h.redraw, h.prevent_default)
}

#[test]
fn codes_map_to_keys() {
    let table = "Enter=Enter NumpadEnter=Enter Escape=Escape ArrowLeft=Left ArrowRight=Right \
        ArrowUp=Up ArrowDown=Down KeyH=H KeyJ=J KeyK=K KeyL=L KeyQ=Q KeyF=F KeyO=O \
        Digit0=Other Numpad0=Other Digit10=Other Digit=Other Numpad=Other NumpadAdd=Other \
        KeyA=Other keyh=Other enter=Other Tab=Other AltLeft=Other Space=Other =Other";
    for entry in table.split_whitespace() {
        let (code, want) = entry.split_once('=').expect("code=Key");
        assert_eq!(format!("{:?}", key_of(code)), want, "{code:?}");
    }
    for n in 1..=9 {
        assert_eq!(key_of(&format!("Digit{n}")), Key::Digit(n));
        assert_eq!(key_of(&format!("Numpad{n}")), Key::Digit(n));
    }
}

#[test]
fn events_map_to_inputs() {
    assert_eq!(input_of(&key("KeyQ", false, "a")), None);
    let (x, y, button) = (3.0, 4.0, 2);
    let moved = Input::PointerMove { x, y };
    assert_eq!(input_of(&Event::PointerMove { x, y }), Some(moved));
    let down = Input::PointerDown { x, y, button };
    assert_eq!(input_of(&Event::PointerDown { x, y, button }), Some(down));
    let up = Input::PointerUp { x, y, button };
    assert_eq!(input_of(&Event::PointerUp { x, y, button }), Some(up));
    assert_eq!(input_of(&Event::PointerLeave), Some(Input::PointerLeave));
    let sized = Input::Resize { w: 8.0, h: 6.0 };
    assert_eq!(input_of(&resize(8.0, 6.0)), Some(sized));
}

#[test]
fn modifiers_pass_through_but_altgr_is_not_ctrl_alt() {
    // Held=seen, in `key`'s letters. Chrome and Edge on Windows send AltGr as
    // "cag"; left Ctrl+Alt ("ca") and AltGraph without both keep what they hold.
    for entry in "sm=sm cag= scamg=sm ca=ca sca=sca ag=a cg=c g= =".split(' ') {
        let (held, want) = entry.split_once('=').expect("held=seen");
        let Some(Input::Key { key: k, mods: m }) = input_of(&key("KeyQ", true, held)) else {
            panic!("{held}");
        };
        let seen = [(m.shift, 's'), (m.ctrl, 'c'), (m.alt, 'a'), (m.meta, 'm')];
        let seen: String = seen.iter().filter_map(|&(on, c)| on.then_some(c)).collect();
        assert_eq!((k, seen.as_str()), (Key::Q, want), "{held}");
    }
}

#[test]
fn key_ups_prevent_only_modifiers() {
    for code in "AltLeft AltRight MetaLeft MetaRight".split(' ') {
        let h = key_up(&key(code, false, ""));
        assert!(h.prevent_default && !h.redraw, "{code}");
    }
    for code in "KeyQ Enter ControlLeft ShiftLeft OSLeft Digit1".split(' ') {
        assert_eq!(key_up(&key(code, false, "a")), Handled::default(), "{code}");
    }
}

#[test]
fn shell_starts_at_the_first_usable_size() {
    let mut desk = Desktop::default();
    assert_eq!(send(&mut desk, key("Enter", true, "a")), NOTHING);
    assert_eq!(send(&mut desk, Event::PointerLeave), NOTHING);
    for (w, h) in [(0.0, 0.0), (800.0, 36.5), (0.5, 600.0), (f32::NAN, 600.0)] {
        assert_eq!(send(&mut desk, resize(w, h)), NOTHING, "{w}x{h}");
        assert!(desk.shell.is_none(), "{w}x{h}");
    }
    assert_eq!(send(&mut desk, resize(1280.0, 800.0)), REDRAW);
    // The startup windows split as they would at that size from the start.
    let (got, fresh) = (desk.shell.expect("created"), Shell::new(1280.0, 800.0));
    assert_eq!(got.wm().state_hash(), fresh.wm().state_hash());
    assert_eq!(got.wm().layout(), fresh.wm().layout());
}

#[test]
fn desktop_routes_events_through_the_shell() {
    let mut desk = Desktop::default();
    send(&mut desk, resize(1280.0, 800.0));
    let state = |d: &Desktop| d.shell.as_ref().map(|s| s.wm().state_hash());
    assert_eq!(send(&mut desk, key("Enter", true, "a")), (true, true));
    let opened = state(&desk);
    // A key without the modifier belongs to apps: no frame, not prevented.
    assert_eq!(send(&mut desk, key("KeyQ", true, "")), NOTHING);
    // Key-ups never reach the shell: releasing Alt+Q closes nothing.
    assert_eq!(send(&mut desk, key("KeyQ", false, "a")), NOTHING);
    assert_eq!(send(&mut desk, key("AltLeft", false, "")), (false, true));
    assert_eq!(state(&desk), opened);
    // AltGr+H/J/K/L types letters on some layouts: it goes to apps, and
    // never resizes. Left Ctrl+Alt+L still does.
    send(&mut desk, key("ArrowLeft", true, "a"));
    let focused = state(&desk);
    for code in "KeyH KeyJ KeyK KeyL".split(' ') {
        assert_eq!(send(&mut desk, key(code, true, "cag")), NOTHING, "{code}");
    }
    assert_eq!(state(&desk), focused);
    assert_eq!(send(&mut desk, key("KeyL", true, "ca")), (true, true));
    // Pointer events are consumed; resizes redraw but are not.
    let (x, y) = (640.0, 400.0);
    assert!(send(&mut desk, Event::PointerMove { x, y }).1);
    assert_eq!(send(&mut desk, resize(1024.0, 768.0)), REDRAW);
    assert!(!send(&mut desk, Event::PointerLeave).1);
    assert_eq!(desk.shell.map(|s| s.wm().layout().len()), Some(4));
}
