//! Black-box tests for `wm`, written from its spec alone (public API and
//! semantics), independently of `src/`.
//!
//! Most tests are `play` scripts: `;`-separated steps per line, then maybe
//! `| layout`, the active workspace (or `view N`) as `id:x,y,w,h` items in
//! layout order, `~` floating, `*` focused (`| ... tail` checks the end).
//! Steps must report Changed (Opened for `open N`/`float N`) unless they end
//! in `= noop`, `= unknown`, `= nows` or `= tiled`. Commands: open, float,
//! close W, focus W, fdir D, move D, resize D PX, tfloat, torient, rect W X
//! Y W H, ws I, mv W I, area X Y W H, gaps O I (D: < > ^ v; max/min: i32).
//! Directives: new X Y W H OUTER INNER N, mark, same (hash == mark), seen
//! (hash unlike any earlier seen). Noop and Err must leave the hash and all
//! observable state untouched; anything else must change the hash.

use std::collections::BTreeSet;
use wm::Dir::{Down, Left, Right, Up};
use wm::Outcome::{Changed, Noop, Opened};
use wm::{Axis, Cmd, Dir, FLOAT_MIN, Gaps, MAX_COORD, MAX_WORKSPACES, MIN_TILE};
use wm::{Outcome, Placement, RATIO_MIN, RATIO_ONE, Rect, WinId, Wm, WmError};

type Res = Result<Outcome, WmError>;
type Snap = (u64, (Rect, Gaps, usize), Option<WinId>, Vec<Vec<Placement>>);

fn gaps(outer: i32, inner: i32) -> Gaps {
    Gaps { outer, inner }
}

fn rect(n: &[i32]) -> Rect {
    Rect::new(n[0], n[1], n[2], n[3])
}

fn render(lay: &[Placement]) -> String {
    let one = |p: &Placement| {
        let tag = ["", "*", "~", "~*"][usize::from(p.floating) * 2 + usize::from(p.focused)];
        let r = p.rect;
        format!("{}{tag}:{},{},{},{}", p.win.0, r.x, r.y, r.w, r.h)
    };
    lay.iter().map(one).collect::<Vec<_>>().join(" ")
}

fn snap(wm: &Wm) -> Snap {
    let lays = (0..wm.workspace_count()).map(|i| wm.layout_of(i)).collect();
    let top = (wm.area(), wm.gaps(), wm.active_workspace());
    (wm.state_hash(), top, wm.focused(), lays)
}

/// A script number: an integer, `max`/`min`, or a direction's index.
fn num(w: &str) -> i32 {
    let dir = ["<", ">", "^", "v"].iter().position(|d| *d == w);
    match w {
        "max" => i32::MAX,
        "min" => i32::MIN,
        _ => dir.map_or_else(|| w.parse().expect(w), |i| i as i32),
    }
}

/// Runs a script on a 1920x1080 area, gaps 8/8, 4 workspaces; returns the Wm.
fn play(script: &str) -> Wm {
    let mut wm = Wm::new(Rect::new(0, 0, 1920, 1080), gaps(8, 8), 4);
    let (mut mark, mut seen) = (0, BTreeSet::new());
    for line in script.lines().map(|l| l.split('#').next().unwrap().trim()) {
        let mut view = None;
        for step in line.split('|').next().unwrap().split(';').map(str::trim) {
            let (body, out) = step.split_once(" = ").unwrap_or((step, "ok"));
            let (name, args) = body.split_once(' ').unwrap_or((body, ""));
            let n: Vec<i32> = args.split_whitespace().map(num).collect();
            match name {
                "" => {}
                "new" => wm = Wm::new(rect(&n), gaps(n[4], n[5]), n[6] as usize),
                "view" => view = Some(n[0] as usize),
                "mark" => mark = wm.state_hash(),
                "same" => assert_eq!(wm.state_hash(), mark, "{line}"),
                "seen" => assert!(seen.insert(wm.state_hash()), "{line}"),
                _ => exec(&mut wm, name, &n, out, line),
            }
        }
        if let Some((_, want)) = line.split_once('|') {
            let got = render(&wm.layout_of(view.unwrap_or(wm.active_workspace())));
            match want.trim().strip_prefix("... ") {
                Some(tail) => assert!(got.ends_with(tail), "{line}\n got: {got}"),
                None => assert_eq!(got, want.trim(), "{line}"),
            }
        }
    }
    wm
}

