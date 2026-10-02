use super::*;
use crate::paint::{BAND, FADE, FLIGHT, TOUCH, TRACK};
use gfx::{DrawList, Kind};
use profiles::{Pin, fnv};
use record::{Record, Timing};
use ui::TextSystem;

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

/// A welcome made from `kv` (no failure to report).
fn welcome(size: (f32, f32), kv: &[(&str, &str)]) -> Logon {
    let (l, outs) = Logon::new(size, &store(kv), true);
    assert_eq!(outs, []);
    l
}

/// The welcome at `size` (`dpr`), drawn once at 1000 ms (its start known: segments in).
fn drawn(size: (f32, f32), dpr: f32, kv: &[(&str, &str)]) -> (Logon, DrawList, TextSystem) {
    let (mut l, mut list, mut t) = (welcome(size, kv), DrawList::new(), text(dpr));
    l.draw(&mut list, &mut t, &started(true, true), (1000.0, false));
    (l, list, t)
}

/// What `l` does with `inputs` at 50 ms, `kv` in storage: the outs, in order.
fn feed(l: &mut Logon, kv: &[(&str, &str)], inputs: &[Input]) -> Vec<Out> {
    inputs.iter().flat_map(|i| l.input(i, 50.0, &store(kv)).1).collect()
}

fn tap(x: f32, y: f32) -> [Input; 2] {
    [Input::PointerDown { x, y, button: 0, touch: true }, Input::PointerUp { x, y, button: 0 }]
}

fn down(key: Key) -> Input {
    Input::Key { key, mods: ui::Mods::default() }
}

/// The middle of what `l` last drew for `t`.
fn spot(l: &Logon, t: Target) -> (f32, f32) {
    let r = l.hits.iter().rev().find(|h| h.1 == t).expect("drawn").0;
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// What signing in to `id` asks (the device seen already).
fn signed_in(id: &str) -> Vec<Out> {
    let n: u32 = id.parse().expect("an id");
    vec![Out::Set(LAST.into(), id.into()), Out::Session(id.into()), Out::SignIn(n)]
}

/// Two profiles: guest, and ana (id 2: a removed 1 is never reused).
const TWO: &str = "CSPR 1 3\n0 be5cdbf3 - guest\n2 0c55aa31 - ana";

#[test]
fn the_hairline_keeps_gaps_and_lands_on_device_pixels() {
    for dpr in [1.0, 2.0, 3.5] {
        let (l, list, _) = drawn((1280.0, 800.0), dpr, &[(SEEN, "1")]);
        // The segments: the text_dim fills on the track's line, as wide as the mark (233).
        let dim = l.theme().text_dim;
        let segs: Vec<[f32; 4]> = list
            .instances()
            .iter()
            .filter(|i| i.kind == Kind::Fill as u32 as f32 && i.color == dim && i.rect[3] < 2.0)
            .map(|i| i.rect)
            .collect();
        assert_eq!(segs.len(), 4, "{dpr}");
        let x0 = ((1280.0 - 233.0) / 2.0 * dpr).round() / dpr;
        let spans = [(0.0, 7.0), (22.0, 57.0), (57.0, 98.0), (98.0, 104.0)];
        for (s, (from, to)) in segs.iter().zip(spans) {
            let on = |v: f32| ((v * dpr).round() - v * dpr).abs() < 1e-3;
            assert!(on(s[0]) && on(s[0] + s[2]) && on(s[1]), "{dpr} {s:?}");
            let near = |v: f32, ms: f32| (v - (x0 + ms / 104.0 * 233.0)).abs() <= 1.0 / dpr + 1e-3;
            assert!(near(s[0], from) && near(s[0] + s[2], to), "{dpr} {s:?}");
        }
        // The gap the browser spent parsing stays: about 15% of the line.
        assert!(segs[1][0] - (segs[0][0] + segs[0][2]) > 0.13 * 233.0, "{dpr}");
    }
}

#[test]
fn profiles_keep_their_own_keys_and_the_first_keeps_todays() {
    // Profile 0's keys are the ones from before profiles: nothing was moved.
    let today = "compusophy.home compusophy.home.bad compusophy.home.mark compusophy.theme \
        compusophy.dock compusophy.home.order compusophy.grain compusophy.ai.model \
        compusophy.reports compusophy.outbox";
    assert_eq!(PER_PROFILE.map(|k| key(0, k)).join(" "), today);
    assert_eq!(
        [key(7, "home"), key(1000, "theme")],
        ["compusophy.7.home", "compusophy.1000.theme"]
    );
    // Every profile's keys (ids to 1,000) and the device's are apart.
    let mut all: Vec<String> = (0..=1000).flat_map(|id| PER_PROFILE.map(|k| key(id, k))).collect();
    all.extend([LIST, profiles::LIST_BAD, LAST, SEEN, SESSION].map(String::from));
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n);
    // Before a sign-in, profile 0's; then the signed-in one's, by name or by a first key.
    // A panic stays unreported if that one's reports are off (before: any listed one's).
    let off = [(LIST, TWO), ("compusophy.2.reports", "off")];
    assert_eq!(
        (active(), own("compusophy.theme"), quiet(&store(&off))),
        (None, key(0, "theme"), true)
    );
    sign(2);
    assert_eq!((own("compusophy.outbox"), own("ai.model")), (key(2, "outbox"), key(2, "ai.model")));
    assert!(quiet(&store(&off)) && !quiet(&store(&[])));
    // Its files go unkept once another tab removed it; the first is listed with no list.
    assert!(listed(Some(TWO)) && !listed(None) && !listed(Some("CSPR 1 3\n0 be5cdbf3 - guest")));
    sign(0);
    assert!(listed(None) && !quiet(&store(&off)));
    // A reload goes back to a listed profile's desktop, never another's.
    for (s, want) in [(Some("2"), Some(2)), (Some("1"), None), (Some("x"), None), (None, None)] {
        assert_eq!(session(s, &store(&[(LIST, TWO)])), want, "{s:?}");
    }
    assert_eq!(session(Some("0"), &store(&[])), Some(0));
}

