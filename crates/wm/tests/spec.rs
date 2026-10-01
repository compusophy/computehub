//! Black-box tests for `wm`, written from its spec (public API and
//! semantics) alone. Every command goes through `run`, which checks its
//! outcome and the hash rules: `Noop` and `Err` leave everything observable
//! untouched, and anything else changes the hash.
//!
//! Layouts render as `id:x,y,w,h` items bottom to top, the id tagged `^` when
//! maximized, `@Snap` when snapped and `*` when focused.

use std::collections::BTreeSet;
use wm::Outcome::{Changed, Noop, Opened};
use wm::{CASCADE, Cmd, MAX_COORD, MIN_H, MIN_W, Outcome, Placement, Rect, Snap, State};
use wm::{TITLE_H, VISIBLE_W, WinId, Wm, WmError};

type Res = Result<Outcome, WmError>;
type View = (u64, Rect, Option<WinId>, Vec<Placement>, Vec<(WinId, State, Option<Rect>)>);

const AREA: Rect = Rect::new(0, 0, 1920, 1080);
const SNAPS: [Snap; 6] =
    [Snap::Left, Snap::Right, Snap::TopLeft, Snap::TopRight, Snap::BottomLeft, Snap::BottomRight];

fn id(n: u32) -> WinId {
    WinId(n)
}

fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(x, y, w, h)
}

fn mv(win: u32, x: i32, y: i32) -> Cmd {
    Cmd::Move { win: id(win), x, y }
}

fn rs(win: u32, x: i32, y: i32, w: i32, h: i32) -> Cmd {
    Cmd::Resize { win: id(win), rect: r(x, y, w, h) }
}

fn snap(win: u32, snap: Snap) -> Cmd {
    Cmd::SnapTo { win: id(win), snap }
}

/// Everything observable about a `Wm`.
fn view(wm: &Wm) -> View {
    let wins = wm.windows().into_iter().map(|(w, s)| (w, s, wm.normal_rect(w))).collect();
    (wm.state_hash(), wm.area(), wm.focused(), wm.layout(), wins)
}

/// Applies `cmd`, expecting `want` and the hash rules.
fn run(wm: &mut Wm, cmd: Cmd, want: Res) {
    let before = view(wm);
    assert_eq!(wm.apply(cmd.clone()), want, "{cmd:?}");
    let after = view(wm);
    match want {
        Ok(Noop) | Err(_) => assert_eq!(after, before, "{cmd:?} changed state"),
        Ok(_) => assert_ne!(after.0, before.0, "{cmd:?} kept the hash"),
    }
}

fn ok<const N: usize>(wm: &mut Wm, cmds: [Cmd; N]) {
    cmds.into_iter().for_each(|c| run(wm, c, Ok(Changed)));
}

fn noop<const N: usize>(wm: &mut Wm, cmds: [Cmd; N]) {
    cmds.into_iter().for_each(|c| run(wm, c, Ok(Noop)));
}

fn open(wm: &mut Wm, size: Option<(i32, i32)>, want: u32) {
    run(wm, Cmd::Open { size }, Ok(Opened(id(want))));
}

fn render(wm: &Wm) -> String {
    let one = |p: &Placement| {
        let mode = match (p.state, p.snap) {
            (State::Maximized, _) => "^".to_string(),
            (_, Some(s)) => format!("@{s:?}"),
            _ => String::new(),
        };
        let (f, q) = (["", "*"][usize::from(p.focused)], p.rect);
        format!("{}{mode}{f}:{},{},{},{}", p.win.0, q.x, q.y, q.w, q.h)
    };
    wm.layout().iter().map(one).collect::<Vec<_>>().join(" ")
}

fn order(wm: &Wm) -> Vec<u32> {
    wm.layout().iter().map(|p| p.win.0).collect()
}