/// Applies one script command and checks its result and the hash rules.
fn exec(wm: &mut Wm, name: &str, n: &[i32], out: &str, line: &str) {
    let a = |i: usize| n.get(i).copied().unwrap_or(0);
    let (win, ws) = (WinId(a(0) as u32), a(n.len().max(1) - 1) as usize);
    let dir = [Left, Right, Up, Down][a(0).clamp(0, 3) as usize];
    let r = |k: usize| rect(&n[k..]);
    let cmd = match name {
        "open" => Cmd::Open { floating: false },
        "float" => Cmd::Open { floating: true },
        "close" => Cmd::Close(win),
        "focus" => Cmd::Focus(win),
        "fdir" => Cmd::FocusDir(dir),
        "move" => Cmd::MoveDir(dir),
        "resize" => Cmd::Resize { dir, px: a(1) },
        "tfloat" => Cmd::ToggleFloat,
        "torient" => Cmd::ToggleOrientation,
        "rect" => Cmd::SetFloatRect { win, rect: r(1) },
        "ws" => Cmd::SwitchWorkspace(ws),
        "mv" => Cmd::MoveToWorkspace { win, ws },
        "area" => Cmd::SetArea(r(0)),
        "gaps" => Cmd::SetGaps(gaps(a(0), a(1))),
        _ => panic!("bad command in: {line}"),
    };
    let want = match out {
        "ok" if matches!(cmd, Cmd::Open { .. }) => Ok(Opened(win)),
        "ok" => Ok(Changed),
        "noop" => Ok(Noop),
        "unknown" => Err(WmError::UnknownWindow(win)),
        "nows" => Err(WmError::NoSuchWorkspace(ws)),
        "tiled" => Err(WmError::NotFloating(win)),
        _ => panic!("bad outcome in: {line}"),
    };
    let before = snap(wm);
    assert_eq!(wm.apply(cmd), want, "{name} in: {line}");
    let (idle, after) = (matches!(want, Ok(Noop) | Err(_)), snap(wm));
    assert!(!idle || after == before, "{name} changed state in: {line}");
    assert!(idle || after.0 != before.0, "hash kept in: {line}");
}

fn norm(r: Rect) -> Rect {
    let (c, m) = (|v: i32, lo: i32| v.clamp(lo, MAX_COORD), -MAX_COORD);
    Rect::new(c(r.x, m), c(r.y, m), c(r.w, 0), c(r.h, 0))
}

fn norm_gaps(g: Gaps) -> Gaps {
    gaps(g.outer.clamp(0, MAX_COORD), g.inner.clamp(0, MAX_COORD))
}

fn keep_visible(r: Rect, a: Rect) -> Rect {
    let size = |v: i32, ext: i32| v.clamp(FLOAT_MIN, FLOAT_MIN.max(ext));
    let (w, h) = (size(r.w, a.w), size(r.h, a.h));
    let fit = |p: i32, len: i32, start: i32, ext: i32| {
        let (lo, hi) = (start + FLOAT_MIN - len, start + ext - FLOAT_MIN);
        if lo > hi { start } else { p.clamp(lo, hi) }
    };
    Rect::new(fit(r.x, w, a.x, a.w), fit(r.y, h, a.y, a.h), w, h)
}

fn v(r: Rect) -> [i64; 4] {
    [r.x, r.y, r.w, r.h].map(i64::from)
}

/// Overlap of `a` and `b` along axis `k` (0 = x, 1 = y); <= 0 means none.
fn overlap(a: [i64; 4], b: [i64; 4], k: usize) -> i64 {
    (a[k] + a[k + 2]).min(b[k] + b[k + 2]) - a[k].max(b[k])
}

/// FocusDir's ranking key for candidate `c` seen from `f`, if `c` qualifies.
fn key(f: Rect, c: Rect, d: Dir) -> Option<(i64, i64, i64)> {
    let (f, c, k) = (v(f), v(c), usize::from(matches!(d, Up | Down)));
    let gap = match d {
        Right | Down => c[k] - f[k] - f[k + 2],
        Left | Up => f[k] - c[k] - c[k + 2],
    };
    let (p, ov) = (1 - k, overlap(f, c, 1 - k));
    let centers = (2 * c[p] + c[p + 2] - 2 * f[p] - f[p + 2]).abs();
    (gap >= 0 && ov >= 1).then_some((gap, -ov, centers))
}

/// The window FocusDir (or, with `tiled`, MoveDir) must pick in `lay`.
fn pick(lay: &[Placement], d: Dir, tiled: bool) -> Option<WinId> {
    let ok = |p: &&Placement| !tiled || !p.floating;
    let f = lay.iter().filter(ok).find(|p| p.focused)?;
    let cands = lay.iter().filter(ok).filter(|c| c.win != f.win);
    let ranked = cands.filter_map(|c| Some((key(f.rect, c.rect, d)?, c.win)));
    ranked.min().map(|(_, w)| w)
}

