//! The compusophyOS desktop: floating windows around a [`host::Host`], which runs one
//! [`ui::App`] per window, and the home screen around them (the `home` crate): a top bar, every
//! app as an icon behind the windows, the bottom row (the person's dock at its left, the
//! Assistant alone at the bottom-right corner), context menus and touch. No browser: [`Shell`]
//! turns [`Input`] into wm commands and app events, draws into a [`gfx::DrawList`] and hands back
//! what only the platform can do in a [`Response`].
//!
//! Logical pixels, origin top-left; non-finite sizes and positions count as 0. Windows and icons
//! live in the work area, the screen below the [`BAR_H`] top bar less the bottom row
//! ([`DOCK_CLEAR`]), which every resize hands the wm; the windows' rects follow at once. On a
//! first visit ([`Prefs::seen`] unset) Welcome opens as soon as the work area is not empty, at
//! [`Shell::new`] or at the first [`Input::Resize`] that makes it so, and the shell sets the
//! `seen` preference.
//!
//! The pointer: button 0 presses, button 2 (or a finger held still for 500 ms) opens a context
//! menu (where there is none, the finger's press goes on: lifted there, it taps, as on an app's
//! widget). Frames judge a long press, never a lift: the first past 500 ms on a page keeping up,
//! one 100 ms after it on a page that stalled, so a quick tap's lift that a busy page hears late,
//! after frames saw the time pass, is a tap. A finger that travels over a window's content
//! scrolls it as the wheel does, and flings it on when it lifts moving. Icons move: a mouse drags
//! one past 4 px; a finger held on one for 500 ms picks it up, then moving it 8 px drags it, and
//! lifting it unmoved opens its menu instead; a tile kept on the dock moves so along it. A mouse
//! dragged on the bare desktop draws a box that selects the icons it touches; dragging a
//! selected icon carries them all. Bindings, pointer rules and motion are those of `DESIGN.md`.
//! While anything moves (or a held finger waits to long-press), [`Shell::draw`] asks for the
//! next frame; otherwise none, but for the living grain's and an app's timer's
//! ([`Shell::frame_in`]). Above the windows lies the overlay, the Assistant that uses the
//! desktop as a person does.

#![forbid(unsafe_code)]

mod bar;
mod chrome;
mod desktop;
mod dock;
mod grid;
mod keys;
mod menus;
mod motion;
mod overlay;
mod touch;

pub use host::{Cursor, Effect, Input, KernelIn, LocalTime, Registry, Response};
pub use ui::{Key, Mods};

use std::mem;

use desktop::Target;
use gfx::{DrawList, RectF, Rgba};
use host::{Host, rectf};
use ui::{AppEvent, TextSystem, WidgetId};
use vfs::Vfs;
use wm::{Rect, WinId, Wm};

/// The top bar's height, and what the work area leaves free at the bottom: the bottom row.
pub const BAR_H: f32 = home::bar::H;
pub const DOCK_CLEAR: f32 = home::CLEAR;
/// How long one pattern of the living grain shows: 8 a second.
pub const GRAIN_MS: f64 = 125.0;

type Widget = (WinId, WidgetId);

/// What the page keeps for the shell between visits: the theme's name (else the default, Mono),
/// the dock's favorites and the home screen's order as stored (the `dock` and `home`
/// preferences; `None` for none), whether Welcome was shown on a first visit (the `seen`
/// preference) and whether the grain is still (the `grain` preference `"off"`); and whether the
/// page is cross-origin isolated, which programs need: the kernel knows before Welcome, a
/// program, opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub theme: String,
    pub dock: Option<String>,
    pub home: Option<String>,
    pub seen: bool,
    pub grain_off: bool,
    pub isolated: bool,
}

/// Everything whose change means a new frame, but the clock.
#[derive(PartialEq)]
struct Visuals {
    wm: u64,
    buttons: [Option<Target>; 2],
    widgets: [Option<Widget>; 2],
    wins: usize,
    zone: Option<host::frame::Zone>,
    menu: Option<(RectF, Option<usize>)>,
    home: (usize, usize, usize),
    /// Carried icons (or a dock tile): lifted, their slot, the pointer; the selection box's far
    /// corner.
    carry: Option<(bool, usize, (f32, f32))>,
    lasso: Option<(f32, f32)>,
    /// The overlay, and whether it works.
    agent: (overlay::Overlay, bool),
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
    grab: Option<host::grab::Grab>,
    last_title: Option<(WinId, f64)>,
    /// The focus and its want of text input, and the cursor, as last told; the focus as last
    /// settled.
    ime: Option<(Option<WinId>, bool)>,
    focus: Option<WinId>,
    cursor: Cursor,
    clock: Option<LocalTime>,
    /// The dock (its favorites, the row's layout, a tile carried) and the apps it shows, as
    /// they show (favorites first).
    dock: home::dock::Dock,
    tiles: Vec<dock::Item>,
    /// The home screen's icons.
    grid: home::grid::Grid,
    /// Whether the page asks for reduced motion (the grain stays still).
    reduced: bool,
    /// The open context menu.
    menu: Option<menus::Open>,
    /// A finger down (and what it scrolls), and content flinging on; whether the last press was
    /// a finger's.
    touch: Option<(home::touch::Touch, touch::Scroll)>,
    fling: Option<(touch::Scroll, home::touch::Fling)>,
    finger: bool,
    motion: motion::Motion,
    /// Whether this input's wm changes follow the pointer, not animate.
    instant: bool,
    /// Whether Welcome still waits for a work area (on a first visit).
    startup: bool,
    /// A layer drawn, then replayed scaled and faded.
    scratch: DrawList,
    overlay: overlay::Overlay,
}