#[test]
fn opens_centered_then_cascades() {
    let consts = (MIN_W, MIN_H, TITLE_H, CASCADE, VISIBLE_W, MAX_COORD);
    assert_eq!(consts, (320, 200, 40, 28, 64, 1 << 20));
    let mut wm = Wm::new(AREA);
    assert_eq!((render(&wm), wm.focused(), wm.windows()), (String::new(), None, vec![]));
    for n in 1..=9 {
        open(&mut wm, None, n);
    }
    // Seven corners fit (488 + 1280 <= 1920, 348 + 720 <= 1080); the eighth
    // step would cross the bottom edge, so the run wraps to the area's corner.
    let corners: Vec<(i32, i32)> = wm.layout().iter().map(|p| (p.rect.x, p.rect.y)).collect();
    let mut want: Vec<(i32, i32)> = (0..7).map(|k| (320 + 28 * k, 180 + 28 * k)).collect();
    want.extend([(28, 28), (56, 56)]);
    assert_eq!(corners, want);
    assert!(wm.layout().iter().all(|p| (p.rect.w, p.rect.h) == (1280, 720)));
    assert_eq!((wm.focused(), wm.layout()[8].focused), (Some(id(9)), true));
    // A minimized window does not hold its corner.
    ok(&mut wm, [Cmd::Minimize(id(1))]);
    open(&mut wm, None, 10);
    assert!(render(&wm).ends_with(" 9:56,56,1280,720 10*:320,180,1280,720"));

    // Sizes clamp to [MIN, area] on each side.
    let mut wm = Wm::new(AREA);
    open(&mut wm, Some((10, 10)), 1);
    open(&mut wm, Some((99_999, -5)), 2);
    open(&mut wm, Some((i32::MIN, i32::MAX)), 3);
    assert_eq!(render(&wm), "1:800,440,320,200 2:0,440,1920,200 3*:800,0,320,1080");
    // When even the wrapped corner would leave the area, windows stack.
    let mut wm = Wm::new(AREA);
    for n in 1..=3 {
        open(&mut wm, Some((1900, 1000)), n);
    }
    assert_eq!(render(&wm), "1:10,40,1900,1000 2:10,40,1900,1000 3*:10,40,1900,1000");
    // Default size is w * 2 / 3 (not w / 3 * 2), centered in an offset area.
    let mut wm = Wm::new(r(5, 7, 1001, 1001));
    open(&mut wm, None, 1);
    assert_eq!(render(&wm), "1*:172,174,667,667");
    // An area smaller than the minimum: the titlebar stays inside it.
    let mut wm = Wm::new(r(0, 0, 100, 100));
    open(&mut wm, None, 1);
    assert_eq!(render(&wm), "1*:-110,0,320,200");
}

#[test]
fn ids_are_never_reused_and_unknown_windows_fail() {
    let mut wm = Wm::new(AREA);
    for n in 1..=3 {
        open(&mut wm, None, n);
    }
    ok(&mut wm, [Cmd::Close(id(2))]);
    open(&mut wm, None, 4);
    let normal = State::Normal;
    assert_eq!(wm.windows(), [(id(1), normal), (id(3), normal), (id(4), normal)]);
    assert_eq!(render(&wm), "1:320,180,1280,720 3:376,236,1280,720 4*:348,208,1280,720");
    for n in [0, 2, 5, u32::MAX] {
        let w = id(n);
        let cmds = [
            Cmd::Close(w),
            Cmd::Focus(w),
            mv(n, 0, 0),
            rs(n, 0, 0, 500, 500),
            Cmd::Maximize(w),
            Cmd::Restore(w),
            Cmd::ToggleMaximize(w),
            Cmd::Minimize(w),
            snap(n, Snap::Left),
        ];
        cmds.into_iter().for_each(|c| run(&mut wm, c, Err(WmError::UnknownWindow(w))));
        assert_eq!(wm.normal_rect(w), None);
    }
    let e: Box<dyn std::error::Error> = Box::new(WmError::UnknownWindow(id(7)));
    assert_eq!(e.to_string(), "unknown window 7");
}

