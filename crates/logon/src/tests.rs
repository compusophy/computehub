use super::*;
use gfx::Kind;
use record::Timing;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn text(dpr: f32) -> TextSystem {
    let mut t = TextSystem::new(SANS.to_vec()).expect("the boot font loads");
    t.set_dpr(dpr);
    t
}

/// `localStorage` holding `kv`.
fn store<'a>(kv: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| kv.iter().find(|p| p.0 == k).map(|p| p.1.to_string())
}

fn load(name: &str, at: [f64; 4]) -> Timing {
    (name.to_string(), at)
}

/// The page 0-7 ms, os.js 22-42, the wasm 45-57 (all over the network), the first frame at 98.
fn timings() -> Vec<Timing> {
    vec![
        load("", [0.0, 7.0, 831.0, 531.0]),
        load("https://h/os.js", [22.0, 42.0, 7586.0, 7286.0]),
        load("https://h/os_bg.wasm", [45.0, 57.0, 195_531.0, 195_231.0]),
    ]
}

/// The fixture's start, with the fonts landed at 103 and 104 (`ok`) if `fonts`.
fn started(fonts: bool, ok: bool) -> Record {
    let mut r = Record::default();
    r.first(98.0, timings());
    if fonts {
        let mut t = timings();
        t.push(load("https://h/fonts/deferred/Inter-SemiBold.ttf", [98.0, 103.0, 8485.0, 8185.0]));
        let mono = "https://h/fonts/deferred/JetBrainsMono-Regular.ttf";
        t.push(load(mono, [98.0, 104.0, 16_363.0, 16_063.0]));
        assert_eq!(r.font(0, ok, 103.0, t.clone()), None);
        let note = r.font(1, true, 104.0, t);
        assert_eq!(note.as_deref(), Some("boot ready 104 ff 41 net 222"));
    }
    r
}

#[test]
fn the_record_tells_the_real_start_stage_by_stage() {
    // Before the first frame nothing is known; then the stages over, gaps kept (7 to 22 ms the
    // browser parsed), the fonts pending; Ready is the last end, not a sum.
    assert_eq!((Record::default().caption(), Record::default().stages()), (String::new(), vec![]));
    let r = started(false, true);
    let spans: Vec<_> = r.stages().iter().map(|s| (s.i, s.from, s.to, s.learned)).collect();
    assert_eq!(spans, [(0, 0.0, 7.0, 98.0), (1, 22.0, 57.0, 98.0), (2, 57.0, 98.0, 98.0)]);
    assert_eq!(
        (r.caption().as_str(), r.ready(), r.scale(500.0)),
        ("Loading fonts\u{2026}", 98.0, 98.0)
    );
    let r = started(true, true);
    let fonts = r.stages()[3];
    assert_eq!((fonts.from, fonts.to, fonts.bytes), (98.0, 104.0, Some(24_248.0)));
    assert_eq!((r.caption().as_str(), r.ready()), ("Ready in 104 ms \u{b7} 222 KB", 104.0));
    // The line rescales from the first frame's end to the fonts' as their segment fades in.
    assert!(r.scale(104.0) == 98.0 && r.scale(104.0 + record::FADE_MS) == 104.0);
    let rows: Vec<String> = r.rows().iter().map(|r| r.join("|")).collect();
    let want = [
        "Page|0.5 KB|7 ms",
        "compusophyOS|198 KB|35 ms",
        "Start|budget 100|41 ms",
        "Fonts|24 KB|6 ms",
    ];
    assert_eq!(rows, [&want[..], &["Ready||104 ms"]].concat());
    // A font that failed says so (the name then shows in Regular); all from the cache says that;
    // with no timings, the time alone, and the rows say nothing they do not know.
    assert_eq!(started(true, false).caption(), "Ready in 104 ms \u{b7} fonts did not load");
    let mut cached = Record::default();
    cached.first(
        98.0,
        timings().into_iter().map(|(n, t)| load(&n, [t[0], t[1], 0.0, t[3]])).collect(),
    );
    cached.fonts = [Some((true, 1100.0)), Some((true, 1205.0))];
    assert_eq!(cached.caption(), "Ready in 1.20 s \u{b7} from cache");
    assert_eq!(cached.rows()[1][1], "cache");
    let mut blind = Record::default();
    blind.first(98.0, vec![]);
    blind.fonts = [Some((true, 120.0)), Some((true, 130.0))];
    let rows: Vec<String> = blind.rows().iter().map(|r| r.join("|")).collect();
    assert_eq!(
        (blind.caption().as_str(), rows[0].as_str()),
        ("Ready in 130 ms", "Page|\u{2014}|\u{2014}")
    );
    // Numbers in plain integer text.
    let fmt = |f: fn(&mut String, f64), v: f64| {
        let mut s = String::new();
        f(&mut s, v);
        s
    };
    let ms = [0.0, 412.4, 999.0, 1000.0, 1214.0, 61_000.0].map(|v| fmt(record::ms, v));
    assert_eq!(ms, ["0 ms", "412 ms", "999 ms", "1.00 s", "1.21 s", "61.00 s"]);
    let kb = [531.0, 7475.2, 10_189.0, 223_232.0].map(|v| fmt(record::kb, v));
    assert_eq!(kb, ["0.5 KB", "7.3 KB", "10 KB", "218 KB"]);
    assert_eq!(record::home_note(212 * 1024, 18.4), "home 212 18");
}

