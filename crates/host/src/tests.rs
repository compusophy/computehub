use std::cell::RefCell;
use std::rc::Rc;

use gfx::{Kind, Rgba};
use kernel::wire::{Msg, Stdout, VERSION};
use ui::{App, AppEvent as E, THEMES, Ui};

use super::motion::{Lerp, Themes, Tween, Vis, blend, ease, replay};
use super::search::{Entry, Search, rank};
use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const SYM_B: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-b.ttf");
/// The names the registry knows.
const KNOWN: &str = "welcome terminal /apps/counter.app sized huge nan";

type Log = Rc<RefCell<Vec<(u32, E)>>>;

/// A scripted app: its window number (while every open succeeds), name and
/// log. It logs every event and runs the `;`-separated commands of its text.
struct Probe(u32, &'static str, Log);

impl App for Probe {
    fn title(&self) -> String {
        self.1.to_uppercase()
    }
    fn draw(&mut self, ui: &mut Ui<'_>) {
        ui.hit(ui::WidgetId(1), ui.rect(), ui::Sense::Click);
    }
    fn event(&mut self, ev: E, cx: &mut Cx<'_>) -> bool {
        self.2.borrow_mut().push((self.0, ev.clone()));
        let E::Text(cmds) = ev else { return false };
        for cmd in cmds.split(';') {
            match cmd.split_once(' ').unwrap_or((cmd, "")) {
                ("open", arg) => cx.open(arg),
                ("close", _) => cx.close_self(),
                ("fonts", _) => cx.load_fallback_fonts(),
                ("theme", arg) => cx.set_theme(arg),
                ("spawn", _) => _ = cx.kernel.spawn(spin()),
                ("size", _) => cx.set_size(500, 300),
                ("seen", _) => cx.pref("seen", &cx.ai.model.clone()),
                ("pref", kv) => {
                    let (key, value) = kv.split_once('=').unwrap_or((kv, ""));
                    cx.pref(key, value);
                }
                _ => {}
            }
        }
        true
    }
    fn preferred_size(&self) -> Option<(f32, f32)> {
        let i = ["sized", "huge", "nan"].iter().position(|n| *n == self.1)?;
        Some([(400.0, 300.0), (5e3, 5e3), (f32::NAN, 100.0)][i])
    }
    fn icon(&self) -> AppIcon {
        AppIcon { glyph: ui::icon::Glyph::Window, hue: Rgba(self.1.len() as u8, 1, 2, 255) }
    }
    fn frame(&mut self, pid: u32, frame: &[u8], _: &mut Cx<'_>) -> bool {
        let said = [&pid.to_string(), " ", &String::from_utf8_lossy(frame)].concat();
        self.2.borrow_mut().push((self.0, E::Text(said)));
        pid == self.0
    }
    fn closing(&mut self, _: &mut Cx<'_>) {
        self.2.borrow_mut().push((self.0, E::Text("closing".into())));
    }
}

/// A host on a 1280 x 800 screen with welcome and terminal open, window 2
/// focused, settled, its log cleared; `made` counts the apps made.
fn host() -> (Host, Log, Rc<RefCell<u32>>) {
    let (log, n) = (Log::default(), Rc::new(RefCell::new(0)));
    let (l, made) = (log.clone(), n.clone());
    let registry: Registry = Box::new(move |name| {
        let k = KNOWN.split(' ').find(|k| *k == name)?;
        *n.borrow_mut() += 1;
        Some(Box::new(Probe(*n.borrow(), k, l.clone())) as Box<dyn App>)
    });
    let (wm, text) = (Wm::new(Rect::new(0, 32, 1280, 684)), TextSystem::new(SANS.to_vec()));
    let mut h = Host::new(wm, text.unwrap(), Vfs::new(), registry, "Midnight");
    let mut out = Response::default();
    h.open("welcome", Some((680, 480)), &mut out);
    h.open("terminal", None, &mut out);
    h.settle(&mut out);
    log.take();
    (h, log, made)
}

/// What a Probe's `spawn` starts: `spin`, on no console.
fn spin() -> kernel::Spawn {
    let (argv, cwd, roots) = (vec!["spin".into()], "/".into(), vec!["/".into()]);
    let program = kernel::Program::Url("bin/toolbox.wasm".into());
    kernel::Spawn { argv, program, cwd, tty: None, stdout: Stdout::Console, roots }
}

impl Host {
    fn say(&mut self, n: u32, t: &str) -> Response {
        let mut out = Response::default();
        self.deliver(WinId(n), E::Text(t.to_string()), &mut out);
        out
    }
    fn fetch(&mut self, id: u32, got: Result<Vec<u8>, String>) -> Response {
        let mut out = Response::default();
        self.fetched(id, got, &mut out);
        out
    }
    fn rect_of(&self, n: u32) -> Option<Rect> {
        self.wm().layout().iter().find(|p| p.win == WinId(n)).map(|p| p.rect)
    }
    fn names(&self) -> Vec<(u32, &str, bool)> {
        self.wins.iter().map(|w| (w.id.0, &*w.name, !self.live(w.id))).collect()
    }
}

#[test]
fn apps_open_in_floating_windows() {
    let (mut h, log, _) = host();
    assert_eq!(h.rect_of(1), Some(Rect::new(300, 134, 680, 480)));
    h.say(2, "open sized;open nope;open huge;open nan");
    assert_eq!(h.names()[2..], [(3, "sized", false), (4, "huge", false), (5, "nan", false)]);
    // The wm places them: preferred sizes are content sizes; non-finite ones get the default.
    let mut wm = Wm::new(Rect::new(0, 32, 1280, 684));
    let sizes = [Some((680, 480)), None, Some((402, 341)), Some((5002, 5041)), None];
    sizes.into_iter().for_each(|size| _ = wm.apply(Cmd::Open { size }).unwrap());
    assert_eq!(h.wm().state_hash(), wm.state_hash());
    assert_eq!(h.rect_of(4), Some(Rect::new(0, 32, 1280, 684)));
    assert_eq!(h.focused_app(), Some(WinId(5)));
    h.settle(&mut Response::default());
    let r = rectf(h.rect_of(3).unwrap());
    assert_eq!(content_rect(r), RectF::new(r.x + 1.0, r.y + 40.0, 400.0, 300.0));
    let evs = log.take();
    assert!(evs.contains(&(3, E::Resized { w: 400.0, h: 300.0 })));
    assert!(evs.contains(&(2, E::Focus(false))) && evs.contains(&(5, E::Focus(true))));
    // Content is laid out, then clipped.
    let (mut list, layout) = (DrawList::new(), RectF::new(10.0, 50.0, 400.0, 300.0));
    let (clip, st) = (RectF::new(0.0, 0.0, 200.0, 100.0), UiState::default());
    h.draw_content(&mut list, WinId(1), [layout, clip], &THEMES[1], st);
    assert_eq!(h.win(WinId(1)).unwrap().hits[0].rect, RectF::new(10.0, 50.0, 190.0, 50.0));
    h.draw_content(&mut list, WinId(1), [RectF::default(), clip], &THEMES[1], st);
    assert!(h.win(WinId(1)).unwrap().hits.is_empty());
    assert_eq!(list.clip(), RectF::new(-1e9, -1e9, 2e9, 2e9));
}

#[test]
fn closed_apps_wait_to_be_reaped_and_their_processes_die_with_them() {
    let (mut h, log, _) = host();
    // A closed app asks for nothing more and hears nothing more.
    assert!(h.say(2, "close;fonts;open terminal").effects.is_empty());
    assert_eq!(h.names(), [(1, "welcome", false), (2, "terminal", true)]);
    log.take();
    assert_eq!((h.focused_app(), h.wm().focused()), (Some(WinId(1)), Some(WinId(1))));
    h.say(2, "open welcome");
    h.tick(&mut Response::default());
    h.settle(&mut Response::default());
    assert!(log.take().iter().all(|e| e.0 == 1));
    // A window closed in the wm directly is marked when the host settles.
    h.apply(Cmd::Close(WinId(1)));
    h.settle(&mut Response::default());
    assert!(h.names().iter().all(|w| w.2));
    h.reap(WinId(2));
    h.reap(WinId(9));
    assert_eq!(h.names(), [(1, "welcome", true)]);
    // Processes wake their window and die with it.
    let (mut h, log, _) = host();
    h.kernel.set_isolated(true);
    let spawned = |pid| Effect::Kernel(kernel::Effect::Spawn { pid, sab: true });
    assert_eq!(h.say(2, "spawn;spawn").effects, [spawned(2), spawned(3)]);
    // What pid 2 sends wakes window 2 alone.
    let mut out = Response::default();
    for msg in [Msg::Ready { version: VERSION }, Msg::Exit { status: 3 }] {
        h.kernel_in(KernelIn::Msg { pid: 2, msg: msg.encode() }, &mut out);
    }
    assert_eq!((&log.take()[1..], h.kernel.reap(2)), (&[(2, E::Io)][..], Some(3)));
    // Reaping a live window does nothing; reaped once closed, it kills pid 3.
    h.reap(WinId(2));
    assert!(h.say(2, "close").effects.is_empty() && h.wins.len() == 2);
    h.reap(WinId(2));
    h.pump(&mut out);
    assert!(out.effects.contains(&Effect::Kernel(kernel::Effect::Kill { pid: 3 })));
}

#[test]
fn frames_reach_every_app_and_closing_windows_hear_it_first() {
    let (mut h, log, _) = host();
    h.kernel.set_isolated(true);
    h.say(2, "spawn");
    // A frame goes to the apps, never to the platform; its reply does.
    let mut out = Response::default();
    h.kernel_in(KernelIn::Msg { pid: 2, msg: [&[0x20][..], b"hi"].concat() }, &mut out);
    let reply = kernel::Effect::Reply { pid: 2, errno: 0, data: vec![] };
    assert_eq!((out.effects, out.redraw), (vec![Effect::Kernel(reply)], true));
    let said = |n: u32, t: &str| (n, E::Text(t.into()));
    assert_eq!(log.take()[1..], [said(1, "2 hi"), said(2, "2 hi")]);
    // Size resizes the window's content, keeping its corner.
    let r = h.rect_of(2).unwrap();
    h.say(2, "size");
    assert_eq!(h.rect_of(2), Some(Rect::new(r.x, r.y, 502, 300 + TITLEBAR_H as i32 + 1)));
    // Closing, by the wm or by the app itself, tells the app first, once.
    log.take();
    h.apply(Cmd::Close(WinId(2)));
    h.say(1, "close");
    h.apply(Cmd::Close(WinId(2)));
    assert_eq!(log.take(), [said(2, "closing"), (1, E::Text("close".into())), said(1, "closing")]);
}

#[test]
fn apps_get_icons_labels_themes_and_fonts_fetched_once_in_order() {
    let (mut h, _, made) = host();
    assert!(!h.launcher && !h.theme.is_running(0.0));
    h.say(1, "theme Dawn;open launcher;theme nope");
    h.settle(&mut Response::default());
    assert_eq!((h.theme.current().name, h.launcher, h.wins.len()), ("Dawn", true, 2));
    assert!(h.theme.is_running(0.0));
    // Icons come from running apps, else from the registry, once.
    let icon = |name: &str| {
        Some(AppIcon { glyph: ui::icon::Glyph::Window, hue: Rgba(name.len() as u8, 1, 2, 255) })
    };
    assert_eq!((h.icon("terminal"), *made.borrow()), (icon("terminal"), 2));
    assert_eq!((h.icon("sized"), h.icon("sized")), (icon("sized"), icon("sized")));
    assert_eq!((h.icon("nope"), h.icon("nope"), *made.borrow()), (None, None, 3));
    let labels = ["terminal", "/apps/counter.app", "/tmp/éclair.app", ""].map(app_label);
    assert_eq!(labels, ["Terminal", "Counter", "Éclair", ""]);
    // Fonts are fetched once and added in order.
    let fetch = |id, f| Effect::Fetch { id, url: format!("fonts/symbols-{f}.ttf") };
    assert_eq!(h.say(1, "fonts;fonts").effects, [fetch(1, "a"), fetch(2, "b")]);
    assert!(h.say(1, "fonts").effects.is_empty());
    // b waits for a; repeats and unknown ids do nothing.
    assert!(!h.fetch(2, Ok(SYM_B.to_vec())).redraw);
    assert!(h.fetch(1, Ok(SYM_A.to_vec())).redraw && h.text.fallback_count() == 2);
    for id in [0, 1, 2, 3] {
        assert_eq!(h.fetch(id, Ok(SYM_A.to_vec())), Response::default());
    }
    for first in [Err("404".to_string()), Ok(vec![1, 2, 3])] {
        let (mut h, _, _) = host();
        h.say(1, "fonts");
        assert!(!h.fetch(1, first).redraw && h.fetch(2, Ok(SYM_B.to_vec())).redraw);
        assert_eq!(h.text.fallback_count(), 1);
    }
    let (mut h, _, _) = host();
    h.text.add_fallback(SYM_A.to_vec()).unwrap();
    assert!(h.say(1, "fonts").effects.is_empty());
    // Apps see the AI settings; preferences leave as effects, the AI model's updating them.
    h.ai.model = "m".into();
    let pref = |k: &str, v: &str| Effect::Pref { key: k.into(), value: v.into() };
    let prefs = [pref("seen", "m"), pref("ai.model", "l"), pref("dock", "left")];
    assert_eq!(h.say(1, "seen;pref ai.model=l;pref dock=left").effects, prefs);
    assert_eq!(h.ai.model, "l");
}

#[test]
fn tweens_ease_out_from_their_first_frame_replays_scale_and_themes_crossfade() {
    assert_eq!([-1.0, f32::NAN, 0.0, 1.0, 9.0].map(ease), [0.0, 0.0, 0.0, 1.0, 1.0]);
    let samples: Vec<f32> = (1..10).map(|i| ease(i as f32 / 10.0)).collect();
    assert!(samples.windows(2).all(|w| w[0] < w[1]));
    assert!(samples[1] > 0.5 && samples[4] > 0.85 && samples[8] < 1.0);
    let mut t = Tween::new(0.0);
    assert!(!t.is_running(0.0) && t.value(5.0) == 0.0);
    // Pending until armed, however long after; arming again changes nothing.
    t.to(10.0, 1000.0, 100.0);
    assert!(t.is_running(5000.0) && t.value(5000.0) == 0.0);
    t.arm(2000.0);
    t.arm(2050.0);
    assert_eq!((t.value(2000.0), t.value(2100.0), t.target()), (0.0, 10.0, 10.0));
    let mid = t.value(2050.0);
    assert!(mid > 5.0 && mid < 10.0 && !t.is_running(2100.0));
    // Retargeting starts from where it is; the same target changes nothing.
    t.to(20.0, 2050.0, 100.0);
    assert_eq!(t.value(2050.0), mid);
    t.to(20.0, 2060.0, 100.0);
    assert_eq!(t.value(2070.0), mid);
    // Chasing keeps a running course; at rest it jumps.
    t.arm(2050.0);
    t.chase(30.0, 2075.0);
    assert!(t.value(2075.0) < 30.0 && t.value(2150.0) == 30.0);
    t.chase(40.0, 2150.0);
    assert_eq!(t.value(2150.0), 40.0);
    // A rect moves linearly; fading out lags (t squared), fading in does not.
    let (a, b) = (Vis::at(RectF::new(0.0, 0.0, 10.0, 20.0)), RectF::new(10.0, 10.0, 30.0, 40.0));
    let out = a.lerp(Vis { a: 0.0, s: 0.5, ..Vis::at(b) }, 0.5);
    assert_eq!((out.rect, out.s, out.a), (RectF::new(5.0, 5.0, 20.0, 30.0), 0.75, 0.75));
    assert_eq!(Vis { a: 0.0, ..a }.lerp(a, 0.5).a, 0.5);
    // Replays scale, move and fade every kind.
    let mut src = DrawList::new();
    let r = RectF::new(100.0, 100.0, 40.0, 20.0);
    src.push_clip(RectF::new(90.0, 90.0, 100.0, 100.0));
    src.fill(r, 4.0, Rgba(1, 2, 3, 200));
    src.border(r, 4.0, 2.0, Rgba(1, 2, 3, 100));
    src.shadow(r, 4.0, 10.0, Rgba(0, 0, 0, 255));
    src.icon(r, gfx::Icon::Square, 2.0, Rgba(9, 9, 9, 255));
    src.glyph(r, RectF::new(1.0, 2.0, 3.0, 4.0), Rgba(5, 5, 5, 255));
    src.gradient(r, 4.0, Rgba(1, 1, 1, 255), Rgba(2, 2, 2, 51), -1.5);
    src.glow(r, Rgba(7, 7, 7, 100));
    src.grain(r, 20, -3.0);
    let vis = Vis { rect: RectF::new(0.0, 0.0, 200.0, 200.0), s: 0.5, dx: 10.0, dy: -10.0, a: 0.5 };
    let mut dst = DrawList::new();
    replay(&mut dst, &src, vis);
    let out = dst.instances();
    assert_eq!(out.len(), 8);
    for (o, i) in out.iter().zip(src.instances()) {
        assert_eq!((o.kind, o.uv, o.rect), (i.kind, i.uv, [110.0, 90.0, 20.0, 10.0]));
        assert_eq!(o.clip, [105.0, 85.0, 50.0, 50.0]);
        assert_eq!((o.color.3, o.color2.3), (i.color.3.div_ceil(2), i.color2.3.div_ceil(2)));
    }
    let params: Vec<_> = out.iter().map(|o| (o.radius, o.p0, o.p1)).collect();
    let icon = gfx::Icon::Square as u8 as f32;
    let want = [(2.0, 0.0, 0.0), (2.0, 1.0, 0.0), (2.0, 5.0, 0.0), (0.0, icon, 1.0)];
    assert_eq!(params[..4], want);
    assert_eq!((params[5].1, params[7].1), (-1.5, -3.0));
    assert_eq!(out[7].kind, Kind::Grain as u8 as f32);
    assert!(Vis::at(r).is_plain() && !vis.is_plain());
    // Faded to nothing, nothing is drawn.
    replay(&mut dst, &src, Vis { a: 0.0, ..vis });
    assert_eq!(dst.len(), 8);
    // Themes crossfade and switch by name.
    let [mid, dawn, mono] = &THEMES;
    assert_eq!(blend(mid, dawn, 0.0).base, mid.base);
    assert_eq!((blend(mid, dawn, 1.0), blend(dawn, mid, 1.0)), (*dawn, *mid));
    let half = blend(mid, mono, 0.5);
    assert_eq!((half.name, half.grain), ("Mono", 7));
    // Mono has no lights: Midnight's stay in place and fade out.
    let (g, h) = (mid.glows[0], half.glows[0]);
    assert_eq!((h.cx, h.rx, h.color), (g.cx, g.rx, g.color.with_alpha(45)));
    assert_eq!(blend(mono, mid, 0.5).glows[1].cy, mid.glows[1].cy);
    assert_eq!(blend(mono, mono, 0.3), *mono);
    let mut t = Themes::new("DAWN");
    assert_eq!((t.current().name, t.next()), ("Dawn", "Mono"));
    assert!(!t.set("nope", 0.0, 200.0) && !t.set("dawn", 0.0, 200.0) && !t.is_running(0.0));
    assert!(t.set("mono", 100.0, 200.0));
    assert_eq!((t.current().name, t.next(), t.at(100.0).base), ("Mono", "Midnight", dawn.base));
    assert!(t.is_running(5000.0));
    t.arm(1000.0);
    assert_eq!((t.at(1000.0).grain, t.at(1100.0).grain), (6, 5));
    assert_eq!((t.at(1200.0), t.is_running(1200.0)), (*mono, false));
    assert_eq!(Themes::new("").current().name, "Mono");
}

#[test]
fn frames_have_controls_edges_and_snap_zones_and_the_layout_places_the_dock_and_launcher() {
    use frame::*;
    let r = RectF::new(100.0, 50.0, 400.0, 300.0);
    let [min, max, close] = controls(r).unwrap();
    assert_eq!((min.x, max.x, close.x, close.y), (436.0, 456.0, 476.0, 64.0));
    assert_eq!((controls(RectF { w: 91.0, ..r }), controls(RectF { h: 39.0, ..r })), (None, None));
    let at = |x, y| edge(r, x, y);
    let sides = [at(98.0, 200.0), at(502.0, 200.0), at(300.0, 52.0), at(300.0, 349.0)];
    assert_eq!(sides, [Some((-1, 0)), Some((1, 0)), Some((0, -1)), Some((0, 1))]);
    let corners = [at(101.0, 60.0), at(110.0, 51.0), at(499.0, 340.0), at(102.0, 349.0)];
    assert_eq!(corners, [Some((-1, -1)), Some((-1, -1)), Some((1, 1)), Some((-1, 1))]);
    assert_eq!([at(300.0, 200.0), at(96.0, 200.0), at(f32::NAN, 0.0)], [None; 3]);
    let cursors = [(-1, 0), (0, 1), (1, 1), (1, -1)].map(edge_cursor);
    let want = [Cursor::EwResize, Cursor::NsResize, Cursor::NwseResize, Cursor::NeswResize];
    assert_eq!(cursors, want);
    let w = Rect::new(100, 100, 400, 300);
    assert_eq!(resized(w, (-1, 0), (-50.0, 9.0), 32), Rect::new(50, 100, 450, 300));
    assert_eq!(resized(w, (-1, -1), (300.0, -300.0), 32), Rect::new(180, 32, 320, 368));
    assert_eq!(resized(w, (1, 1), (-500.0, f32::NAN), 32), Rect::new(100, 100, 320, 300));
    let z = |x, y| zone((1280.0, 800.0), x, y);
    let snap = |s| Some(Zone::Snap(s));
    let edges = [z(3.0, 400.0), z(1276.0, 400.0), z(640.0, 6.0), z(640.0, 7.0)];
    assert_eq!(edges, [snap(wm::Snap::Left), snap(wm::Snap::Right), Some(Zone::Max), None]);
    let corners = [z(20.0, 20.0), z(1260.0, 790.0)];
    assert_eq!(corners, [snap(wm::Snap::TopLeft), snap(wm::Snap::BottomRight)]);
    assert_eq!(Zone::Max.rect(Rect::new(0, 32, 1280, 684)), Rect::new(0, 32, 1280, 684));
    // The layout places the dock and the launcher.
    use layout::*;
    let screen = (1280.0, 800.0);
    assert_eq!(dock(4, screen), RectF::new(529.0, 728.0, 222.0, 60.0));
    assert_eq!(dock_tile(4, screen, 1), RectF::new(591.0, 736.0, 44.0, 44.0));
    assert_eq!(dock(0, screen).w, 16.0);
    let at = |x| dock_at(4, screen, x, 750.0);
    let found = [at(530.0), at(560.0), at(585.0), at(586.0), at(749.0), at(800.0)];
    assert_eq!(found, [Some(None), Some(Some(0)), Some(Some(0)), Some(Some(1)), Some(None), None]);
    assert_eq!(dock_at(0, screen, 640.0, 750.0), None);
    let p = Panel::new(screen, 5);
    assert_eq!((p.rect, p.cols()), (RectF::new(340.0, 180.0, 600.0, 440.0), 4));
    assert_eq!(p.field(), RectF::new(356.0, 196.0, 568.0, 48.0));
    assert_eq!((p.tile(0), p.tile(4).y), (RectF::new(387.0, 260.0, 80.0, 84.0), 352.0));
    assert_eq!((p.list_top(), p.row(1).y, p.fit()), (452.0, 492.0, 3));
    let at = |x, y, first| p.at(x, y, first, 9);
    assert_eq!((at(400.0, 300.0, 0), at(400.0, 500.0, 2)), (Some(Some(0)), Some(Some(8))));
    assert_eq!((at(345.0, 300.0, 0), at(10.0, 10.0, 0)), (Some(None), None));
    let tiny = Panel::new((40.0, 30.0), 3);
    assert_eq!((tiny.rect.w, tiny.rect.h, tiny.cols(), tiny.fit()), (0.0, 0.0, 1, 0));
    assert_eq!(Panel::new((300.0, 800.0), 0).cols(), 2);
}

/// A search over apps, then files, by label.
fn search(apps: &[&str], files: &[&str]) -> Search {
    let entry = |(label, file): (&&str, bool)| {
        let (name, place) = (label.to_lowercase(), file.then(|| "/apps".to_string()));
        Entry { name, label: label.to_string(), icon: AppIcon::default(), place }
    };
    let all = apps.iter().map(|l| (l, false)).chain(files.iter().map(|l| (l, true)));
    Search::new(all.map(entry).collect())
}

#[test]
fn searches_rank_then_move_through_the_grid_then_the_list() {
    let ranks = [rank("", "Terminal"), rank("TRM", "terminal"), rank("al", "Terminal")];
    assert_eq!(ranks, [Some(0), Some(0), Some(6)]);
    assert_eq!((rank("x", "Terminal"), rank("ÉC", "éclair")), (None, Some(0)));
    // Best first, ties in order.
    let mut s = search(&["Studio", "Terminal", "Settings", "About"], &[]);
    assert_eq!(s.tiles, [0, 1, 2, 3]);
    s.type_text("t");
    assert_eq!(s.tiles, [1, 0, 2, 3]);
    s.key(Key::Backspace, 4);
    s.type_text("s");
    assert_eq!(s.tiles, [0, 2]);
    let apps = ["Terminal", "Studio", "Settings", "Welcome", "About"];
    let mut s = search(&apps, &["Clicker", "Counter", "Greeter", "Notes"]);
    assert_eq!((s.count(), s.get(5).unwrap().label.as_str(), s.get(9)), (9, "Clicker", None));
    let (d, u, want) = (Key::Down, Key::Up, [4, 5, 6, 7, 8, 8, 7, 6, 5, 4, 0, 0, 1, 4, 3]);
    let keys = [d, d, d, d, d, d, u, u, u, u, u, u, Key::Right, d, Key::Left];
    for (i, k) in keys.into_iter().enumerate() {
        assert!(s.key(k, 4) && s.sel == want[i], "step {i}: {k:?} to {}", s.sel);
    }
    // The selected row stays on screen.
    s.fit = 2;
    (0..5).for_each(|_| _ = s.key(Key::Down, 4));
    assert_eq!((s.sel, s.first), (8, 2));
    s.scroll(-1e9, 40.0);
    assert_eq!(s.first, 0);
    s.scroll(80.0, 40.0);
    assert_eq!(s.first, 2);
    s.scroll(f32::NAN, 40.0);
    assert!(!s.key(Key::Enter, 4) && s.first == 2);
    // Typing filters and selects the best; Backspace widens again.
    s.type_text("ter\u{7}");
    assert_eq!((s.query.as_str(), &s.tiles[..], &s.rows[..]), ("ter", &[0][..], &[6, 7][..]));
    s.key(Key::Down, 4);
    s.type_text("");
    assert_eq!(s.sel, 1);
    s.key(Key::Backspace, 4);
    assert_eq!((s.query.as_str(), s.count(), s.sel), ("te", 4, 0));
    let mut none = Search::new(Vec::new());
    let keys = [Key::Up, Key::Down, Key::Left, Key::Right, Key::Backspace];
    assert!(keys.into_iter().all(|k| none.key(k, 4) && none.sel == 0));
}

#[test]
fn glyphs_draw_on_whole_pixels_and_time_reads_as_the_bar_shows_it() {
    use paint::*;
    let (text, mut list) = (TextSystem::new(SANS.to_vec()).unwrap(), DrawList::new());
    let c = RectF::new(100.0, 14.0, 12.0, 12.0);
    let [fill, ink] = [Rgba(1, 1, 1, 255), Rgba(2, 2, 2, 255)];
    let glyphs = [(0, false), (1, false), (1, true), (2, false)];
    glyphs.into_iter().for_each(|(i, max)| control_glyph(&mut list, &text, c, i, max, [fill, ink]));
    let rects: Vec<[f32; 4]> = list.instances().iter().map(|i| i.rect).collect();
    let want = [103., 20., 6., 1., 103., 17., 6., 6., 105., 16., 5., 5., 102., 19., 5., 5.];
    assert_eq!(rects[..4].concat(), want);
    assert_eq!((list.len(), list.instances()[3].color, list.instances()[5].kind), (6, fill, 3.0));
    list.clear();
    mark(&mut list, (19.0, 16.0), 14.0, ink);
    sliders(&mut list, (50.0, 16.0), 14.0, ink);
    contrast(&mut list, (80.0, 16.0), 14.0, ink);
    magnifier(&mut list, (20.0, 20.0), ink);
    assert_eq!((list.len(), list.instances()[2].rect), (2 + 4 + 2 + 8, [43.0, 13.0, 14.0, 1.0]));
    let fades = (faded(ink, 0.5), faded(ink, 9.0), faded(ink, f32::NAN));
    assert_eq!(fades, (Rgba(2, 2, 2, 128), ink, Rgba(2, 2, 2, 0)));
    assert_eq!((px(&text, 1.0), px(&text, 0.2)), (1.0, 1.0));
    // Local time reads as the bar shows it.
    let t = LocalTime { year: 2026, month: 10, day: 1, weekday: 3, hour: 9, minute: 5 };
    assert_eq!((t.date(), t.clock()), ("Wed 1 Oct".to_string(), "09:05".to_string()));
    let odd = LocalTime { day: 31, month: 0, weekday: 9, hour: 23, minute: 59, ..t };
    assert_eq!((odd.date(), odd.clock()), ("Tue 31 Jan".to_string(), "23:59".to_string()));
    assert_eq!(LocalTime::default().date(), "Sun 0 Jan");
}
