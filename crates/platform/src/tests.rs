use super::ctl::{is_relative_url, strip_hash};
use super::io::{http_error, inserts_text};
use super::render::{ATTRIBS, MIN_CAPACITY, backing_size, band_bytes, clear_rgb, grow_capacity};
use super::*;
use gfx::{INSTANCE_BYTES, Rgba};

#[test]
fn backing_store_is_rounded_physical_pixels() {
    assert_eq!(backing_size((800.0, 600.0), 1.0), (800, 600));
    assert_eq!(backing_size((800.0, 600.0), 2.0), (1600, 1200));
    assert_eq!(backing_size((333.3, 100.2), 1.5), (500, 150));
    assert_eq!(backing_size((0.0, 10.0), 2.0), (1, 20));
    assert_eq!(backing_size((f32::NAN, -5.0), 1.0), (1, 1));
}

#[test]
fn buffer_grows_by_powers_of_two() {
    assert_eq!(grow_capacity(1), MIN_CAPACITY);
    assert_eq!(grow_capacity(MIN_CAPACITY), MIN_CAPACITY);
    assert_eq!(grow_capacity(MIN_CAPACITY + 1), 32768);
    assert_eq!(grow_capacity(1000 * INSTANCE_BYTES), 131072);
    assert_eq!(grow_capacity(1 << 20), 1 << 20);
    assert_eq!(grow_capacity(usize::MAX), usize::MAX);
}

#[test]
fn attributes_match_the_gfx_layout() {
    let got: Vec<(u32, i32)> = ATTRIBS.iter().map(|a| (a.0, a.3)).collect();
    assert_eq!(got, [(0, 0), (1, 16), (2, 32), (3, 36), (4, 52)]);
    // vec4 f32, vec4 f32, four normalized bytes, vec4 f32, vec4 f32: each
    // attribute ends where the next starts, and the last fills the instance.
    let sizes = [16, 16, 4, 16, 16];
    for (a, next) in ATTRIBS.iter().zip(ATTRIBS.iter().skip(1)) {
        assert_eq!(a.3 + sizes[a.0 as usize], next.3);
    }
    assert_eq!(ATTRIBS[4].3 as usize + 16, INSTANCE_BYTES);
    let normalized: Vec<bool> = ATTRIBS.iter().map(|a| a.2).collect();
    assert_eq!(normalized, [false, false, true, false, false]);
    let names = ["a_rect", "a_params", "a_color", "a_clip", "a_uv"];
    for (loc, name) in names.iter().enumerate() {
        let decl = format!("layout(location = {loc}) in vec4 {name};");
        assert!(gfx::VERTEX_SHADER.contains(&decl), "missing {decl}");
    }
    for u in ["u_atlas;", "u_atlas_size;", "u_viewport;", "u_dpr;"] {
        let both = [gfx::VERTEX_SHADER, gfx::FRAGMENT_SHADER].concat();
        assert!(both.contains(u), "missing uniform {u}");
    }
}

#[test]
fn dirty_bands_upload_whole_rows() {
    // A 64-wide atlas of 32 rows: rows [1, 3) are bytes 64..192.
    assert_eq!(band_bytes((1, 3), 64, 64 * 32), Some((1, 2, 64..192)));
    assert_eq!(band_bytes((0, 32), 64, 64 * 32), Some((0, 32, 0..2048)));
    assert_eq!(band_bytes((5, 5), 64, 64 * 32), None);
    assert_eq!(band_bytes((30, 33), 64, 64 * 32), None);
    assert_eq!(band_bytes((0, 2), 64, 100), None);
    assert_eq!(band_bytes((3, 1), 64, 64 * 32), None);
    // The band the gfx atlas reports lines up with its pixels.
    let mut atlas = gfx::Atlas::new(16, 8);
    let _ = atlas.take_dirty();
    let (x, y) = atlas.alloc(2, 2).unwrap();
    assert!(atlas.write(x, y, 2, 2, &[7; 4]));
    let (y0, rows, bytes) = band_bytes(atlas.take_dirty().unwrap(), 16, 128).unwrap();
    assert_eq!((y0, rows), (1, 2));
    assert_eq!(atlas.pixels()[bytes].iter().filter(|&&b| b == 7).count(), 4);
}