#[test]
fn worked_example_and_ids() {
    let s = "open 1 | 1*:8,8,1904,1064
        open 2 | 1:8,8,948,1064 2*:964,8,948,1064
        open 3 | 1:8,8,948,1064 2:964,8,948,528 3*:964,544,948,528
        close 3; ws 2; float 4; open 5 | 5*:8,8,1904,1064 4~:320,180,1280,720
        view 0 | 1:8,8,948,1064 2*:964,8,948,1064  # ids are global, never reused
        close 1; close 2; close 4; close 5; open 6 | 6*:8,8,1904,1064
        close 0 = unknown; close 3 = unknown; focus 7 = unknown; close max = unknown";
    let wm = play(s);
    let q = |i| (wm.workspace_of(WinId(i)), wm.is_floating(WinId(i)));
    assert_eq!([q(4), q(6)], [(None, None), (Some(2), Some(false))]);
    let e: Box<dyn std::error::Error> = Box::new(WmError::NotFloating(WinId(6)));
    assert!(!e.to_string().is_empty() && Axis::Horizontal != Axis::Vertical);
}

#[test]
fn normalization_and_degenerate_geometry() {
    let c = (RATIO_ONE, RATIO_MIN, MIN_TILE, FLOAT_MIN, MAX_COORD);
    assert_eq!(c, (65_536, 3_277, 48, 48, 1 << 20));
    let wm = Wm::new(Rect::new(i32::MIN, i32::MAX, -5, 9), gaps(-3, i32::MAX), 0);
    assert_eq!(wm.area(), Rect::new(-MAX_COORD, MAX_COORD, 0, 9));
    assert_eq!((wm.gaps(), wm.workspace_count()), (gaps(0, MAX_COORD), 1));
    let same = Wm::new(wm.area(), wm.gaps(), 1).state_hash();
    assert!(same == wm.state_hash() && wm.layout_of(usize::MAX).is_empty());
    let new = |n| Wm::new(Rect::default(), Gaps::default(), n);
    let counts = [5, 65, usize::MAX].map(|n| new(n).workspace_count());
    assert_eq!(counts, [5, MAX_WORKSPACES, 64]);
    assert_ne!(new(2).state_hash(), new(3).state_hash());
    let s = "new 0 0 0 0 8 8 1; open 1; open 2; open 3; float 4  # zero area
        | 1:8,8,0,0 2:8,8,0,0 3:8,8,0,0 4~*:0,0,48,48
        focus 2; tfloat; focus 1 | 1*:8,8,0,0 3:8,8,0,0 4~:0,0,48,48 2~:0,0,48,48
        resize > max = noop  # avail 0
        new 5 5 20 10 3 4 1; open 1; open 2; open 3 | 1:8,8,5,4 2:17,8,0,4 3*:21,8,1,4
        gaps 60 4 | 1:65,65,0,0 2:65,65,0,0 3*:65,65,0,0  # outer beyond the area
        new 0 0 max max 0 0 1; open 1; open 2  # huge: RATIO_MIN binds (I4)
        | 1:0,0,524288,1048576 2*:524288,0,524288,1048576
        resize < max | 1:0,0,52432,1048576 2*:52432,0,996144,1048576
        resize < max = noop; focus 1; resize > max
        | 1*:0,0,996144,1048576 2:996144,0,52432,1048576
        float 3 | ... 3~*:174763,174763,699050,699050
        gaps max min; gaps 1048577 -1 = noop
        | ... 2:1048576,1048576,0,0 3~*:174763,174763,699050,699050
        rect 3 max min max min | ... 3~*:1048528,0,1048576,48
        area 0 0 max 1048576 = noop; area 10 20 -100 -1
        | ... 2:1048586,1048596,0,0 3~*:10,20,48,48";
    assert_eq!(play(s).area(), Rect::new(10, 20, 0, 0));
}

