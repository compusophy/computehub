use std::cell::RefCell;
use std::rc::Rc;

use gfx::{DrawList, Kind, RectF};
use host::frame::controls;
use host::{content_rect, rectf};
use ui::Key::*;
use ui::{App, AppEvent as E, Cx, Sense, THEMES, Ui, WidgetId as W};
use wm::{Rect, Snap, State};

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
                ("close", _) => cx.close_self(),
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
    let log = Log::default();
    let l = log.clone();
    let reg: Registry = Box::new(move |name| {
        let k = KNOWN.split(' ').find(|k| *k == name)?;
        Some(Box::new(Probe(k, l.clone())) as Box<dyn App>)
    });
    let text = TextSystem::new(SANS.to_vec()).unwrap();
    (Shell::new(w, h, text, Vfs::new(), reg, "midnight"), log)
}

/// A 1280 x 800 desktop with welcome open and at rest at t = 1000, its
/// log cleared.
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
        let now = self.now();
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
    fn click(&mut self, at: (f32, f32)) -> Response {
        self.to(at);
        self.down(at);
        self.up(at)
    }
    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) -> Response {
        self.to(from);
        self.down(from);
        self.to(to);
        self.up(to)
    }
    fn rect(&self, n: u32) -> Option<Rect> {
        self.placement(WinId(n)).map(|p| p.rect)
    }
    fn names(&self) -> Vec<&str> {
        self.host.wins().iter().filter(|w| !w.closing()).map(|w| &*w.name).collect()
    }
    fn dock_names(&self) -> Vec<&str> {
        self.dock.iter().map(|d| &*d.0).collect()
    }
    fn tile_at(&self, i: usize) -> (f32, f32) {
        let r = self.dock_tile(i);
        (r.x + 22.0, r.y + 22.0)
    }
}

#[test]
fn startup_opens_welcome_centered() {
    let (mut s, _) = desk_of(1280.0, 800.0);
    assert_eq!(s.wm().area(), Rect::new(0, 32, 1280, 684));
    assert_eq!(s.rect(1), Some(Rect::new(300, 134, 680, 480)));
    assert_eq!((s.names(), s.theme_name()), (vec!["welcome"], "Midnight"));
    assert_eq!(s.dock_names(), ["terminal", "studio", "settings", "welcome"]);
    // The window fades in from its first frame; then frames stop.
    assert!(s.animating());
    assert!(s.at(5000.0) && s.at(5100.0) && !s.at(5180.0));
    let r = s.input(Input::PointerLeave);
    assert_eq!(
        (r.redraw, r.consumed, r.text_input, r.animating),
        (false, false, Some(false), false)
    );
    assert_eq!(s.input(Input::PointerLeave), Response::default());
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
    assert_eq!(
        (r.redraw, r.animating, s.rect(1)),
        (true, false, Some(Rect::new(400, 234, 680, 480)))
    );
    assert_eq!(s.up((500.0, 250.0)).cursor, Some(Cursor::Grab));
    // Two presses within 350 ms maximize, two more restore.
    for (t, state) in [(2000.0, State::Maximized), (3000.0, State::Normal)] {
        let r = s.rect(1).unwrap();
        let at = (r.x as f32 + 100.0, r.y as f32 + 10.0);
        s.set_now(t);
        s.click(at);
        s.set_now(t + 300.0);
        assert!(s.click(at).animating);
        assert_eq!(s.placement(WinId(1)).unwrap().state, state);
    }
    // Presses further apart are two clicks.
    s.set_now(4000.0);
    s.click((500.0, 244.0));
    s.set_now(4400.0);
    s.click((500.0, 244.0));
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
        let zone = s.grab.and_then(Grab::zone);
        // The preview shows where it will go, then the window goes there.
        assert!(zone.is_some() && s.animating());
        s.rest(1000.0);
        assert_eq!(rectf(zone.unwrap().rect(area)), s.motion.preview.value(s.now()).rect);
        s.up(to);
        s.rest(1000.0);
        let p = s.placement(WinId(1)).unwrap();
        assert_eq!((p.snap, p.state == State::Maximized), (snap, snap.is_none()));
        assert_eq!(p.rect, zone.unwrap().rect(area));
        assert_eq!(s.motion.preview.value(s.now()).a, 0.0);
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
    // The left edge moves; the right one stays; the size stays legal.
    s.drag((300.0, 300.0), (250.0, 300.0));
    assert_eq!(s.rect(1), Some(Rect::new(250, 134, 730, 480)));
    s.drag((250.0, 300.0), (900.0, 300.0));
    assert_eq!(s.rect(1), Some(Rect::new(660, 134, 320, 480)));
    // The top edge stops under the bar.
    s.drag((800.0, 134.0), (800.0, 0.0));
    assert_eq!(s.rect(1), Some(Rect::new(660, 32, 320, 582)));
    s.drag((980.0, 613.0), (1100.0, 700.0));
    assert_eq!(s.rect(1), Some(Rect::new(660, 32, 440, 669)));
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
        list.instances()
            .iter()
            .any(|i| i.color == THEMES[0].danger && i.kind == Kind::Fill as u8 as f32)
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
    assert_eq!((s.names().len(), s.host.wins().len()), (0, 1));
    s.rest(1000.0);
    assert_eq!(s.host.wins().len(), 0);
    assert_eq!(log.take().last(), Some(&("welcome", E::Focus(true))));
}