#[test]
fn a_second_pointer_cannot_steal_or_end_a_press() {
    use Ptr::{Down, Leave, Move, Up};
    let mut pressed = None;
    let mut heard = |id, primary, kind| {
        let next = gate(pressed, id, primary, kind);
        pressed = next.unwrap_or(pressed);
        next.is_some()
    };
    // Finger 1 drags; finger 2 (not primary) and mouse 3 cut in.
    assert!(heard(1, true, Down) && heard(1, true, Move));
    assert!(!heard(2, false, Down) && !heard(2, false, Move));
    assert!(!heard(3, true, Down) && !heard(3, true, Move));
    assert!(!heard(2, false, Up) && !heard(2, false, Leave));
    assert!(!heard(3, true, Up) && heard(1, true, Move));
    // Finger 1 lifts; the mouse is heard again and takes its own press.
    assert!(heard(1, true, Up) && heard(1, true, Leave));
    assert!(heard(3, true, Move) && heard(3, true, Down));
    assert!(!heard(1, true, Down) && heard(3, true, Up));
    assert!(!heard(2, false, Move) && heard(3, true, Leave));
}

#[test]
fn ctl_queues_requests_in_order() {
    let mut ctl = Ctl::new();
    assert!(ctl.effects().is_empty());
    let (id, url, bytes) = (7, "ws://127.0.0.1:7777/pty".to_owned(), b"ls\r".to_vec());
    let font = "fonts/lazy/symbols-a.ttf".to_owned();
    ctl.set_text_input(true);
    ctl.ws_open(id, &url);
    ctl.ws_send(id, &bytes);
    ctl.fetch(2, &font);
    ctl.clear_location_hash();
    ctl.ws_close(id);
    ctl.set_text_input(false);
    ctl.guard_unload(true);
    ctl.guard_unload(false);
    let want = [
        Effect::TextInput(true),
        Effect::WsOpen { id, url },
        Effect::WsSend { id, bytes },
        Effect::Fetch { id: 2, url: font },
        Effect::ClearHash,
        Effect::WsClose { id },
        Effect::TextInput(false),
        Effect::GuardUnload(true),
        Effect::GuardUnload(false),
    ];
    assert_eq!((ctl.effects(), ctl.clone().into_effects()), (&want[..], want.to_vec()));
    // Reads are live in the browser and neutral natively, as is taking the
    // hash before the page starts.
    assert_eq!((ctl.location_hash(), ctl.now_ms(), ctl.local_minutes()), (String::new(), 0.0, 0));
    assert_eq!(take_location_hash(), "");
}

#[test]
fn fetch_takes_only_same_origin_relative_urls() {
    let ok = [
        "fonts/a.ttf",
        "/fonts/a.ttf",
        "./a",
        "../a",
        "?q=1",
        "#x",
        "",
        "a/b:c",
        "/x?y=http://z",
        "#a:b",
        "?a:b",
        "caf\u{e9}/menu",
        "a b",
    ];
    for u in ok {
        assert!(is_relative_url(u), "{u:?} should be allowed");
    }
    let bad = [
        "http://example.com/a",
        "https:a",
        "//example.com/a",
        "/\\example.com",
        "\\\\example.com",
        "/\t/example.com",
        "\n//example.com",
        " //example.com",
        "javascript:alert(1)",
        "data:text/plain,x",
        "blob:x",
        "C:foo",
        "a\u{7f}b",
        "caf\u{e9}:x",
    ];
    for u in bad {
        assert!(!is_relative_url(u), "{u:?} should be refused");
    }
}

#[test]
fn text_comes_from_inserts_but_not_compositions() {
    for t in "insertText insertFromPaste insertLineBreak insertReplacementText".split(' ') {
        assert!(inserts_text(t), "{t}");
    }
    // The last is "".
    let not = "insertCompositionText insertFromComposition deleteCompositionText \
        deleteContentBackward historyUndo ";
    for t in not.split(' ') {
        assert!(!inserts_text(t), "{t}");
    }
}