#[test]
fn close_focus_and_move_dir() {
    let s = "open 1; open 2; open 3; focus 2; resize v 100
        | 1:8,8,948,1064 2*:964,8,948,628 3:964,644,948,428
        close 1 | 2*:8,8,1904,628 3:8,644,1904,428  # the sibling keeps its ratio
        close 2 | 3*:8,8,1904,1064  # focus falls back along the history
        close 3; close 3 = unknown; focus 3 = unknown |
        open 4; float 5; float 6; focus 6 = noop
        focus 5 | 4:8,8,1904,1064 6~:320,180,1280,720 5~*:320,180,1280,720
        focus 4; focus 4 = noop | 4*:8,8,1904,1064 6~:320,180,1280,720 5~:320,180,1280,720
        ws 1; open 7; focus 4 | 4*:8,8,1904,1064 6~:320,180,1280,720 5~:320,180,1280,720
        view 1 | 7*:8,8,1904,1064
        close 4 | 6~:320,180,1280,720 5~*:320,180,1280,720
        new 0 0 1920 1080 8 8 4; move < = noop; open 1; open 2; open 3
        move < | 3*:8,8,948,1064 2:964,8,948,528 1:964,544,948,528
        move < = noop; move ^ = noop; move v = noop
        float 4; rect 4 956 100 48 48; move > = noop  # the focused window floats
        focus 3; fdir > | ... 4~*:956,100,48,48  # FocusDir sees the float at gap 0
        # MoveDir ignores floats; tiled 1 and 2 tie on everything but the id.
        focus 3; move > | 1:8,8,948,1064 2:964,8,948,528 3*:964,544,948,528 4~:956,100,48,48";
    play(s);
}

/// FocusDir(d) from float 1 at (400, 400, 100, 100) among floats 2 and 3 at
/// `r2` and `r3`, under both stacking orders of 2 and 3; `want` 0 = Noop.
fn dir_is(r2: &str, r3: &str, d: &str, want: usize) {
    let rects = ["400 400 100 100", r2, r3];
    let (w, noop) = (want.max(1), [" = noop", ""][want.min(1)]);
    let top = format!("{w}~*:{}", rects[w - 1].replace(' ', ","));
    let s = "new 0 0 1000 1000 0 0 1; float 1; float 2; float 3; rect 1 400 400 100 100";
    for raise in ["focus 2; focus 1", "focus 1"] {
        play(&format!(
            "{s}; rect 2 {r2}; rect 3 {r3}; {raise}; fdir {d}{noop} | ... {top}"
        ));
    }
}

#[test]
fn focus_dir_ranking_and_tie_breaks() {
    dir_is("600 400 100 100", "550 480 60 60", ">", 3); // (1) gap beats overlap
    dir_is("600 425 100 50", "600 400 100 80", ">", 3); // (2) overlap beats centers
    dir_is("600 350 100 100", "600 450 100 60", ">", 3); // (3) centers beat ids
    dir_is("600 300 100 150", "600 450 100 150", ">", 2); // (4) full tie: smaller id
    dir_is("500 499 60 60", "900 400 100 100", ">", 2); // gap 0, 1 px overlap count
    dir_is("450 400 100 100", "600 500 100 100", ">", 0); // not beyond; no overlap
    dir_is("100 400 100 100", "250 450 100 100", "<", 3);
    dir_is("400 100 100 100", "430 250 48 100", "^", 3);
    dir_is("400 600 100 100", "430 250 48 100", "v", 2);
    let s = "fdir < = noop; open 1; fdir < = noop; open 2; open 3; focus 1
        fdir < = noop; fdir ^ = noop; fdir v = noop
        fdir > | 1:8,8,948,1064 2*:964,8,948,528 3:964,544,948,528  # 2 and 3 tie: 2
        fdir v; fdir <; fdir > | 1:8,8,948,1064 2*:964,8,948,528 3:964,544,948,528";
    play(s);
}

#[test]
fn resize_moves_the_divider_exactly() {
    // avail 1896 <= RATIO_ONE, so while the new a_len stays in [95, 1801] (no
    // clamp binds) the focused window's width changes by exactly px.
    let mut wm = play("open 1; open 2");
    let pxs = [100, -300, 1, -1, 7, 555, -13, 600, 3, -1609, -1706, -64];
    for (i, px) in pxs.into_iter().enumerate() {
        let (win, dir, k) = [(1, Right, 0), (2, Left, 1)][i % 2];
        assert!(wm.apply(Cmd::Focus(WinId(win))).is_ok());
        let old = wm.layout()[k].rect.w;
        assert_eq!(wm.apply(Cmd::Resize { dir, px }), Ok(Changed), "step {i}");
        let (l, a) = (wm.layout(), wm.layout()[0].rect.w);
        assert_eq!(l[k].rect.w, old + px, "step {i}");
        assert_eq!(l[0].rect, Rect::new(8, 8, a, 1064));
        assert_eq!(l[1].rect, Rect::new(16 + a, 8, 1896 - a, 1064));
    }
}

