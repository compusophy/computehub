//! Black-box tests for `wm`, from its spec alone. Every command goes through
//! `apply`, which checks the hash rules: `Noop` and `Err` leave everything
//! observable untouched, anything else changes the hash. Layouts render as
//! `id:x,y,w,h` bottom to top, the id tagged `^` when maximized, `@Snap` when
//! snapped and `*` when focused.

use std::collections::BTreeSet;
use wm::Cmd::*;
use wm::Outcome::{Changed, Noop, Opened};
use wm::*;

type Res = Result<Outcome, WmError>;
type View = (u64, Rect, Option<WinId>, Vec<Placement>, Vec<(WinId, State, Option<Rect>)>);

const C: Res = Ok(Changed);
const N: Res = Ok(Noop);
const O: Cmd = Open { size: None };
const AREA: Rect = Rect::new(0, 0, 1920, 1080);
const SNAPS: [Snap; 6] =
    [Snap::Left, Snap::Right, Snap::TopLeft, Snap::TopRight, Snap::BottomLeft, Snap::BottomRight];

fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(x, y, w, h)
}

fn mv(win: u32, x: i32, y: i32) -> Cmd {
    Move { win: WinId(win), x, y }
}

fn rs(win: u32, x: i32, y: i32, w: i32, h: i32) -> Cmd {
    Resize { win: WinId(win), rect: r(x, y, w, h) }
}

fn snap(win: u32, snap: Snap) -> Cmd {
    SnapTo { win: WinId(win), snap }
}

fn op(n: u32) -> Res {
    Ok(Opened(WinId(n)))
}

/// Applies `cmd`, checking the hash rules; returns its result and views of
/// everything observable before and after it.
fn apply(wm: &mut Wm, cmd: &Cmd) -> (Res, View, View) {
    let view = |wm: &Wm| {
        let wins = wm.windows().into_iter().map(|(w, s)| (w, s, wm.normal_rect(w))).collect();
        (wm.state_hash(), wm.area(), wm.focused(), wm.layout(), wins)
    };
    let before = view(wm);
    let res = wm.apply(cmd.clone());
    let after = view(wm);
    match res {
        Ok(Noop) | Err(_) => assert_eq!(after, before, "{cmd:?} changed state"),
        Ok(_) => assert_ne!(after.0, before.0, "{cmd:?} kept the hash"),
    }
    (res, before, after)
}

/// Scripted steps: `play` runs (command, result, layout after it) steps, an
/// empty layout unchecked; `all` applies each command, expecting `want`.
trait Steps {
    fn play(&mut self, steps: &[(Cmd, Res, &str)]);
    fn all(&mut self, want: Res, cmds: &[Cmd]);
}

impl Steps for Wm {
    fn play(&mut self, steps: &[(Cmd, Res, &str)]) {
        for (cmd, want, layout) in steps {
            assert_eq!(apply(self, cmd).0, *want, "{cmd:?}");
            let got = render(self);
            assert!(layout.is_empty() || got == *layout, "{cmd:?}: {got} != {layout}");
        }
    }

    fn all(&mut self, want: Res, cmds: &[Cmd]) {
        cmds.iter().for_each(|c| assert_eq!(apply(self, c).0, want, "{c:?}"));
    }
}

/// A `Wm` over `area` with `n` windows of `size` opened.
fn desk(area: Rect, size: Option<(i32, i32)>, n: u32) -> Wm {
    let mut wm = Wm::new(area);
    (1..=n).for_each(|n| assert_eq!(apply(&mut wm, &Open { size }).0, op(n)));
    wm
}

fn render(wm: &Wm) -> String {
    let one = |p: &Placement| {
        let max = if p.state == State::Maximized { "^" } else { "" };
        let snap = p.snap.map(|s| format!("@{s:?}")).unwrap_or_default();
        let (f, q) = (["", "*"][usize::from(p.focused)], p.rect);
        format!("{}{max}{snap}{f}:{},{},{},{}", p.win.0, q.x, q.y, q.w, q.h)
    };
    wm.layout().iter().map(one).collect::<Vec<_>>().join(" ")
}