impl Shell {
    /// A desktop of `w` x `h` with the stored `prefs`.
    #[rustfmt::skip]
    pub fn new(w: f32, h: f32, text: TextSystem, vfs: Vfs, reg: Registry, prefs: Prefs) -> Shell {
        let size = (coord(w).max(0.0), coord(h).max(0.0));
        let host = Host::new(Wm::new(work_area(size)), text, vfs, reg, &prefs.theme);
        let dock = home::dock::Dock::new(prefs.dock.as_deref());
        let grid = home::grid::Grid::new(prefs.home.as_deref());
        let mut shell = Shell { host, pending: Vec::new(), size, pointer: None, hover: None,
            armed: None, app_hover: None, app_press: None, down: None, grab: None, last_title: None,
            ime: None, focus: None, cursor: Cursor::Default, clock: None, dock, tiles: Vec::new(),
            grid, reduced: false, menu: None, touch: None, fling: None,
            finger: false, motion: Default::default(), instant: false, startup: !prefs.seen,
            scratch: DrawList::new(), overlay: Default::default() };
        shell.place();
        shell.host.grain = !prefs.grain_off;
        shell.host.kernel.set_isolated(prefs.isolated);
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

    /// Whether the page asks for reduced motion, which keeps the grain still.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced = reduced;
    }

    /// The grain's pattern now: a new one each [`GRAIN_MS`] while it lives (the `grain`
    /// preference on, motion not reduced, a theme with grain), else always the first.
    fn grain_seed(&self) -> u32 {
        match self.grain_lives() {
            true => (self.host.now_ms / GRAIN_MS) as u32 % 4096 + 1,
            false => 0,
        }
    }

    fn grain_lives(&self) -> bool {
        self.host.grain && !self.reduced && self.host.theme.current().grain > 0
    }

    /// While the grain lives, the ms until its next pattern: when an idle desktop wants its next
    /// frame (by a timer, never a frame loop; nothing else redraws for it).
    pub fn grain_in(&self) -> Option<u32> {
        // Not `rem_euclid`: its float remainder links in 128-bit integer math.
        let now = self.host.now_ms;
        let next = GRAIN_MS * (now / GRAIN_MS).floor() + GRAIN_MS - now;
        self.grain_lives().then_some((next.ceil() as u32).max(1))
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
        if matches!(input, Input::PointerLeave) {
            // Carried icons and tiles slide back from where they show, which needs the pointer.
            (self.touch, self.grid.lasso) = (None, None);
            self.drop_icons(false);
            self.dock.drop(false, &mut self.pending);
        }
        if let Input::PointerDown { touch, .. } = input {
            self.finger = touch;
        }
        let was = self.pointer;
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
        match input {
            Input::Key { key, mods } => self.key(key, mods, &mut out),
            Input::Text(s) => {
                if let Some(win) = self.key_target().filter(|_| !s.is_empty()) {
                    self.host.deliver(win, AppEvent::Text(s), &mut out);
                    out.consumed = true;
                }
            }
            Input::PointerMove { .. } => {
                self.finger(&mut out);
                self.drag_to();
                if self.pointer != was {
                    self.drag_app(&mut out);
                }
                if self.grid.carry_to(self.pointer) | self.dock.carry_to(self.pointer) {
                    self.armed = None;
                }
                self.grid.lasso_to(self.pointer);
                self.point_menu();
            }
            Input::PointerDown { button: 0, touch, .. } => self.press(touch, &mut out),
            Input::PointerDown { button: 2, touch, .. } => {
                self.secondary(self.pointer.unwrap_or_default(), touch, &mut out)
            }
            Input::PointerUp { button, .. } => self.release(button == 0, &mut out),
            Input::PointerDown { .. } | Input::PointerLeave => {}
            Input::Wheel { x, y, dy } => self.wheel(coord(x), coord(y), dy, &mut out),
            Input::Resize { w, h } => {
                self.resize((coord(w).max(0.0), coord(h).max(0.0)));
                self.start(&mut out);
                (self.instant, out.redraw) = (true, true);
            }
            Input::Tick { time } => {
                out.redraw |= self.clock.replace(time) != Some(time);
                self.host.tick(true, &mut out);
            }
        }
        self.finish(before, &mut out);
        out
    }