#[test]
fn resize_clamps_and_picks_the_split() {
    let s = "new 0 0 608 400 0 8 1; resize > 5 = noop; open 1; resize > 5 = noop
        open 2; focus 1; resize < 50 = noop; resize ^ 50 = noop; resize > 0 = noop
        resize > 10000 | 1*:0,0,552,400 2:560,0,48,400  # MIN_TILE binds
        resize > 1 = noop; resize > -10000 | 1*:0,0,48,400 2:56,0,552,400
        float 3; resize < 5 = noop; area 0 0 100 100; focus 1; resize > 10 = noop  # avail 92
        # The divider must lie on the dir side: 3 is the b child of the inner split.
        new 0 0 2000 500 0 0 1; open 1; open 2; focus 1; open 3
        | 1:0,0,500,500 3*:500,0,500,500 2:1000,0,1000,500
        resize > 100 | 1:0,0,550,500 3*:550,0,550,500 2:1100,0,900,500
        resize < 100 | 1:0,0,450,500 3*:450,0,650,500 2:1100,0,900,500
        # Walking up skips splits on the other axis.
        new 0 0 1920 1080 8 8 4; open 1; open 2; open 3; resize < 50
        | 1:8,8,898,1064 2:914,8,998,528 3*:914,544,998,528
        focus 2; resize ^ 50 = noop
        # The nearest matching split is too small (avail 51): stop there.
        new 0 0 1000 1000 0 0 1; open 1; open 2; open 3; torient; focus 1; resize > max
        focus 3; resize < 10 = noop";
    play(s);
}

#[test]
fn toggles_and_workspaces() {
    let s = "tfloat = noop; torient = noop; open 1; torient = noop; open 2; open 3; mark
        tfloat | 1:8,8,948,1064 2:964,8,948,1064 3~*:964,544,948,528
        tfloat; same | 1:8,8,948,1064 2:964,8,948,528 3*:964,544,948,528
        focus 2; torient | 1:8,8,948,1064 2*:964,8,470,1064 3:1442,8,470,1064
        focus 1; resize > 100; torient | 1*:8,8,1904,583 2:8,599,948,473 3:964,599,948,473
        float 4; torient = noop; resize < 9 = noop; tfloat  # splits 1, not the float
        | 1:8,8,948,583 4*:964,8,948,583 2:8,599,948,473 3:964,599,948,473
        ws 1; open 5; float 6; open 7 | 5:8,8,948,1064 7*:964,8,948,1064 6~:320,180,1280,720
        new 0 0 1920 1080 8 8 4; open 1; open 2; open 3; ws 4 = nows; ws max = nows
        mv 1 4 = nows; mv 7 1 = unknown; mv 7 9 = unknown; ws 0 = noop; mv 1 0 = noop
        mv 3 1 | 1:8,8,948,1064 2*:964,8,948,1064  # the active workspace stays
        mv 1 1 | 2*:8,8,1904,1064  # splits ws 1's most recently focused tiled window
        view 1 | 3:8,8,948,1064 1*:964,8,948,1064
        float 4; rect 4 10 20 300 200; mv 4 1 | 2*:8,8,1904,1064
        ws 1 | 3:8,8,948,1064 1:964,8,948,1064 4~*:10,20,300,200  # same rect, on top
        ws 3; ws 0; close 4 | 2*:8,8,1904,1064
        view 1 | 3:8,8,948,1064 1*:964,8,948,1064";
    play(s);
}

#[test]
fn floats_stay_visible() {
    let s = "open 1; float 2; float 3; rect 1 0 0 99 99 = tiled; rect 9 0 0 99 99 = unknown
        rect 2 320 180 1280 720 = noop; rect 2 5000 -5000 10 99999
        | 1:8,8,1904,1064 2~:1872,-1032,48,1080 3~*:320,180,1280,720
        rect 2 9999 -9999 1 5000 = noop; rect 2 min max min max
        | ... 2~:0,1032,48,1080 3~*:320,180,1280,720
        # SetArea re-applies keep_visible to floats on every workspace.
        rect 3 1800 1000 100 60; ws 1; float 4; open 5; rect 4 -40 -10 200 100
        area 0 0 1920 1080 = noop; area 0 0 800 600 | 5*:8,8,784,584 4~:-40,-10,200,100
        view 0 | 1:8,8,784,584 2~:0,552,48,600 3~*:752,552,100,60
        area 100 100 150 50 | 5*:108,108,134,34 4~:-2,98,150,50
        view 0 | 1:108,108,134,34 2~:100,102,48,50 3~*:202,102,100,50
        area 0 0 3000 3000; view 0 | 1:8,8,2984,2984 2~:100,102,48,50 3~*:202,102,100,50
        float 6 | ... 6~*:500,500,2000,2000  # a floating Open uses the current area
        new 0 0 1000 500 0 0 1; open 1; open 2 | 1:0,0,500,500 2*:500,0,500,500
        gaps -5 min = noop; gaps 10 20; gaps 10 20 = noop | 1:10,10,480,480 2*:510,10,480,480
        new 0 0 1001 1001 0 0 1; float 1 | 1~*:167,167,667,667  # w * 2 / 3, not w / 3 * 2
        new -1048576 0 1048576 1000 0 0 1; float 1; rect 1 min 0 1048576 100 | 1~*:-1048576,0,1048576,100";
    play(s);
}

