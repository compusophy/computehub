use std::cell::RefCell;
use std::rc::Rc;

use gfx::{DrawList, Kind, RectF};
use host::frame::controls;
use host::motion::Vis;
use host::{content_rect, rectf};
use ui::Key::*;
use ui::kernel::{Effect as K, Program, Spawn, wire::Stdout::Console};
use ui::{App, AppEvent as E, Cx, Sense, THEMES, Ui, WidgetId as W};
use wm::{Snap, State};

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const KNOWN: &str = "welcome terminal assistant studio settings about files feedback";
/// The compact apps, each 680 x 480 as a window.
const COMPACT: &str = "welcome settings about feedback";
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

    fn compact(&self) -> bool {
        COMPACT.split(' ').any(|c| c == self.0)
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        self.compact().then_some((678.0, 439.0))
    }
}

fn desk_with(w: f32, h: f32, prefs: Prefs) -> (Shell, Log) {
    let (log, text) = (Log::default(), TextSystem::new(SANS.to_vec()).unwrap());
    let l = log.clone();
    let reg: Registry = Box::new(move |name| {
        let k = KNOWN.split(' ').find(|k| *k == name)?;
        Some(Box::new(Probe(k, l.clone())) as Box<dyn App>)
    });
    (Shell::new(w, h, text, Vfs::new(), reg, prefs), log)
}

/// A first visit (Welcome opens) in Midnight.
fn desk_of(w: f32, h: f32) -> (Shell, Log) {
    desk_with(w, h, Prefs { theme: "midnight".into(), ..Prefs::default() })
}

/// A 1280 x 800 first visit with welcome at rest at t = 1000, its log and effects cleared.
fn desk() -> (Shell, Log) {
    let (mut s, log) = desk_of(1280.0, 800.0);
    s.at(0.0);
    s.at(1000.0);
    s.take_effects();
    log.take();
    (s, log)
}

/// The everything bar's middle on a 1280 x 800 screen, and Home's icon.
const BAR: (f32, f32) = (640.0, 765.0);
const HOME: (f32, f32) = (57.0, 105.0);

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
    fn push(&mut self, (x, y): (f32, f32), button: u8, touch: bool) -> Response {
        self.input(Input::PointerDown { x, y, button, touch })
    }
    fn down(&mut self, at: (f32, f32)) -> Response {
        self.push(at, 0, false)
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
    /// A right click: the menu opens on the press.
    fn right(&mut self, at: (f32, f32)) {
        self.to(at);
        self.push(at, 2, false);
        self.input(Input::PointerUp { x: at.0, y: at.1, button: 2 });
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
        (self.dock_tile(i).x + 22.0, self.dock_tile(i).y + 22.0)
    }
    /// The open menu's labels, and where its item `label` is.
    fn labels(&self) -> Vec<&str> {
        self.menu.as_ref().map_or(Vec::new(), |m| m.0.items.iter().map(|i| i.0).collect())
    }
    fn item(&self, label: &str) -> (f32, f32) {
        let m = &self.menu.as_ref().expect("a menu").0;
        let r = m.item(m.items.iter().position(|i| i.0 == label).expect(label));
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }
}