    /// A new screen size: a new work area, which a keyboard shortening the page while the person
    /// types squeezes the windows into only until it goes (`Host::resize`).
    fn resize(&mut self, size: (f32, f32)) {
        let typing = matches!(self.ime, Some((_, true)));
        self.host.resize((self.size, size), work_area(size), typing);
        self.size = size;
        self.place();
    }

    /// The grid lays out in the work area.
    fn place(&mut self) {
        let a = work_area(self.size);
        (self.grid.area, self.grid.narrow) = (rectf(a), a.w < host::NARROW);
    }

    /// Opens Welcome on a first visit once there is a work area, and remembers it was shown.
    fn start(&mut self, out: &mut Response) {
        let a = work_area(self.size);
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
        self.host.tick(false, &mut out);
        self.settle(&mut out);
        let now = self.host.now_ms;
        self.arm(now);
        let theme = self.host.theme.at(now);
        for _ in 0..2 {
            for _ in 0..2 {
                list.clear();
                let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
                theme.draw_backdrop(list, screen, self.grain_seed() as f32);
                self.draw_icons(list, &theme, now);
                self.draw_windows(list, &theme, now);
                self.draw_overlay(list, &theme);
                self.draw_dock(list, &theme);
                self.draw_agent(list, &theme, now);
                self.draw_bar(list, &theme);
                self.draw_carried(list, &theme);
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

    /// Brings the apps up to date with the wm, then the shell with the apps and the files. An app
    /// that takes the focus ends the home screen's selection: Enter and Escape are then its own.
    fn settle(&mut self, out: &mut Response) {
        self.host.settle(out);
        self.tiles = self.host.dock_apps(&self.dock.favs);
        let favs = &self.dock.favs;
        let kept: Vec<usize> =
            self.tiles.iter().map_while(|d| favs.iter().position(|f| *f == d.0)).collect();
        self.dock.layout(kept, self.tiles.len(), self.size);
        // A tile carried shows where it would land.
        if let Some((from, to)) = self.dock.moving() {
            home::dock::shift(&mut self.tiles, from, to);
        }
        self.place_overlay(out);
        let focus = self.key_target();
        if mem::replace(&mut self.focus, focus) != focus && focus.is_some() {
            self.grid.selected.clear();
        }
        let keep = |w: &Widget| self.host.live(w.0) || w.0 == host::OVERLAY && self.overlay.open;
        (self.app_hover, self.app_press) =
            (self.app_hover.filter(keep), self.app_press.filter(keep));
        self.list_icons();
        self.sync();
    }

    fn visuals(&self) -> Visuals {
        let button = |t: Option<Target>| t.filter(|t| t.is_button());
        let g = &self.grid;
        let at = self.pointer.unwrap_or_default();
        let carry = self.carried().map(|c| (c.lifted, c.slot, at));
        let lasso = g.lasso.and(self.pointer);
        Visuals {
            wm: self.host.wm().state_hash(),
            buttons: [button(self.hover), self.armed],
            widgets: [self.app_hover, self.app_press],
            wins: self.host.wins.len(),
            zone: self.grab.and_then(|g| g.zone()),
            menu: self.menu.as_ref().map(|m| (m.0.rect, m.0.sel)),
            home: (self.tiles.len() + self.dock.favs.len(), g.icons.len(), g.selected.len()),
            carry,
            lasso,
            agent: (self.overlay, self.host.agent.working),
        }
    }

    /// Ends every event: settles, finds what is under the pointer, and fills in the response.
    fn finish(&mut self, before: Visuals, out: &mut Response) {
        self.settle(out);
        // Nothing is under what the pointer holds.
        let free = self.grab.is_none() && !self.carrying();
        let under = self.pointer.and_then(|(x, y)| Some((self.hit(x, y)?, x, y)));
        self.hover = under.map(|u| u.0).filter(|_| free);
        let widget = under.and_then(|(t, x, y)| match t {
            Target::Body(win) if free => Some((win, self.widget_at(win, x, y)?)),
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
        let focus = self.key_target();
        let app = focus.and_then(|w| self.host.win(w));
        // The overlay holding the keyboard back wants none yet.
        let quiet = focus == Some(host::OVERLAY) && self.overlay.quiet;
        let wants = app.is_some_and(|w| w.app.wants_text_input()) && !quiet;
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

/// The work area of a `w` x `h` screen: below the bar, above the bottom row.
fn work_area((w, h): (f32, f32)) -> Rect {
    let h = (h - BAR_H - DOCK_CLEAR).max(0.0);
    Rect::new(0, BAR_H as i32, w.round() as i32, h.round() as i32)
}

#[cfg(test)]
mod tests;