#[test]
fn focus_raises_and_falls_back_to_the_top() {
    let mut wm = Wm::new(AREA);
    for n in 1..=3 {
        open(&mut wm, Some((400, 300)), n);
    }
    assert_eq!((order(&wm), wm.focused()), (vec![1, 2, 3], Some(id(3))));
    ok(&mut wm, [Cmd::Focus(id(1))]);
    noop(&mut wm, [Cmd::Focus(id(1))]);
    assert_eq!(render(&wm), "2:788,418,400,300 3:816,446,400,300 1*:760,390,400,300");
    ok(&mut wm, [Cmd::Close(id(1))]);
    assert_eq!((order(&wm), wm.focused()), (vec![2, 3], Some(id(3))));
    ok(&mut wm, [Cmd::Minimize(id(2))]); // a background window: focus stays
    assert_eq!((order(&wm), wm.focused()), (vec![3], Some(id(3))));
    ok(&mut wm, [Cmd::Minimize(id(3))]);
    noop(&mut wm, [Cmd::Minimize(id(3))]);
    let min = State::Minimized;
    assert_eq!((render(&wm), wm.focused()), (String::new(), None));
    assert_eq!(wm.windows(), [(id(2), min), (id(3), min)]);
    // Shown again on top, in the mode it was minimized in.
    ok(&mut wm, [Cmd::Focus(id(2)), Cmd::Maximize(id(2)), Cmd::Minimize(id(2))]);
    assert_eq!(wm.windows(), [(id(2), min), (id(3), min)]);
    ok(&mut wm, [Cmd::Focus(id(2))]);
    assert_eq!(render(&wm), "2^*:0,0,1920,1080");
    ok(&mut wm, [snap(3, Snap::Right)]);
    assert_eq!(render(&wm), "2^:0,0,1920,1080 3@Right*:960,0,960,1080");
    ok(&mut wm, [Cmd::Minimize(id(3)), Cmd::Focus(id(3))]);
    assert_eq!(render(&wm), "2^:0,0,1920,1080 3@Right*:960,0,960,1080");
    // Mode commands never restack a visible window.
    ok(&mut wm, [Cmd::Restore(id(2))]);
    assert_eq!(render(&wm), "2:788,418,400,300 3@Right*:960,0,960,1080");
    assert_eq!(wm.windows(), [(id(2), State::Normal), (id(3), State::Normal)]);
}

#[test]
fn move_keeps_the_titlebar_reachable() {
    let mut wm = Wm::new(AREA);
    open(&mut wm, None, 1);
    ok(&mut wm, [mv(1, 100, 50)]);
    noop(&mut wm, [mv(1, 100, 50)]);
    assert_eq!(render(&wm), "1*:100,50,1280,720");
    ok(&mut wm, [mv(1, -5000, -5000)]);
    assert_eq!(render(&wm), "1*:-1216,0,1280,720"); // 64 px stay in on the left
    noop(&mut wm, [mv(1, -1300, -1)]);
    ok(&mut wm, [mv(1, 5000, 5000)]);
    assert_eq!(render(&wm), "1*:1856,1040,1280,720"); // and the titlebar at the bottom
    ok(&mut wm, [mv(1, i32::MIN, i32::MAX)]);
    assert_eq!(render(&wm), "1*:-1216,1040,1280,720");

    let mut wm = Wm::new(r(100, 50, 1000, 600));
    open(&mut wm, None, 1);
    assert_eq!(render(&wm), "1*:267,150,666,400");
    ok(&mut wm, [mv(1, -9999, -9999)]);
    assert_eq!(render(&wm), "1*:-502,50,666,400");
    ok(&mut wm, [mv(1, 9999, 9999)]);
    assert_eq!(render(&wm), "1*:1036,610,666,400");
    // A maximized or snapped window moves at its normal size and is freed.
    ok(&mut wm, [Cmd::Maximize(id(1))]);
    assert_eq!(render(&wm), "1^*:100,50,1000,600");
    assert_eq!(wm.normal_rect(id(1)), Some(r(1036, 610, 666, 400)));
    ok(&mut wm, [mv(1, 300, 60)]);
    assert_eq!(render(&wm), "1*:300,60,666,400");
    ok(&mut wm, [snap(1, Snap::BottomRight)]);
    assert_eq!(render(&wm), "1@BottomRight*:600,350,500,300");
    ok(&mut wm, [mv(1, 300, 60)]); // its own normal corner: the snap still clears
    assert_eq!(render(&wm), "1*:300,60,666,400");
    ok(&mut wm, [Cmd::Minimize(id(1))]);
    noop(&mut wm, [mv(1, 0, 0)]);
}