#[test]
fn the_list_reads_back_what_it_wrote_and_codes_what_it_cannot() {
    let implied = Profiles::implied();
    assert_eq!((implied.list[0].seed, implied.list[0].name.as_str()), (fnv("guest"), "guest"));
    assert_eq!(Profiles::read(None), (implied.clone(), None));
    assert_eq!(implied.format(), "CSPR 1 1\n0 be5cdbf3 - guest");
    // Up to eight, names with spaces or past ASCII, each reads back; ids are never reused.
    let mut p = implied;
    let names = ["kai", "Ana María", "\u{674e}", "x y z", "a-b", "Zo\u{eb}", "seven"];
    for (i, name) in names.into_iter().enumerate() {
        let seed = i as u32 * 0x1234_5679;
        let id = p.apply(Op::Add { name: name.into(), seed, pin: None }, &|_| false);
        assert_eq!(id, Ok(i as u32 + 1));
        assert_eq!(Profiles::read(Some(&p.format())), (p.clone(), None));
    }
    let add = |name: &str| Op::Add { name: name.into(), seed: 1, pin: None };
    assert_eq!(p.apply(add("nine"), &|_| false), Err(profiles::FULL));
    assert_eq!(p.apply(Op::Remove(3), &|_| false), Ok(3));
    assert_eq!(p.apply(add("KAI"), &|_| false), Err(profiles::TAKEN));
    // A new one skips ids whose keys are still there: it inherits no one's files.
    assert_eq!(p.apply(add("eight"), &|id| id == 8), Ok(9));
    assert_eq!(p.apply(Op::Rename(9, "Kai".into()), &|_| false), Err(profiles::TAKEN));
    assert_eq!(p.apply(Op::Rename(9, "EIGHT".into()), &|_| false), Ok(9));
    assert_eq!(p.apply(Op::Remove(3), &|_| false), Err(profiles::GONE));
    let mut one = Profiles::implied();
    assert_eq!(one.apply(Op::Remove(0), &|_| false), Err(profiles::LAST_ONE));
    // Damaged lists offer profile 0 alone; a newer one reads as far as it can, read-only.
    let guest = "0 be5cdbf3 - guest";
    #[rustfmt::skip]
    let bad = ["", "CSPR", "CSPX 1 1\n0 be5cdbf3 - guest", "CSPR 1 1\n0 5be1c0dz - guest",
        "CSPR 1 1\n1 be5cdbf3 - guest", "CSPR 1 2\n0 be5cdbf3 - guest\n1 be5cdbf3 - GUEST",
        "CSPR 1 1\n0 be5cdbf3 - ", "CSPR 1 1\n0 be5cdbf3 -  guest", "CSPR 1 1", "CSPR 0 1\n0",
        "CSPR 1 1\n0 be5cdbf3\t- guest"];
    for s in bad {
        assert_eq!(
            Profiles::read(Some(s)),
            (Profiles::implied(), Some(profiles::DAMAGED)),
            "{s:?}"
        );
    }
    let nine: String = (0..9).map(|i| format!("\n{i} 0000000{i} - p{i}")).collect();
    assert_eq!(Profiles::read(Some(&format!("CSPR 1 9{nine}"))).1, Some(profiles::DAMAGED));
    let newer = Profiles::read(Some(&format!("CSPR 2 5\n{guest}\n4 00000001 - kai")));
    assert_eq!((newer.0.list.len(), newer.1), (2, Some(profiles::NEWER)));
    // Names: control chars go, the ends are trimmed, 24 chars at most, never none.
    let long = "abcdefghijklmnopqrstuvwxyz";
    assert_eq!(profiles::clean("  k\u{7}ai \n"), Ok("kai".into()));
    assert_eq!(profiles::clean(long), Ok(long[..24].into()));
    assert_eq!(profiles::clean(" \t "), Err(profiles::NO_NAME));
}