/// The welcome at `size` (`dpr`), drawn once at 1000 ms (its start known: segments in) with the fixture.
fn drawn(size: (f32, f32), dpr: f32, kv: &[(&str, &str)]) -> (Logon, DrawList, TextSystem) {
    let (mut l, mut list, mut t) = (Logon::new(size, &store(kv)), DrawList::new(), text(dpr));
    l.draw(&mut list, &mut t, &started(true, true), (1000.0, false));
    (l, list, t)
}

#[test]
fn the_hairline_keeps_gaps_and_lands_on_device_pixels() {
    for dpr in [1.0, 2.0, 3.5] {
        let (l, list, _) = drawn((1280.0, 800.0), dpr, &[(SEEN, "1")]);
        // The segments: the text_dim fills on the track's line, as wide as the mark (233).
        let dim = l.theme.text_dim;
        let segs: Vec<[f32; 4]> = list
            .instances()
            .iter()
            .filter(|i| i.kind == Kind::Fill as u32 as f32 && i.color == dim && i.rect[3] < 2.0)
            .map(|i| i.rect)
            .collect();
        assert_eq!(segs.len(), 4, "{dpr}");
        let x0 = ((1280.0 - 233.0) / 2.0 * dpr).round() / dpr;
        for (s, (from, to)) in
            segs.iter().zip([(0.0, 7.0), (22.0, 57.0), (57.0, 98.0), (98.0, 104.0)])
        {
            let on = |v: f32| ((v * dpr).round() - v * dpr).abs() < 1e-3;
            assert!(on(s[0]) && on(s[0] + s[2]) && on(s[1]), "{dpr} {s:?}");
            assert!((s[0] - (x0 + from / 104.0 * 233.0)).abs() <= 1.0 / dpr + 1e-3, "{dpr} {s:?}");
            assert!((s[0] + s[2] - (x0 + to / 104.0 * 233.0)).abs() <= 1.0 / dpr + 1e-3, "{s:?}");
        }
        // The gap the browser spent parsing stays: about 15% of the line.
        assert!(segs[1][0] - (segs[0][0] + segs[0][2]) > 0.13 * 233.0, "{dpr}");
    }
}

/// What `l` does with `inputs` at `now`: the outs, in order.
fn feed(l: &mut Logon, inputs: &[Input]) -> Vec<Out> {
    inputs.iter().flat_map(|i| l.input(i, 50.0).1).collect()
}

fn tap(x: f32, y: f32) -> [Input; 2] {
    [Input::PointerDown { x, y, button: 0, touch: true }, Input::PointerUp { x, y, button: 0 }]
}

fn key(key: Key) -> Input {
    Input::Key { key, mods: ui::Mods::default() }
}

/// The ins of a return to profile 0 (no hello: the device was seen).
fn signed_in() -> Vec<Out> {
    vec![Out::Session("0".into()), Out::SignIn(0)]
}