#[test]
fn opens_cascading_from_the_focused_window() {
    let consts = (MIN_W, MIN_H, TITLE_H, CASCADE, VISIBLE_W, MAX_COORD);
    assert_eq!(consts, (320, 200, 40, 28, 64, 1 << 20));
    let wm = Wm::new(AREA);
    assert_eq!((render(&wm), wm.focused(), wm.windows()), (String::new(), None, vec![]));
    // The first window is centered, each next 28 px right of and below the top
    // one; the eighth would cross the bottom edge, so it opens at the corner.
    let mut wm = desk(AREA, None, 9);
    let rects: Vec<Rect> = wm.layout().iter().map(|p| p.rect).collect();
    let want = (0..7).map(|k| (320 + 28 * k, 180 + 28 * k)).chain([(28, 28), (56, 56)]);
    assert_eq!(rects, want.map(|(x, y)| r(x, y, 1280, 720)).collect::<Vec<_>>());
    assert_eq!((wm.focused(), wm.layout()[8].focused), (Some(WinId(9)), true));
    // The base is the focused window, not the newest; minimized ones do not count.
    wm.play(&[(Focus(WinId(2)), C, ""), (O, op(10), "")]);
    assert_eq!(wm.normal_rect(WinId(10)), Some(r(376, 236, 1280, 720)));
    wm.all(C, &[Minimize(WinId(10)), Minimize(WinId(2))]);
    wm.play(&[(O, op(11), "")]);
    assert!(render(&wm).ends_with(" 9:56,56,1280,720 11*:84,84,1280,720"));

    // A snapped base counts where it is drawn; past the right edge, the corner;
    // with nothing visible, the center.
    let mut wm = desk(AREA, Some((400, 300)), 1);
    wm.play(&[
        (snap(1, Snap::Right), C, ""),
        (Open { size: Some((400, 300)) }, op(2), "1@Right:960,0,960,1080 2*:988,28,400,300"),
        (mv(2, 1600, 50), C, ""),
        (Open { size: Some((400, 300)) }, op(3), ""),
    ]);
    assert_eq!(wm.normal_rect(WinId(3)), Some(r(28, 28, 400, 300)));
    wm.all(C, &[Minimize(WinId(1)), Minimize(WinId(2)), Minimize(WinId(3))]);
    wm.play(&[(Open { size: Some((400, 300)) }, op(4), "4*:760,390,400,300")]);

    // Sizes clamp to [MIN, area] on each side; where neither the step nor the
    // corner fits, windows open centered.
    let mut wm = Wm::new(AREA);
    wm.play(&[
        (Open { size: Some((10, 10)) }, op(1), "1*:800,440,320,200"),
        (Open { size: Some((99_999, -5)) }, op(2), "1:800,440,320,200 2*:0,440,1920,200"),
        (Open { size: Some((i32::MIN, i32::MAX)) }, op(3), ""),
    ]);
    assert_eq!(wm.normal_rect(WinId(3)), Some(r(800, 0, 320, 1080)));
    let wm = desk(AREA, Some((1900, 1000)), 3);
    assert_eq!(render(&wm), "1:10,40,1900,1000 2:10,40,1900,1000 3*:10,40,1900,1000");
    // The default size is w * 2 / 3 (not w / 3 * 2), centered in an offset
    // area; in an area below the minimum, the titlebar stays inside.
    assert_eq!(render(&desk(r(5, 7, 1001, 1001), None, 1)), "1*:172,174,667,667");
    assert_eq!(render(&desk(r(0, 0, 100, 100), None, 1)), "1*:-110,0,320,200");
}

#[test]
fn ids_are_never_reused_and_unknown_windows_fail() {
    let mut wm = desk(AREA, None, 3);
    let layout = "1:320,180,1280,720 3:376,236,1280,720 4*:404,264,1280,720";
    wm.play(&[(Close(WinId(2)), C, ""), (O, op(4), layout)]);
    assert_eq!(wm.windows(), [1, 3, 4].map(|n| (WinId(n), State::Normal)));
    for n in [0, 2, 5, u32::MAX] {
        let (w, e) = (WinId(n), Err(WmError::UnknownWindow(WinId(n))));
        wm.all(e, &[Close(w), Focus(w), mv(n, 0, 0), rs(n, 0, 0, 500, 500), Maximize(w)]);
        wm.all(e, &[Restore(w), ToggleMaximize(w), Minimize(w), snap(n, Snap::Left)]);
        assert_eq!(wm.normal_rect(w), None);
    }
}