#[test]
fn dock_tiles_open_minimize_and_focus() {
    let (mut s, log) = desk();
    assert!(s.to(s.tile_at(0)).redraw);
    assert!(s.animating());
    s.click(s.tile_at(0));
    assert_eq!(s.names(), ["welcome", "terminal"]);
    assert_eq!(s.wm().focused(), Some(WinId(2)));
    s.click(s.tile_at(0));
    assert_eq!((s.wm().windows()[1].1, s.wm().focused()), (State::Minimized, Some(WinId(1))));
    s.click(s.tile_at(0));
    assert_eq!((s.wm().focused(), s.wm().windows()[1].1), (Some(WinId(2)), State::Normal));
    s.click(s.tile_at(3));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.click(s.tile_at(3));
    assert_eq!(s.wm().windows()[0].1, State::Minimized);
    // The gap between tiles splits; the shelf's ends are bare.
    let d = s.dock_rect();
    assert_eq!(s.hit(d.x + 56.0, d.y + 30.0), Some(Target::Dock(0)));
    assert_eq!(s.hit(d.x + 57.0, d.y + 30.0), Some(Target::Dock(1)));
    assert_eq!(s.hit(d.x + 2.0, d.y + 2.0), Some(Target::DockBar));
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
}

#[test]
fn the_launcher_searches_and_opens() {
    let (mut s, log) = desk();
    let r = s.k(Space, "a");
    assert_eq!((r.consumed, r.text_input, s.launcher.open), (true, Some(true), true));
    let found = |s: &Shell| {
        (0..s.launcher.search.count())
            .map(|k| s.launcher.search.get(k).unwrap().name.clone())
            .collect::<Vec<_>>()
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
    s.host.deliver(WinId(1), E::Text("open launcher".into()), &mut Response::default());
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
    let f = |s: &Shell| s.placement(WinId(2)).map(|p| (p.state, p.snap));
    s.k(Up, "m");
    assert_eq!(f(&s), Some((State::Maximized, None)));
    s.k(Down, "a");
    assert_eq!(f(&s), Some((State::Normal, None)));
    s.k(Left, "a");
    assert_eq!(f(&s), Some((State::Normal, Some(Snap::Left))));
    s.k(Right, "a");
    assert_eq!(f(&s), Some((State::Normal, Some(Snap::Right))));
    s.k(Char('`'), "a");
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.k(Char('`'), "as");
    assert_eq!(s.wm().focused(), Some(WinId(2)));
    s.k(Down, "a");
    assert_eq!((f(&s), s.wm().focused()), (None, Some(WinId(1))));
    assert!(s.k(Char('q'), "a").consumed);
    assert_eq!(s.names(), ["terminal"]);
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Key { .. })));
}