#[test]
fn a_return_picks_a_profile_and_the_band_opens_the_card() {
    // The circles, the one that signed in last in focus (its theme the welcome's): Enter or
    // Space signs it in, a tap any.
    let kv = [(SEEN, "1"), (LIST, TWO), (LAST, "2"), ("compusophy.2.theme", "dawn")];
    for start in [vec![down(Key::Enter)], vec![down(Key::Space)]] {
        let (mut l, ..) = drawn((1280.0, 800.0), 1.0, &kv);
        assert_eq!((l.theme().name, l.state, l.focus), ("Dawn", State::Pick, 1));
        assert_eq!(feed(&mut l, &kv, &start), signed_in("2"), "{start:?}");
        assert!(l.leaving() && !l.gone(50.0 + FLIGHT - 1.0) && l.gone(50.0 + FLIGHT));
    }
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let (x, y) = spot(&l, Target::Circle(0));
    assert_eq!(feed(&mut l, &kv, &tap(x, y)), signed_in("0"));
    // The arrows go round the circles, Add, then the record, where Enter opens the card.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let focus = |l: &mut Logon, k: Key| {
        feed(l, &kv, &[down(k)]);
        l.focus
    };
    let path = [Key::Left, Key::Left, Key::Right, Key::Tab, Key::Tab, Key::Tab];
    assert_eq!(path.map(|k| focus(&mut l, k)), [0, 3, 0, 1, 2, 3]);
    assert_eq!((feed(&mut l, &kv, &[down(Key::Enter)]), l.card), (vec![], true));
    // A tap on the band opens the card and signs no one in; a tap outside closes it, and only
    // that. No keyboard ever shows.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let (x, y) = spot(&l, Target::Record);
    let mut answers = vec![];
    for i in tap(x, y).iter().chain(&tap(200.0, 100.0)) {
        let (r, outs) = l.input(i, 50.0, &store(&kv));
        answers.push((r.text_input, outs.len(), l.card));
    }
    assert!(answers.iter().all(|a| a.0.is_none() && a.1 == 0));
    assert_eq!(answers.iter().map(|a| a.2).collect::<Vec<_>>(), [false, true, true, false]);
}

#[test]
fn a_first_visit_says_hello_and_starts_with_start() {
    // Nothing kept: hello. A list, kept files or the device's mark mean a return.
    let lists = [(LIST, TWO)];
    for (kv, want) in [(&[][..], State::Hello), (&[("compusophy.home", "x")][..], State::Pick)] {
        assert_eq!(welcome((411.0, 794.0), kv).state, want);
    }
    assert_eq!(welcome((411.0, 794.0), &lists).state, State::Pick);
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[]);
    let start = l.hits.iter().find(|h| h.1 == Target::Start).expect("Start").0;
    assert!(start.h >= TOUCH && start.w >= TOUCH && start.y + start.h < 794.0 - TRACK);
    // A tap elsewhere does nothing; Start (or Enter) marks the device seen and signs in.
    assert_eq!(feed(&mut l, &[], &tap(20.0, 300.0)), []);
    let seen = Out::Set(SEEN.into(), "1".into());
    let tapped = feed(&mut l, &[], &tap(start.x + 2.0, start.y + 2.0));
    assert_eq!(tapped, [&[seen.clone()][..], &signed_in("0")].concat());
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[]);
    assert_eq!(feed(&mut l, &[], &[down(Key::Enter)]), [&[seen][..], &signed_in("0")].concat());
}

#[test]
fn add_names_a_new_profile_and_signs_in_to_it() {
    let kv = [(SEEN, "1")];
    let (mut l, _, mut t) = drawn((411.0, 794.0), 3.5, &kv);
    l.secure = false; // an insecure page: no PIN is offered (a secure one's: below)
    // Add asks for a name, and the phone's keyboard comes up inside the tap.
    let (x, y) = spot(&l, Target::Circle(1));
    let answers: Vec<_> =
        tap(x, y).iter().map(|i| l.input(i, 50.0, &store(&kv)).0.text_input).collect();
    assert_eq!((answers, l.state), (vec![None, Some(true)], State::Name(None)));
    l.draw(&mut DrawList::new(), &mut t, &started(true, true), (1000.0, false));
    // A tap on the field asks for the keyboard again (one dismissed comes back).
    let (x, y) = spot(&l, Target::Field);
    let answers: Vec<_> =
        tap(x, y).iter().map(|i| l.input(i, 50.0, &store(&kv)).0.text_input).collect();
    assert_eq!(answers, [None, Some(true)]);
    // Names refused say why and store nothing: none, or one taken under case folding.
    for typed in ["   ", "GUEST"] {
        let ins = [Input::Text(typed.into()), down(Key::Enter)];
        assert_eq!((feed(&mut l, &kv, &ins), l.state), (vec![], State::Name(None)), "{typed}");
        assert!(l.note.is_some_and(|n| n.1));
        l.typed.clear();
    }
    // Typed, a char taken back: the new profile is stored, on the list as it is now (another
    // tab's profile kept), its face from fresh bytes, and signed in to.
    let other = [(SEEN, "1"), (LIST, TWO)];
    l.fresh[..4].copy_from_slice(&[1, 2, 3, 4]);
    let ins = [Input::Text(" Kai\u{7}x".into()), down(Key::Backspace), down(Key::Enter)];
    let list = "CSPR 1 4\n0 be5cdbf3 - guest\n2 0c55aa31 - ana\n3 04030201 - Kai";
    let want = [&[Out::Set(LIST.into(), list.into())][..], &signed_in("3")].concat();
    assert_eq!(feed(&mut l, &other, &ins), want);
    // (the keyboard went with the name: nothing more to say).
    let (r, _) = l.input(&Input::Tick { time: LocalTime::default() }, 60.0, &store(&other));
    assert_eq!((r.text_input, l.ime), (None, false));
    // Escape lets a name go, the keyboard with it.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let (x, y) = spot(&l, Target::Circle(1));
    feed(&mut l, &kv, &tap(x, y));
    let (r, outs) = l.input(&down(Key::Escape), 50.0, &store(&kv));
    assert_eq!((r.text_input, outs, l.state), (Some(false), vec![], State::Pick));
}

