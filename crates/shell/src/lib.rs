//! The compusophyOS desktop: floating windows around a [`host::Host`], which runs one
//! [`ui::App`] per window, and the home screen around them (the `home` crate): a top bar, the
//! dock of favorites, the everything bar under it with the launcher's panel above it, desktop
//! icons, context menus and touch. No browser: [`Shell`] turns [`Input`] into wm commands and
//! app events, draws into a [`gfx::DrawList`] and hands back what only the platform can do in a
//! [`Response`].
//!
//! Logical pixels, origin top-left; non-finite sizes and positions count as 0. Windows live in the
//! work area, the screen below the [`BAR_H`] top bar less [`DOCK_CLEAR`] at the bottom, which every
//! resize hands the wm; the windows' rects follow at once. On a first visit ([`Prefs::seen`]
//! unset) Welcome opens as soon as the work area is not empty, at [`Shell::new`] or at the first
//! [`Input::Resize`] that makes it so, and the shell sets the `seen` preference.
//!
//! The pointer: button 0 presses, button 2 (or a finger held still for 500 ms) opens a context
//! menu; a finger that travels over a window's content (or the launcher) scrolls it as the wheel
//! does, and flings it on when it lifts moving. Bindings, pointer rules and motion are those of
//! `DESIGN.md`. While anything moves (or a held finger waits to long-press), [`Shell::draw`] asks
//! for the next frame; otherwise none.

#![forbid(unsafe_code)]

mod bar;
mod chrome;
mod desktop;
mod dock;
mod icons;
mod keys;
mod launcher;
mod menus;
mod motion;
mod touch;

pub use host::{Cursor, Effect, Input, KernelIn, LocalTime, Registry, Response};
pub use ui::{Key, Mods};

use std::mem;

use desktop::{Grab, Target};
use gfx::{DrawList, RectF, Rgba};
use home::dock::Shelf;
use host::Host;
use host::search::Entry;
use ui::{AppEvent, TextSystem, WidgetId};
use vfs::Vfs;
use wm::{Cmd, Rect, WinId, Wm};

/// The top bar's height, and what the work area leaves free at the bottom for the dock and the
/// everything bar.
pub const BAR_H: f32 = 44.0;
pub const DOCK_CLEAR: f32 = home::CLEAR;

type Widget = (WinId, WidgetId);
/// The screen before a keyboard shortened it; each free window then, its rect then, and where
/// the last squeeze left it.
type Kept = ((f32, f32), Vec<(WinId, Rect, Rect)>);

/// What the page keeps for the shell between visits: the theme's name (else the default, Mono),
/// the dock's favorites as stored (the `dock` preference; `None` for the default) and whether
/// Welcome was shown on a first visit (the `seen` preference).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub theme: String,
    pub dock: Option<String>,
    pub seen: bool,
}

/// Everything whose change means a new frame, but the clock.
#[derive(PartialEq)]
struct Visuals {
    wm: u64,
    buttons: [Option<Target>; 2],
    widgets: [Option<Widget>; 2],
    wins: usize,
    launcher: (bool, bool, usize, usize, usize),
    zone: Option<host::frame::Zone>,
    menu: Option<(RectF, Option<usize>)>,
    home: (usize, usize),
}

/// The desktop: the host (the wm, text, files, apps and theme) and the home screen, chrome,
/// motion and bindings around it.
pub struct Shell {
    host: Host,
    /// Effects from outside a [`Response`], for [`Shell::take_effects`].
    pending: Vec<Effect>,
    size: (f32, f32),
    /// The pointer, the targets and app widgets under it and pressed, a held
    /// window, and the last titlebar press (for double clicks).
    pointer: Option<(f32, f32)>,
    hover: Option<Target>,
    armed: Option<Target>,
    app_hover: Option<Widget>,
    app_press: Option<Widget>,
    /// A finger's press into content, delivered when it lifts as a tap.
    down: Option<(WinId, AppEvent)>,
    grab: Option<Grab>,
    last_title: Option<(WinId, f64)>,
    /// The focus and its want of text input, and the cursor, as last told.
    ime: Option<(Option<WinId>, bool)>,
    cursor: Cursor,
    clock: Option<LocalTime>,
    /// The dock: the favorites, the apps it shows (favorites first), where they sit.
    favs: Vec<String>,
    dock: Vec<dock::Item>,
    shelf: Shelf,
    /// The desktop's icons, and the [`Vfs::generation`] they were listed at.
    icons: Vec<Entry>,
    listed: Option<u64>,
    launcher: launcher::Launcher,
    /// The open context menu.
    menu: Option<menus::Open>,
    /// A finger down (and what it scrolls), and content flinging on.
    touch: Option<(home::touch::Touch, touch::Scroll)>,
    fling: Option<(touch::Scroll, home::touch::Fling)>,
    motion: motion::Motion,
    /// Whether this input's wm changes follow the pointer, not animate.
    instant: bool,
    /// Whether Welcome still waits for a work area (on a first visit).
    startup: bool,
    kept: Option<Kept>,
    /// A layer drawn, then replayed scaled and faded.
    scratch: DrawList,
}