#[test]
fn a_return_starts_with_a_tap_enter_or_space_and_the_band_opens_the_card() {
    let kv = [(SEEN, "1"), ("compusophy.theme", "dawn")];
    for start in [vec![key(Key::Enter)], vec![key(Key::Space)], tap(640.0, 300.0).to_vec()] {
        let (mut l, ..) = drawn((1280.0, 800.0), 1.0, &kv);
        assert_eq!((l.theme.name, l.state), ("Dawn", State::Ready));
        assert_eq!(feed(&mut l, &start), signed_in(), "{start:?}");
        assert!(l.leaving() && !l.gone(50.0 + FLIGHT - 1.0) && l.gone(50.0 + FLIGHT));
    }
    // A tap on the record's band opens its card and signs no one in; a tap outside closes it,
    // and only that; Tab then Enter opens it too, Escape closes it. No keyboard ever shows.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let band = l.hits.iter().find(|h| h.1 == Target::Record).expect("a band").0;
    assert!(band.h >= TOUCH && band.y + band.h <= 794.0);
    let mid = (band.x + band.w / 2.0, band.y + band.h / 2.0);
    let mut answers = vec![];
    for i in
        tap(mid.0, mid.1).iter().chain(&tap(200.0, 100.0)).chain(&[key(Key::Tab), key(Key::Enter)])
    {
        let (r, outs) = l.input(i, 50.0);
        answers.push((r.text_input, outs.len(), l.card));
    }
    assert!(answers.iter().all(|a| a.0.is_none() && a.1 == 0));
    assert_eq!(
        answers.iter().map(|a| a.2).collect::<Vec<_>>(),
        [false, true, true, false, false, true]
    );
    feed(&mut l, &[key(Key::Escape)]);
    assert!(!l.card);
    // The hint follows the pointer: a finger's, or a phone's, is a tap.
    let mut t = text(1.0);
    let hint = |l: &mut Logon, t: &mut TextSystem| {
        let mut list = DrawList::recording();
        l.draw(&mut list, t, &started(true, true), (0.0, false));
        let runs = list.take_sem().expect("recorded").runs;
        runs.iter().any(|r| r.text == TAP)
    };
    let mut wide = Logon::new((1280.0, 800.0), &store(&kv));
    assert!(!hint(&mut wide, &mut t));
    feed(&mut wide, &[Input::PointerDown { x: 1.0, y: 1.0, button: 2, touch: true }]);
    assert!(hint(&mut wide, &mut t) && hint(&mut Logon::new((411.0, 794.0), &store(&kv)), &mut t));
}

#[test]
fn a_first_visit_says_hello_and_starts_with_start() {
    // Nothing kept: hello. Kept files mean a return, as does the device's mark.
    for (kv, want) in [(&[][..], State::Hello), (&[("compusophy.home", "x")][..], State::Ready)] {
        assert_eq!(Logon::new((411.0, 794.0), &store(kv)).state, want);
    }
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[]);
    let start = l.hits.iter().find(|h| h.1 == Target::Start).expect("Start").0;
    assert!(start.h >= TOUCH && start.w >= TOUCH && start.y + start.h < 794.0 - TRACK);
    // A tap elsewhere does nothing; Start marks the device seen and signs in.
    assert_eq!(feed(&mut l, &tap(20.0, 300.0)), []);
    let seen = Out::Set(SEEN.into(), "1".into());
    let tapped = feed(&mut l, &tap(start.x + 2.0, start.y + 2.0));
    assert_eq!(tapped, [&[seen.clone()][..], &signed_in()].concat());
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[]);
    assert_eq!(feed(&mut l, &[key(Key::Enter)]), [&[seen][..], &signed_in()].concat());
}

#[test]
fn frames_come_only_while_something_moves() {
    let (mut l, mut list, mut t) =
        (Logon::new((1280.0, 800.0), &store(&[])), DrawList::new(), text(1.0));
    let mut at = |l: &mut Logon, rec: &Record, now: f64, reduced: bool| {
        l.draw(&mut list, &mut t, rec, (now, reduced))
    };
    // The reveal (618 ms from the first frame), then rest: only the living grain's timer.
    let rec = started(false, true);
    assert_eq!(at(&mut l, &rec, 1000.0, false), Some(0));
    assert_eq!(at(&mut l, &rec, 1617.0, false), Some(0));
    assert_eq!(at(&mut l, &rec, 1700.0, false), Some(50));
    // Waiting on the fonts draws nothing more; their landing fades the name and a segment in.
    let mut still = Logon::new((1280.0, 800.0), &store(&[("compusophy.grain", "off")]));
    assert_eq!(at(&mut still, &rec, 0.0, false), Some(0));
    assert_eq!(at(&mut still, &rec, 5000.0, false), None);
    let mut rec = rec;
    rec.fonts = [Some((true, 6000.0)), Some((true, 6000.0))];
    assert_eq!(at(&mut still, &rec, 6100.0, false), Some(0));
    assert_eq!(at(&mut still, &rec, 6000.0 + FADE, false), None);
    // Reduced motion: still from the first frame, the mark whole (no dot clear).
    let mut calm = Logon::new((1280.0, 800.0), &store(&[]));
    assert_eq!(at(&mut calm, &rec, 0.0, true), None);
    let mut list = DrawList::new();
    calm.draw(&mut list, &mut text(1.0), &rec, (0.0, true));
    let ink = calm.theme.text;
    let dots = list.instances().iter().filter(|i| i.color == ink && i.radius > 0.0).count();
    assert_eq!(dots, 365);
}