#[test]
fn resize_clamps_and_frees_snapped_windows() {
    let mut wm = Wm::new(AREA);
    open(&mut wm, None, 1);
    ok(&mut wm, [rs(1, 10, 20, 500, 300)]);
    noop(&mut wm, [rs(1, 10, 20, 500, 300)]);
    let steps = [
        (rs(1, 10, 20, 5, 5), "1*:10,20,320,200"),
        (rs(1, 0, 0, 99_999, 99_999), "1*:0,0,1920,1080"),
        (rs(1, 5000, -100, 400, 300), "1*:1856,0,400,300"),
        (rs(1, i32::MIN, i32::MIN, i32::MIN, i32::MIN), "1*:-256,0,320,200"),
    ];
    for (cmd, want) in steps {
        ok(&mut wm, [cmd]);
        assert_eq!(render(&wm), want);
    }
    ok(&mut wm, [Cmd::Maximize(id(1))]);
    noop(&mut wm, [rs(1, 0, 0, 500, 500)]);
    // Dragging a snapped window's edge frees it at the new rect.
    ok(&mut wm, [snap(1, Snap::Right), rs(1, 900, 0, 1020, 1080)]);
    assert_eq!(render(&wm), "1*:900,0,1020,1080");
    assert_eq!(wm.normal_rect(id(1)), Some(r(900, 0, 1020, 1080)));
    noop(&mut wm, [Cmd::Restore(id(1))]);
    ok(&mut wm, [Cmd::Minimize(id(1))]);
    noop(&mut wm, [rs(1, 0, 0, 500, 500)]);
}

#[test]
fn maximize_restore_and_toggle() {
    let mut wm = Wm::new(AREA);
    open(&mut wm, None, 1);
    let normal = Some(r(320, 180, 1280, 720));
    ok(&mut wm, [Cmd::Maximize(id(1))]);
    noop(&mut wm, [Cmd::Maximize(id(1))]);
    assert_eq!(render(&wm), "1^*:0,0,1920,1080");
    assert_eq!(wm.windows(), [(id(1), State::Maximized)]);
    assert_eq!(wm.normal_rect(id(1)), normal);
    ok(&mut wm, [Cmd::Restore(id(1))]);
    noop(&mut wm, [Cmd::Restore(id(1))]);
    assert_eq!(render(&wm), "1*:320,180,1280,720");
    ok(&mut wm, [Cmd::ToggleMaximize(id(1))]);
    assert_eq!(render(&wm), "1^*:0,0,1920,1080");
    ok(&mut wm, [Cmd::ToggleMaximize(id(1))]);
    assert_eq!(render(&wm), "1*:320,180,1280,720");
    // A snapped window toggles to maximized, then back to its normal rect.
    ok(&mut wm, [snap(1, Snap::TopLeft), Cmd::ToggleMaximize(id(1))]);
    assert_eq!(render(&wm), "1^*:0,0,1920,1080");
    ok(&mut wm, [Cmd::ToggleMaximize(id(1))]);
    assert_eq!(render(&wm), "1*:320,180,1280,720");
    ok(&mut wm, [snap(1, Snap::Left), Cmd::Restore(id(1))]);
    assert_eq!((render(&wm), wm.normal_rect(id(1))), ("1*:320,180,1280,720".into(), normal));
    // Each of them shows a minimized window on top, focused.
    open(&mut wm, None, 2);
    let show = [Cmd::Maximize(id(1)), Cmd::Restore(id(1)), Cmd::ToggleMaximize(id(1))];
    for cmd in show.into_iter().chain([snap(1, Snap::Left)]) {
        ok(&mut wm, [Cmd::Minimize(id(1))]);
        assert_eq!(wm.focused(), Some(id(2)));
        ok(&mut wm, [cmd]);
        assert_eq!(wm.focused(), Some(id(1)));
    }
    assert_eq!(render(&wm), "2:348,208,1280,720 1@Left*:0,0,960,1080");
}

#[test]
fn snaps_split_the_area() {
    let area = r(10, 20, 1921, 1081);
    let want = [
        r(10, 20, 960, 1081),
        r(970, 20, 961, 1081),
        r(10, 20, 960, 540),
        r(970, 20, 961, 540),
        r(10, 560, 960, 541),
        r(970, 560, 961, 541),
    ];
    let mut wm = Wm::new(area);
    open(&mut wm, None, 1);
    let normal = wm.normal_rect(id(1));
    for (s, want) in SNAPS.into_iter().zip(want) {
        assert_eq!(s.rect(area), want);
        ok(&mut wm, [snap(1, s)]);
        noop(&mut wm, [snap(1, s)]);
        let p = wm.layout()[0];
        assert_eq!((p.rect, p.state, p.snap), (want, State::Normal, Some(s)));
        assert_eq!(wm.normal_rect(id(1)), normal);
    }
    // Snap::rect normalizes the area first, so any input is safe.
    let wild = r(i32::MAX, i32::MIN, i32::MAX, 3);
    assert_eq!(Snap::Right.rect(wild), r(3 << 19, -MAX_COORD, 1 << 19, 3));
    assert_eq!(Snap::BottomRight.rect(r(0, 0, 1, 1)), r(0, 0, 1, 1));
    assert_eq!(Snap::TopLeft.rect(r(0, 0, 1, 1)), r(0, 0, 0, 0));
}

