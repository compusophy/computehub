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
/// content's top-left 60 x 20, a Text hit (2) below that and a Pad (3) below
/// it. A terminal and the Assistant want text input; a program starts its process.
struct Probe(&'static str, Log);

impl App for Probe {
    fn title(&self) -> String {
        self.0.to_uppercase()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        ui.hit(W(1), RectF::new(r.x, r.y, 60.0, 20.0), Sense::Click);
        ui.hit(W(2), RectF::new(r.x, r.y + 20.0, r.w, 40.0), Sense::Text);
        ui.hit(W(3), RectF::new(r.x, r.y + 60.0, r.w, 40.0), Sense::Pad);
    }

    fn event(&mut self, ev: E, cx: &mut Cx<'_>) -> bool {
        self.1.borrow_mut().push((self.0, ev.clone()));
        // A program's window starts its process at its first size.
        if let (E::Resized { .. }, "program") = (&ev, self.0) {
            let (argv, roots, program) = (vec!["welcome".into()], vec![], Program::Url("x".into()));
            let s = Spawn { argv, program, cwd: "/".into(), tty: None, stdout: Console, roots };
            let started = cx.kernel.spawn(s).is_ok().to_string();
            self.1.borrow_mut().push((self.0, E::Text(started)));
        }
        let E::Text(cmds) = ev else {
            return matches!(ev, E::Click(_));
        };
        for cmd in cmds.split(';') {
            match cmd.split_once(' ').unwrap_or((cmd, "")) {
                ("theme", name) => cx.set_theme(name),
                ("open", name) => cx.open(name),
                ("grain", value) => cx.pref("grain", value),
                _ => {}
            }
        }
        true
    }

    fn wants_text_input(&self) -> bool {
        self.0 == "terminal" || self.0 == "assistant"
    }

    fn compact(&self) -> bool {
        COMPACT.split(' ').any(|c| c == self.0)
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        self.compact().then_some((678.0, 439.0))
    }

    /// About's timer wants a frame each 40 ms; Files animates.
    fn frame_in(&self, _: f64) -> Option<u32> {
        [("about", 40), ("files", 0)].iter().find(|a| a.0 == self.0).map(|a| a.1)
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

/// The Assistant's tile's middle on a 1280 x 800 screen, and the first icon's (Studio's).
const AI: (f32, f32) = (1248.0, 768.0);
const STUDIO: (f32, f32) = (57.0, 105.0);

/// Test drivers.
impl Shell {
    /// Draws a frame at `t` ms; whether more are wanted.
    fn at(&mut self, t: f64) -> bool {
        self.set_now(t);
        self.draw(&mut DrawList::new())
    }
    /// Frames at `t` and 100 ms on: a finger still (no frames) since `t - 500` long-presses.
    fn held(&mut self, t: f64) -> bool {
        self.at(t) && self.at(t + home::touch::LATE_MS)
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
    /// The home screen's labels, in order.
    fn labels_home(&self) -> Vec<&str> {
        self.grid.icons.iter().map(|e| &*e.label).collect()
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
    // The work area leaves the bottom row free.
    let (mut s, _) = desk_of(1280.0, 800.0);
    assert_eq!(s.wm().area(), Rect::new(0, 44, 1280, 694));
    assert_eq!(s.rect(1), Some(Rect::new(300, 151, 680, 480)));
    assert_eq!((s.names(), s.theme_name()), (vec!["welcome"], "Midnight"));
    assert_eq!(s.take_effects(), [Effect::Pref { key: "seen".into(), value: "1".into() }]);
    // The dock holds no favorites at first: only what runs.
    let dock: Vec<&str> = s.tiles.iter().map(|d| &*d.0).collect();
    assert_eq!((dock, s.dock.favs.len()), (vec!["welcome"], 0));
    // The window fades in from its first frame; then frames stop.
    assert!(s.animating() && s.at(5000.0) && s.at(5100.0) && !s.at(5180.0));
    let r = s.input(Input::PointerLeave);
    let got = (r.redraw, r.consumed, r.text_input, r.animating);
    assert_eq!(got, (false, false, Some(false), false));
    assert_eq!(s.input(Input::PointerLeave), Response::default());
    // Made before the screen has a size, it opens at the first size that leaves a work area,
    // as it would have from the start; on a phone, maximized.
    let (big, phone) = (Rect::new(300, 151, 680, 480), Rect::new(0, 44, 600, 294));
    for (w, h, want) in [(1280.0, 800.0, big), (600.0, 400.0, phone)] {
        let (mut s, _) = desk_of(0.0, 0.0);
        for (w, h) in [(0.0, 0.0), (1280.0, 106.0), (0.4, 800.0)] {
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
fn the_first_visits_welcome_runs_as_a_program_on_an_isolated_page() {
    // The kernel knows the page is isolated before Welcome opens, at the shell's start: the
    // program it runs starts at its first size.
    for isolated in [true, false] {
        let (log, text) = (Log::default(), TextSystem::new(SANS.to_vec()).unwrap());
        let l = log.clone();
        let reg: Registry = Box::new(move |_| Some(Box::new(Probe("program", l.clone())) as _));
        Shell::new(1280.0, 800.0, text, Vfs::new(), reg, Prefs { isolated, ..Prefs::default() });
        assert!(log.take().contains(&("program", E::Text(isolated.to_string()))));
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
fn windows_open_placed_for_the_screen_and_stay_full_on_a_phone() {
    let (mut s, _) = desk();
    // Among other windows a main app opens large, cascaded from the area's corner.
    s.say(1, "open studio");
    assert_eq!(s.rect(2), Some(Rect::new(28, 72, 1088, 589)));
    // Alone, it opens maximized; restored, it is large and centered, not small.
    s.k(Char('q'), "a");
    s.k(Char('q'), "a");
    s.rest(1000.0);
    s.k(Enter, "a");
    let p = s.placement(WinId(3)).map(|p| (p.state, p.rect));
    assert_eq!(p, Some((State::Maximized, s.wm().area())));
    s.k(Down, "a");
    assert_eq!(s.rect(3), Some(Rect::new(96, 96, 1088, 589)));
    // On a phone every window is full; titlebars do not drag; the controls are a fingertip wide.
    s.input(Input::Resize { w: 390.0, h: 800.0 });
    s.say(3, "open settings");
    let full: Vec<_> = s.wm().layout().iter().map(|p| (p.state, p.rect)).collect();
    assert_eq!(full, [(State::Maximized, Rect::new(0, 44, 390, 694)); 2]);
    s.rest(1000.0);
    assert_eq!(s.to((100.0, 60.0)).cursor, None);
    s.drag((100.0, 60.0), (200.0, 300.0));
    s.k(Left, "a");
    assert_eq!(s.wm().layout()[1].rect, Rect::new(0, 44, 390, 694));
    // No maximize there: minimize takes its place, beside close.
    let hits = [(350.0, 80.0), (330.0, 50.0), (280.0, 50.0)].map(|(x, y)| s.hit(x, y).unwrap());
    let (c, w) = (Target::Ctl, WinId(4));
    assert_eq!(hits, [c(w, 2), c(w, 0), Target::Title(w)]);
    let r = rectf(s.rect(4).unwrap());
    assert_eq!(controls(r, s.ctl_step()).unwrap()[2].x, 362.0);
    let ctl: Vec<_> = s.controls(r).iter().map(|c| (c.0, c.1.x)).collect();
    assert_eq!(ctl, [(0, 318.0), (2, 362.0)]);
    // The window menu has no Maximize there.
    s.right((100.0, 60.0));
    assert_eq!(s.labels(), ["Minimize", "Close"]);
}

#[test]
fn titlebars_and_edges_hold_windows_with_their_cursors() {
    // Windows held by the pointer (`host::grab`), wired: a titlebar shows a hand, grabbing while
    // it moves the window (exactly, under 4 px not at all; no animation).
    let (mut s, log) = desk();
    let title = (400.0, 168.0);
    assert_eq!(s.to(title).cursor, Some(Cursor::Grab));
    assert_eq!(s.down(title).cursor, Some(Cursor::Grabbing));
    assert!(!s.to((402.0, 169.0)).redraw);
    let r = s.to((500.0, 268.0));
    assert!(r.redraw && !r.animating && s.rect(1) == Some(Rect::new(400, 251, 680, 480)));
    assert_eq!(s.up((500.0, 268.0)).cursor, Some(Cursor::Grab));
    // Two presses within 350 ms maximize; held on to the left edge, the preview shows where it
    // will go, then the window goes there.
    for t in [2000.0, 2300.0] {
        s.set_now(t);
        s.click((500.0, 261.0));
    }
    assert_eq!(s.placement(WinId(1)).unwrap().state, State::Maximized);
    s.rest(1000.0);
    let (area, grab) = (s.wm().area(), (500.0, 60.0));
    s.to(grab);
    s.down(grab);
    s.to((520.0, 80.0));
    s.to((3.0, 400.0));
    let zone = s.visuals().zone.unwrap().rect(area);
    assert!(s.animating());
    s.rest(1000.0);
    assert_eq!(rectf(zone), s.motion.preview.value(s.host.now_ms).rect);
    s.up((3.0, 400.0));
    s.rest(1000.0);
    assert_eq!((s.rect(1), s.motion.preview.value(s.host.now_ms).a), (Some(zone), 0.0));
    assert!(log.take().iter().all(|e| matches!(e.1, E::Focus(_) | E::Resized { .. })));
    // Edges and corners show which way they resize, and do.
    let (mut s, _) = desk();
    let cursors = [
        ((300.0, 300.0), Cursor::EwResize),
        ((980.0, 300.0), Cursor::EwResize),
        ((600.0, 152.0), Cursor::NsResize),
        ((298.0, 152.0), Cursor::NwseResize),
        ((979.0, 630.0), Cursor::NwseResize),
        ((975.0, 152.0), Cursor::NeswResize),
        ((600.0, 400.0), Cursor::Default),
    ];
    for (at, c) in cursors {
        s.to(at);
        assert_eq!(s.cursor, c, "{at:?}");
    }
    s.drag((300.0, 300.0), (250.0, 300.0));
    assert_eq!(s.rect(1), Some(Rect::new(250, 151, 730, 480)));
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
    s.click(s.tile_at(0));
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
    // Welcome runs: its tile, at the bottom-left, minimizes it and brings it back.
    assert_eq!(s.dock_tile(0), RectF::new(10.0, 746.0, 44.0, 44.0));
    assert!(s.to(s.tile_at(0)).redraw && s.animating());
    let (m, n) = (State::Minimized, State::Normal);
    for (state, focus) in [(m, None), (n, Some(WinId(1)))] {
        s.click(s.tile_at(0));
        assert_eq!((s.wm().windows()[0].1, s.wm().focused()), (state, focus));
    }
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
    // Nothing kept, nothing running: the Assistant alone, the work area as ever. The Assistant's
    // icon only opens: its corner is its own.
    let (mut s, _) = desk_with(1280.0, 800.0, Prefs { seen: true, ..Prefs::default() });
    let alone = (s.wm().area(), s.tiles.len(), s.hit(32.0, 768.0));
    assert_eq!(alone, (Rect::new(0, 44, 1280, 694), 0, Some(Target::Desktop)));
    s.right((57.0, 201.0));
    assert_eq!(s.labels(), ["Open"]);
    // The favorites come from the page (those the registry knows show), first; the gap between
    // tiles splits, the row's bare rest is the desktop's; menus change them.
    let prefs = Prefs { dock: Some("terminal,nope,files".into()), seen: true, ..Prefs::default() };
    let (mut s, _) = desk_with(1280.0, 800.0, prefs);
    assert_eq!((s.tiles.len(), s.dock.favs.len(), &*s.tiles[0].0), (2, 3, "terminal"));
    let hits = [57.0, 58.0, 150.0].map(|x| s.hit(x, 760.0).unwrap());
    assert_eq!(hits, [Target::Dock(0), Target::Dock(1), Target::Desktop]);
    s.right(s.tile_at(0));
    assert_eq!(s.labels(), ["Open", "Remove from dock"]);
    let pref = |v: &str| vec![Effect::Pref { key: "dock".into(), value: v.into() }];
    assert_eq!(s.click(s.item("Remove from dock")).effects, pref("nope,files"));
    assert_eq!((s.menu.is_none(), s.tiles.len(), s.dock.favs.len()), (true, 1, 2));
    s.right(STUDIO);
    assert_eq!(s.click(s.item("Add to dock")).effects, pref("nope,files,studio"));
    s.right(s.tile_at(1));
    s.click(s.item("Open"));
    s.right(s.tile_at(1));
    assert_eq!(s.labels(), ["New window", "Remove from dock", "Close"]);
    s.k(Down, "");
    s.k(Enter, "");
    assert_eq!(s.names(), ["studio", "studio"]);
    s.right(s.tile_at(1));
    s.click(s.item("Close"));
    assert!(s.names().is_empty());
    // A running app not kept follows the favorites, past a hairline.
    s.k(Enter, "a");
    let sep = s.dock.strip.sep.unwrap();
    let got = (s.tiles.len(), s.hit(sep, 760.0), s.hit(s.dock_tile(2).x, 760.0));
    assert_eq!(got, (3, Some(Target::Desktop), Some(Target::Dock(2))));
}

#[test]
fn kept_tiles_move_along_the_dock_by_a_mouse_or_a_held_finger() {
    let prefs =
        Prefs { dock: Some("terminal,files,studio".into()), seen: true, ..Prefs::default() };
    let (mut s, _) = desk_with(1280.0, 800.0, prefs);
    let shown = |s: &Shell| s.tiles.iter().map(|d| d.0.clone()).collect::<Vec<_>>().join(",");
    let pref = |v: &str| vec![Effect::Pref { key: "dock".into(), value: v.into() }];
    // A mouse carries one past 4 px (grabbing, nothing under it lit), the others giving way;
    // dropped, the order is kept and nothing opens.
    s.to(s.tile_at(0));
    s.down(s.tile_at(0));
    assert!(s.to(s.tile_at(2)).cursor == Some(Cursor::Grabbing) && s.hover.is_none());
    assert_eq!(shown(&s), "files,studio,terminal");
    assert!(s.up(s.tile_at(2)).effects == pref("files,studio,terminal") && s.names().is_empty());
    // The pointer leaving, or Escape, puts it back.
    for cancel in [Input::PointerLeave, Input::Key { key: Escape, mods: Mods::default() }] {
        s.to(s.tile_at(0));
        s.down(s.tile_at(0));
        s.to(s.tile_at(1));
        assert!(s.input(cancel).redraw && s.up(s.tile_at(1)).effects.is_empty());
        assert_eq!(shown(&s), "files,studio,terminal");
    }
    // A finger held on one picks it up: lifted unmoved, its menu; moved 8 px, it drags.
    for (t, to, menu) in [(5000.0, 1, true), (7000.0, 0, false)] {
        s.set_now(t);
        s.push(s.tile_at(1), 0, true);
        s.held(t + 600.0);
        assert!(s.dock.carry.as_ref().is_some_and(|c| c.lifted) && s.menu.is_none());
        s.to(s.tile_at(to));
        s.up(s.tile_at(to));
        assert_eq!(s.labels().len(), if menu { 2 } else { 0 });
        s.k(Escape, "");
    }
    // An icon carried onto the row opens a gap under the pointer: dropped, its app is kept
    // there, and the icon goes back to its place (the home screen's order as it was).
    s.to(cell(4));
    s.down(cell(4));
    s.to(s.tile_at(1));
    let got = (s.dock.strip.gap, s.up(s.tile_at(1)).effects);
    assert_eq!(got, ((1, 1), pref("studio,settings,files,terminal")));
    // A click, carrying nothing, opens it.
    s.click(s.tile_at(0));
    assert_eq!((shown(&s), s.names()), ("studio,settings,files,terminal".into(), vec!["studio"]));
}

#[test]
fn the_assistant_opens_the_overlay_which_halts_when_the_person_takes_over() {
    let (mut s, log) = desk();
    // Its tile in the bottom-right corner, under no window; named while hovered.
    assert_eq!(s.hit(AI.0, AI.1), Some(Target::Assistant));
    assert!(s.to(AI).redraw && s.animating());
    // A click opens the overlay above it with the keys (and, a mouse's, the keyboard), told its
    // size; never a window. It takes keys, text and the pointer as a window's content does.
    assert_eq!(s.click(AI).text_input, Some(true));
    let (card, sized) = (RectF::new(710.0, 258.0, 560.0, 480.0), |w, h| E::Resized { w, h });
    assert_eq!((s.overlay_rects(), s.key_target()), ((card, card), Some(host::OVERLAY)));
    s.k(Char('x'), "");
    s.input(Input::Text("hi".into()));
    s.click((720.0, 264.0));
    let key = E::Key { key: Char('x'), mods: Mods::default() };
    let down = E::PointerDown { x: 10.0, y: 6.0, id: Some(W(1)) };
    let got: Vec<E> = log.take().into_iter().filter(|e| e.0 == "assistant").map(|e| e.1).collect();
    assert_eq!(got, [sized(560.0, 480.0), key, E::Text("hi".into()), down, E::Click(W(1))]);
    assert_eq!(s.names(), ["welcome"]);
    // A press on a window takes the keys back, the overlay staying; the tile hides it and
    // shows it again; Escape hides it; its menu (or Alt+Space) asks the Assistant.
    s.click((320.0, 300.0));
    assert!(s.overlay.open && s.key_target() == Some(WinId(1)));
    s.click(AI);
    assert!(!s.overlay.open && s.hit(720.0, 264.0) == Some(Target::Body(WinId(1))));
    s.click(AI);
    s.k(Escape, "");
    s.right(AI);
    assert!(!s.overlay.open && s.labels() == ["Ask the Assistant"]);
    s.click(s.item("Ask the Assistant"));
    assert!(s.overlay.open && s.overlay.focus);
    // While it works: a pill over a line of buttons, the tile's dot beating. A press, key or
    // wheel in it is its own; outside it, the person takes over: it hears Halt, once.
    s.host.agent.working = true;
    s.input(Input::PointerLeave);
    let pill = RectF::new(850.0, 686.0, 420.0, 52.0);
    assert!(s.overlay_rects() == (pill, RectF { y: 676.0, h: 72.0, ..pill }) && s.animating());
    s.click((860.0, 700.0));
    s.k(Char('y'), "");
    s.input(Input::Wheel { x: 860.0, y: 700.0, dy: 3.0 });
    assert!(s.host.agent.working && log.take().contains(&("assistant", sized(420.0, 72.0))));
    s.click((320.0, 300.0));
    s.k(Char('y'), "");
    let halts = log.take().iter().filter(|e| e.1 == E::Agent(ui::uiwire::Event::Halt)).count();
    assert_eq!((halts, s.host.agent.working, s.overlay_rects().0), (1, false, card));
    // An act's mark flashes for 600 ms; then, the overlay hidden by a press on the bare desktop,
    // the desktop is idle again: no frames.
    s.rest(1000.0);
    let now = s.host.now_ms;
    s.host.agent.flash = Some((RectF::new(10.0, 50.0, 20.0, 20.0), now));
    assert!(s.at(now + 599.0) && !s.at(now + 600.0));
    s.click((200.0, 640.0));
    assert!(!s.overlay.open && !s.at(now + 2000.0));
    // The work area keeps its place as the dock fills.
    let area = s.wm().area();
    s.k(Enter, "a");
    s.right(s.tile_at(1));
    s.click(s.item("Add to dock"));
    assert_eq!((s.wm().area(), s.dock.favs.len()), (area, 1));
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
    s.to(STUDIO);
    assert!(s.down(STUDIO).redraw && s.menu.is_none());
    s.up(STUDIO);
    assert_eq!(s.names(), ["welcome"]);
    // Hover and the arrows select; Enter does it: Ask shows the Assistant.
    s.right((600.0, 640.0));
    s.to(s.item("Settings"));
    assert_eq!(s.menu.as_ref().unwrap().0.sel, Some(3));
    s.k(Up, "");
    s.k(Enter, "");
    assert_eq!((s.names(), s.menu.is_none(), s.overlay.open), (vec!["welcome"], true, true));
    s.k(Escape, "");
    // A click on an item: a terminal opens.
    s.right((600.0, 640.0));
    s.click(s.item("Open Terminal"));
    assert_eq!(s.names(), ["welcome", "terminal"]);
    // A titlebar's: Minimize, Maximize, Close, for its window (which comes to the top).
    s.rest(1000.0);
    s.k(Char('`'), "a");
    s.right((600.0, 90.0));
    assert_eq!(s.labels(), ["Minimize", "Maximize", "Close"]);
    assert_eq!(s.wm().focused(), Some(WinId(2)));
    s.click(s.item("Maximize"));
    s.right((600.0, 60.0));
    assert_eq!(s.labels(), ["Minimize", "Restore", "Close"]);
    s.click(s.item("Close"));
    assert_eq!(s.names(), ["welcome"]);
    // An icon's: open it, or add it to the dock; a right press inside a menu does nothing.
    s.right((57.0, 297.0));
    assert_eq!(s.labels(), ["Open", "Add to dock"]);
    s.right(s.item("Open"));
    s.click(s.item("Open"));
    assert_eq!(s.names(), ["welcome", "terminal"]);
    // Nothing for a window's content or the top bar.
    for at in [(640.0, 400.0), (640.0, 20.0)] {
        s.right(at);
        assert!(s.menu.is_none(), "{at:?}");
    }
    assert!(log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
}

#[test]
fn a_finger_held_still_long_presses_and_only_a_menu_ends_its_press() {
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    // On a button in a window: frames come while it waits (and only then); at 500 ms (frames
    // 16 ms apart) nothing opens (content has no menu), so its press goes on: lifted, it taps.
    s.set_now(2000.0);
    assert!(s.push((c.x + 10.0, c.y + 10.0), 0, true).animating);
    assert!((1..32).all(|i| s.at(2000.0 + 16.0 * f64::from(i))) && !s.at(2512.0));
    assert!(!s.up((c.x + 10.0, c.y + 10.0)).gesture);
    assert!(matches!(log.take()[..], [(_, E::PointerDown { .. }), (_, E::Click(W(1)))]));
    // On the desktop: its menu, where the finger is, with touch-high items; lifting keeps it.
    s.set_now(3000.0);
    s.push((600.0, 640.0), 0, true);
    s.held(3600.0);
    s.up((600.0, 640.0));
    let m = &s.menu.as_ref().expect("a menu").0;
    assert_eq!((m.row, m.items.len()), (44.0, 6));
    // A finger that wanders, or lifts early, is no long press; only the early one a tap.
    s.k(Escape, "");
    s.push((600.0, 640.0), 0, true);
    s.to((600.0, 652.0));
    assert!(!s.animating());
    s.set_now(5000.0);
    assert!(s.up((600.0, 652.0)).gesture);
    s.push((600.0, 640.0), 0, true);
    assert!(!s.up((600.0, 640.0)).gesture);
    s.set_now(9000.0);
    assert!(!s.at(9000.0) && s.menu.is_none());
    // Held on what has no menu (Settings in the top bar), it still acts when it lifts.
    let cog = (1280.0 - 27.0, 22.0);
    s.push(cog, 0, true);
    s.at(9600.0);
    s.up(cog);
    assert_eq!((s.menu.is_none(), s.names()), (true, vec!["welcome", "settings"]));
    // Held on an icon, it picks it up, no menu yet: lifted unmoved, the icon's menu; moved
    // 8 px, a drag, which a mouse starts at 4. A finger on the bare desktop draws no box.
    s.set_now(10_000.0);
    s.push(STUDIO, 0, true);
    s.held(10_600.0);
    assert!(s.grid.carry.as_ref().is_some_and(|c| c.lifted && !c.moved) && s.menu.is_none());
    s.up(STUDIO);
    assert_eq!((s.grid.carry.is_none(), s.labels()), (true, vec!["Open", "Add to dock"]));
    s.k(Escape, "");
    s.set_now(11_000.0);
    s.push(STUDIO, 0, true);
    s.held(11_600.0);
    s.to((STUDIO.0 + 7.0, STUDIO.1));
    assert!(!s.grid.carry.as_ref().unwrap().moved);
    s.to((STUDIO.0, STUDIO.1 + 192.0));
    s.up((STUDIO.0, STUDIO.1 + 192.0));
    assert!(s.menu.is_none() && s.grid.spots[..3] == [2, 1, 3]);
    s.push((600.0, 640.0), 0, true);
    s.to((400.0, 500.0));
    assert!(s.grid.lasso.is_none() && s.up((400.0, 500.0)).gesture);
    // A finger that drifted 9 px while it waited (on the Assistant's icon; Studio's cell, left
    // empty, is the desktop's) drags only 8 px past where it picked one up.
    s.set_now(13_000.0);
    assert_eq!(s.hit(STUDIO.0, STUDIO.1), Some(Target::Desktop));
    s.push((57.0, 201.0), 0, true);
    s.to((66.0, 201.0));
    s.held(13_600.0);
    s.to((73.0, 201.0));
    s.up((73.0, 201.0));
    assert_eq!(s.labels(), ["Open"]);
    // Held on the Assistant's tile: its menu, which is then what the finger is on (no tooltip).
    s.k(Escape, "");
    s.set_now(14_000.0);
    s.push(AI, 0, true);
    s.held(14_600.0);
    assert!(s.labels() == ["Ask the Assistant"] && s.hover != Some(Target::Assistant));
}

#[test]
fn fingers_scroll_what_they_hold_and_fling_it_on() {
    let (mut s, log) = desk();
    let c = content_rect(rectf(s.rect(1).unwrap()));
    let at = (c.x + 10.0, c.y + 10.0);
    let wheel = |e: &(_, E)| if let E::Wheel { dy, .. } = e.1 { Some(dy) } else { None };
    let wheels = |log: &Log| -> Vec<f32> { log.take().iter().filter_map(wheel).collect() };
    // A tap presses into the content as it lifts, then clicks.
    s.push(at, 0, true);
    assert!(log.take().is_empty() && !s.up(at).gesture);
    assert!(matches!(log.take()[..], [(_, E::PointerDown { .. }), (_, E::Click(W(1)))]));
    // Past 8 px the finger presses nothing, and the content follows it.
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
    assert!(r.animating && r.gesture && log.take().is_empty());
    let mut t = 1040.0;
    while s.at(t + 16.0) {
        t += 16.0;
    }
    let steps = wheels(&log);
    let sum: f32 = steps.iter().sum();
    let slowing = steps.windows(2).all(|w| w[0] > w[1]);
    assert!(steps.len() > 60 && slowing && (sum - 251.0).abs() < 4.0, "{sum}");
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
}

#[test]
fn the_home_screen_shows_every_app_and_the_top_bar_its_buttons() {
    let (mut s, _) = desk();
    let all =
        ["Studio", "Assistant", "Terminal", "Files", "Settings", "Feedback", "About", "Welcome"];
    assert_eq!(s.labels_home(), all);
    // Icons light up under the pointer and open with a click.
    let hits = (s.hit(STUDIO.0, STUDIO.1), s.hit(200.0, 640.0));
    assert_eq!(hits, (Some(Target::Icon(0)), Some(Target::Desktop)));
    assert!(s.to(STUDIO).redraw && !s.to((STUDIO.0 + 5.0, STUDIO.1)).redraw);
    s.click((57.0, 201.0));
    assert_eq!((s.names(), s.overlay.open), (vec!["welcome"], true));
    // A new app saved to ~/apps takes the first free cell, with its sigil; the order is kept.
    let mine = [Vfs::HOME, "/apps"].concat();
    s.host.vfs.mkdir_all(&mine).unwrap();
    s.host.vfs.write(&[&mine, "/clock.app"].concat(), b"").unwrap();
    let r = s.input(Input::PointerLeave);
    let kept = "@2,studio::,assistant::,terminal::,files::,settings::,feedback::,about::,welcome::";
    let order = [kept, ",", &mine, "/clock.app::"].concat();
    assert_eq!(r.effects, [Effect::Pref { key: "home.order".into(), value: order }]);
    assert!(r.redraw && s.labels_home()[8] == "Clock" && s.grid.icons[8].sigil.is_some());
    // The mark shows Welcome; the right buttons Feedback (a bug) and Settings.
    s.click((27.0, 22.0));
    assert_eq!(s.wm().focused(), Some(WinId(1)));
    s.click((1280.0 - 71.0, 22.0));
    s.click((1280.0 - 27.0, 22.0));
    s.click((1280.0 - 27.0, 22.0));
    assert_eq!(s.names(), ["welcome", "feedback", "settings"]);
}

/// The middle of the icon in cell `i` of a 1280 x 800 desktop.
fn cell(i: usize) -> (f32, f32) {
    let r = home::icons::cell(i, RectF::new(0.0, 44.0, 1280.0, 694.0), false);
    (r.x + r.w / 2.0, r.y + 30.0)
}

#[test]
fn icons_move_and_open_by_the_pointer_and_the_keys() {
    // The grid's ways (`home::grid`) wired: a mouse carries an icon (grabbing, the cell under it
    // bare) and drops it there, its own left empty, every cell kept, nothing opened; the pointer
    // leaving or Escape puts it back.
    let (mut s, log) = desk();
    s.to(cell(0));
    s.down(cell(0));
    let r = s.to(cell(3));
    assert!(
        r.cursor == Some(Cursor::Grabbing) && s.hit(cell(3).0, cell(3).1) == Some(Target::Desktop)
    );
    let kept = "@2,studio:0.3:,assistant:0.1:,terminal:0.2:,files:0.4:,settings:0.5:,feedback:1.0:";
    let order = [kept, ",about:1.1:,welcome:1.2:"].concat();
    assert_eq!(s.up(cell(3)).effects, [Effect::Pref { key: "home.order".into(), value: order }]);
    assert!(s.names() == ["welcome"] && log.take().iter().all(|e| !matches!(e.1, E::Click(_))));
    for cancel in [Input::PointerLeave, Input::Key { key: Escape, mods: Mods::default() }] {
        s.down(cell(1));
        s.to(cell(6));
        assert!(
            s.input(cancel).redraw && s.up(cell(6)).effects.is_empty() && s.grid.carry.is_none()
        );
    }
    // A box from the bare desktop selects; Enter opens what it holds (the Assistant: the overlay).
    let (mut s, log) = desk();
    s.drag((5.0, 50.0), (95.0, 160.0));
    assert!(s.k(Enter, "").consumed && s.grid.selected.is_empty());
    assert_eq!((s.names(), s.overlay.open), (vec!["welcome", "studio"], true));
    // An app that takes the focus ends it (a selected icon clicked, a binding): Enter is the app's.
    s.k(Char('q'), "a");
    s.k(Char('q'), "a");
    s.drag((5.0, 50.0), (95.0, 260.0));
    s.click(cell(2));
    log.take();
    s.k(Enter, "");
    let enter = E::Key { key: Enter, mods: Mods::default() };
    assert!(s.grid.selected.is_empty() && log.take() == [("terminal", enter)]);
    s.drag((5.0, 50.0), (95.0, 260.0));
    s.k(Space, "a");
    assert!(s.grid.selected.is_empty() && s.overlay.focus && s.names().ends_with(&["terminal"]));
}

#[test]
fn the_grain_lives_eight_times_a_second_by_timer_unless_told_not_to() {
    let (mut s, log) = desk();
    s.rest(1000.0);
    // Each 125 ms a new pattern; the next frame by a timer, never a frame loop.
    let frame = |s: &mut Shell, t: f64| {
        s.set_now(t);
        let mut list = DrawList::new();
        let animating = s.draw(&mut list);
        let grain =
            list.instances().iter().find(|i| i.kind == Kind::Grain as u8 as f32).map(|i| i.p0);
        (grain.unwrap(), animating, s.grain_in())
    };
    let [a, b, c] = [10_000.0, 10_100.0, 10_125.0].map(|t| frame(&mut s, t));
    assert_eq!((a.1, a.2, b.0 == a.0, b.2), (false, Some(125), true, Some(25)));
    assert_eq!((c.0 == a.0, c.2), (false, Some(125)));
    // Not while the page asks for reduced motion, nor once Settings turned it off: then still.
    s.set_reduced_motion(true);
    assert_eq!((frame(&mut s, 10_300.0), frame(&mut s, 10_500.0).0), ((0.0, false, None), 0.0));
    s.set_reduced_motion(false);
    s.say(1, "grain off");
    assert!(!s.host.grain && s.grain_in().is_none());
    // A shown app's timer wants its frame by timer too; one that animates, every frame, hearing
    // each.
    s.say(1, "open about");
    s.rest(1000.0);
    assert_eq!((s.frame_in(), s.animating()), (Some(40), false));
    s.say(1, "open files");
    s.rest(1000.0);
    assert!(s.animating() && log.take().iter().any(|e| matches!(e, ("files", E::Tick { .. }))));
    let (s, _) = desk_with(1280.0, 800.0, Prefs { grain_off: true, ..Prefs::default() });
    assert_eq!(s.grain_in(), None);
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
    // Alt+A shows the Assistant with the keys, and hides it again.
    s.k(Char('a'), "a");
    assert!(s.overlay.focus && s.k(Char('a'), "a").consumed && !s.overlay.open);
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
    // While it types, a keyboard that shortens the page squeezes the windows until it goes
    // (`Host::resize`).
    let before = s.wm().layout();
    s.input(Input::Resize { w: 1280.0, h: 450.0 });
    assert_ne!(s.wm().layout(), before);
    s.input(Input::Resize { w: 1280.0, h: 800.0 });
    assert_eq!(s.wm().layout(), before);
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
    // A mouse held down on content and moved: the app hears where to (a grid draws by it).
    s.drag((c.x + 10.0, c.y + 10.0), (c.x + 30.0, c.y + 12.0));
    assert_eq!(log.take()[1], ("welcome", E::Drag { x: 30.0, y: 12.0 }));
    // A finger on a pad presses it at once and drags it, never scrolling it; no click either.
    s.push((c.x + 10.0, c.y + 70.0), 0, true);
    assert_eq!(log.take(), [("welcome", E::PointerDown { x: 10.0, y: 70.0, id: Some(W(3)) })]);
    s.to((c.x + 40.0, c.y + 30.0));
    assert!(s.up((c.x + 40.0, c.y + 30.0)).gesture && s.fling.is_none());
    assert_eq!(log.take(), [("welcome", E::Drag { x: 40.0, y: 30.0 })]);
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
fn a_finger_taps_however_late_its_lift_is_heard() {
    // The phone bug: a busy page heard a tap's lift after frames (here two, back to back) saw
    // 500 ms pass, and that long press ended the press. Only a menu ends a press now, and after
    // a stall only a frame 100 ms on long-presses: on an icon, the Assistant, a dock tile...
    let (mut s, log) = desk_with(390.0, 844.0, Prefs { seen: true, ..Prefs::default() });
    s.rest(1000.0);
    let tap = |s: &mut Shell, at: (f32, f32), t: f64, late: f64| {
        let _ = (s.set_now(t), s.push(at, 0, true), s.at(t + late), s.at(t + late + 2.0));
        s.set_now(t + late + 10.0);
        s.up(at)
    };
    let term = home::icons::cell(2, rectf(s.wm().area()), true);
    let r = tap(&mut s, (term.x + term.w / 2.0, term.y + 30.0), 2000.0, 600.0);
    let want = (vec!["terminal"], Some(true), true);
    assert_eq!((s.names(), r.text_input, s.grid.carry.is_none()), want);
    assert_eq!(tap(&mut s, (390.0 - 22.0, BAR_H / 2.0), 3000.0, 100.0).text_input, Some(false));
    // The first tap lands while Settings still opens (one frame, mid-motion): it finds it too.
    let c = content_rect(rectf(s.rect(2).unwrap()));
    log.take();
    for (t, late) in [(3200.0, 0.0), (6000.0, 100.0), (7000.0, 600.0), (8000.0, 1500.0)] {
        assert!(!tap(&mut s, (c.x + 10.0, c.y + 10.0), t, late).gesture);
        let got: Vec<_> =
            log.take().into_iter().filter(|e| !matches!(e.1, E::Resized { .. })).collect();
        assert!(matches!(got[..], [(_, E::PointerDown { .. }), (_, E::Click(W(1)))]), "{got:?}");
    }
    // The Assistant's tile, in the corner; a finger's tap opens the overlay with the keys but
    // not the keyboard, which comes only once its text field is tapped (last, below).
    let (ai, title) = (s.dock_rect(), (200.0, rectf(s.rect(2).unwrap()).y + 10.0));
    assert_eq!(ai, RectF::new(336.0, 790.0, 44.0, 44.0));
    let r = tap(&mut s, (ai.x + ai.w / 2.0, ai.y + ai.h / 2.0), 9000.0, 600.0);
    assert_eq!((r.text_input, s.key_target()), (Some(false), Some(host::OVERLAY)));
    assert!(tap(&mut s, title, 10_000.0, 600.0).redraw && s.menu.is_none());
    let tile = s.tile_at(0);
    tap(&mut s, tile, 11_000.0, 600.0);
    // The Assistant opened the overlay (no window); the dock tile then raised the Terminal.
    let names = vec!["terminal", "settings"];
    assert_eq!((s.names(), s.wm().focused(), s.overlay.open), (names, Some(WinId(1)), true));
    let o = s.overlay_rects().1;
    assert_eq!(tap(&mut s, (o.x + 10.0, o.y + 30.0), 12_000.0, 0.0).text_input, Some(true));
}
