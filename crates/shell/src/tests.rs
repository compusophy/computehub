use std::cell::RefCell;
use std::rc::Rc;

use gfx::{DrawList, Kind, RectF};
use host::frame::controls;
use host::motion::Vis;
use host::{content_rect, rectf};
use ui::Key::*;
use ui::{App, AppEvent as E, Cx, Sense, THEMES, Ui, WidgetId as W};
use wm::{Snap, State};

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const KNOWN: &str = "welcome terminal studio settings about";
type Log = Rc<RefCell<Vec<(&'static str, E)>>>;

/// A scripted app: its name and log. It logs every event, runs the
/// `;`-separated commands of its text, and draws a Click hit (1) over its
/// content's top-left 60 x 20 and a Text hit (2) below that. A terminal
/// wants text input.
struct Probe(&'static str, Log);

impl App for Probe {
    fn title(&self) -> String {
        self.0.to_uppercase()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        ui.hit(W(1), RectF::new(r.x, r.y, 60.0, 20.0), Sense::Click);
        ui.hit(W(2), RectF::new(r.x, r.y + 20.0, r.w, 40.0), Sense::Text);
    }

    fn event(&mut self, ev: E, cx: &mut Cx<'_>) -> bool {
        self.1.borrow_mut().push((self.0, ev.clone()));
        let E::Text(cmds) = ev else {
            return matches!(ev, E::Click(_));
        };
        for cmd in cmds.split(';') {
            match cmd.split_once(' ').unwrap_or((cmd, "")) {
                ("theme", name) => cx.set_theme(name),
                ("open", name) => cx.open(name),
                _ => {}
            }
        }
        true
    }

    fn wants_text_input(&self) -> bool {
        self.0 == "terminal"
    }
}

fn desk_of(w: f32, h: f32) -> (Shell, Log) {
    let (log, text) = (Log::default(), TextSystem::new(SANS.to_vec()).unwrap());
    let l = log.clone();
    let reg: Registry = Box::new(move |name| {
        let k = KNOWN.split(' ').find(|k| *k == name)?;
        Some(Box::new(Probe(k, l.clone())) as Box<dyn App>)
    });
    (Shell::new(w, h, text, Vfs::new(), reg, "midnight"), log)
}

/// A 1280 x 800 desktop with welcome at rest at t = 1000, its log cleared.
fn desk() -> (Shell, Log) {
    let (mut s, log) = desk_of(1280.0, 800.0);
    s.at(0.0);
    s.at(1000.0);
    log.take();
    (s, log)
}

/// Test drivers.
impl Shell {
    /// Draws a frame at `t` ms; whether more are wanted.
    fn at(&mut self, t: f64) -> bool {
        self.set_now(t);
        self.draw(&mut DrawList::new())
    }
    /// Lets every animation finish, `t` ms on.
    fn rest(&mut self, t: f64) {
        let now = self.host.now_ms;
        self.at(now);
        self.at(now + t);
    }
    fn k(&mut self, key: Key, m: &str) -> Response {
        let mut mods = Mods::default();
        [mods.shift, mods.ctrl, mods.alt, mods.meta] = ['s', 'c', 'a', 'm'].map(|c| m.contains(c));
        self.input(Input::Key { key, mods })
    }
    fn down(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerDown { x, y, button: 0 })
    }
    fn up(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerUp { x, y, button: 0 })
    }
    fn to(&mut self, (x, y): (f32, f32)) -> Response {
        self.input(Input::PointerMove { x, y })
    }
    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) -> Response {
        self.to(from);
        self.down(from);
        self.to(to);
        self.up(to)
    }
    fn click(&mut self, at: (f32, f32)) -> Response {
        self.drag(at, at)
    }
    fn say(&mut self, n: u32, text: &str) {
        self.host.deliver(WinId(n), E::Text(text.into()), &mut Response::default());
    }
    fn rect(&self, n: u32) -> Option<Rect> {
        self.placement(WinId(n)).map(|p| p.rect)
    }
    fn names(&self) -> Vec<&str> {
        self.host.wins.iter().filter(|w| self.host.live(w.id)).map(|w| &*w.name).collect()
    }
    fn tile_at(&self, i: usize) -> (f32, f32) {
        let r = self.dock_tile(i);
        (r.x + 22.0, r.y + 22.0)
    }
}