#[test]
fn focus_cycles_through_visible_windows() {
    let mut wm = Wm::new(AREA);
    noop(&mut wm, [Cmd::FocusNext, Cmd::FocusPrev]);
    open(&mut wm, None, 1);
    noop(&mut wm, [Cmd::FocusNext, Cmd::FocusPrev]);
    open(&mut wm, None, 2);
    open(&mut wm, None, 3);
    let steps = [
        (Cmd::FocusNext, [3, 1, 2].as_slice()),
        (Cmd::FocusNext, &[2, 3, 1]),
        (Cmd::FocusPrev, &[3, 1, 2]),
        (Cmd::Minimize(id(2)), &[3, 1]),
        (Cmd::FocusNext, &[1, 3]),
        (Cmd::FocusPrev, &[3, 1]),
        (Cmd::Close(id(3)), &[1]),
    ];
    for (cmd, want) in steps {
        ok(&mut wm, [cmd]);
        assert_eq!((order(&wm), wm.focused()), (want.to_vec(), want.last().copied().map(id)));
    }
    noop(&mut wm, [Cmd::FocusNext, Cmd::FocusPrev]);
}

#[test]
fn set_area_refits_every_window() {
    let mut wm = Wm::new(AREA);
    for n in 1..=3 {
        open(&mut wm, None, n);
    }
    let cmds = [mv(1, 1500, 900), Cmd::Maximize(id(2)), snap(3, Snap::Right), Cmd::Minimize(id(3))];
    ok(&mut wm, cmds);
    noop(&mut wm, [Cmd::SetArea(AREA)]);
    ok(&mut wm, [Cmd::SetArea(r(0, 0, 800, 600))]);
    assert_eq!(render(&wm), "1:736,560,800,600 2^*:0,0,800,600");
    assert_eq!(wm.normal_rect(id(2)), Some(r(348, 208, 800, 600)));
    ok(&mut wm, [Cmd::Focus(id(3))]);
    assert!(render(&wm).ends_with(" 3@Right*:400,0,400,600"));
    assert_eq!(wm.normal_rect(id(3)), Some(r(376, 236, 800, 600)));
    // Extreme areas normalize; a hugely offset one drags windows along.
    ok(&mut wm, [Cmd::SetArea(r(i32::MIN, i32::MAX, i32::MIN, i32::MAX))]);
    assert_eq!(wm.area(), r(-MAX_COORD, MAX_COORD, 0, MAX_COORD));
    noop(&mut wm, [Cmd::SetArea(r(-MAX_COORD - 1, i32::MAX, -1, MAX_COORD + 1))]);
    assert_eq!(wm.normal_rect(id(1)), Some(r(-MAX_COORD - 64, MAX_COORD, 320, 600)));
    // Lower than a titlebar: the top edge sits on the area's top.
    ok(&mut wm, [Cmd::SetArea(r(0, 0, 500, 30))]);
    assert_eq!(wm.normal_rect(id(1)), Some(r(-256, 0, 320, 200)));
    ok(&mut wm, [Cmd::SetArea(r(5, 5, 0, 0))]);
    assert_eq!(render(&wm), "1:-251,5,320,200 2^:5,5,0,0 3@Right*:5,5,0,0");
}