#[test]
fn state_hash_depends_only_on_logical_state() {
    // The same logical state via different histories, round trips back to a
    // marked state, and a walk through states that must all hash apart.
    let s = "open 1; open 2; open 3; close 2; mark | 1:8,8,948,1064 3*:964,8,948,1064
        new 0 0 1920 1080 8 8 4; open 1; open 2; close 2; open 3; same
        new 0 0 1920 1080 8 8 4; open 1; open 2; open 3; mv 2 3; close 2; same
        new 0 0 1920 1080 8 8 4; open 1; open 2; open 3; mark; tfloat; tfloat; same
        torient; torient; same; ws 2; ws 0; same; resize < 100; resize < -100; same
        move ^; move v; same; gaps 0 0; gaps 8 8; same; focus 1; focus 2; focus 3; same
        new 0 0 1920 1080 8 8 4; seen; open 1; seen; close 1; seen; open 2; open 3; seen
        focus 2; seen; move >; seen; torient; seen; tfloat; seen; rect 2 0 0 100 100; seen
        ws 1; seen; mv 3 1; seen; area 0 0 1920 1081; seen; gaps 8 9; seen";
    let wm = play(s);
    assert_eq!(play(s).state_hash(), wm.state_hash());
    assert_eq!(wm.clone().state_hash(), wm.state_hash());
}

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005);
        self.0 = self.0.wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n.max(1))) as u32
    }

    /// Mostly ordinary coordinates, sometimes extremes beyond MAX_COORD.
    fn int(&mut self) -> i32 {
        let (i, v) = (self.below(16) as usize, self.below(2600) as i32 - 300);
        *[i32::MIN, i32::MAX, MAX_COORD + 1].get(i).unwrap_or(&v)
    }

    /// Any rect at all or, usually when `tame`, an ordinary screen area.
    fn rect(&mut self, tame: bool) -> Rect {
        let n = [100, 100, 2400, 1400].map(|s| self.below(s) as i32);
        let wild = Rect::new(self.int(), self.int(), self.int(), self.int());
        [wild, Rect::new(n[0] - 50, n[1] - 50, n[2], n[3])][usize::from(tame && self.below(4) > 0)]
    }
}

/// Each workspace's focus history as (window, floats?), least recent first;
/// by the spec its floating stack is its floating windows in this order.
#[derive(Default)]
struct Model {
    hist: Vec<Vec<(WinId, bool)>>,
    active: usize,
    next: u32,
}

impl Model {
    fn find(&self, w: WinId) -> Option<(usize, bool)> {
        let ws = self.hist.iter().position(|h| h.iter().any(|e| e.0 == w))?;
        Some((ws, self.hist[ws].contains(&(w, true))))
    }

    fn put(&mut self, w: WinId, to: Option<(usize, bool)>) {
        let ws = self.find(w).expect("live window").0;
        self.hist[ws].retain(|e| e.0 != w);
        if let Some((ws, fl)) = to {
            self.hist[ws].push((w, fl));
        }
    }

    fn focus(&mut self, w: WinId) {
        let at = self.find(w);
        self.put(w, at);
        self.active = at.unwrap().0;
    }
}

fn random_cmd(r: &mut Lcg, m: &Model, cap: usize) -> Cmd {
    let live: Vec<WinId> = m.hist.iter().flatten().map(|e| e.0).collect();
    let any = WinId(r.below(m.next + 2)); // 0, closed, live or not yet opened
    let i = r.below(live.len() as u32) as usize;
    let win = *live.get(i).filter(|_| r.below(5) > 0).unwrap_or(&any);
    let ws = r.below(m.hist.len() as u32 + 2) as usize;
    let dir = [Left, Right, Up, Down][r.below(4) as usize];
    let (rect, floating) = (r.rect(false), r.below(3) == 0);
    let px = [0, r.int(), r.below(241) as i32 - 120][r.below(6).min(2) as usize];
    let g = gaps(r.below(30) as i32 - 4, r.int() / 64);
    match r.below(24) {
        0..=3 if live.len() < cap => Cmd::Open { floating },
        0..=5 => Cmd::Close(win),
        6 => Cmd::Focus(win),
        7..=9 => Cmd::FocusDir(dir),
        10 | 11 => Cmd::MoveDir(dir),
        12..=14 => Cmd::Resize { dir, px },
        15 => Cmd::ToggleFloat,
        16 => Cmd::ToggleOrientation,
        17 => Cmd::SetFloatRect { win, rect },
        18 => Cmd::SwitchWorkspace(ws),
        19 | 20 => Cmd::MoveToWorkspace { win, ws },
        21 => Cmd::SetArea(r.rect(true)),
        _ => Cmd::SetGaps(g),
    }
}