#[test]
fn focus_raises_and_falls_back_to_the_top() {
    let mut wm = desk(AREA, Some((400, 300)), 3);
    wm.play(&[
        (Focus(WinId(1)), C, "2:788,418,400,300 3:816,446,400,300 1*:760,390,400,300"),
        (Focus(WinId(1)), N, ""),
        (Close(WinId(1)), C, "2:788,418,400,300 3*:816,446,400,300"),
        (Minimize(WinId(2)), C, "3*:816,446,400,300"), // a background window: focus stays
        (Minimize(WinId(3)), C, ""),
        (Minimize(WinId(3)), N, ""),
    ]);
    let min = vec![(WinId(2), State::Minimized), (WinId(3), State::Minimized)];
    assert_eq!((render(&wm), wm.focused(), wm.windows()), (String::new(), None, min.clone()));
    // Shown again on top, in the mode it was minimized in.
    wm.all(C, &[Focus(WinId(2)), Maximize(WinId(2)), Minimize(WinId(2))]);
    assert_eq!(wm.windows(), min);
    wm.play(&[
        (Focus(WinId(2)), C, "2^*:0,0,1920,1080"),
        (snap(3, Snap::Right), C, "2^:0,0,1920,1080 3@Right*:960,0,960,1080"),
        (Minimize(WinId(3)), C, ""),
        (Focus(WinId(3)), C, "2^:0,0,1920,1080 3@Right*:960,0,960,1080"),
        // Mode commands never restack a visible window.
        (Restore(WinId(2)), C, "2:788,418,400,300 3@Right*:960,0,960,1080"),
    ]);
    assert_eq!(wm.windows(), [(WinId(2), State::Normal), (WinId(3), State::Normal)]);

    // FocusNext and FocusPrev cycle through the visible windows.
    let (mut wm, cycle) = (Wm::new(AREA), [FocusNext, FocusPrev]);
    wm.all(N, &cycle);
    wm.play(&[(O, op(1), "")]);
    wm.all(N, &cycle);
    wm.play(&[(O, op(2), ""), (O, op(3), "")]);
    let steps = [
        (FocusNext, [3, 1, 2].as_slice()),
        (FocusNext, &[2, 3, 1]),
        (FocusPrev, &[3, 1, 2]),
        (Minimize(WinId(2)), &[3, 1]),
        (FocusNext, &[1, 3]),
        (FocusPrev, &[3, 1]),
        (Close(WinId(3)), &[1]),
    ];
    for (cmd, want) in steps {
        wm.all(C, &[cmd]);
        let order: Vec<u32> = wm.layout().iter().map(|p| p.win.0).collect();
        assert_eq!((order, wm.focused()), (want.to_vec(), want.last().copied().map(WinId)));
    }
    wm.all(N, &cycle);
}