#[test]
fn state_hash_is_canonical_and_pinned() {
    let build = |cmds: &[Cmd]| {
        let mut wm = Wm::new(AREA);
        cmds.iter().for_each(|c| assert!(wm.apply(c.clone()).is_ok()));
        wm
    };
    let o = Cmd::Open { size: None };
    let (o2, o3) = ([o.clone(), o.clone()], [o.clone(), o.clone(), o.clone()]);
    let a = build(&[o2.as_slice(), &[Cmd::Close(id(2))]].concat());
    let detour = [Cmd::Focus(id(1)), Cmd::Minimize(id(2)), Cmd::Close(id(2))];
    assert_eq!(a.state_hash(), build(&[o2.as_slice(), &detour].concat()).state_hash());
    assert_eq!(a.clone().state_hash(), a.state_hash());

    // Round trips come back to the same hash.
    let mut wm = build(&o3);
    let h = wm.state_hash();
    let trips = [
        vec![Cmd::Maximize(id(1)), Cmd::Restore(id(1))],
        vec![snap(2, Snap::TopRight), Cmd::Restore(id(2))],
        vec![Cmd::Focus(id(1)), Cmd::Focus(id(2)), Cmd::Focus(id(3))],
        vec![Cmd::FocusNext, Cmd::FocusPrev],
        vec![Cmd::Minimize(id(3)), Cmd::Focus(id(3))],
        vec![mv(1, 0, 0), mv(1, 320, 180)],
        vec![Cmd::SetArea(r(0, 0, 1921, 1080)), Cmd::SetArea(AREA)],
    ];
    for trip in trips {
        trip.into_iter().for_each(|c| ok(&mut wm, [c]));
        assert_eq!(wm.state_hash(), h);
    }

    // A walk through distinct states hashes them all apart.
    let mut seen = BTreeSet::from([Wm::new(AREA).state_hash()]);
    let mut wm = Wm::new(AREA);
    let walk = [
        o.clone(),
        o.clone(),
        Cmd::Focus(id(1)),
        Cmd::Minimize(id(1)),
        Cmd::Maximize(id(2)),
        snap(2, Snap::Left),
        snap(2, Snap::Right),
        mv(2, 10, 10),
        rs(2, 10, 10, 500, 400),
        Cmd::Close(id(1)),
        Cmd::SetArea(r(0, 0, 1280, 720)),
        Cmd::Open { size: Some((400, 300)) },
        Cmd::FocusNext,
    ];
    for cmd in walk {
        assert!(matches!(wm.apply(cmd.clone()), Ok(Changed | Opened(_))), "{cmd:?}");
        assert!(seen.insert(wm.state_hash()), "{cmd:?} revisits a hash");
    }

    // Pinned: a change to the encoding would orphan every recorded hash.
    assert_eq!(Wm::new(AREA).state_hash(), GOLDEN[0]);
    let script = [o3.as_slice(), &[snap(1, Snap::TopRight), Cmd::Maximize(id(2))]].concat();
    let script = [script.as_slice(), &[Cmd::Minimize(id(3)), mv(1, 7, 9)]].concat();
    assert_eq!(build(&script).state_hash(), GOLDEN[1]);
}

const GOLDEN: [u64; 2] = [0x2457_e8dc_7bdb_028d, 0x565d_13d7_6a26_5cfb];

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) % n.max(1)
    }

    /// Mostly ordinary coordinates, sometimes extremes beyond MAX_COORD.
    fn int(&mut self) -> i32 {
        match self.below(12) {
            0 => i32::MIN,
            1 => i32::MAX,
            2 => MAX_COORD + 1,
            _ => self.below(2600) as i32 - 300,
        }
    }

    /// Usually an ordinary screen-ish rect, sometimes anything at all.
    fn rect(&mut self) -> Rect {
        let n = [100, 100, 2400, 1400].map(|s| self.below(s) as i32);
        let wild = r(self.int(), self.int(), self.int(), self.int());
        [wild, r(n[0] - 50, n[1] - 50, n[2], n[3])][usize::from(self.below(4) > 0)]
    }

    fn cmd(&mut self, live: &[WinId], next: u32) -> Cmd {
        let any = WinId(self.below(u64::from(next) + 2) as u32); // 0, closed, live or unborn
        let pick = live.get(self.below(live.len() as u64) as usize).copied();
        let win = pick.filter(|_| self.below(6) > 0).unwrap_or(any);
        let size = Some((self.int(), self.int()));
        let size = [None, size, Some((self.below(900) as i32, self.below(700) as i32))];
        let (x, y) = (self.int(), self.int());
        match self.below(20) {
            0..=2 if live.len() < 24 => Cmd::Open { size: size[self.below(3) as usize] },
            0..=3 => Cmd::Close(win),
            4 | 5 => Cmd::Focus(win),
            6 | 7 => Cmd::Move { win, x, y },
            8 | 9 => Cmd::Resize { win, rect: self.rect() },
            10 => Cmd::Maximize(win),
            11 => Cmd::Restore(win),
            12 => Cmd::ToggleMaximize(win),
            13 | 14 => Cmd::Minimize(win),
            15 => Cmd::SnapTo { win, snap: SNAPS[self.below(6) as usize] },
            16 => Cmd::FocusNext,
            17 => Cmd::FocusPrev,
            _ => Cmd::SetArea(self.rect()),
        }
    }
}

