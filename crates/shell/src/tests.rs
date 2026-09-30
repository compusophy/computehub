use super::Key::*;
use super::*;
use gfx::{Instance, Kind};

fn shell() -> Shell {
    Shell::new(1280.0, 800.0)
}

/// Presses `key` with modifiers named by letter: `a`lt, `m`eta, `s`hift, `c`trl.
fn key(s: &mut Shell, key: Key, m: &str) -> Response {
    let mut mods = Mods::default();
    [mods.shift, mods.ctrl, mods.alt, mods.meta] = ['s', 'c', 'a', 'm'].map(|c| m.contains(c));
    s.input(Input::Key { key, mods })
}

fn resp(redraw: bool, consumed: bool) -> Response {
    Response { redraw, consumed }
}

fn down(s: &mut Shell, (x, y): (f32, f32)) -> Response {
    s.input(Input::PointerDown { x, y, button: 0 })
}

fn up(s: &mut Shell, (x, y): (f32, f32)) -> Response {
    s.input(Input::PointerUp { x, y, button: 0 })
}

fn move_to(s: &mut Shell, (x, y): (f32, f32)) -> Response {
    s.input(Input::PointerMove { x, y })
}

fn click(s: &mut Shell, at: (f32, f32)) {
    down(s, at);
    up(s, at);
}

fn mid(r: RectF) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn rect_of(s: &Shell, id: u32) -> Option<Rect> {
    let layout = s.wm().layout();
    layout.iter().find(|p| p.win == WinId(id)).map(|p| p.rect)
}

/// Centers of window `id`'s float and close buttons.
fn buttons(s: &Shell, id: u32) -> [(f32, f32); 2] {
    let r = rectf(rect_of(s, id).unwrap());
    title_buttons(r, WinId(id)).unwrap().map(|b| mid(b.0))
}

/// Center of a panel button on the default screen.
fn panel(hit: Hit) -> (f32, f32) {
    let targets = shell().panel_targets();
    mid(targets.iter().find(|t| t.1 == hit).unwrap().0)
}

fn drawn(s: &Shell) -> Vec<Instance> {
    let mut list = DrawList::new();
    s.draw(&mut list);
    list.instances().to_vec()
}

fn count(all: &[Instance], kind: Kind, color: Rgba) -> usize {
    let kind = kind as u8 as f32;
    let hit = |i: &&Instance| i.kind == kind && i.color == color;
    all.iter().filter(hit).count()
}

#[test]
fn startup_layout_and_tints() {
    let s = shell();
    let (g, n) = (s.wm().gaps(), s.wm().workspace_count());
    assert_eq!((g.outer, g.inner, n), (10, 10, 4));
    assert_eq!(s.wm().area(), Rect::new(0, 36, 1280, 764));
    assert!(s.wm().layout().iter().all(|p| !p.floating));
    assert_eq!(rect_of(&s, 1), Some(Rect::new(10, 46, 625, 744)));
    assert_eq!((s.wm().layout().len(), s.clear_color()), (3, BG));
    assert_eq!(Shell::new(99.6, 36.4).wm().area(), Rect::new(0, 36, 100, 0));
    let tints: Vec<Rgba> = (1..=16).map(app_tint).collect();
    assert_eq!(tints, (1..=16).map(app_tint).collect::<Vec<_>>());
    for (i, t) in tints.iter().enumerate() {
        assert!(tints[i + 1..].iter().all(|u| u != t));
        let (lo, hi) = (t.0.min(t.1).min(t.2), t.0.max(t.1).max(t.2));
        assert!(t.3 == 255 && lo >= 120 && hi <= 235 && hi - lo >= 80);
    }
    assert_eq!(app_tint(0), Rgba(230, 126, 126, 255));
}