#[test]
fn other_keys_go_to_the_focused_app() {
    let (mut s, log) = desk();
    for (key, m) in [(Escape, "a"), (Char('x'), ""), (Enter, "ac"), (Char('q'), "as"), (F(5), "")] {
        let r = s.k(key, m);
        assert_eq!(r.consumed, key != F(5), "{key:?} {m}");
        assert!(matches!(log.take()[..], [("welcome", E::Key { .. })]));
    }
    // Paste keys are the browser's; the text comes after.
    assert_eq!(s.k(Char('v'), "c"), Response::default());
    assert!(s.input(Input::Text("hi".into())).consumed);
    assert_eq!(log.take(), [("welcome", E::Text("hi".into()))]);
    // A terminal wants text: reload is its own.
    let r = s.k(Enter, "a");
    assert_eq!(r.text_input, Some(true));
    assert!(s.k(F(5), "").consumed);
}

#[test]
fn content_gets_the_pointer() {
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    assert_eq!(s.to((c.x + 30.0, c.y + 40.0)).cursor, Some(Cursor::Text));
    s.click((c.x + 10.0, c.y + 10.0));
    let want = [
        ("welcome", E::PointerDown { x: 10.0, y: 10.0, id: Some(W(1)) }),
        ("welcome", E::Click(W(1))),
    ];
    assert_eq!(log.take(), want);
    // A release elsewhere is no click; the wheel goes to the window under it.
    s.down((c.x + 10.0, c.y + 10.0));
    s.up((c.x + 100.0, c.y + 10.0));
    assert!(s.input(Input::Wheel { x: c.x + 5.0, y: c.y + 5.0, dy: 3.0 }).consumed);
    assert_eq!(log.take().len(), 2);
}

#[test]
fn themes_switch_and_crossfade() {
    let (mut s, _) = desk();
    let theme = (1280.0 - 19.0, 16.0);
    for name in ["Dawn", "Mono", "Midnight"] {
        s.click(theme);
        assert_eq!((s.theme_name(), s.clear_color()), (name, ui::theme(name).base));
        assert!(s.animating());
        s.rest(1000.0);
        assert!(!s.animating());
    }
    s.host.deliver(WinId(1), E::Text("theme MONO;theme nope".into()), &mut Response::default());
    assert!(s.input(Input::PointerLeave).redraw);
    assert_eq!(s.theme_name(), "Mono");
    let (s2, _) = desk_of(800.0, 600.0);
    assert_eq!(s2.theme_name(), "Midnight");
    // The settings button opens settings, then focuses it.
    s.click((1280.0 - 47.0, 16.0));
    s.click((1280.0 - 47.0, 16.0));
    assert_eq!(s.names(), ["welcome", "settings"]);
}

#[test]
fn the_clock_ticks() {
    let (mut s, _) = desk();
    let time = LocalTime { year: 2026, month: 10, day: 1, weekday: 3, hour: 9, minute: 5 };
    assert!(s.input(Input::Tick { time }).redraw);
    assert!(!s.input(Input::Tick { time }).redraw);
}

#[test]
fn degenerate_sizes_never_panic() {
    for (w, h) in [(0.0, 0.0), (1.0, 1.0), (f32::NAN, f32::INFINITY), (90.0, 40.0), (2e7, 2e7)] {
        let (mut s, _) = desk_of(w, h);
        s.at(0.0);
        for input in [
            Input::Key { key: Enter, mods: Mods { alt: true, ..Mods::default() } },
            Input::PointerDown { x: 10.0, y: 40.0, button: 0 },
            Input::PointerMove { x: -50.0, y: f32::NAN },
            Input::PointerUp { x: 0.0, y: 0.0, button: 0 },
            Input::Key { key: Space, mods: Mods { alt: true, ..Mods::default() } },
            Input::Text("x".into()),
            Input::Key { key: Down, mods: Mods::default() },
            Input::Resize { w: 3.0, h: f32::NAN },
        ] {
            s.input(input);
            s.at(50.0);
        }
        s.rest(1000.0);
    }
}