impl Shell {
    /// A desktop of `w` x `h` with the stored `prefs`.
    #[rustfmt::skip]
    pub fn new(w: f32, h: f32, text: TextSystem, vfs: Vfs, reg: Registry, prefs: Prefs) -> Shell {
        let size = (coord(w).max(0.0), coord(h).max(0.0));
        let host = Host::new(Wm::new(work_area(size)), text, vfs, reg, &prefs.theme);
        let favs = home::dock::favorites(prefs.dock.as_deref());
        let mut shell = Shell { host, pending: Vec::new(), size, pointer: None, hover: None,
            armed: None, app_hover: None, app_press: None, down: None, grab: None, last_title: None,
            ime: None, cursor: Cursor::Default, clock: None, favs, dock: Vec::new(),
            shelf: Shelf::default(), icons: Vec::new(), listed: None, launcher: Default::default(),
            menu: None, touch: None, fling: None, motion: Default::default(), instant: false,
            startup: !prefs.seen, kept: None, scratch: DrawList::new() };
        let mut out = Response::default();
        shell.start(&mut out);
        shell.settle(&mut out);
        shell.pending = out.effects;
        shell
    }

    pub fn wm(&self) -> &Wm {
        self.host.wm()
    }

    /// The apps of the open windows, oldest first.
    pub fn open_apps(&self) -> Vec<&str> {
        let h = &self.host;
        h.wins.iter().filter(|w| h.live(w.id)).map(|w| w.name.as_str()).collect()
    }

    pub fn vfs(&self) -> &Vfs {
        &self.host.vfs
    }

    pub fn text_mut(&mut self) -> &mut TextSystem {
        &mut self.host.text
    }

    pub fn set_dpr(&mut self, dpr: f32) {
        self.host.set_dpr(dpr);
    }

    /// Sets the page clock (monotonic ms, if finite); call it before each input and frame.
    pub fn set_now(&mut self, now_ms: f64) {
        if now_ms.is_finite() {
            self.host.now_ms = now_ms;
        }
    }

