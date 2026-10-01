use super::ctl::is_relative_url;
use super::io::{http_error, inserts_text};
use super::proc::{RING_AT, RING_BYTES, ring_spans};
use super::render::{ATTRIBS, MIN_CAPACITY, backing_size, band_bytes, clear_rgb, grow_capacity};
use super::*;
use gfx::{INSTANCE_BYTES, Rgba};
use web_sys::WebGl2RenderingContext as Gl;

#[test]
fn renderer_math() {
    let sizes = [((800.0, 600.0), 1.0), ((800.0, 600.0), 2.0), ((333.3, 100.2), 1.5)];
    let more = [((0.0, 10.0), 2.0), ((f32::NAN, -5.0), 1.0)];
    let got = sizes.iter().chain(&more).map(|&(css, dpr)| backing_size(css, dpr));
    assert!(got.eq([(800, 600), (1600, 1200), (500, 150), (1, 20), (1, 1)]));
    let needs = [1, MIN_CAPACITY, MIN_CAPACITY + 1, 1000 * INSTANCE_BYTES, 1 << 20, usize::MAX];
    let caps = [MIN_CAPACITY, MIN_CAPACITY, 32768, 131072, 1 << 20, usize::MAX];
    assert_eq!(needs.map(grow_capacity), caps);
    // The clear color is opaque sRGB as GL floats.
    assert_eq!(clear_rgb(Rgba::hex(0xff0000)), [1.0, 0.0, 0.0]);
    assert_eq!(clear_rgb(Rgba(0, 51, 255, 0)), [0.0, 0.2, 1.0]);
}

#[test]
fn attributes_match_the_gfx_layout() {
    // vec4 f32 x2, four normalized bytes, vec4 f32 x2, four normalized bytes:
    // each attribute ends where the next starts, and the last fills the
    // instance.
    let sizes = [16, 16, 4, 16, 16, 4];
    let names = ["a_rect", "a_params", "a_color", "a_clip", "a_uv", "a_color2"];
    let mut end = 0;
    for (i, &(loc, ty, normalized, offset)) in ATTRIBS.iter().enumerate() {
        assert_eq!((loc as usize, offset), (i, end));
        let bytes = sizes[i] == 4;
        assert_eq!((ty == Gl::UNSIGNED_BYTE, normalized), (bytes, bytes));
        end += sizes[i];
        let decl = format!("layout(location = {i}) in vec4 {};", names[i]);
        assert!(gfx::VERTEX_SHADER.contains(&decl), "missing {decl}");
    }
    assert_eq!(end as usize, INSTANCE_BYTES);
    let both = [gfx::VERTEX_SHADER, gfx::FRAGMENT_SHADER].concat();
    for u in ["u_atlas;", "u_atlas_size;", "u_viewport;", "u_dpr;"] {
        assert!(both.contains(u), "missing uniform {u}");
    }
}

#[test]
fn dirty_bands_upload_whole_rows() {
    // A 64-wide atlas of 32 rows: rows [1, 3) are bytes 64..192.
    let len = 64 * 32;
    assert_eq!(band_bytes((1, 3), 64, len), Some((1, 2, 64..192)));
    assert_eq!(band_bytes((0, 32), 64, len), Some((0, 32, 0..2048)));
    for (band, len) in [((5, 5), len), ((30, 33), len), ((0, 2), 100), ((3, 1), len)] {
        assert_eq!(band_bytes(band, 64, len), None, "{band:?}");
    }
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
    let mut ctl = Ctl::default();
    assert!(ctl.effects().is_empty());
    let (a, b) = ("fonts/symbols-a.ttf".to_owned(), "fonts/symbols-b.ttf".to_owned());
    ctl.set_text_input(true);
    ctl.fetch(2, &a);
    ctl.request_frame();
    ctl.set_cursor("pointer");
    ctl.storage_set("theme", "dusk");
    ctl.request_frame(); // asked once already: one frame
    ctl.set_cursor("nwse-resize"); // the last cursor wins
    ctl.fetch(1, &b);
    ctl.storage_set("theme", "dawn");
    ctl.storage_set("dock", "left");
    ctl.abort(3);
    ctl.set_text_input(false);
    ctl.stream(3, "https://h/p", vec![], vec![1]);
    ctl.frame_in(125);
    let store = |k: &str, v: &str| Effect::Store { key: k.to_owned(), value: v.to_owned() };
    #[rustfmt::skip]
    let want = [Effect::TextInput(true), Effect::Fetch { id: 2, url: a }, Effect::RequestFrame,
        store("theme", "dusk"), Effect::Cursor("nwse-resize"), Effect::Fetch { id: 1, url: b },
        store("theme", "dawn"), store("dock", "left"), Effect::Abort(3), Effect::TextInput(false),
        Effect::Stream { id: 3, url: "https://h/p".into(), headers: vec![], body: vec![1] },
        Effect::FrameIn(125)];
    assert_eq!(ctl.effects(), want);
    // A queued write reads back, the newest first; natively nothing else is
    // stored, the clocks are neutral and the page is not isolated.
    let got = ["theme", "dock", "Theme"].map(|k| ctl.storage_get(k));
    assert_eq!(got, [Some("dawn".into()), Some("left".into()), None]);
    assert_eq!(Ctl::default().storage_get("theme"), None);
    let neutral = (ctl.monotonic_ms(), ctl.local_time(), ctl.isolated(), ctl.reduced_motion());
    assert_eq!(neutral, (0.0, LocalTime::EPOCH, false, false));
}