#[test]
fn every_binding_reaches_the_wm() {
    let f = WinId(0); // stands for the window focused at the time
    let mut table = vec![
        (Enter, "a", Cmd::Open { floating: false }),
        (Enter, "as", Cmd::Open { floating: true }),
        (F, "m", Cmd::ToggleFloat),
        (O, "a", Cmd::ToggleOrientation),
    ];
    let dirs = [Dir::Left, Dir::Down, Dir::Up, Dir::Right];
    let keys = [Left, Down, Up, Right, H, J, K, L];
    for (k, d) in keys.into_iter().zip(dirs.into_iter().cycle()) {
        table.push((k, "a", Cmd::FocusDir(d)));
        table.push((k, "as", Cmd::MoveDir(d)));
        table.push((k, "ac", Cmd::Resize { dir: d, px: 40 }));
    }
    table.extend([
        (Digit(3), "as", Cmd::MoveToWorkspace { win: f, ws: 2 }),
        (Digit(3), "m", Cmd::SwitchWorkspace(2)),
        (Q, "a", Cmd::Close(f)),
        (Digit(1), "ma", Cmd::SwitchWorkspace(0)),
        (Digit(4), "a", Cmd::SwitchWorkspace(3)),
    ]);
    let (mut s, mut kinds) = (shell(), Vec::new());
    for (k, m, cmd) in table {
        let mut want = s.wm().clone();
        let focused = want.focused().unwrap_or(f);
        let cmd = match cmd {
            Cmd::Close(_) => Cmd::Close(focused),
            Cmd::MoveToWorkspace { ws, .. } => Cmd::MoveToWorkspace { win: focused, ws },
            c => c,
        };
        let (before, kind) = (want.state_hash(), std::mem::discriminant(&cmd));
        let _ = want.apply(cmd);
        let r = key(&mut s, k, m);
        assert_eq!(s.wm().state_hash(), want.state_hash(), "{k:?} {m}");
        assert_eq!(r, resp(want.state_hash() != before, true), "{k:?} {m}");
        if r.redraw && !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    assert_eq!(kinds.len(), 9, "a binding never changed the wm");
}

#[test]
fn unbound_keys_pass_through_and_noops_still_consume() {
    let mut s = shell();
    let hash = s.wm().state_hash();
    let keys = [Enter, Q, Left, Escape, Other, Left, Enter, Q, F, O];
    let mods = ["", "s", "c", "a", "m", "acs", "ac", "as", "mc", "ms"];
    let digits = [(0, "a"), (5, "a"), (1, "s"), (2, "ac")].map(|(n, m)| (Digit(n), m));
    for (k, m) in keys.into_iter().zip(mods).chain(digits) {
        assert_eq!(key(&mut s, k, m), Response::default(), "{k:?} {m}");
    }
    assert_eq!(s.wm().state_hash(), hash);
    assert_eq!(key(&mut s, Right, "a"), resp(false, true));
    assert_eq!(key(&mut s, Digit(1), "a"), resp(false, true));
    for _ in 0..3 {
        assert_eq!(key(&mut s, Q, "m"), resp(true, true));
    }
    assert_eq!(key(&mut s, Q, "a"), resp(false, true));
    assert_eq!(key(&mut s, Digit(2), "as"), resp(false, true));
    let resize = Input::Resize { w: 800.0, h: 600.0 };
    assert_eq!(s.input(resize), resp(true, false));
    assert_eq!(s.wm().area(), Rect::new(0, 36, 800, 564));
    assert_eq!(s.input(resize), resp(true, false));
}

#[test]
fn titlebar_buttons_fire_only_on_release_over_the_same_button() {
    let mut s = shell();
    let [float, close] = buttons(&s, 1);
    assert_eq!(down(&mut s, close), resp(true, true));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    up(&mut s, float);
    down(&mut s, close);
    up(&mut s, (300.0, 400.0));
    assert!(rect_of(&s, 1).is_some());
    click(&mut s, close);
    assert!(rect_of(&s, 1).is_none());
    // Window 2 now spans the top; its float button floats it in place.
    let ([float, close], r) = (buttons(&s, 2), rect_of(&s, 2));
    click(&mut s, float);
    assert_eq!(s.wm().is_floating(WinId(2)), Some(true));
    assert_eq!((s.wm().focused(), rect_of(&s, 2)), (Some(WinId(2)), r));
    // Other buttons do nothing, but a chord's last release (button 2) disarms.
    let (x, y) = close;
    down(&mut s, close);
    assert!(s.input(Input::PointerUp { x, y, button: 2 }).consumed);
    assert!(s.input(Input::PointerDown { x, y, button: 2 }).consumed);
    assert!(rect_of(&s, 2).is_some() && s.armed.is_none());
}

#[test]
fn panel_launches_windows_and_switches_workspaces() {
    let mut s = shell();
    let tiled = panel(Hit::Launch { floating: false });
    let floating = panel(Hit::Launch { floating: true });
    let ws: Vec<(f32, f32)> = (0..4).map(|i| panel(Hit::Workspace(i))).collect();
    assert_eq!((tiled, floating), ((18.0, 18.0), (50.0, 18.0)));
    // Centered: 578 + 4 * 28 + 3 * 4 = 702 = 1280 - 578.
    assert_eq!((ws[0], ws[3]), ((592.0, 18.0), (688.0, 18.0)));
    click(&mut s, tiled);
    assert_eq!(s.wm().is_floating(WinId(4)), Some(false));
    click(&mut s, floating);
    assert_eq!(s.wm().is_floating(WinId(5)), Some(true));
    down(&mut s, tiled);
    up(&mut s, floating);
    assert_eq!(s.wm().layout().len(), 5);
    click(&mut s, ws[2]);
    assert_eq!(s.wm().active_workspace(), 2);
    down(&mut s, ws[3]);
    up(&mut s, ws[1]);
    assert_eq!(s.wm().active_workspace(), 2);
    // The bare panel swallows clicks.
    let hash = s.wm().state_hash();
    assert_eq!(s.hit(300.0, 10.0), Some(Hit::Panel));
    click(&mut s, (300.0, 10.0));
    assert_eq!(s.wm().state_hash(), hash);
}

#[test]
fn dragging_a_floating_titlebar_moves_the_window() {
    let mut s = shell();
    key(&mut s, Enter, "as");
    let at = |x, y| Some(Rect::new(x, y, 853, 509));
    assert_eq!(rect_of(&s, 4), at(213, 163));
    down(&mut s, (253.0, 175.0));
    assert_eq!(move_to(&mut s, (303.0, 205.0)), resp(true, true));
    assert_eq!(rect_of(&s, 4), at(263, 193));
    // A chord's last release (button 2) ends the drag; the top stays below the panel.
    let (x, y) = (0.0, -500.0);
    move_to(&mut s, (x, y));
    s.input(Input::PointerUp { x, y, button: 2 });
    move_to(&mut s, (500.0, 500.0));
    assert_eq!(rect_of(&s, 4), at(-40, 36));
    down(&mut s, (0.0, 48.0));
    s.input(Input::PointerLeave);
    move_to(&mut s, (245.6, 172.0));
    assert_eq!(rect_of(&s, 4), at(206, 160));
    up(&mut s, (0.0, 0.0));
    move_to(&mut s, (900.0, 700.0));
    // Bodies and tiled titlebars focus without dragging.
    down(&mut s, (500.0, 500.0));
    move_to(&mut s, (600.0, 600.0));
    assert_eq!(rect_of(&s, 4), at(206, 160));
    let tiled = rect_of(&s, 1);
    down(&mut s, (20.0, 50.0));
    move_to(&mut s, (120.0, 150.0));
    assert_eq!((s.wm().focused(), rect_of(&s, 1)), (Some(WinId(1)), tiled));
    // A drag ends when its window stops floating.
    down(&mut s, (300.0, 170.0));
    assert!(s.drag.is_some());
    key(&mut s, F, "a");
    move_to(&mut s, (400.0, 400.0));
    assert!(s.drag.is_none());
}

#[test]
fn hover_redraws_only_when_the_target_changes() {
    let mut s = shell();
    assert_eq!(move_to(&mut s, (18.0, 18.0)), resp(true, true));
    assert_eq!(s.hover, Some(Hit::Launch { floating: false }));
    assert!(!move_to(&mut s, (20.0, 21.0)).redraw);
    assert!(move_to(&mut s, (300.0, 18.0)).redraw);
    assert!(!move_to(&mut s, (310.0, 18.0)).redraw);
    assert!(!move_to(&mut s, (300.0, 400.0)).redraw);
    let [_, close] = buttons(&s, 2);
    assert!(move_to(&mut s, close).redraw);
    assert_eq!(s.hover, Some(Hit::Close(WinId(2))));
    assert_eq!(count(&drawn(&s), Kind::Fill, HOVER), 1);
    assert_eq!(s.input(Input::PointerLeave), resp(true, false));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    assert_eq!(count(&drawn(&s), Kind::Fill, HOVER), 0);
    // Closing the hovered window by key moves the hover off it.
    let [_, close] = buttons(&s, 1);
    move_to(&mut s, close);
    key(&mut s, Left, "a");
    assert_eq!(key(&mut s, Q, "a"), resp(true, true));
    assert_eq!(s.hover, None);
}

#[test]
fn draw_stays_on_screen_with_the_panel_on_top() {
    let mut s = shell();
    key(&mut s, Enter, "as");
    let mut list = DrawList::new();
    list.fill(RectF::new(0.0, 0.0, 5.0, 5.0), 0.0, ICON);
    s.draw(&mut list);
    let (all, shadow) = (list.instances(), Kind::Shadow as u8 as f32);
    assert_eq!(all[0].kind, shadow);
    for [x, y, w, h] in all.iter().filter(|i| i.kind != shadow).map(|i| i.rect) {
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1280.0 && y + h <= 800.0);
    }
    let bar = all.iter().position(|i| i.color == PANEL).unwrap();
    assert_eq!(all[bar].rect, [0.0, 0.0, 1280.0, 36.0]);
    assert!(all[bar..].iter().all(|i| i.rect[1] + i.rect[3] <= PANEL_H));
    let borders = [BORDER, BORDER_FOCUSED].map(|c| count(all, Kind::Border, c));
    assert_eq!(borders, [3, 1]);
    assert_eq!(count(all, Kind::Fill, ACCENT), 1);
    assert_eq!(count(all, Kind::Fill, app_tint(4).with_alpha(44)), 1);
    let icons = [ICON, ICON_DIM].map(|c| count(all, Kind::Icon, c));
    assert_eq!(icons, [2 + 2, 3 * 2]);
    // Occupied workspaces show brighter dots than empty ones.
    key(&mut s, Digit(2), "as");
    let all = drawn(&s);
    let dots = [ICON_DIM, ICON_DIM.with_alpha(150)].map(|c| count(&all, Kind::Fill, c));
    assert_eq!(dots, [1, 2]);
    // Tiles 30 px tall get no titlebar, 40 px wide no buttons, all a body.
    for (w, h, bars) in [(1000.0, 86.0, 0), (60.0, 400.0, 6)] {
        let all = drawn(&Shell::new(w, h));
        let fills = [WINDOW, TITLEBAR, TITLEBAR_FOCUSED].map(|c| count(&all, Kind::Fill, c));
        let edges = [BORDER, ACCENT].map(|c| count(&all, Kind::Border, c));
        assert_eq!([fills[0], fills[1] + fills[2]], [3, bars]);
        assert_eq!(edges[0] + edges[1], 3);
        assert_eq!(count(&all, Kind::Icon, ICON), 2);
    }
}