#[test]
fn welcome_opens_centered_once_there_is_a_work_area() {
    let (mut s, _) = desk_of(1280.0, 800.0);
    assert_eq!(s.wm().area(), Rect::new(0, 32, 1280, 684));
    assert_eq!(s.rect(1), Some(Rect::new(300, 134, 680, 480)));
    assert_eq!((s.names(), s.theme_name()), (vec!["welcome"], "Midnight"));
    let dock: Vec<&str> = s.dock.iter().map(|d| &*d.0).collect();
    assert_eq!(dock, ["terminal", "studio", "settings", "welcome"]);
    // The window fades in from its first frame; then frames stop.
    assert!(s.animating() && s.at(5000.0) && s.at(5100.0) && !s.at(5180.0));
    let r = s.input(Input::PointerLeave);
    let got = (r.redraw, r.consumed, r.text_input, r.animating);
    assert_eq!(got, (false, false, Some(false), false));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    // Made before the screen has a size, it opens at the first size that
    // leaves a work area, as it would have from the start; clamped to it.
    let sizes = [
        (1280.0, 800.0, Rect::new(300, 134, 680, 480)),
        (600.0, 400.0, Rect::new(0, 32, 600, 284)),
    ];
    for (w, h, want) in sizes {
        let (mut s, _) = desk_of(0.0, 0.0);
        for (w, h) in [(0.0, 0.0), (1280.0, 100.0), (0.4, 800.0)] {
            s.input(Input::Resize { w, h });
            assert!(s.wm().layout().is_empty());
        }
        let r = s.input(Input::Resize { w, h });
        assert!(r.redraw && r.animating && s.rect(1) == Some(want));
        assert_eq!(s.wm().state_hash(), desk_of(w, h).0.wm().state_hash());
        s.input(Input::Resize { w: 1000.0, h: 700.0 });
        assert_eq!(s.names(), ["welcome"]);
    }
}

#[test]
fn resizes_set_the_work_area_and_windows_follow_at_once() {
    // Welcome free, a terminal snapped, studio maximized.
    let (mut s, _) = desk();
    s.k(Enter, "a");
    s.k(Left, "a");
    s.say(1, "open studio");
    s.k(Up, "a");
    s.rest(1000.0);
    for (w, h) in [(800.0, 500.0), (1280.0, 800.0), (500.0, 330.0), (2000.0, 1000.0)] {
        s.input(Input::Resize { w, h });
        let (a, now) = (work_area((w, h)), s.host.now_ms);
        assert_eq!((s.wm().area(), s.wm().layout().len()), (a, 3));
        // Inside the new work area, and drawn there from the next frame on.
        for p in s.wm().layout() {
            let r = p.rect;
            assert!(r.x >= a.x && r.y >= a.y && r.x + r.w <= a.x + a.w && r.y + r.h <= a.y + a.h);
            let t = s.motion.wins.iter().find(|w| w.0 == p.win).unwrap().1;
            assert_eq!(t.value(now), Vis::at(rectf(r)), "{w}x{h}");
        }
        assert!(!s.at(now));
    }
}

#[test]
fn titlebars_drag_and_double_click() {
    let (mut s, log) = desk();
    let title = (400.0, 150.0);
    assert_eq!(s.to(title).cursor, Some(Cursor::Grab));
    assert_eq!(s.down(title).cursor, Some(Cursor::Grabbing));
    // Under 4 px of travel nothing moves; then the window follows.
    assert!(!s.to((402.0, 151.0)).redraw);
    let r = s.to((500.0, 250.0));
    assert!(r.redraw && !r.animating && s.rect(1) == Some(Rect::new(400, 234, 680, 480)));
    assert_eq!(s.up((500.0, 250.0)).cursor, Some(Cursor::Grab));
    // Two presses within 350 ms maximize, two more restore; further apart
    // they are two clicks.
    let (max, normal) = (State::Maximized, State::Normal);
    for (t, gap, state) in [(2000.0, 300.0, max), (3000.0, 300.0, normal), (4000.0, 400.0, normal)]
    {
        let r = s.rect(1).unwrap();
        let at = (r.x as f32 + 100.0, r.y as f32 + 10.0);
        s.set_now(t);
        s.click(at);
        s.set_now(t + gap);
        assert!(s.click(at).animating);
        assert_eq!(s.placement(WinId(1)).unwrap().state, state);
    }
    assert_eq!(s.rect(1), Some(Rect::new(400, 234, 680, 480)));
    assert!(log.take().iter().all(|e| matches!(e.1, E::Focus(_) | E::Resized { .. })));
}