#[test]
fn the_column_fits_every_screen() {
    // (w, h), then a return's mark (its side and top): as large as the screen and column allow.
    #[rustfmt::skip]
    let table = [((1280.0, 800.0), Some((233.0, 190.0))), ((1440.0, 900.0), Some((233.0, 228.0))),
        ((411.0, 794.0), Some((144.0, 231.0))), ((794.0, 411.0), Some((144.0, 85.0))),
        ((320.0, 568.0), Some((89.0, 173.0))), ((794.0, 200.0), None)];
    for ((w, h), want) in table {
        for dpr in [1.0, 2.0, 3.5] {
            for kv in [&[][..], &[(SEEN, "1")][..]] {
                let (l, ..) = drawn((w, h), dpr, kv);
                let lay = l.layout();
                if l.state == State::Ready {
                    assert_eq!(lay.mark.map(|m| (m.w, m.y)), want, "{w}x{h}");
                }
                // Every target on screen, at least 44 px, none over another.
                for (i, (r, t)) in l.hits.iter().enumerate() {
                    assert!(
                        r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h,
                        "{t:?} {w}x{h}"
                    );
                    assert!(r.w >= TOUCH && r.h >= TOUCH, "{t:?} {w}x{h}");
                    let apart = |o: &RectF| {
                        r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y
                    };
                    assert!(l.hits[i + 1..].iter().all(|o| apart(&o.0)), "{t:?} {w}x{h}");
                }
                // The column clears the clock band and the record's band (when it shows).
                let band = l.hits.iter().find(|h| h.1 == Target::Record).map_or(h, |b| b.0.y);
                let bottom = lay.block + l.block_h();
                assert!(lay.mark.is_none_or(|m| m.y >= BAND) && bottom <= band, "{w}x{h} {lay:?}");
            }
        }
    }
}

#[test]
fn the_first_frame_is_cheap() {
    // The backdrop, the clock, the mark's center dot (the rest are clear), the hint, the record:
    // well under 200 instances at 1280 x 800, and no vector shape to rasterize.
    let mut l = Logon::new((1280.0, 800.0), &store(&[(SEEN, "1")]));
    let tick = LocalTime { year: 2026, month: 10, day: 2, weekday: 5, hour: 1, minute: 32 };
    l.input(&Input::Tick { time: tick }, 0.0);
    let (mut list, mut t, rec) = (DrawList::new(), text(1.0), started(false, true));
    let fresh = t.atlas_mut().take_dirty();
    l.draw(&mut list, &mut t, &rec, (0.0, false));
    let clear = list.instances().iter().filter(|i| i.color.3 == 0).count();
    assert!(list.len() - clear < 200, "{}", list.len() - clear);
    assert!(fresh.is_some() && t.atlas_mut().take_dirty().is_some_and(|d| d.1 - d.0 < 64));
}

#[test]
fn signing_in_flies_the_mark_to_the_bar_and_fades_off_the_desktop() {
    let (mut l, _, mut t) = drawn((1280.0, 800.0), 1.0, &[(SEEN, "1")]);
    let mark = l.layout().mark.expect("a mark");
    feed(&mut l, &[key(Key::Enter)]);
    let rec = started(true, true);
    let mut at = |ms: f64| {
        let mut list = DrawList::new();
        let next = l.layer(&mut list, &mut t, &rec, 50.0 + ms);
        let ink = l.theme.text;
        let dots: Vec<_> = list
            .instances()
            .iter()
            .filter(|i| i.color.0 == ink.0 && i.radius > 0.0)
            .map(|i| (i.rect, i.color.3))
            .collect();
        (next, dots)
    };
    // At first the mark is where it was; at the end, in the bar's mark's place, fading into it.
    let (next, dots) = at(0.0);
    let center = |d: &[([f32; 4], u8)]| {
        let (x, y) = d
            .iter()
            .fold((0.0, 0.0), |a, (r, _)| (a.0 + r[0] + r[2] / 2.0, a.1 + r[1] + r[3] / 2.0));
        (x / d.len() as f32, y / d.len() as f32)
    };
    let (cx, cy) = center(&dots);
    assert!(
        next == Some(0)
            && (cx - (mark.x + mark.w / 2.0)).abs() < 1.0
            && (cy - (mark.y + mark.h / 2.0)).abs() < 1.0
    );
    let (_, dots) = at(FLIGHT - 1.0);
    let bar = home::bar::mark_rect();
    let (cx, cy) = center(&dots);
    assert!(
        (cx - (bar.x + bar.w / 2.0)).abs() < 1.0 && (cy - (bar.y + bar.h / 2.0)).abs() < 1.0,
        "{cx} {cy}"
    );
    assert!(dots.iter().all(|d| d.1 < 20));
    assert_eq!(at(FLIGHT), (None, vec![]));
}