#[test]
fn welcome_opens_on_a_first_visit_once_there_is_a_work_area() {
    let (mut s, _) = desk_of(1280.0, 800.0);
    assert_eq!(s.wm().area(), Rect::new(0, 44, 1280, 619));
    assert_eq!(s.rect(1), Some(Rect::new(300, 113, 680, 480)));
    assert_eq!((s.names(), s.theme_name()), (vec!["welcome"], "Midnight"));
    assert_eq!(s.take_effects(), [Effect::Pref { key: "seen".into(), value: "1".into() }]);
    let dock: Vec<&str> = s.dock.iter().map(|d| &*d.0).collect();
    assert_eq!(dock, ["studio", "assistant", "terminal", "files", "settings", "welcome"]);
    // The window fades in from its first frame; then frames stop.
    assert!(s.animating() && s.at(5000.0) && s.at(5100.0) && !s.at(5180.0));
    let r = s.input(Input::PointerLeave);
    let got = (r.redraw, r.consumed, r.text_input, r.animating);
    assert_eq!(got, (false, false, Some(false), false));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    // Made before the screen has a size, it opens at the first size that leaves a work area,
    // as it would have from the start; on a phone, maximized.
    let (big, phone) = (Rect::new(300, 113, 680, 480), Rect::new(0, 44, 600, 219));
    for (w, h, want) in [(1280.0, 800.0, big), (600.0, 400.0, phone)] {
        let (mut s, _) = desk_of(0.0, 0.0);
        for (w, h) in [(0.0, 0.0), (1280.0, 180.0), (0.4, 800.0)] {
            s.input(Input::Resize { w, h });
            assert!(s.wm().layout().is_empty());
        }
        let r = s.input(Input::Resize { w, h });
        assert!(r.redraw && r.animating && s.rect(1) == Some(want));
        assert_eq!(s.wm().state_hash(), desk_of(w, h).0.wm().state_hash());
        s.input(Input::Resize { w: 1000.0, h: 700.0 });
        assert_eq!(s.names(), ["welcome"]);
    }
    // Once seen, the desktop starts empty.
    let (mut s, _) = desk_with(1280.0, 800.0, Prefs { seen: true, ..Prefs::default() });
    assert!(s.names().is_empty() && s.take_effects().is_empty() && s.theme_name() == "Mono");
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
fn windows_open_placed_for_the_screen_and_stay_full_on_a_phone() {
    let (mut s, _) = desk();
    // Among other windows a main app opens large, cascaded from the area's corner.
    s.say(1, "open studio");
    assert_eq!(s.rect(2), Some(Rect::new(28, 72, 1088, 526)));
    // Alone, it opens maximized; restored, it is large and centered, not small.
    s.k(Char('q'), "a");
    s.k(Char('q'), "a");
    s.rest(1000.0);
    s.k(Enter, "a");
    assert_eq!(
        s.placement(WinId(3)).map(|p| (p.state, p.rect)),
        Some((State::Maximized, s.wm().area()))
    );
    s.k(Down, "a");
    assert_eq!(s.rect(3), Some(Rect::new(96, 90, 1088, 526)));
    // On a phone every window is full; titlebars do not drag; the controls are a fingertip wide.
    s.input(Input::Resize { w: 390.0, h: 800.0 });
    s.say(3, "open settings");
    let full: Vec<_> = s.wm().layout().iter().map(|p| (p.state, p.rect)).collect();
    assert_eq!(full, [(State::Maximized, Rect::new(0, 44, 390, 619)); 2]);
    s.rest(1000.0);
    assert_eq!(s.to((100.0, 60.0)).cursor, None);
    s.drag((100.0, 60.0), (200.0, 300.0));
    s.k(Left, "a");
    assert_eq!(s.wm().layout()[1].rect, Rect::new(0, 44, 390, 619));
    assert_eq!(
        [(350.0, 80.0), (330.0, 50.0)].map(|(x, y)| s.hit(x, y)),
        [Some(Target::Ctl(WinId(4), 2)), Some(Target::Ctl(WinId(4), 1)),]
    );
    let r = rectf(s.rect(4).unwrap());
    assert_eq!(controls(r, s.ctl_step()).unwrap()[2].x, 362.0);
    // The window menu has no Maximize there.
    s.right((100.0, 60.0));
    assert_eq!(s.labels(), ["Minimize", "Close"]);
}

#[test]
fn titlebars_drag_and_double_click() {
    let (mut s, log) = desk();
    let title = (400.0, 130.0);
    assert_eq!(s.to(title).cursor, Some(Cursor::Grab));
    assert_eq!(s.down(title).cursor, Some(Cursor::Grabbing));
    // Under 4 px of travel nothing moves; then the window follows.
    assert!(!s.to((402.0, 131.0)).redraw);
    let r = s.to((500.0, 230.0));
    assert!(r.redraw && !r.animating && s.rect(1) == Some(Rect::new(400, 213, 680, 480)));
    assert_eq!(s.up((500.0, 230.0)).cursor, Some(Cursor::Grab));
    // Two presses within 350 ms maximize, two more restore; further apart
    // they are two clicks.
    let (max, up) = (State::Maximized, State::Normal);
    for (t, gap, state) in [(2000.0, 300.0, max), (3000.0, 300.0, up), (4000.0, 400.0, up)] {
        let r = s.rect(1).unwrap();
        let at = (r.x as f32 + 100.0, r.y as f32 + 10.0);
        s.set_now(t);
        s.click(at);
        s.set_now(t + gap);
        assert!(s.click(at).animating);
        assert_eq!(s.placement(WinId(1)).unwrap().state, state);
    }
    assert_eq!(s.rect(1), Some(Rect::new(400, 213, 680, 480)));
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
    s.drag((320.0, 60.0), (600.0, 300.0));
    assert_eq!(s.rect(1), Some(Rect::new(430, 284, 680, 480)));
}

#[test]
fn edges_resize_with_their_cursors() {
    let (mut s, _) = desk();
    let cursors = [
        ((300.0, 300.0), Cursor::EwResize),
        ((980.0, 300.0), Cursor::EwResize),
        ((600.0, 114.0), Cursor::NsResize),
        ((298.0, 114.0), Cursor::NwseResize),
        ((979.0, 592.0), Cursor::NwseResize),
        ((975.0, 114.0), Cursor::NeswResize),
        ((600.0, 400.0), Cursor::Default),
    ];
    for (at, c) in cursors {
        s.to(at);
        assert_eq!(s.cursor, c, "{at:?}");
    }
    // The left edge moves, the right one stays, the size stays legal, and
    // the top edge stops under the bar.
    let drags = [
        ((300.0, 300.0), (250.0, 300.0), Rect::new(250, 113, 730, 480)),
        ((250.0, 300.0), (900.0, 300.0), Rect::new(660, 113, 320, 480)),
        ((800.0, 113.0), (800.0, 0.0), Rect::new(660, 44, 320, 549)),
        ((980.0, 593.0), (1100.0, 700.0), Rect::new(660, 44, 440, 619)),
    ];
    for (from, to, want) in drags {
        s.drag(from, to);
        assert_eq!(s.rect(1), Some(want));
    }
}

#[test]
fn controls_minimize_maximize_and_close() {
    let (mut s, log) = desk();
    // Welcome runs a process; the kernel's effects leave with the shell's.
    let k = s.kernel_mut();
    k.set_isolated(true);
    k.set_owner(1);
    let (argv, roots, program) = (vec!["spin".into()], vec!["/".into()], Program::Url("x".into()));
    _ = k.spawn(Spawn { argv, program, cwd: "/".into(), tty: None, stdout: Console, roots });
    assert_eq!(s.take_effects(), [Effect::Kernel(K::Spawn { pid: 2, sab: true })]);
    let step = s.ctl_step();
    let [min, max, close] =
        controls(rectf(s.rect(1).unwrap()), step).unwrap().map(|c| (c.x + 6.0, c.y + 6.0));
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
    s.click(s.tile_at(5));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.rest(1000.0);
    // Closing fades it out, then drops the app and kills what it ran.
    assert!(s.click(close).animating);
    assert_eq!((s.names().len(), s.host.wins.len()), (0, 1));
    s.rest(1000.0);
    assert_eq!(s.host.wins.len(), 0);
    assert_eq!(s.take_effects(), [Effect::Kernel(K::Kill { pid: 2 })]);
    assert_eq!(log.take().last(), Some(&("welcome", E::Focus(true))));
}

#[test]
fn the_dock_opens_minimizes_and_focuses_and_keeps_the_persons_favorites() {
    let (mut s, log) = desk();
    assert!(s.to(s.tile_at(2)).redraw && s.animating());
    // Terminal opens, minimizes and comes back; then welcome is focused and minimized.
    let (m, n) = (State::Minimized, State::Normal);
    let steps = [(2, [n, n], 2), (2, [n, m], 1), (2, [n, n], 2), (5, [n, n], 1), (5, [m, n], 2)];
    for (tile, states, focus) in steps {
        s.click(s.tile_at(tile));
        let got: Vec<State> = s.wm().windows().iter().map(|w| w.1).collect();
        assert_eq!((&got[..], s.wm().focused()), (&states[..], Some(WinId(focus))));
    }
    assert_eq!(s.names(), ["welcome", "terminal"]);
    // The gap between tiles splits; the shelf's ends and the hairlines' gaps are bare.
    let d = s.dock_rect();
    let hits = [55.0, 56.0, 2.0, 272.0].map(|x| s.hit(d.x + x, d.y + 30.0));
    assert_eq!(
        hits.map(|h| h.unwrap()),
        [Target::Dock(0), Target::Dock(1), Target::Shelf, Target::Shelf]
    );
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
    // The favorites come from the page (those the registry knows show); the menu changes them.
    let prefs = Prefs { dock: Some("terminal,nope".into()), seen: true, ..Prefs::default() };
    let (mut s, _) = desk_with(1280.0, 800.0, prefs);
    assert_eq!((s.dock.len(), s.favs.len(), &*s.dock[0].0), (1, 2, "terminal"));
    s.right(s.tile_at(0));
    assert_eq!(s.labels(), ["Open", "Remove from dock"]);
    let pref = |v: &str| vec![Effect::Pref { key: "dock".into(), value: v.into() }];
    assert_eq!(s.click(s.item("Remove from dock")).effects, pref("nope"));
    assert_eq!((s.menu.is_none(), s.dock.len(), s.favs.len()), (true, 0, 1));
    s.right(HOME);
    assert_eq!(s.click(s.item("Keep in dock")).effects, pref("nope,files"));
    s.right(s.tile_at(0));
    s.click(s.item("Open"));
    s.right(s.tile_at(0));
    assert_eq!(s.labels(), ["New window", "Remove from dock", "Close"]);
    s.k(Down, "");
    s.k(Enter, "");
    assert_eq!(s.names(), ["files", "files"]);
    s.right(s.tile_at(0));
    s.click(s.item("Close"));
    assert!(s.names().is_empty());
    // The Apps button, last, shows the launcher; a press on the veil hides it.
    s.rest(1000.0);
    s.click(s.tile_at(1));
    assert!(s.launcher.open && s.launcher.focus && s.launcher.search.query.is_empty());
    s.click(s.tile_at(1));
    assert!(!s.launcher.open && !s.launcher.focus);
}

#[test]
fn menus_open_where_pressed_follow_the_keys_and_act() {
    let (mut s, log) = desk();
    // The bare desktop's menu, at the pointer (kept on screen).
    s.right((600.0, 640.0));
    let desktop =
        ["Open Terminal", "Ask the Assistant", "", "Settings", "Send feedback", "About compusophy"];
    assert_eq!(s.labels(), desktop);
    assert_eq!(s.menu.as_ref().map(|m| (m.0.rect.x, m.0.rect.y)), Some((600.0, 447.0)));
    // Escape closes it; a press outside closes it and does nothing else.
    s.k(Escape, "");
    assert!(s.menu.is_none());
    s.right((600.0, 640.0));
    s.to(HOME);
    assert!(s.down(HOME).redraw && s.menu.is_none());
    s.up(HOME);
    assert_eq!(s.names(), ["welcome"]);
    // Hover and the arrows select; Enter does it: Ask focuses the everything bar.
    s.right((600.0, 640.0));
    s.to(s.item("Settings"));
    assert_eq!(s.menu.as_ref().unwrap().0.sel, Some(3));
    s.k(Up, "");
    let r = s.k(Enter, "");
    assert_eq!((r.text_input, s.launcher.focus, s.menu.is_none()), (Some(true), true, true));
    s.k(Escape, "");
    // A click on an item: a terminal opens.
    s.right((600.0, 640.0));
    s.click(s.item("Open Terminal"));
    assert_eq!(s.names(), ["welcome", "terminal"]);
    // A titlebar's: Minimize, Maximize, Close, for its window (which comes to the top).
    s.rest(1000.0);
    s.k(Char('`'), "a");
    s.right((600.0, 90.0));
    assert_eq!(
        (s.labels(), s.wm().focused()),
        (vec!["Minimize", "Maximize", "Close"], Some(WinId(2)))
    );
    s.click(s.item("Maximize"));
    s.right((600.0, 60.0));
    assert_eq!(s.labels(), ["Minimize", "Restore", "Close"]);
    s.click(s.item("Close"));
    assert_eq!(s.names(), ["welcome"]);
    // An icon's: open it, or keep it in the dock; a right press inside a menu does nothing.
    s.right((57.0, 297.0));
    assert_eq!(s.labels(), ["Open", "Keep in dock"]);
    s.right(s.item("Open"));
    s.click(s.item("Open"));
    assert_eq!(s.names(), ["welcome", "about"]);
    // Nothing for a window's content, the top bar or the everything bar.
    for at in [(640.0, 400.0), (640.0, 20.0), BAR] {
        s.right(at);
        assert!(s.menu.is_none(), "{at:?}");
    }
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
}

#[test]
fn a_finger_held_still_long_presses_and_its_press_does_nothing_more() {
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    // On a button in a window: frames come while it waits (and only then); at 500 ms its
    // press is over, so lifting it clicks nothing.
    s.set_now(2000.0);
    assert!(s.push((c.x + 10.0, c.y + 10.0), 0, true).animating);
    assert!(s.at(2100.0) && s.at(2499.0) && !s.at(2500.0));
    s.up((c.x + 10.0, c.y + 10.0));
    assert!(matches!(log.take()[..], [("welcome", E::PointerDown { .. })]));
    // On the desktop: its menu, where the finger is, with touch-high items; lifting keeps it.
    s.set_now(3000.0);
    s.push((600.0, 640.0), 0, true);
    s.set_now(3600.0);
    s.up((600.0, 640.0));
    let m = &s.menu.as_ref().expect("a menu").0;
    assert_eq!((m.row, m.items.len()), (44.0, 6));
    // A finger that wanders, or lifts early, is no long press.
    s.k(Escape, "");
    s.push((600.0, 640.0), 0, true);
    s.to((600.0, 652.0));
    assert!(!s.animating());
    s.set_now(5000.0);
    s.up((600.0, 652.0));
    s.push((600.0, 640.0), 0, true);
    s.up((600.0, 640.0));
    s.set_now(9000.0);
    assert!(!s.at(9000.0) && s.menu.is_none());
}

#[test]
fn fingers_scroll_what_they_hold_and_fling_it_on() {
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    let at = (c.x + 10.0, c.y + 10.0);
    let wheels = |log: &Log| -> Vec<f32> {
        log.take()
            .iter()
            .filter_map(|e| if let E::Wheel { dy, .. } = e.1 { Some(dy) } else { None })
            .collect()
    };
    // Past 8 px the press is no longer a click, and the content follows the finger.
    s.set_now(1000.0);
    s.push(at, 0, true);
    for (t, dy) in [(1010.0, 5.0), (1020.0, 10.0), (1036.0, 26.0)] {
        s.set_now(t);
        s.to((at.0, at.1 - dy));
    }
    assert_eq!(wheels(&log), [16.0]);
    s.set_now(1040.0);
    let r = s.up((at.0, at.1 - 26.0));
    // Let go moving, it flings: a frame's step each frame, slowing, until it stops.
    assert!(r.animating && log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
    let mut t = 1040.0;
    while s.at(t + 16.0) {
        t += 16.0;
    }
    let steps = wheels(&log);
    let sum: f32 = steps.iter().sum();
    assert!(
        steps.len() > 60 && steps.windows(2).all(|w| w[0] > w[1]) && (sum - 251.0).abs() < 4.0,
        "{sum}"
    );
    assert!(!s.at(t + 100.0));
    // A press stops a fling; a mouse dragged over content never scrolls it.
    s.push(at, 0, true);
    s.set_now(t + 200.0);
    s.to((at.0, at.1 - 100.0));
    s.set_now(t + 210.0);
    s.to((at.0, at.1 - 120.0));
    assert!(s.up((at.0, at.1 - 120.0)).animating);
    s.down(at);
    assert!(!s.at(t + 300.0));
    s.to((at.0, at.1 - 100.0));
    s.up((at.0, at.1 - 100.0));
    assert_eq!(wheels(&log), [20.0]);
    // The launcher's list scrolls by finger, a row at a time.
    for i in 0..20 {
        _ = s.host.vfs.write(&format!("/apps/a{i}.app"), b"");
    }
    s.k(Space, "a");
    s.rest(1000.0);
    let row = s.panel().row(1);
    let at = (row.x + 100.0, row.y + 10.0);
    s.push(at, 0, true);
    s.to((at.0, at.1 - 9.0));
    s.to((at.0, at.1 - 100.0));
    assert_eq!(s.launcher.search.first, 2);
}

#[test]
fn the_everything_bar_opens_apps_or_asks_the_assistant() {
    let (mut s, log) = desk();
    // A tap focuses it, asking for the keyboard; typing shows the panel above it, the Ask row
    // first, the first app named by what was typed selected.
    let r = s.click(BAR);
    assert_eq!((r.text_input, s.launcher.focus, s.launcher.open), (Some(true), true, false));
    assert_eq!(s.click(BAR).text_input, Some(true));
    s.input(Input::Text("st".into()));
    let l = &s.launcher.search;
    assert!(s.launcher.open && l.ask() && l.get(l.sel).is_some_and(|e| e.name == "studio"));
    assert!(s.panel().rect.y + s.panel().rect.h <= home::field::rect(s.size).y);
    let r = s.k(Enter, "");
    assert_eq!((s.names(), r.text_input), (vec!["welcome", "studio"], Some(false)));
    assert!(!s.launcher.open && !s.launcher.focus);
    // Words no app starts with select the Ask row: Enter asks the Assistant, opening it.
    s.click(BAR);
    s.input(Input::Text("build me a clock".into()));
    assert_eq!(s.launcher.search.sel, 0);
    s.k(Enter, "");
    assert_eq!(s.names(), ["welcome", "studio", "assistant"]);
    assert!(log.take().contains(&("assistant", E::Ask("build me a clock".into()))));
    // A click on the Ask row asks too.
    s.click(BAR);
    s.input(Input::Text("hi".into()));
    s.rest(1000.0);
    let a = s.panel().ask_row();
    s.click((a.x + 40.0, a.y + 20.0));
    assert!(log.take().contains(&("assistant", E::Ask("hi".into()))) && s.names().len() == 3);
    // Escape clears and blurs; a press elsewhere takes the keys away; so does the veil.
    s.click(BAR);
    s.input(Input::Text("x".into()));
    s.k(Escape, "");
    assert!(!s.launcher.open && !s.launcher.focus);
    s.click(BAR);
    s.click((600.0, 640.0));
    assert!(!s.launcher.focus);
    // Alt+Space shows every app, the bar focused; with no query Enter opens the first.
    let r = s.k(Space, "a");
    assert_eq!((r.text_input, s.launcher.open, s.launcher.search.sel), (Some(true), true, 0));
    s.input(Input::Text("zzz".into()));
    s.k(Backspace, "");
    s.k(Backspace, "");
    s.k(Backspace, "");
    assert!(s.launcher.open && !s.launcher.search.ask());
    s.k(Enter, "");
    assert_eq!(s.names().last(), Some(&"studio"));
    // An app can ask for the launcher; its tiles open what they show; paste stays the browser's.
    s.rest(1000.0);
    s.say(1, "open launcher");
    s.input(Input::PointerLeave);
    assert!(s.launcher.open);
    s.rest(1000.0);
    let t = s.panel().tile(2);
    s.click((t.x + 40.0, t.y + 40.0));
    assert_eq!(s.names().last(), Some(&"terminal"));
    s.k(Space, "a");
    assert!(!s.k(Char('v'), "c").consumed);
}

#[test]
fn the_desktop_shows_icons_and_the_top_bar_its_buttons() {
    let (mut s, _) = desk();
    let icons: Vec<&str> = s.icons.iter().map(|e| &*e.label).collect();
    assert_eq!(icons, ["Home", "Welcome", "About", "Feedback"]);
    // Icons light up under the pointer and open with a click (Home is Files at ~).
    assert_eq!(
        (s.hit(HOME.0, HOME.1), s.hit(200.0, 640.0)),
        (Some(Target::Icon(0)), Some(Target::Desktop))
    );
    assert!(s.to(HOME).redraw && !s.to((HOME.0 + 5.0, HOME.1)).redraw);
    s.click((57.0, 201.0));
    assert_eq!((s.names(), s.wm().focused()), (vec!["welcome"], Some(WinId(1))));
    s.click(HOME);
    assert_eq!((s.names(), s.wm().focused()), (vec!["welcome", "files"], Some(WinId(2))));
    // A new app saved to ~/apps shows up.
    let mine = [Vfs::HOME, "/apps"].concat();
    s.host.vfs.mkdir_all(&mine).unwrap();
    s.host.vfs.write(&[&mine, "/clock.app"].concat(), b"").unwrap();
    assert!(s.input(Input::PointerLeave).redraw && s.icons.len() == 5);
    // The mark shows Welcome; the right buttons Feedback and Settings.
    s.click((27.0, 22.0));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.click((1280.0 - 71.0, 22.0));
    s.click((1280.0 - 27.0, 22.0));
    s.click((1280.0 - 27.0, 22.0));
    assert_eq!(s.names(), ["welcome", "files", "feedback", "settings"]);
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
    // Alt+A opens the Assistant once, then brings it back.
    s.k(Char('a'), "a");
    s.k(Down, "a");
    assert!(s.k(Char('a'), "a").consumed);
    assert_eq!((s.names(), s.wm().focused()), (vec!["terminal", "assistant"], Some(WinId(3))));
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
fn themes_crossfade_and_the_clock_ticks() {
    let (mut s, _) = desk();
    for name in ["Dawn", "Mono", "Midnight"] {
        assert!(s.set_theme(name));
        assert_eq!((s.theme_name(), s.clear_color()), (name, ui::theme(name).base));
        assert!(s.animating());
        s.rest(1000.0);
        assert!(!s.animating());
    }
    s.say(1, "theme MONO;theme nope");
    assert!(s.input(Input::PointerLeave).redraw);
    assert_eq!((s.theme_name(), desk_of(800.0, 600.0).0.theme_name()), ("Mono", "Midnight"));
    assert!(!s.set_theme("mono") && !s.set_theme("nope"));
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
            Input::PointerDown { x: 10.0, y: 40.0, button: 0, touch: false },
            Input::PointerMove { x: -50.0, y: f32::NAN },
            Input::PointerUp { x: 0.0, y: 0.0, button: 0 },
            Input::PointerDown { x: 5.0, y: 50.0, button: 2, touch: false },
            Input::Key { key: Down, mods: none },
            Input::PointerDown { x: 20.0, y: 60.0, button: 0, touch: true },
            Input::PointerMove { x: 20.0, y: f32::INFINITY },
            Input::PointerUp { x: 20.0, y: 0.0, button: 0 },
            Input::Key { key: Space, mods: alt },
            Input::Text("x".into()),
            Input::Wheel { x: 1.0, y: 1.0, dy: f32::NAN },
            Input::Key { key: Down, mods: none },
            Input::Resize { w: 3.0, h: f32::NAN },
            Input::Resize { w: 400.0, h: 300.0 },
        ] {
            s.input(input);
            s.at(50.0);
        }
        s.set_now(5000.0);
        s.rest(1000.0);
    }
}

#[test]
fn a_finger_taps_into_a_window_it_just_opened() {
    // However few frames it drew (mid-motion, or none), a tap finds what is there now.
    for frames in [vec![], vec![16.0], vec![16.0, 90.0], vec![16.0, 90.0, 400.0, 3000.0]] {
        let (mut s, log) = desk_with(390.0, 844.0, Prefs { seen: true, ..Prefs::default() });
        s.rest(1000.0);
        let t0 = s.host.now_ms + 100.0;
        // The cog, tapped: Settings opens (maximized on a phone).
        let cog = (390.0 - 22.0, BAR_H / 2.0);
        s.set_now(t0);
        s.push(cog, 0, true);
        s.up(cog);
        for dt in &frames {
            s.at(t0 + dt);
        }
        let win = s.host.wins.last().map(|w| w.id).expect("settings opened");
        let c = content_rect(rectf(s.placement(win).unwrap().rect));
        let at = (c.x + 10.0, c.y + 10.0);
        log.take();
        s.set_now(t0 + 3500.0);
        s.push(at, 0, true);
        s.set_now(t0 + 3560.0);
        s.up(at);
        let got = log.take();
        assert!(got.iter().any(|e| e == &("settings", E::Click(W(1)))), "{frames:?}: {got:?}");
    }
}