#[test]
fn drops_snap_and_snapped_windows_come_back_under_the_pointer() {
    let (mut s, _) = desk();
    let area = s.wm().area();
    let cases = [
        ((3.0, 400.0), Some(Snap::Left)),
        ((1278.0, 400.0), Some(Snap::Right)),
        ((10.0, 10.0), Some(Snap::TopLeft)),
        ((1270.0, 790.0), Some(Snap::BottomRight)),
        ((640.0, 4.0), None),
    ];
    for (to, snap) in cases {
        let r = rectf(s.rect(1).unwrap());
        let grab = (r.x + r.w / 2.0, r.y + 10.0);
        s.to(grab);
        s.down(grab);
        s.to((grab.0 + 20.0, grab.1 + 20.0));
        s.to(to);
        // The preview shows where it will go, then the window goes there.
        let zone = s.visuals().zone.unwrap().rect(area);
        assert!(s.animating());
        s.rest(1000.0);
        assert_eq!(rectf(zone), s.motion.preview.value(s.host.now_ms).rect);
        s.up(to);
        s.rest(1000.0);
        let p = s.placement(WinId(1)).unwrap();
        assert_eq!((p.snap, p.state == State::Maximized, p.rect), (snap, snap.is_none(), zone));
        assert_eq!(s.motion.preview.value(s.host.now_ms).a, 0.0);
    }
    // Dragged at a quarter of its width, a maximized window comes back to
    // its normal size with the pointer a quarter across.
    s.drag((320.0, 40.0), (600.0, 300.0));
    assert_eq!(s.rect(1), Some(Rect::new(430, 292, 680, 480)));
}

#[test]
fn edges_resize_with_their_cursors() {
    let (mut s, _) = desk();
    let cursors = [
        ((300.0, 300.0), Cursor::EwResize),
        ((980.0, 300.0), Cursor::EwResize),
        ((600.0, 135.0), Cursor::NsResize),
        ((298.0, 135.0), Cursor::NwseResize),
        ((979.0, 612.0), Cursor::NwseResize),
        ((975.0, 135.0), Cursor::NeswResize),
        ((600.0, 400.0), Cursor::Default),
    ];
    for (at, c) in cursors {
        s.to(at);
        assert_eq!(s.cursor, c, "{at:?}");
    }
    // The left edge moves, the right one stays, the size stays legal, and
    // the top edge stops under the bar.
    let drags = [
        ((300.0, 300.0), (250.0, 300.0), Rect::new(250, 134, 730, 480)),
        ((250.0, 300.0), (900.0, 300.0), Rect::new(660, 134, 320, 480)),
        ((800.0, 134.0), (800.0, 0.0), Rect::new(660, 32, 320, 582)),
        ((980.0, 613.0), (1100.0, 700.0), Rect::new(660, 32, 440, 669)),
    ];
    for (from, to, want) in drags {
        s.drag(from, to);
        assert_eq!(s.rect(1), Some(want));
    }
}

#[test]
fn controls_minimize_maximize_and_close() {
    let (mut s, log) = desk();
    let [min, max, close] =
        controls(rectf(s.rect(1).unwrap())).unwrap().map(|c| (c.x + 6.0, c.y + 6.0));
    // Hovering them shows their glyphs and the close button's danger.
    let danger = |s: &mut Shell| {
        let mut list = DrawList::new();
        s.draw(&mut list);
        let fill = Kind::Fill as u8 as f32;
        list.instances().iter().any(|i| i.color == THEMES[0].danger && i.kind == fill)
    };
    assert!(!danger(&mut s));
    assert!(s.to(min).redraw && danger(&mut s));
    s.click(max);
    assert_eq!(s.placement(WinId(1)).unwrap().state, State::Maximized);
    s.rest(1000.0);
    s.k(Down, "a");
    s.rest(1000.0);
    s.click(min);
    assert_eq!((s.wm().windows()[0].1, s.wm().focused()), (State::Minimized, None));
    // It flies into its dock tile: drawn until it gets there.
    assert!(s.animating());
    s.rest(1000.0);
    s.click(s.tile_at(3));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.rest(1000.0);
    // Closing fades it out, then drops the app.
    assert!(s.click(close).animating);
    assert_eq!((s.names().len(), s.host.wins.len()), (0, 1));
    s.rest(1000.0);
    assert_eq!(s.host.wins.len(), 0);
    assert_eq!(log.take().last(), Some(&("welcome", E::Focus(true))));
}