    pub fn theme_name(&self) -> &'static str {
        self.host.theme.current().name
    }

    /// Crossfades to the theme named `name` (any ASCII case); whether it is a new one.
    pub fn set_theme(&mut self, name: &str) -> bool {
        self.host.theme.set(name, self.host.now_ms, host::THEME_MS)
    }

    /// The frame's clear color.
    pub fn clear_color(&self) -> Rgba {
        self.host.theme.current().base
    }

    /// The AI settings apps see (from the page's storage).
    pub fn set_ai(&mut self, ai: ui::AiStatus) {
        self.host.ai = ai;
    }

    /// The kernel, for os to set up; its effects leave by [`Shell::take_effects`].
    pub fn kernel_mut(&mut self) -> &mut ui::kernel::Kernel {
        &mut self.host.kernel
    }

    /// The effects that arose outside a [`Response`] (while drawing, in
    /// [`Shell::new`], in the kernel); call it after every frame and event.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        let mut out = Response { effects: mem::take(&mut self.pending), ..Response::default() };
        self.host.pump(&mut out);
        out.effects
    }

    /// The result of an [`Effect::Fetch`].
    pub fn fetched(&mut self, id: u32, got: Result<Vec<u8>, String>) -> Response {
        let (mut out, before) = (Response::default(), self.visuals());
        self.host.fetched(id, got, &mut out);
        self.finish(before, &mut out);
        out
    }

    /// What the platform heard for the kernel: workers, the timer, hiding.
    pub fn kernel(&mut self, ev: KernelIn) -> Response {
        let (mut out, before) = (Response::default(), self.visuals());
        self.host.kernel_in(ev, &mut out);
        self.finish(before, &mut out);
        out
    }

    pub fn input(&mut self, input: Input) -> Response {
        let (mut out, before) = (Response::default(), self.visuals());
        self.instant = false;
        self.pointer = match input {
            Input::PointerMove { x, y }
            | Input::PointerDown { x, y, .. }
            | Input::PointerUp { x, y, .. } => {
                out.consumed = true;
                Some((coord(x), coord(y)))
            }
            Input::PointerLeave => None,
            _ => self.pointer,
        };
        self.hold(&mut out);
        match input {
            Input::Key { key, mods } => self.key(key, mods, &mut out),
            Input::Text(s) if self.launcher.focus => {
                self.typed(&s);
                out.consumed = true;
            }
            Input::Text(s) => {
                if let Some(win) = self.host.focused_app().filter(|_| !s.is_empty()) {
                    self.host.deliver(win, AppEvent::Text(s), &mut out);
                    out.consumed = true;
                }
            }
            Input::PointerMove { .. } => {
                self.finger(&mut out);
                self.drag_to();
                self.point_menu();
            }
            Input::PointerDown { button: 0, touch, .. } => self.press(touch, &mut out),
            Input::PointerDown { button: 2, touch, .. } => {
                self.secondary(self.pointer.unwrap_or_default(), touch, &mut out)
            }
            Input::PointerUp { button, .. } => self.release(button == 0, &mut out),
            Input::PointerDown { .. } => {}
            Input::PointerLeave => self.touch = None,
            Input::Wheel { x, y, dy } => self.wheel(coord(x), coord(y), dy, &mut out),
            Input::Resize { w, h } => {
                self.resize((coord(w).max(0.0), coord(h).max(0.0)));
                self.start(&mut out);
                (self.instant, out.redraw) = (true, true);
            }
            Input::Tick { time } => {
                out.redraw |= self.clock.replace(time) != Some(time);
                self.host.tick(&mut out);
            }
        }
        self.finish(before, &mut out);
        out
    }

    /// A new screen size. A keyboard that shortens the page (the width kept, while typing)
    /// squeezes the free windows only until the page is that tall again: they come back then,
    /// but for those the person moved or resized meanwhile, which stay where they put them.
    fn resize(&mut self, size: (f32, f32)) {
        let free = |p: &wm::Placement| p.state == wm::State::Normal && p.snap.is_none();
        let typing = matches!(self.ime, Some((_, true)));
        if self.kept.is_none() && typing && size.0 == self.size.0 && size.1 < self.size.1 {
            let wins = self.host.wm().layout().into_iter().filter(free).map(|p| (p.win, p.rect));
            self.kept = Some((self.size, wins.map(|(w, r)| (w, r, r)).collect()));
        }
        // Each kept window must still be free and where the last squeeze left it.
        let layout = self.host.wm().layout();
        let left = |&(win, _, at): &(WinId, Rect, Rect)| {
            layout.iter().any(|p| p.win == win && free(p) && p.rect == at)
        };
        if let Some((_, wins)) = &mut self.kept {
            wins.retain(left);
        }
        self.size = size;
        self.host.apply(Cmd::SetArea(work_area(size)));
        let Some((full, mut wins)) = self.kept.take() else { return };
        if size.0 != full.0 || size.1 < full.1 {
            // Still short, it waits (noting where each window is now); a new width (a phone
            // turned) forgets them.
            for w in &mut wins {
                w.2 = self.placement(w.0).map_or(w.2, |p| p.rect);
            }
            self.kept = (size.0 == full.0).then_some((full, wins));
            return;
        }
        for (win, rect, _) in wins {
            self.host.apply(Cmd::Resize { win, rect });
        }
    }

    /// Opens Welcome on a first visit once there is a work area, and remembers it was shown.
    fn start(&mut self, out: &mut Response) {
        let a = self.host.wm().area();
        if self.startup && a.w > 0 && a.h > 0 {
            self.startup = false;
            self.host.open("welcome", None, out);
            out.effects.push(Effect::Pref { key: "seen".into(), value: "1".into() });
        }
    }

    /// Draws the desktop into `list` (again if the glyph atlas was reset, or
    /// an app told its new size asks to redraw); whether an animation runs.
    pub fn draw(&mut self, list: &mut DrawList) -> bool {
        let mut out = Response::default();
        self.instant = false;
        self.hold(&mut out);
        self.flinging(&mut out);
        self.settle(&mut out);
        let now = self.host.now_ms;
        self.arm(now);
        let theme = self.host.theme.at(now);
        for _ in 0..2 {
            for _ in 0..2 {
                list.clear();
                theme.draw_backdrop(list, RectF::new(0.0, 0.0, self.size.0, self.size.1));
                self.draw_icons(list, &theme);
                self.draw_windows(list, &theme, now);
                self.draw_dock(list, &theme);
                self.draw_bar(list, &theme);
                self.draw_launcher(list, &theme, now);
                self.draw_field(list, &theme);
                self.draw_menu(list, &theme);
                self.draw_tooltip(list, &theme, now);
                if !self.host.text.take_atlas_reset() {
                    break;
                }
            }
            out.redraw = false;
            self.host.redrawn(&mut out);
            if !out.redraw {
                break;
            }
            self.settle(&mut out);
        }
        self.sync();
        self.pending.append(&mut out.effects);
        self.animating()
    }

    /// Brings the apps up to date with the wm, then the shell with the apps and the files.
    fn settle(&mut self, out: &mut Response) {
        self.host.settle(out);
        if mem::take(&mut self.host.launcher) && !self.launcher.open {
            self.show_launcher();
        }
        let h = &self.host;
        (self.app_hover, self.app_press) =
            (self.app_hover.filter(|w| h.live(w.0)), self.app_press.filter(|w| h.live(w.0)));
        self.dock = self.host.dock_apps(&self.favs);
        let favs = self.dock.iter().take_while(|d| self.favs.contains(&d.0)).count();
        let top = home::field::rect(self.size).y - home::dock::GAP - home::dock::H;
        self.shelf = Shelf::new(favs, self.dock.len() - favs, self.size.0, top);
        let listed = self.host.vfs.generation();
        if self.listed.replace(listed) != Some(listed) {
            self.icons = self.host.desktop();
        }
        self.sync();
    }

    fn visuals(&self) -> Visuals {
        let button = |t: Option<Target>| t.filter(|t| t.is_button());
        let (l, s) = (&self.launcher, &self.launcher.search);
        Visuals {
            wm: self.host.wm().state_hash(),
            buttons: [button(self.hover), self.armed],
            widgets: [self.app_hover, self.app_press],
            wins: self.host.wins.len(),
            launcher: (l.open, l.focus, s.query.len(), s.sel, s.first),
            zone: if let Some(Grab::Move { zone, .. }) = self.grab { zone } else { None },
            menu: self.menu.as_ref().map(|m| (m.0.rect, m.0.sel)),
            home: (self.dock.len() + self.favs.len(), self.icons.len()),
        }
    }

    /// Ends every event: settles, finds what is under the pointer, and fills in the response.
    fn finish(&mut self, before: Visuals, out: &mut Response) {
        self.settle(out);
        let under = self.pointer.and_then(|(x, y)| Some((self.hit(x, y)?, x, y)));
        self.hover = under.map(|u| u.0).filter(|_| self.grab.is_none());
        let widget = under.and_then(|(t, x, y)| match t {
            Target::Body(win) if self.grab.is_none() => Some((win, self.widget_at(win, x, y)?)),
            _ => None,
        });
        self.app_hover = widget.map(|(win, hit)| (win, hit.id));
        let text = widget.is_some_and(|w| w.1.sense == ui::Sense::Text);
        self.sync();
        let cursor = self.cursor_for(text);
        if cursor != mem::replace(&mut self.cursor, cursor) {
            out.cursor = Some(cursor);
        }
        out.animating = self.animating();
        out.redraw |= out.animating || self.visuals() != before;
        self.pending.append(&mut out.effects);
        out.effects = mem::take(&mut self.pending);
        let focus = self.host.focused_app();
        let app = focus.and_then(|w| self.host.win(w));
        let wants = self.launcher.focus || app.is_some_and(|w| w.app.wants_text_input());
        if self.ime != Some((focus, wants)) {
            self.ime = Some((focus, wants));
            out.text_input = Some(wants);
        }
    }
}

/// A finite coordinate within the wm's range (not `clamp`: its panic path
/// links in float formatting).
fn coord(v: f32) -> f32 {
    let max = wm::MAX_COORD as f32;
    let v = if v.is_finite() { v } else { 0.0 };
    v.max(-max).min(max)
}

fn work_area((w, h): (f32, f32)) -> Rect {
    let h = (h - BAR_H - DOCK_CLEAR).max(0.0);
    Rect::new(0, BAR_H as i32, w.round() as i32, h.round() as i32)
}

#[cfg(test)]
mod tests;