#[test]
fn move_and_resize_keep_the_titlebar_reachable() {
    let mut wm = desk(AREA, None, 1);
    wm.play(&[
        (mv(1, 100, 50), C, "1*:100,50,1280,720"),
        (mv(1, 100, 50), N, ""),
        (mv(1, -5000, -5000), C, "1*:-1216,0,1280,720"), // 64 px stay in on the left
        (mv(1, -1300, -1), N, ""),
        (mv(1, 5000, 5000), C, "1*:1856,1040,1280,720"), // and the titlebar at the bottom
        (mv(1, i32::MIN, i32::MAX), C, "1*:-1216,1040,1280,720"),
    ]);
    let mut wm = desk(r(100, 50, 1000, 600), None, 1);
    assert_eq!(render(&wm), "1*:267,150,666,400");
    wm.play(&[
        (mv(1, -9999, -9999), C, "1*:-502,50,666,400"),
        (mv(1, 9999, 9999), C, "1*:1036,610,666,400"),
        // A maximized or snapped window moves at its normal size and is freed.
        (Maximize(WinId(1)), C, "1^*:100,50,1000,600"),
        (mv(1, 300, 60), C, "1*:300,60,666,400"),
        (snap(1, Snap::BottomRight), C, "1@BottomRight*:600,350,500,300"),
        (mv(1, 300, 60), C, "1*:300,60,666,400"), // its own normal corner: the snap still clears
        (Minimize(WinId(1)), C, ""),
        (mv(1, 0, 0), N, ""),
    ]);

    // Resizes clamp, and dragging a snapped window's edge frees it.
    let mut wm = desk(AREA, None, 1);
    wm.play(&[
        (rs(1, 10, 20, 500, 300), C, "1*:10,20,500,300"),
        (rs(1, 10, 20, 500, 300), N, ""),
        (rs(1, 10, 20, 5, 5), C, "1*:10,20,320,200"),
        (rs(1, 0, 0, 99_999, 99_999), C, "1*:0,0,1920,1080"),
        (rs(1, 5000, -100, 400, 300), C, "1*:1856,0,400,300"),
        (rs(1, i32::MIN, i32::MIN, i32::MIN, i32::MIN), C, "1*:-256,0,320,200"),
        (Maximize(WinId(1)), C, ""),
        (rs(1, 0, 0, 500, 500), N, ""),
        (snap(1, Snap::Right), C, ""),
        (rs(1, 900, 0, 1020, 1080), C, "1*:900,0,1020,1080"),
        (Restore(WinId(1)), N, ""),
        (Minimize(WinId(1)), C, ""),
        (rs(1, 0, 0, 500, 500), N, ""),
    ]);
    assert_eq!(wm.normal_rect(WinId(1)), Some(r(900, 0, 1020, 1080)));
}

#[test]
fn maximize_restore_and_toggle() {
    let (mut wm, w) = (desk(AREA, None, 1), WinId(1));
    let normal = "1*:320,180,1280,720";
    wm.play(&[(Maximize(w), C, "1^*:0,0,1920,1080"), (Maximize(w), N, "")]);
    assert_eq!(wm.windows(), [(w, State::Maximized)]);
    assert_eq!(wm.normal_rect(w), Some(r(320, 180, 1280, 720)));
    wm.play(&[
        (Restore(w), C, normal),
        (Restore(w), N, ""),
        (ToggleMaximize(w), C, "1^*:0,0,1920,1080"),
        (ToggleMaximize(w), C, normal),
        // A snapped window toggles to maximized, then back to its normal rect.
        (snap(1, Snap::TopLeft), C, ""),
        (ToggleMaximize(w), C, "1^*:0,0,1920,1080"),
        (ToggleMaximize(w), C, normal),
        (snap(1, Snap::Left), C, ""),
        (Restore(w), C, normal),
        (O, op(2), ""),
    ]);
    // Each of them shows a minimized window on top, focused.
    for cmd in [Maximize(w), Restore(w), ToggleMaximize(w), snap(1, Snap::Left)] {
        wm.play(&[(Minimize(w), C, "2*:348,208,1280,720"), (cmd, C, "")]);
        assert_eq!(wm.focused(), Some(w));
    }
    assert_eq!(render(&wm), "2:348,208,1280,720 1@Left*:0,0,960,1080");
}

#[test]
fn snaps_split_the_area() {
    let area = r(10, 20, 1921, 1081);
    // Rows (full, top, bottom) by columns (left, right), in `SNAPS` order.
    let (cols, rows) = ([(10, 960), (970, 961)], [(20, 1081), (20, 540), (560, 541)]);
    let want = rows.iter().flat_map(|&(y, h)| cols.map(|(x, w)| r(x, y, w, h)));
    let mut wm = desk(area, None, 1);
    let normal = wm.normal_rect(WinId(1));
    for (s, want) in SNAPS.into_iter().zip(want) {
        assert_eq!(s.rect(area), want);
        wm.play(&[(snap(1, s), C, ""), (snap(1, s), N, "")]);
        let p = wm.layout()[0];
        let got = (p.rect, p.state, p.snap, wm.normal_rect(WinId(1)));
        assert_eq!(got, (want, State::Normal, Some(s), normal));
    }
    // Snap::rect normalizes the area first, so any input is safe.
    let wild = r(i32::MAX, i32::MIN, i32::MAX, 3);
    assert_eq!(Snap::Right.rect(wild), r(3 << 19, -MAX_COORD, 1 << 19, 3));
    assert_eq!(Snap::BottomRight.rect(r(0, 0, 1, 1)), r(0, 0, 1, 1));
    assert_eq!(Snap::TopLeft.rect(r(0, 0, 1, 1)), r(0, 0, 0, 0));
}