#[test]
fn paste_shortcuts_are_never_prevented() {
    let mods = |s: &str| ["shift", "ctrl", "alt", "meta"].map(|m| s.contains(m));
    assert!(is_paste("KeyV", "v", mods("ctrl")));
    assert!(is_paste("KeyV", "V", mods("ctrl shift")));
    assert!(is_paste("KeyV", "v", mods("meta")));
    assert!(is_paste("KeyV", "\u{43c}", mods("ctrl"))); // Cyrillic layout
    assert!(is_paste("Period", "v", mods("ctrl"))); // Dvorak
    assert!(is_paste("Period", "V", mods("ctrl shift")));
    // Dvorak's Ctrl+K sits on KeyV: it is the app's key, not a paste, so
    // it is prevented and the browser does not search instead.
    assert!(!is_paste("KeyV", "k", mods("ctrl")));
    assert!(!is_paste("KeyV", "K", mods("meta")));
    assert!(is_paste("KeyV", "Unidentified", mods("ctrl")));
    assert!(is_paste("Insert", "Insert", mods("shift")));
    assert!(!is_paste("KeyV", "v", mods("")));
    assert!(!is_paste("KeyV", "v", mods("shift")));
    assert!(!is_paste("KeyV", "v", mods("ctrl alt")));
    assert!(!is_paste("KeyC", "c", mods("ctrl")));
    assert!(!is_paste("Insert", "Insert", mods("ctrl shift")));
}

#[test]
fn small_helpers() {
    let dprs = [2.0, 1.25, 0.0, -1.0, f64::NAN, f64::INFINITY].map(sane_dpr);
    assert_eq!(dprs, [2.0, 1.25, 1.0, 1.0, 1.0, 1.0]);
    // The clear color is opaque sRGB as GL floats.
    assert_eq!(clear_rgb(Rgba::hex(0xff0000)), [1.0, 0.0, 0.0]);
    assert_eq!(clear_rgb(Rgba(0, 51, 255, 0)), [0.0, 0.2, 1.0]);
    assert!(is_debug("?debug") && is_debug("?x=1&debug=1"));
    assert!(!is_debug("") && !is_debug("?dbg=1"));
    assert!(has("?a=debug", "debug") && has("debug", "debug") && has("xy", "y"));
    assert!(!has("debu", "debug") && !has("", "d") && !has("Debug", "debug"));
    assert_eq!([0, 2, -1, 300].map(button_u8), [0, 2, 0, 0]);
    assert_eq!(strip_hash("#/apps/term"), "/apps/term");
    assert_eq!(strip_hash(""), "");
    let codes = [0, 7, 10, 200, 404, 503, u16::MAX].map(http_error);
    let want = ["0", "7", "10", "200", "404", "503", "65535"];
    assert_eq!(codes, want.map(|n| ["HTTP ", n].concat()));
}

#[test]
fn hash_changes_carry_the_hash_without_its_mark() {
    let ev = Event::HashChange(strip_hash("#node=7777&token=ab").to_owned());
    assert_eq!(ev, Event::HashChange("node=7777&token=ab".into()));
    assert_ne!(ev, Event::HashChange(String::new()));
}

#[test]
fn wheel_deltas_become_css_pixels() {
    assert_eq!(wheel_px(-120.0, 0, 600.0), -120.0);
    assert_eq!(wheel_px(3.0, 1, 600.0), 48.0);
    assert_eq!(wheel_px(1.0, 2, 600.0), 600.0);
    assert_eq!(wheel_px(0.5, 7, 600.0), 0.5);
    assert_eq!(wheel_px(f64::NAN, 0, 600.0), 0.0);
    assert_eq!(wheel_px(f64::MAX, 1, 600.0), 0.0);
}

#[test]
fn the_minute_timer_lands_just_past_the_boundary() {
    assert_eq!(ms_to_next_minute(0, 0), 60_010);
    assert_eq!(ms_to_next_minute(59, 999), 11);
    assert_eq!(ms_to_next_minute(30, 500), 29_510);
    // Out-of-range fields (a leap second) still give a short, positive wait.
    assert_eq!(ms_to_next_minute(60, 5000), 11);
}