#[test]
fn degenerate_input_never_panics() {
    let inf = f32::INFINITY;
    let bad = [0.0, -5.0, 20.0, 1e30, f32::NAN, inf, f32::MIN, f32::MAX];
    let mut list = DrawList::new();
    for (w, h) in bad.iter().flat_map(|&w| bad.map(|h| (w, h))) {
        let mut s = Shell::new(w, h);
        let a = s.wm().area();
        assert!(a.w >= 0 && a.h >= 0 && a.y == 36, "{w} {h}");
        for &x in &bad {
            move_to(&mut s, (x, h));
            down(&mut s, (x, x));
            move_to(&mut s, (-x, 1e9));
            up(&mut s, (x, 5.0));
        }
        key(&mut s, Enter, "as");
        down(&mut s, (w / 2.0, h / 2.0));
        s.input(Input::Resize { w: h, h: w });
        move_to(&mut s, (f32::NAN, inf));
        s.draw(&mut list);
        let finite = |i: &Instance| i.rect.iter().all(|v| v.is_finite());
        assert!(list.instances().iter().all(finite));
    }
    // A seeded run of mixed events, drawing after each.
    let (mut s, mut seed) = (shell(), 0x2545_f491_4f6c_dd1d_u64);
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as usize
    };
    let keys = [Enter, Q, F, O, H, Down, Other, Digit(2)];
    let mods = ["", "a", "as", "ac", "m", "ms", "mc", "asc"];
    for _ in 0..3000 {
        let (x, y) = (next(1400) as f32 - 60.0, next(900) as f32 - 60.0);
        let button = u8::from(next(4) == 0);
        match next(8) {
            0 | 1 => s.input(Input::PointerMove { x, y }),
            2 => s.input(Input::PointerDown { x, y, button }),
            3 => s.input(Input::PointerUp { x, y, button }),
            4 => s.input(Input::PointerLeave),
            5 if next(20) == 0 => s.input(Input::Resize { w: x, h: y }),
            _ => key(&mut s, keys[next(8)], mods[next(8)]),
        };
        s.draw(&mut list);
    }
}