#[test]
fn dock_tiles_open_minimize_and_focus() {
    let (mut s, log) = desk();
    assert!(s.to(s.tile_at(0)).redraw && s.animating());
    // Terminal opens, minimizes and comes back; then welcome is focused and
    // minimized.
    let (min, up) = (State::Minimized, State::Normal);
    let steps = [
        (0, [up, up], 2),
        (0, [up, min], 1),
        (0, [up, up], 2),
        (3, [up, up], 1),
        (3, [min, up], 2),
    ];
    for (tile, states, focus) in steps {
        s.click(s.tile_at(tile));
        let got: Vec<State> = s.wm().windows().iter().map(|w| w.1).collect();
        assert_eq!((&got[..], s.wm().focused()), (&states[..], Some(WinId(focus))));
    }
    assert_eq!(s.names(), ["welcome", "terminal"]);
    // The gap between tiles splits; the shelf's ends are bare.
    let d = s.dock_rect();
    let hits = [(56.0, 30.0), (57.0, 30.0), (2.0, 2.0)].map(|(x, y)| s.hit(d.x + x, d.y + y));
    assert_eq!(hits, [Some(Target::Dock(0)), Some(Target::Dock(1)), Some(Target::DockBar)]);
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
}

#[test]
fn the_launcher_searches_and_opens() {
    let (mut s, log) = desk();
    let r = s.k(Space, "a");
    assert_eq!((r.consumed, r.text_input, s.launcher.open), (true, Some(true), true));
    let found = |s: &Shell| {
        let l = &s.launcher.search;
        (0..l.count()).map(|k| l.get(k).unwrap().name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(found(&s), ["terminal", "studio", "settings", "welcome", "about"]);
    s.input(Input::Text("st".into()));
    assert_eq!(found(&s), ["studio", "settings"]);
    assert!(s.k(Char('s'), "").consumed && s.k(Right, "").redraw);
    let r = s.k(Enter, "");
    assert_eq!((s.launcher.open, r.text_input), (false, Some(false)));
    assert_eq!(s.names(), ["welcome", "settings"]);
    // The mark shows it; Escape, the veil and Alt+Space hide it.
    for hide in 0..3 {
        s.click((19.0, 16.0));
        assert!(s.launcher.open && s.hit(640.0, 400.0) == Some(Target::Panel));
        match hide {
            0 => _ = s.k(Escape, ""),
            1 => _ = s.click((5.0, 400.0)),
            _ => _ = s.k(Space, "m"),
        }
        assert!(!s.launcher.open && s.animating());
    }
    // An app can ask for it; tiles open what they show.
    s.rest(1000.0);
    s.input(Input::Text("".into()));
    s.say(1, "open launcher");
    s.input(Input::PointerLeave);
    assert!(s.launcher.open);
    s.rest(1000.0);
    let t = s.panel().tile(0);
    s.click((t.x + 40.0, t.y + 40.0));
    assert_eq!(s.names(), ["welcome", "settings", "terminal"]);
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Key { .. })));
    // Paste reaches the query through the browser.
    s.k(Space, "a");
    assert!(!s.k(Char('v'), "c").consumed);
}

#[test]
fn bindings_drive_the_wm() {
    let (mut s, log) = desk();
    assert!(s.k(Enter, "a").animating);
    assert_eq!(s.names(), ["welcome", "terminal"]);
    let (max, up, right) = (State::Maximized, State::Normal, Some(Snap::Right));
    let steps = [
        (Up, "m", Some((max, None)), 2),
        (Down, "a", Some((up, None)), 2),
        (Left, "a", Some((up, Some(Snap::Left))), 2),
        (Right, "a", Some((up, right)), 2),
        (Char('`'), "a", Some((up, right)), 1),
        (Char('`'), "as", Some((up, right)), 2),
        (Down, "a", None, 1),
    ];
    for (key, m, terminal, focus) in steps {
        assert!(s.k(key, m).consumed);
        let got = s.placement(WinId(2)).map(|p| (p.state, p.snap));
        assert_eq!((got, s.wm().focused()), (terminal, Some(WinId(focus))), "{key:?} {m}");
    }
    assert!(s.k(Char('q'), "a").consumed);
    assert_eq!(s.names(), ["terminal"]);
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Key { .. })));
}