#[test]
fn set_area_pulls_windows_inside() {
    let mut wm = desk(AREA, Some((800, 600)), 4);
    wm.all(C, &[mv(1, 1500, 900), mv(2, -300, 100), Maximize(WinId(3))]);
    wm.all(C, &[snap(4, Snap::Right), Minimize(WinId(4))]);
    // An unchanged area changes nothing, though windows stick out.
    wm.all(N, &[SetArea(AREA)]);
    // Windows that fit move the least to lie inside; maximized and snapped ones
    // follow the area, and their normal rects come inside too.
    let fitted = "1:480,120,800,600 2:0,100,800,600 3^";
    wm.play(&[
        (SetArea(r(0, 0, 1280, 720)), C, &format!("{fitted}*:0,0,1280,720")),
        (Focus(WinId(4)), C, &format!("{fitted}:0,0,1280,720 4@Right*:640,0,640,720")),
    ]);
    let normals = |wm: &Wm| (1..=4).map(|n| wm.normal_rect(WinId(n)).unwrap()).collect::<Vec<_>>();
    assert_eq!(normals(&wm)[2..], [r(480, 120, 800, 600); 2]);
    // Larger windows shrink to the area, but never below the minimum.
    wm.all(C, &[SetArea(r(100, 50, 700, 500))]);
    assert_eq!(normals(&wm), [r(100, 50, 700, 500); 4]);
    // Extreme areas normalize; a hugely offset one drags windows along.
    wm.all(C, &[SetArea(r(i32::MIN, i32::MAX, i32::MIN, i32::MAX))]);
    assert_eq!(wm.area(), r(-MAX_COORD, MAX_COORD, 0, MAX_COORD));
    wm.all(N, &[SetArea(r(-MAX_COORD - 1, i32::MAX, -1, MAX_COORD + 1))]);
    assert_eq!(wm.normal_rect(WinId(1)), Some(r(-MAX_COORD - 64, MAX_COORD, 320, 500)));
    // Lower than a titlebar: the top edge sits on the area's top.
    wm.all(C, &[SetArea(r(0, 0, 500, 30))]);
    assert_eq!(wm.normal_rect(WinId(1)), Some(r(0, 0, 320, 200)));
    let tiny = "1:-59,5,320,200 2:-59,5,320,200 3^:5,5,0,0 4@Right*:5,5,0,0";
    wm.play(&[(SetArea(r(5, 5, 0, 0)), C, tiny)]);
}