fn target(cmd: &Cmd) -> Option<WinId> {
    match *cmd {
        Cmd::Close(w) | Cmd::Focus(w) | Cmd::Maximize(w) | Cmd::Restore(w) => Some(w),
        Cmd::ToggleMaximize(w) | Cmd::Minimize(w) => Some(w),
        Cmd::Move { win, .. } | Cmd::Resize { win, .. } | Cmd::SnapTo { win, .. } => Some(win),
        Cmd::Open { .. } | Cmd::FocusNext | Cmd::FocusPrev | Cmd::SetArea(_) => None,
    }
}

/// Applies `cmd` and checks it against the spec, given the open windows.
fn step(wm: &mut Wm, live: &mut Vec<WinId>, next: &mut u32, cmd: &Cmd) -> Res {
    let before = view(wm);
    let res = wm.apply(cmd.clone());
    let after = view(wm);
    match res {
        Ok(Noop) | Err(_) => assert_eq!(after, before, "{cmd:?} changed state"),
        Ok(_) => assert_ne!(after.0, before.0, "{cmd:?} kept the hash"),
    }
    let ids = |v: &View| v.3.iter().map(|p| p.win).collect::<Vec<_>>();
    let (was, now) = (ids(&before), ids(&after));
    let place = |v: &View, w| v.3.iter().find(|p| p.win == w).copied();
    if let Some(w) = target(cmd) {
        // Window commands fail exactly when the window is not open.
        assert_eq!(res.is_err(), !live.contains(&w), "{cmd:?}");
        res?; // an unknown window: nothing more to check
        let (hidden, max) = (!was.contains(&w), place(&before, w).map(|p| p.state));
        let restack = match cmd {
            Cmd::Close(_) | Cmd::Minimize(_) | Cmd::Focus(_) => true,
            Cmd::Move { .. } | Cmd::Resize { .. } => false,
            _ => hidden,
        };
        if !restack {
            assert_eq!(now, was, "{cmd:?} restacked");
        } else if matches!(cmd, Cmd::Close(_) | Cmd::Minimize(_)) {
            let rest: Vec<WinId> = was.iter().copied().filter(|&x| x != w).collect();
            assert_eq!(now, rest, "{cmd:?}: the rest keep their order");
        } else {
            assert_eq!(now.last(), Some(&w), "{cmd:?} did not raise");
        }
        let p = place(&after, w);
        let state = p.map(|p| (p.state, p.snap));
        match *cmd {
            Cmd::Close(_) => live.retain(|&x| x != w),
            Cmd::Minimize(_) => assert!(p.is_none() && wm.normal_rect(w).is_some()),
            Cmd::Maximize(_) => assert_eq!(state, Some((State::Maximized, None))),
            Cmd::Restore(_) => assert_eq!(state, Some((State::Normal, None))),
            Cmd::SnapTo { snap, .. } => assert_eq!(state, Some((State::Normal, Some(snap)))),
            Cmd::ToggleMaximize(_) if !hidden => {
                let old = max.map(|s| s == State::Maximized);
                assert_eq!(p.map(|p| p.state != State::Maximized), old, "{cmd:?}");
            }
            // Minimized windows ignore moves and resizes; maximized ones resizes.
            Cmd::Move { .. } | Cmd::Resize { .. } if hidden => assert_eq!(res, Ok(Noop)),
            Cmd::Resize { .. } if max == Some(State::Maximized) => assert_eq!(res, Ok(Noop)),
            Cmd::Resize { .. } => assert_eq!(state, Some((State::Normal, None))),
            Cmd::Move { .. } => {
                let size = |r: Option<Rect>| r.map(|r| (r.w, r.h));
                let normal = before.4.iter().find(|e| e.0 == w).and_then(|e| e.2);
                assert_eq!(size(p.map(|p| p.rect)), size(normal));
                assert_eq!(state, Some((State::Normal, None)));
            }
            _ => {}
        }
    }
    match *cmd {
        Cmd::Open { size } => {
            assert_eq!(res, Ok(Opened(WinId(*next))));
            let (a, p) = (after.1, after.3.last().copied().expect("opened on top"));
            let (w, h) = size.unwrap_or((a.w * 2 / 3, a.h * 2 / 3));
            let want = (w.clamp(MIN_W, MIN_W.max(a.w)), h.clamp(MIN_H, MIN_H.max(a.h)));
            assert_eq!((p.win, p.focused, (p.rect.w, p.rect.h)), (WinId(*next), true, want));
            live.push(WinId(*next));
            *next += 1;
        }
        Cmd::SetArea(a) => {
            let n = |v: i32, lo: i32| v.clamp(lo, MAX_COORD);
            let norm = r(n(a.x, -MAX_COORD), n(a.y, -MAX_COORD), n(a.w, 0), n(a.h, 0));
            assert_eq!((wm.area(), now), (norm, was));
        }
        Cmd::FocusNext | Cmd::FocusPrev if was.len() > 1 => {
            let mut want = was.clone();
            if *cmd == Cmd::FocusNext {
                want.rotate_right(1)
            } else {
                want.rotate_left(1)
            }
            assert_eq!(now, want);
        }
        _ => {}
    }
    res
}