#[test]
fn the_ring_drains_in_at_most_two_spans() {
    // kernel::wire's layout: 16 words, a 64 KiB payload, a 64 KiB ring.
    assert_eq!((RING_AT, RING_AT + RING_BYTES), (64 + 65_536, 131_136));
    // (tail, head, end of the first span, length of the second): counts wrap
    // at 2^32, a multiple of the ring; more than a ring (a broken worker)
    // reads one ring, never past it.
    let r = RING_BYTES;
    let cases = [(0, 0, 0, 0), (3, 8, 8, 0), (r - 2, r + 3, r, 3), (u32::MAX - 1, 2, r, 2)];
    for (tail, head, end, wrapped) in cases.into_iter().chain([(7, 7 + r, r, 7), (7, 6, r, 7)]) {
        assert_eq!(ring_spans(tail, head), [(tail % r, end), (0, wrapped)], "{tail}..{head}");
    }
}

#[test]
fn local_time_comes_from_date_getters() {
    // 2026-09-30 14:07, a Wednesday: Date's month counts from 0.
    let t = LocalTime::from_js(2026, 8, 30, 3, 14, 7);
    assert_eq!(t, LocalTime { year: 2026, month: 9, day: 30, weekday: 3, hour: 14, minute: 7 });
    // Out-of-range values (an invalid Date reads NaN, which arrives as 0; a
    // far-future clock) land inside each field's range.
    let zero = LocalTime::from_js(0, 0, 0, 0, 0, 0);
    assert_eq!(zero, LocalTime { year: 0, month: 1, day: 1, weekday: 0, hour: 0, minute: 0 });
    let huge = LocalTime::from_js(275_760, 99, 99, 99, 99, 99);
    let max = LocalTime { year: u16::MAX, month: 12, day: 31, weekday: 6, hour: 23, minute: 59 };
    assert_eq!(huge, max);
}

#[test]
fn fetch_takes_only_same_origin_relative_urls() {
    let ok = "fonts/a.ttf|/fonts/a.ttf|./a|../a|?q=1|#x||a/b:c|/x?y=http://z|#a:b|?a:b|\
        caf\u{e9}/menu|a b";
    for u in ok.split('|') {
        assert!(is_relative_url(u), "{u:?} should be allowed");
    }
    let bad = "http://example.com/a|https:a|//example.com/a|/\\example.com|\\\\example.com|\
        /\t/example.com|\n//example.com| //example.com|javascript:alert(1)|data:text/plain,x|\
        blob:x|C:foo|a\u{7f}b|caf\u{e9}:x";
    for u in bad.split('|') {
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
    // code/key/mods=paste: Cyrillic (KeyV), Dvorak (V on Period; its Ctrl+K
    // on KeyV is the app's key, prevented so the browser does not search).
    let table = "KeyV/v/c=1 KeyV/V/cs=1 KeyV/v/m=1 KeyV/\u{43c}/c=1 Period/v/c=1 Period/V/cs=1 \
        KeyV/k/c=0 KeyV/K/m=0 KeyV/Unidentified/c=1 Insert/Insert/s=1 KeyV/v/=0 KeyV/v/s=0 \
        KeyV/v/ca=0 KeyC/c/c=0 Insert/Insert/cs=0";
    for entry in table.split(' ') {
        let (ev, want) = entry.split_once('=').unwrap();
        let f: Vec<&str> = ev.split('/').collect();
        let mods = ['s', 'c', 'a', 'm'].map(|m| f[2].contains(m));
        assert_eq!(is_paste(f[0], f[1], mods), want == "1", "{entry}");
    }
}

#[test]
fn small_helpers() {
    let dprs = [2.0, 1.25, 0.0, -1.0, f64::NAN, f64::INFINITY].map(sane_dpr);
    assert_eq!(dprs, [2.0, 1.25, 1.0, 1.0, 1.0, 1.0]);
    let hits = "?debug=debug ?x=1&debug=1=debug ?a=debug=debug debug=debug xy=y";
    let misses = "=debug ?dbg=1=debug debu=debug =d Debug=debug";
    for (table, want) in [(hits, true), (misses, false)] {
        for e in table.split(' ') {
            let (hay, needle) = e.rsplit_once('=').unwrap();
            assert_eq!(has(hay, needle), want, "{e}");
        }
    }
    assert_eq!([0, 2, -1, 300].map(button_u8), [0, 2, 0, 0]);
    let codes = [0, 7, 10, 200, 404, 503, u16::MAX].map(http_error);
    let want = ["0", "7", "10", "200", "404", "503", "65535"];
    assert_eq!(codes, want.map(|n| ["HTTP ", n].concat()));
    // Wheel deltas in CSS pixels: pixels, lines, pages, unknown modes.
    let wheels = [(-120.0, 0), (3.0, 1), (1.0, 2), (0.5, 7), (f64::NAN, 0), (f64::MAX, 1)];
    assert_eq!(wheels.map(|(d, m)| wheel_px(d, m, 600.0)), [-120.0, 48.0, 600.0, 0.5, 0.0, 0.0]);
    // The minute timer lands just past the boundary; out-of-range fields (a
    // leap second) still give a short, positive wait.
    let waits = [(0, 0), (59, 999), (30, 500), (60, 5000)].map(|(s, ms)| ms_to_next_minute(s, ms));
    assert_eq!(waits, [60_010, 11, 29_510, 11]);
}