#[test]
fn state_hash_is_canonical_and_pinned() {
    let build = |cmds: &[Cmd]| {
        let mut wm = Wm::new(AREA);
        cmds.iter().for_each(|c| assert!(wm.apply(c.clone()).is_ok()));
        wm
    };
    let detour = build(&[O, O, Focus(WinId(1)), Minimize(WinId(2)), Close(WinId(2))]);
    assert_eq!(build(&[O, O, Close(WinId(2))]).state_hash(), detour.state_hash());

    // Round trips come back to the same hash.
    let mut wm = desk(AREA, None, 3);
    let h = wm.state_hash();
    let trips = [
        vec![Maximize(WinId(1)), Restore(WinId(1))],
        vec![snap(2, Snap::TopRight), Restore(WinId(2))],
        vec![Focus(WinId(1)), Focus(WinId(2)), Focus(WinId(3))],
        vec![FocusNext, FocusPrev],
        vec![Minimize(WinId(3)), Focus(WinId(3))],
        vec![mv(1, 0, 0), mv(1, 320, 180)],
        vec![SetArea(r(0, 0, 1921, 1080)), SetArea(AREA)],
    ];
    for trip in trips {
        wm.all(C, &trip);
        assert_eq!(wm.state_hash(), h);
    }

    // A walk through distinct states hashes them all apart.
    let mut seen = BTreeSet::from([Wm::new(AREA).state_hash()]);
    let mut wm = Wm::new(AREA);
    let walk = [O, O, Focus(WinId(1)), Minimize(WinId(1)), Maximize(WinId(2)), snap(2, Snap::Left)];
    let walk2 = [snap(2, Snap::Right), mv(2, 10, 10), rs(2, 10, 10, 500, 400), Close(WinId(1))];
    for cmd in walk.into_iter().chain(walk2).chain([SetArea(r(0, 0, 1280, 720)), O, FocusNext]) {
        assert!(matches!(apply(&mut wm, &cmd).0, Ok(Changed | Opened(_))), "{cmd:?}");
        assert!(seen.insert(wm.state_hash()), "{cmd:?} revisits a hash");
    }

    // Pinned: a change to the encoding would orphan every recorded hash; the
    // last script also pins how SetArea refits and Open cascades.
    let mut script = vec![O, O, O, snap(1, Snap::TopRight), Maximize(WinId(2)), Minimize(WinId(3))];
    script.push(mv(1, 7, 9));
    let mut hashes = vec![Wm::new(AREA).state_hash(), build(&script).state_hash()];
    script.extend([SetArea(r(0, 0, 1280, 720)), O]);
    hashes.push(build(&script).state_hash());
    assert_eq!(hashes, GOLDEN);
}

const GOLDEN: [u64; 3] = [0x2457_e8dc_7bdb_028d, 0x565d_13d7_6a26_5cfb, 0x8507_b52b_6bce_12bd];

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
        let extreme = [i32::MIN, i32::MAX, MAX_COORD + 1].get(self.below(12) as usize).copied();
        extreme.unwrap_or_else(|| self.below(2600) as i32 - 300)
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
            0..=2 if live.len() < 24 => Open { size: size[self.below(3) as usize] },
            0..=3 => Close(win),
            4 | 5 => Focus(win),
            6 | 7 => Move { win, x, y },
            8 | 9 => Resize { win, rect: self.rect() },
            10 => Maximize(win),
            11 => Restore(win),
            12 => ToggleMaximize(win),
            13 | 14 => Minimize(win),
            15 => SnapTo { win, snap: SNAPS[self.below(6) as usize] },
            16 => FocusNext,
            17 => FocusPrev,
            _ => SetArea(self.rect()),
        }
    }
}