#[test]
fn a_held_circle_opens_its_menu_to_rename_or_remove_it() {
    let kv =
        [(SEEN, "1"), (LIST, TWO), ("compusophy.2.home", "\u{ff}x"), ("compusophy.grain", "off")];
    let (mut l, _, mut t) = drawn((1280.0, 800.0), 1.0, &kv);
    let mut draw = |l: &mut Logon, now: f64| {
        let mut list = DrawList::recording();
        let next = l.draw(&mut list, &mut t, &started(true, true), (now, false));
        (
            next,
            list.take_sem().expect("recorded").runs.into_iter().map(|r| r.text).collect::<Vec<_>>(),
        )
    };
    // A press on a circle wants a frame when it would be held; then its menu opens, and the
    // lift does nothing more.
    let (x, y) = spot(&l, Target::Circle(1));
    l.input(&Input::PointerDown { x, y, button: 0, touch: true }, 2000.0, &store(&kv));
    assert_eq!((draw(&mut l, 2250.0).0, l.menu.is_none()), (Some(250), true));
    draw(&mut l, 2500.0);
    assert_eq!(l.menu.as_ref().map(|m| m.0), Some(2));
    assert_eq!(feed(&mut l, &kv, &[Input::PointerUp { x, y, button: 0 }]), []);
    assert!(l.menu.is_some());
    // Rename: the name to edit, saved in place; no one signs in.
    let item = |l: &Logon, i: usize| {
        let r = l.menu.as_ref().expect("a menu").1.item(i);
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    let (x, y) = item(&l, 0);
    feed(&mut l, &kv, &tap(x, y));
    assert_eq!((l.state, l.typed.as_str()), (State::Name(Some(2)), "ana"));
    let ins = [down(Key::Backspace), Input::Text("A B".into()), down(Key::Enter)];
    let renamed = "CSPR 1 3\n0 be5cdbf3 - guest\n2 0c55aa31 - anA B";
    assert_eq!(feed(&mut l, &kv, &ins), [Out::Set(LIST.into(), renamed.into())]);
    assert_eq!((l.state, l.leaving()), (State::Pick, false));
    // Remove (of the list as stored: here, still ana's name): a right-click opens the menu; a
    // removal says what goes and needs a yes, which takes the profile's keys, then its line.
    draw(&mut l, 2000.0);
    let (x, y) = spot(&l, Target::Circle(1));
    feed(&mut l, &kv, &[Input::PointerDown { x, y, button: 2, touch: false }]);
    draw(&mut l, 2000.0);
    let (x, y) = item(&l, 2);
    feed(&mut l, &kv, &tap(x, y));
    assert_eq!(l.state, State::Confirm(2));
    let said = draw(&mut l, 2000.0).1.join(" ");
    assert!(said.contains("Remove ana and their files (0.0 KB) from this browser?"), "{said}");
    let (x, y) = spot(&l, Target::Cancel);
    assert_eq!((feed(&mut l, &kv, &tap(x, y)), l.state), (vec![], State::Pick));
    feed(&mut l, &kv, &[Input::PointerDown { x: 1.0, y: 1.0, button: 2, touch: false }]);
    l.state = State::Confirm(2);
    let gone = PER_PROFILE.map(|k| Out::Remove(key(2, k)));
    let list = Out::Set(LIST.into(), "CSPR 1 3\n0 be5cdbf3 - guest".into());
    assert_eq!(feed(&mut l, &kv, &[down(Key::Enter)]), [&gone[..], &[list]].concat());
    // The last one cannot go (the list as stored now holds guest alone).
    let kv = [(SEEN, "1"), (LIST, "CSPR 1 3\n0 be5cdbf3 - guest")];
    draw(&mut l, 3000.0);
    let (x, y) = spot(&l, Target::Circle(0));
    feed(&mut l, &kv, &[Input::PointerDown { x, y, button: 2, touch: false }]);
    draw(&mut l, 3000.0);
    let (x, y) = item(&l, 2);
    feed(&mut l, &kv, &tap(x, y));
    assert_eq!((l.state, l.note), (State::Pick, Some((profiles::LAST_ONE, true))));
}

#[test]
fn the_card_over_the_desktop_offers_stay_or_sign_out_anyway() {
    let mut l = Logon::unkept((411.0, 794.0), "dawn");
    let (mut t, rec) = (text(3.5), Record::default());
    let mut list = DrawList::new();
    assert_eq!(l.layer(&mut list, &mut t, &rec, 0.0), None);
    assert!(!list.is_empty() && !l.leaving() && l.theme().name == "Dawn");
    for (t, want) in [(Target::Stay, Out::Close), (Target::Anyway, Out::SignOut)] {
        let (x, y) = spot(&l, t);
        assert_eq!(feed(&mut l, &[], &tap(x, y)), [want]);
    }
    assert_eq!(feed(&mut l, &[], &[down(Key::Escape)]), [Out::Close]);
    // A tap on the card, but off its buttons, does nothing.
    let above = spot(&l, Target::Stay).1 - 60.0;
    assert_eq!(feed(&mut l, &[], &tap(205.0, above)), []);
}

#[test]
fn frames_come_only_while_something_moves() {
    let (mut l, mut list, mut t) = (welcome((1280.0, 800.0), &[]), DrawList::new(), text(1.0));
    let mut at = |l: &mut Logon, rec: &Record, now: f64, reduced: bool| {
        l.draw(&mut list, &mut t, rec, (now, reduced))
    };
    // The reveal (618 ms from the first frame), then rest: only the living grain's timer.
    let rec = started(false, true);
    assert_eq!(at(&mut l, &rec, 1000.0, false), Some(0));
    assert_eq!(at(&mut l, &rec, 1617.0, false), Some(0));
    assert_eq!(at(&mut l, &rec, 1700.0, false), Some(50));
    // Waiting on the fonts draws nothing more; their landing fades the name and a segment in.
    let mut still = welcome((1280.0, 800.0), &[("compusophy.grain", "off")]);
    assert_eq!(at(&mut still, &rec, 0.0, false), Some(0));
    assert_eq!(at(&mut still, &rec, 5000.0, false), None);
    let mut rec = rec;
    rec.fonts = [Some((true, 6000.0)), Some((true, 6000.0))];
    assert_eq!(at(&mut still, &rec, 6100.0, false), Some(0));
    assert_eq!(at(&mut still, &rec, 6000.0 + FADE, false), None);
    // Reduced motion: still from the first frame, the mark whole (no dot clear).
    let mut calm = welcome((1280.0, 800.0), &[]);
    assert_eq!(at(&mut calm, &rec, 0.0, true), None);
    let mut list = DrawList::new();
    calm.draw(&mut list, &mut text(1.0), &rec, (0.0, true));
    let ink = calm.theme().text;
    let dots = list.instances().iter().filter(|i| i.color == ink && i.radius > 0.0).count();
    assert_eq!(dots, 365);
}

#[test]
fn the_column_fits_every_screen() {
    // (w, h), then a return's mark (its side and top): as large as the screen and column allow.
    #[rustfmt::skip]
    let table = [((1280.0, 800.0), Some((233.0, 190.0))), ((1440.0, 900.0), Some((233.0, 228.0))),
        ((411.0, 794.0), Some((144.0, 231.0))), ((794.0, 411.0), Some((89.0, 45.0))),
        ((320.0, 568.0), Some((89.0, 173.0))), ((794.0, 200.0), None)];
    for ((w, h), want) in table {
        for dpr in [1.0, 2.0, 3.5] {
            for kv in [&[][..], &[(SEEN, "1")][..], &[(LIST, TWO)][..]] {
                let (l, _, mut t) = drawn((w, h), dpr, kv);
                let lay = l.layout(&mut t);
                if kv.len() == 1 && kv[0].0 == SEEN {
                    assert_eq!(lay.mark.map(|m| (m.w, m.y)), want, "{w}x{h}");
                }
                // Every target on screen, at least 44 px, none over another.
                for (i, (r, t)) in l.hits.iter().enumerate() {
                    let on = r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h;
                    assert!(on && r.w >= TOUCH && r.h >= TOUCH, "{t:?} {w}x{h}");
                    let apart = |o: &RectF| {
                        r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y
                    };
                    assert!(l.hits[i + 1..].iter().all(|o| apart(&o.0)), "{t:?} {w}x{h}");
                }
                // The column clears the clock band and the record's band (when it shows).
                let band = l.hits.iter().find(|h| h.1 == Target::Record).map_or(h, |b| b.0.y);
                let bottom = lay.block + l.block_h(&mut t);
                assert!(lay.mark.is_none_or(|m| m.y >= BAND) && bottom <= band, "{w}x{h} {lay:?}");
            }
        }
    }
}

#[test]
fn the_first_frame_is_cheap() {
    // The backdrop, the clock, the mark's center dot (the rest are clear), a face and Add, the
    // record: well under 200 instances at 1280 x 800; the atlas gains the face and glyphs only.
    let mut l = welcome((1280.0, 800.0), &[(SEEN, "1")]);
    let tick = LocalTime { year: 2026, month: 10, day: 2, weekday: 5, hour: 1, minute: 32 };
    l.input(&Input::Tick { time: tick }, 0.0, &store(&[]));
    let (mut list, mut t, rec) = (DrawList::new(), text(1.0), started(false, true));
    let fresh = t.atlas_mut().take_dirty();
    l.draw(&mut list, &mut t, &rec, (0.0, false));
    let clear = list.instances().iter().filter(|i| i.color.3 == 0).count();
    assert!(list.len() - clear < 200, "{}", list.len() - clear);
    let rows = t.atlas_mut().take_dirty().map(|d| d.1 - d.0);
    assert!(fresh.is_some() && rows.is_some_and(|r| r < 128), "{rows:?}");
}

#[test]
fn signing_in_flies_the_mark_to_the_bar_and_fades_off_the_desktop() {
    let (mut l, _, mut t) = drawn((1280.0, 800.0), 1.0, &[(SEEN, "1")]);
    let mark = l.layout(&mut t).mark.expect("a mark");
    feed(&mut l, &[(SEEN, "1")], &[down(Key::Enter)]);
    let rec = started(true, true);
    let mut at = |ms: f64| {
        let mut list = DrawList::new();
        let next = l.layer(&mut list, &mut t, &rec, 50.0 + ms);
        let ink = l.theme().text;
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
    let (mx, my) = (mark.x + mark.w / 2.0, mark.y + mark.h / 2.0);
    assert!(next == Some(0) && (cx - mx).abs() < 1.0 && (cy - my).abs() < 1.0);
    let (_, dots) = at(FLIGHT - 1.0);
    let bar = home::bar::mark_rect();
    let (cx, cy) = center(&dots);
    let (bx, by) = (bar.x + bar.w / 2.0, bar.y + bar.h / 2.0);
    assert!((cx - bx).abs() < 1.0 && (cy - by).abs() < 1.0, "{cx} {cy}");
    assert!(dots.iter().all(|d| d.1 < 20));
    assert_eq!(at(FLIGHT), (None, vec![]));
}

/// A stand-in for what the page derives from `digits`: their bytes, padded to 32 (the welcome
/// only compares what comes back).
fn hash_of(digits: &str) -> Vec<u8> {
    let mut h = digits.as_bytes().to_vec();
    h.resize(32, 0);
    h
}

/// The list with kai (id 1) behind the PIN 2468.
fn pinned() -> String {
    let hash = hash_of("2468").try_into().expect("32 bytes");
    let pin = Pin { len: 4, iterations: 9, salt: [7; 16], hash }.format();
    format!("CSPR 1 2\n0 be5cdbf3 - guest\n1 0c55aa31 {pin} kai")
}

#[test]
fn a_pins_record_reads_back_and_only_a_well_formed_one() {
    let pin = Pin { len: 4, iterations: 100_000, salt: [7; 16], hash: [0xab; 32] };
    let rec = pin.format();
    assert_eq!((&rec[..18], Pin::read(&rec)), ("p1:4:100000:070707", Some(pin)));
    let hex = |n: usize| "ab".repeat(n);
    let (salt, hash) = (hex(16), hex(32));
    #[rustfmt::skip]
    let bad = [format!("p1:3:9:{salt}:{hash}"), format!("p1:9:9:{salt}:{hash}"),
        format!("p2:4:9:{salt}:{hash}"), format!("p1:4:0:{salt}:{hash}"),
        format!("p1:4:9:{}:{hash}", hex(15)), format!("p1:4:9:{salt}:{}", hash.to_uppercase()),
        format!("p1:4:9:{salt}:{hash}:"), String::from("-x")];
    assert!(bad.iter().all(|b| Pin::read(b).is_none()));
    // A list keeps a PIN's record and reads it back; a bad one damages it.
    let list = pinned();
    assert_eq!(Profiles::read(Some(&list)).0.format(), list);
    let broken = list.replace("p1:4:9", "p1:4:x");
    assert_eq!(Profiles::read(Some(&broken)).1, Some(profiles::DAMAGED));
}

#[test]
fn a_pin_is_checked_by_the_browser_asked_at_every_load_and_never_kept() {
    let list = pinned();
    let kv = [(SEEN, "1"), (LIST, &list[..]), ("compusophy.grain", "off")];
    let (mut l, _, mut t) = drawn((411.0, 794.0), 3.5, &kv);
    // No reload goes straight back to a PIN's profile.
    assert_eq!(session(Some("1"), &store(&kv)), None);
    // A tap on its circle asks for the PIN, on the phone's number pad, inside the tap.
    let (x, y) = spot(&l, Target::Circle(1));
    let up = tap(x, y).map(|i| l.input(&i, 50.0, &store(&kv))).map(|(r, o)| (r.text_input, o));
    assert_eq!(up[1], (Some(true), vec![Out::Numeric(true)]));
    assert_eq!((l.state, l.note), (State::Pin(1, Then::SignIn), Some((ENTER, false))));
    // Digits only; at its length the browser derives, and the digits are gone from here.
    assert_eq!(feed(&mut l, &kv, &[Input::Text("2a4".into())]), []);
    let derive = Out::Derive { id: 1, pin: b"2468".to_vec(), salt: [7; 16], iterations: 9 };
    assert_eq!(feed(&mut l, &kv, &[Input::Text("68".into())]), [derive]);
    assert!(l.typed.is_empty() && feed(&mut l, &kv, &[Input::Text("1".into())]).is_empty());
    // Wrong: the dots swing (frames while they do) and clear. Another id is not awaited.
    assert_eq!(l.derived(9, Ok(hash_of("2468")), 60.0, &store(&kv)).1, []);
    assert_eq!(l.derived(1, Ok(hash_of("1357")), 100.0, &store(&kv)).1, []);
    assert_eq!((l.note, l.typed.as_str(), l.leaving()), (Some((WRONG, true)), "", false));
    let (mut drawn_, rec) = (DrawList::new(), started(true, true));
    assert_eq!(l.draw(&mut drawn_, &mut t, &rec, (150.0, false)), Some(0));
    assert_eq!(l.draw(&mut drawn_, &mut t, &rec, (5000.0, false)), None);
    // Right: signed in, keeping no session; the keyboard goes.
    let outs = feed(&mut l, &kv, &[Input::Text("2468".into())]);
    assert!(matches!(outs[..], [Out::Derive { id: 2, .. }]));
    let (r, outs) = l.derived(2, Ok(hash_of("2468")), 200.0, &store(&kv));
    let signed = vec![Out::Set(LAST.into(), "1".into()), Out::Numeric(false), Out::SignIn(1)];
    assert_eq!((outs, r.text_input, l.leaving()), (signed, Some(false), true));
    // Back from a PIN: Escape, or a tap off its dots; the dots bring back a keyboard.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &kv);
    let (x, y) = spot(&l, Target::Circle(1));
    feed(&mut l, &kv, &tap(x, y));
    l.draw(&mut DrawList::new(), &mut t, &rec, (1000.0, false));
    let (x, y) = spot(&l, Target::Field);
    let answers: Vec<_> =
        tap(x, y).iter().map(|i| l.input(i, 50.0, &store(&kv)).0.text_input).collect();
    assert_eq!(answers, [None, Some(true)]);
    feed(&mut l, &kv, &tap(20.0, 100.0));
    assert_eq!(l.state, State::Pick);
    // A PIN another tab set since this welcome read the list is asked all the same; a
    // profile it removed is gone.
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[(SEEN, "1"), (LIST, TWO)]);
    let rec = list.split(' ').find(|t| t.starts_with("p1:")).expect("a record");
    let now = TWO.replace("2 0c55aa31 -", &format!("2 0c55aa31 {rec}"));
    let stored = [(SEEN, "1"), (LIST, &now[..])];
    let (x, y) = spot(&l, Target::Circle(1));
    assert_eq!(
        (feed(&mut l, &stored, &tap(x, y)).len(), l.state),
        (1, State::Pin(2, Then::SignIn))
    );
    let (mut l, ..) = drawn((411.0, 794.0), 3.5, &[(SEEN, "1"), (LIST, TWO)]);
    let (x, y) = spot(&l, Target::Circle(1));
    assert_eq!((feed(&mut l, &kv, &tap(x, y)), l.note), (vec![], Some((profiles::GONE, true))));
    // An insecure page cannot check a PIN, so it says so.
    l.secure = false;
    let (x, y) = spot(&l, Target::Circle(1));
    assert_eq!((feed(&mut l, &kv, &tap(x, y)), l.note), (vec![], Some((INSECURE, true))));
}

#[test]
fn a_new_profile_may_take_a_pin_chosen_twice() {
    let kv = [(SEEN, "1")];
    let (mut l, _, mut t) = drawn((411.0, 794.0), 3.5, &kv);
    l.fresh = [1, 2, 3, 4, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9];
    let (x, y) = spot(&l, Target::Circle(1));
    feed(&mut l, &kv, &tap(x, y));
    assert_eq!(feed(&mut l, &kv, &[Input::Text("kai".into()), down(Key::Enter)]), []);
    // Asked whether to take a PIN, in honest words: a curtain, not a lock.
    let mut list = DrawList::recording();
    l.draw(&mut list, &mut t, &started(true, true), (1000.0, false));
    let runs = list.take_sem().expect("recorded").runs;
    let said: String = runs.iter().map(|r| r.text.clone() + " ").collect();
    assert!(l.state == State::Ask && said.contains("not a lock") && said.contains("not encrypted"));
    assert!(spot(&l, Target::Skip).1 > 0.0 && spot(&l, Target::SetPin).1 > 0.0);
    // Escape goes back to the name, kept; Enter there asks again; Enter here sets a PIN.
    feed(&mut l, &kv, &[down(Key::Escape)]);
    assert_eq!((l.state, l.typed.as_str()), (State::Name(None), "kai"));
    feed(&mut l, &kv, &[down(Key::Enter)]);
    assert_eq!(feed(&mut l, &kv, &[down(Key::Enter)]), [Out::Numeric(true)]);
    assert_eq!((l.state, l.note), (State::NewPin(None), Some((CHOOSE, false))));
    // Four digits at least, then the same again; a mismatch starts over.
    let text = |s: &str| Input::Text(s.into());
    feed(&mut l, &kv, &[text("123"), down(Key::Enter)]);
    assert!(l.first.is_empty());
    feed(&mut l, &kv, &[text("4"), down(Key::Enter), text("1235")]);
    assert_eq!((l.first.as_str(), l.note), ("", Some((MISMATCH, true))));
    let outs = feed(&mut l, &kv, &[text("1234"), down(Key::Enter), text("1234")]);
    let iterations = profiles::ITERATIONS;
    assert_eq!(outs, [Out::Derive { id: 1, pin: b"1234".to_vec(), salt: [9; 16], iterations }]);
    // Its record is kept on the list (never the digits), and the new profile signed in to.
    let (_, outs) = l.derived(1, Ok(vec![5; 32]), 100.0, &store(&kv));
    let pin = Pin { len: 4, iterations, salt: [9; 16], hash: [5; 32] }.format();
    let list = format!("CSPR 1 2\n0 be5cdbf3 - guest\n1 04030201 {pin} kai");
    let set = Out::Set(LIST.into(), list.clone());
    let (last, pad) = (Out::Set(LAST.into(), "1".into()), Out::Numeric(false));
    let want = vec![set, last, pad, Out::SignIn(1)];
    assert!(outs == want && !list.contains("1234"));
}

#[test]
fn a_pin_guards_its_profiles_menu_but_never_removal() {
    let list = pinned();
    let kv = [(SEEN, "1"), (LIST, &list[..])];
    let (mut l, _, mut t) = drawn((1280.0, 800.0), 1.0, &kv);
    let mut menu = |l: &mut Logon, i: usize| {
        let (x, y) = spot(l, Target::Circle(1));
        feed(l, &kv, &[Input::PointerDown { x, y, button: 2, touch: false }]);
        l.draw(&mut DrawList::new(), &mut t, &started(true, true), (1000.0, false));
        let m = &l.menu.as_ref().expect("a menu").1;
        let labels: Vec<&str> = m.items.iter().map(|i| i.0).collect();
        let r = m.item(i);
        feed(l, &kv, &tap(r.x + r.w / 2.0, r.y + r.h / 2.0));
        labels
    };
    // Rename asks the PIN first; right, the name to edit, on letters again.
    let labels = menu(&mut l, 0);
    let all = ["Rename\u{2026}", "Change PIN\u{2026}", "Remove PIN", "Remove profile\u{2026}"];
    assert_eq!((labels, l.state), (all.to_vec(), State::Pin(1, Then::Rename)));
    feed(&mut l, &kv, &[Input::Text("2468".into())]);
    let (r, outs) = l.derived(1, Ok(hash_of("2468")), 100.0, &store(&kv));
    assert_eq!((l.state, l.typed.as_str()), (State::Name(Some(1)), "kai"));
    assert_eq!((outs, r.text_input), (vec![Out::Numeric(false)], Some(true)));
    // Remove PIN: behind the PIN too, it takes the record off the list.
    feed(&mut l, &kv, &[down(Key::Escape)]);
    menu(&mut l, 2);
    assert_eq!(l.state, State::Pin(1, Then::Unpin));
    feed(&mut l, &kv, &[Input::Text("2468".into())]);
    let (_, outs) = l.derived(2, Ok(hash_of("2468")), 100.0, &store(&kv));
    let bare = "CSPR 1 2\n0 be5cdbf3 - guest\n1 0c55aa31 - kai";
    assert_eq!((outs, l.state), (vec![Out::Set(LIST.into(), bare.into())], State::Pick));
    // Removing it never asks the PIN: a forgotten one means just that.
    let (mut l, ..) = drawn((1280.0, 800.0), 1.0, &kv);
    menu(&mut l, 3);
    assert_eq!(l.state, State::Confirm(1));
}