/// Applies `cmd`, checking its result against the model's prediction and I6.
fn step(wm: &mut Wm, m: &mut Model, cmd: &Cmd) -> Res {
    let (s, lay) = (snap(wm), wm.layout());
    let (area, cur) = (s.1.0, m.hist[m.active].last().copied());
    let was = |w| s.3.iter().flatten().find(|p| p.win == w).map(|p| p.rect);
    let ok = |same: bool| Some(Ok([Changed, Noop][usize::from(same)]));
    let unknown = |w| Some(Err(WmError::UnknownWindow(w)));
    let bad_ws = |i| Some(Err(WmError::NoSuchWorkspace(i)));
    let want = match *cmd {
        Cmd::Open { floating } => {
            m.hist[m.active].push((WinId(m.next), floating));
            m.next += 1;
            Some(Ok(Opened(WinId(m.next - 1))))
        }
        Cmd::Close(w) | Cmd::Focus(w) if m.find(w).is_none() => unknown(w),
        Cmd::Close(w) => ok(false).inspect(|_| m.put(w, None)),
        Cmd::Focus(w) => ok(cur.map(|c| c.0) == Some(w)).inspect(|_| m.focus(w)),
        Cmd::FocusDir(d) => ok(pick(&lay, d, false).inspect(|&w| m.focus(w)).is_none()),
        Cmd::MoveDir(d) => ok(pick(&lay, d, true).is_none()),
        Cmd::Resize { px, .. } => (cur.is_none_or(|c| c.1) || px == 0).then_some(Ok(Noop)),
        Cmd::ToggleFloat => {
            let flip = cur.map(|(f, fl)| m.put(f, Some((m.active, !fl))));
            ok(flip.is_none())
        }
        Cmd::ToggleOrientation => {
            let tiled = m.hist[m.active].iter().filter(|e| !e.1).count();
            ok(cur.is_none_or(|c| c.1) || tiled == 1)
        }
        Cmd::SetFloatRect { win, rect } => match m.find(win) {
            None => unknown(win),
            Some((_, false)) => Some(Err(WmError::NotFloating(win))),
            Some(_) => ok(Some(keep_visible(norm(rect), area)) == was(win)),
        },
        Cmd::SwitchWorkspace(i) if i >= m.hist.len() => bad_ws(i),
        Cmd::SwitchWorkspace(i) => ok(i == m.active).inspect(|_| m.active = i),
        Cmd::MoveToWorkspace { win, ws } => match (m.find(win), ws < m.hist.len()) {
            (None, _) => unknown(win), // the window is checked before ws
            (Some(_), false) => bad_ws(ws),
            (Some((src, _)), true) if src == ws => ok(true),
            (Some((_, fl)), true) => ok(false).inspect(|_| m.put(win, Some((ws, fl)))),
        },
        Cmd::SetArea(r) => ok(norm(r) == area),
        Cmd::SetGaps(g) => ok(norm_gaps(g) == s.1.1),
    };
    let res = wm.apply(cmd.clone());
    match want {
        Some(w) => assert_eq!(res, w, "{cmd:?}"),
        None => assert!(matches!(res, Ok(Changed | Noop)), "{cmd:?}: {res:?}"),
    }
    let (now, idle) = (snap(wm), matches!(res, Ok(Noop) | Err(_)));
    assert!(!idle || now == s, "I6: {cmd:?} changed state");
    assert!(idle || now.0 != s.0, "{cmd:?} kept the hash");
    match cmd {
        Cmd::SetArea(r) => assert_eq!(wm.area(), norm(*r)),
        Cmd::SetGaps(g) => assert_eq!(wm.gaps(), norm_gaps(*g)),
        &Cmd::MoveDir(d) if res == Ok(Changed) => {
            // The focused window and its tiled neighbour trade rects.
            let (f, x) = (cur.unwrap().0, pick(&lay, d, true).unwrap());
            for p in now.3.iter().flatten() {
                let from = [p.win, x, f][usize::from(p.win == f) + 2 * usize::from(p.win == x)];
                assert_eq!(Some(p.rect), was(from), "{cmd:?}");
            }
        }
        _ => {}
    }
    res
}