/// Applies `cmd` and checks it against the spec, given the open windows.
fn step(wm: &mut Wm, live: &mut Vec<WinId>, next: &mut u32, cmd: &Cmd) -> Res {
    let (res, before, after) = apply(wm, cmd);
    let ids = |v: &View| v.3.iter().map(|p| p.win).collect::<Vec<_>>();
    let (was, now) = (ids(&before), ids(&after));
    let place = |v: &View, w| v.3.iter().find(|p| p.win == w).copied();
    let target = match *cmd {
        Close(w) | Focus(w) | Maximize(w) | Restore(w) | ToggleMaximize(w) | Minimize(w) => Some(w),
        Move { win, .. } | Resize { win, .. } | SnapTo { win, .. } => Some(win),
        Open { .. } | FocusNext | FocusPrev | SetArea(_) => None,
    };
    if let Some(w) = target {
        // Window commands fail exactly when the window is not open.
        assert_eq!(res.is_err(), !live.contains(&w), "{cmd:?}");
        res?; // an unknown window: nothing more to check
        let (hidden, max) = (!was.contains(&w), place(&before, w).map(|p| p.state));
        // Close and Minimize take it off the stack, Focus raises it, and the
        // mode commands show it on top if hidden; nothing else restacks.
        let rest: Vec<WinId> = was.iter().copied().filter(|&x| x != w).collect();
        let raised = [rest.as_slice(), &[w]].concat();
        let stack = match cmd {
            Close(_) | Minimize(_) => rest,
            Move { .. } | Resize { .. } => was.clone(),
            Focus(_) => raised,
            _ if hidden => raised,
            _ => was.clone(),
        };
        assert_eq!(now, stack, "{cmd:?}");
        let p = place(&after, w);
        let state = p.map(|p| (p.state, p.snap));
        match *cmd {
            Close(_) => live.retain(|&x| x != w),
            Minimize(_) => assert!(p.is_none() && wm.normal_rect(w).is_some()),
            Maximize(_) => assert_eq!(state, Some((State::Maximized, None))),
            Restore(_) => assert_eq!(state, Some((State::Normal, None))),
            SnapTo { snap, .. } => assert_eq!(state, Some((State::Normal, Some(snap)))),
            ToggleMaximize(_) if !hidden => {
                let old = max.map(|s| s == State::Maximized);
                assert_eq!(p.map(|p| p.state != State::Maximized), old, "{cmd:?}");
            }
            // Minimized windows ignore moves and resizes; maximized ones resizes.
            Move { .. } | Resize { .. } if hidden => assert_eq!(res, N),
            Resize { .. } if max == Some(State::Maximized) => assert_eq!(res, N),
            Resize { .. } => assert_eq!(state, Some((State::Normal, None))),
            Move { .. } => {
                let size = |r: Option<Rect>| r.map(|r| (r.w, r.h));
                let normal = before.4.iter().find(|e| e.0 == w).and_then(|e| e.2);
                assert_eq!(size(p.map(|p| p.rect)), size(normal));
                assert_eq!(state, Some((State::Normal, None)));
            }
            _ => {}
        }
    }
    match *cmd {
        Open { size } => {
            assert_eq!(res, op(*next));
            let (a, p) = (after.1, *after.3.last().expect("opened on top"));
            let (w, h) = size.unwrap_or((a.w * 2 / 3, a.h * 2 / 3));
            let (w, h) = (w.clamp(MIN_W, MIN_W.max(a.w)), h.clamp(MIN_H, MIN_H.max(a.h)));
            // At the top window's corner plus CASCADE, else the area's, else centered.
            let fits = |&(x, y): &(i32, i32)| x + w <= a.x + a.w && y + h <= a.y + a.h;
            let base = before.3.last().map(|t| [(t.rect.x, t.rect.y), (a.x, a.y)]);
            let at = base.into_iter().flatten().map(|(x, y)| (x + CASCADE, y + CASCADE)).find(fits);
            let (x, y) = at.unwrap_or((a.x + (a.w - w) / 2, a.y + (a.h - h) / 2));
            let want = r(x.max(a.x + VISIBLE_W - w), y.max(a.y), w, h); // then kept visible
            assert_eq!((p.win, p.focused, p.rect), (WinId(*next), true, want));
            live.push(WinId(*next));
            *next += 1;
        }
        SetArea(a) => {
            let c = |v: i32, lo: i32| v.clamp(lo, MAX_COORD);
            let a = r(c(a.x, -MAX_COORD), c(a.y, -MAX_COORD), c(a.w, 0), c(a.h, 0));
            assert_eq!((wm.area(), now), (a, was));
            // A new area shrinks normal rects to it and moves them the least to
            // lie inside it, on each axis where they fit.
            for (old, new) in before.4.iter().zip(&after.4).filter(|_| res == C) {
                let (o, n) = (old.2.expect("open"), new.2.expect("open"));
                let (w, h) = (o.w.min(a.w).max(MIN_W), o.h.min(a.h).max(MIN_H));
                assert_eq!((n.w, n.h), (w, h), "{o:?} in {a:?}");
                assert!(w > a.w || n.x == o.x.clamp(a.x, a.x + a.w - w), "{o:?} {n:?} {a:?}");
                assert!(h > a.h || n.y == o.y.clamp(a.y, a.y + a.h - h), "{o:?} {n:?} {a:?}");
            }
        }
        FocusNext | FocusPrev if was.len() > 1 => {
            let mut want = was.clone();
            want.rotate_left(if *cmd == FocusNext { was.len() - 1 } else { 1 });
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
                changers.insert(format!("{cmd:?}").split(['(', ' ']).next().map(String::from));
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