#[test]
fn the_focused_app_gets_keys_text_and_the_pointer() {
    let (mut s, log) = desk();
    for (key, m) in [(Escape, "a"), (Char('x'), ""), (Enter, "ac"), (Char('q'), "as"), (F(5), "")] {
        assert_eq!(s.k(key, m).consumed, key != F(5), "{key:?} {m}");
        assert!(matches!(log.take()[..], [("welcome", E::Key { .. })]));
    }
    // Paste keys are the browser's; the text comes after.
    assert_eq!(s.k(Char('v'), "c"), Response::default());
    assert!(s.input(Input::Text("hi".into())).consumed);
    assert_eq!(log.take(), [("welcome", E::Text("hi".into()))]);
    // A terminal wants text: reload is its own.
    assert_eq!(s.k(Enter, "a").text_input, Some(true));
    assert!(s.k(F(5), "").consumed);
    // Content gets the pointer, relative to it.
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    assert_eq!(s.to((c.x + 30.0, c.y + 40.0)).cursor, Some(Cursor::Text));
    s.click((c.x + 10.0, c.y + 10.0));
    let down = E::PointerDown { x: 10.0, y: 10.0, id: Some(W(1)) };
    assert_eq!(log.take(), [("welcome", down), ("welcome", E::Click(W(1)))]);
    // A release elsewhere is no click; the wheel goes to the window under it.
    s.down((c.x + 10.0, c.y + 10.0));
    s.up((c.x + 100.0, c.y + 10.0));
    assert!(s.input(Input::Wheel { x: c.x + 5.0, y: c.y + 5.0, dy: 3.0 }).consumed);
    assert_eq!(log.take().len(), 2);
}

#[test]
fn themes_switch_and_crossfade_and_the_clock_ticks() {
    let (mut s, _) = desk();
    for name in ["Dawn", "Mono", "Midnight"] {
        s.click((1280.0 - 19.0, 16.0));
        assert_eq!((s.theme_name(), s.clear_color()), (name, ui::theme(name).base));
        assert!(s.animating());
        s.rest(1000.0);
        assert!(!s.animating());
    }
    s.say(1, "theme MONO;theme nope");
    assert!(s.input(Input::PointerLeave).redraw);
    assert_eq!((s.theme_name(), desk_of(800.0, 600.0).0.theme_name()), ("Mono", "Midnight"));
    // The settings button opens settings, then focuses it.
    s.click((1280.0 - 47.0, 16.0));
    s.click((1280.0 - 47.0, 16.0));
    assert_eq!(s.names(), ["welcome", "settings"]);
    s.rest(1000.0);
    let time = LocalTime { year: 2026, month: 10, day: 1, weekday: 3, hour: 9, minute: 5 };
    assert!(s.input(Input::Tick { time }).redraw && !s.input(Input::Tick { time }).redraw);
}

#[test]
fn degenerate_sizes_never_panic() {
    for (w, h) in [(0.0, 0.0), (1.0, 1.0), (f32::NAN, f32::INFINITY), (90.0, 40.0), (2e7, 2e7)] {
        let (mut s, _) = desk_of(w, h);
        s.at(0.0);
        let (alt, none) = (Mods { alt: true, ..Mods::default() }, Mods::default());
        for input in [
            Input::Key { key: Enter, mods: alt },
            Input::PointerDown { x: 10.0, y: 40.0, button: 0 },
            Input::PointerMove { x: -50.0, y: f32::NAN },
            Input::PointerUp { x: 0.0, y: 0.0, button: 0 },
            Input::Key { key: Space, mods: alt },
            Input::Text("x".into()),
            Input::Key { key: Down, mods: none },
            Input::Resize { w: 3.0, h: f32::NAN },
            Input::Resize { w: 400.0, h: 300.0 },
        ] {
            s.input(input);
            s.at(50.0);
        }
        s.rest(1000.0);
    }
}