/// The invariants that hold after every command.
fn check(wm: &Wm, live: &[WinId]) {
    let (a, lay, wins) = (wm.area(), wm.layout(), wm.windows());
    // Ids are unique, in creation order, and exactly the open windows.
    assert_eq!(wins.iter().map(|e| e.0).collect::<Vec<_>>(), live);
    let shown: Vec<WinId> = wins.iter().filter(|e| e.1 != State::Minimized).map(|e| e.0).collect();
    let mut on: Vec<WinId> = lay.iter().map(|p| p.win).collect();
    on.sort();
    assert_eq!(on, shown, "the layout is the non-minimized windows, once each");
    // At most one window is focused: the top-most visible one.
    assert_eq!(lay.iter().filter(|p| p.focused).count(), usize::from(!lay.is_empty()));
    assert_eq!(wm.focused(), lay.last().map(|p| p.win));
    assert!(lay.last().is_none_or(|p| p.focused));
    for p in &lay {
        let normal = wm.normal_rect(p.win).expect("open");
        assert_eq!(wins.iter().find(|e| e.0 == p.win).map(|e| e.1), Some(p.state));
        let want = match (p.state, p.snap) {
            (State::Maximized, None) => a,
            (State::Normal, Some(s)) => s.rect(a),
            (State::Normal, None) => normal,
            _ => panic!("impossible placement {p:?}"),
        };
        assert_eq!(p.rect, want, "{p:?}");
    }
    for &(w, _) in &wins {
        let n = wm.normal_rect(w).expect("open");
        assert!(n.w >= MIN_W && n.h >= MIN_H, "{n:?} below the minimum");
        assert!(n.w <= MIN_W.max(a.w) && n.h <= MIN_H.max(a.h), "{n:?} beyond {a:?}");
        let [x, y, w, ax, ay, aw, ah] = [n.x, n.y, n.w, a.x, a.y, a.w, a.h].map(i64::from);
        let (vis, title) = (i64::from(VISIBLE_W), i64::from(TITLE_H));
        assert!(x + w >= ax + vis && x <= ax + aw - vis, "{n:?} hidden in {a:?}");
        assert!(y >= ay && (y <= ay + ah - title || y == ay), "{n:?} titlebar lost in {a:?}");
    }
}

#[test]
fn random_runs_keep_invariants_and_replay() {
    let mut changers = BTreeSet::new();
    for seed in 1..=12_u64 {
        let mut rng = Rng(seed);
        let area = rng.rect();
        let mut wm = Wm::new(area);
        let (mut live, mut next, mut log) = (Vec::new(), 1, Vec::new());
        for _ in 0..1500 {
            let cmd = rng.cmd(&live, next);
            let res = step(&mut wm, &mut live, &mut next, &cmd);
            check(&wm, &live);
            if matches!(res, Ok(Changed | Opened(_))) {
                let name = format!("{cmd:?}");
                changers
                    .insert(name.chars().take_while(char::is_ascii_alphabetic).collect::<String>());
            }
            log.push((cmd, res, wm.state_hash()));
        }
        // Replaying the log on a fresh Wm reproduces every outcome and hash.
        let mut re = Wm::new(area);
        for (i, (cmd, res, hash)) in log.into_iter().enumerate() {
            assert_eq!(re.apply(cmd), res, "replay step {i}");
            assert_eq!(re.state_hash(), hash, "replay step {i}");
        }
    }
    assert_eq!(changers.len(), 13, "some command never changed state: {changers:?}");
}