/// I1-I5, plus agreement with the model on focus, stacking and ids.
fn check(wm: &Wm, m: &Model) {
    assert_eq!(wm.active_workspace(), m.active);
    assert_eq!(wm.workspace_count(), m.hist.len());
    assert_eq!(wm.focused(), m.hist[m.active].last().map(|e| e.0));
    assert_eq!(wm.layout(), wm.layout_of(m.active));
    let (a, o) = (wm.area(), i64::from(wm.gaps().outer));
    let span = |len: i32| (i64::from(len) - 2 * o).max(0);
    let lo = [i64::from(a.x) + o, i64::from(a.y) + o];
    let hi = [lo[0] + span(a.w), lo[1] + span(a.h)];
    for (ws, hist) in m.hist.iter().enumerate() {
        let lay = wm.layout_of(ws);
        let ids: BTreeSet<WinId> = lay.iter().map(|p| p.win).collect();
        let want: BTreeSet<WinId> = hist.iter().map(|e| e.0).collect();
        assert!(ids == want && lay.len() == hist.len(), "I1 on ws {ws}");
        let floats: Vec<WinId> = lay.iter().filter(|p| p.floating).map(|p| p.win).collect();
        let stack: Vec<WinId> = hist.iter().filter(|e| e.1).map(|e| e.0).collect();
        assert_eq!(floats, stack, "floating stack order");
        assert!(lay.iter().skip_while(|p| !p.floating).all(|p| p.floating));
        let focused = lay.iter().filter(|p| p.focused).map(|p| p.win);
        assert!(focused.eq(hist.last().map(|e| e.0)), "I2 on ws {ws}");
        for (i, p) in lay.iter().enumerate() {
            let meta = (wm.workspace_of(p.win), wm.is_floating(p.win));
            assert_eq!(meta, (Some(ws), Some(p.floating)));
            let r = v(p.rect);
            assert!(r[2] >= 0 && r[3] >= 0, "{p:?}");
            if p.floating {
                assert!(p.rect.w.min(p.rect.h) >= FLOAT_MIN, "I5: {p:?}");
                assert_eq!(keep_visible(p.rect, a), p.rect, "floats stay visible");
                continue;
            }
            let inside = (0..2).all(|k| r[k] >= lo[k] && r[k] + r[k + 2] <= hi[k]);
            assert!(inside, "I3: {p:?} leaves the inset area");
            for q in lay[i + 1..].iter().filter(|q| !q.floating) {
                let (ix, iy) = (overlap(r, v(q.rect), 0), overlap(r, v(q.rect), 1));
                assert!(ix <= 0 || iy <= 0, "I3: {p:?} overlaps {q:?}");
            }
        }
    }
    for id in (0..=m.next + 1).rev().take(48).chain([0]) {
        assert_eq!(wm.workspace_of(WinId(id)), m.find(WinId(id)).map(|f| f.0));
    }
}

#[test]
fn randomized_invariants_and_replay() {
    // 8 seeds x 2,600 commands, invariants checked after every one.
    for seed in 1..=8_u64 {
        let mut r = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (area, n) = (r.rect(true), r.below(5) as usize); // n = 0 clamps to 1
        let g = gaps(r.below(30) as i32 - 4, r.below(30) as i32 - 4);
        let mut wm = Wm::new(area, g, n);
        let mut m = Model::default();
        (m.hist, m.next) = (vec![vec![]; n.clamp(1, MAX_WORKSPACES)], 1);
        assert_eq!((wm.area(), wm.gaps()), (norm(area), norm_gaps(g)));
        check(&wm, &m);
        let mut log = Vec::new();
        for _ in 0..2_600 {
            let cmd = random_cmd(&mut r, &m, 3 + 3 * seed as usize);
            let res = step(&mut wm, &mut m, &cmd);
            check(&wm, &m);
            log.push((cmd, res, wm.state_hash()));
        }
        // I7: replaying the log on a fresh Wm reproduces each outcome and hash.
        let mut re = Wm::new(area, g, n);
        for (i, (cmd, res, h)) in log.iter().enumerate() {
            assert_eq!(re.apply(cmd.clone()), *res, "replay step {i}");
            assert_eq!(re.state_hash(), *h, "replay step {i}");
        }
    }
}
