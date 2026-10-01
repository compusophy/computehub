//! The compusophyOS desktop: floating windows, a dock, a top bar and a
//! launcher around a [`host::Host`], which runs one [`ui::App`] per window.
//! Pure Rust, no browser.
//!
//! [`Shell`] owns the host, and through it the [`wm::Wm`] (changed only
//! through [`wm::Wm::apply`]), the [`ui::TextSystem`], the [`vfs::Vfs`] and
//! the apps. It turns [`Input`] into wm commands and [`ui::AppEvent`]s,
//! draws into a [`gfx::DrawList`], and hands back what only the platform can
//! do in a [`Response`].
//!
//! # Screen
//!
//! Logical pixels, origin top-left; non-finite sizes and positions count as
//! 0. Bottom to top: the wallpaper (the theme's backdrop), the windows, the
//! dock, the top bar ([`BAR_H`] tall), the launcher, tooltips. Windows live
//! in the work area: the screen below the bar, less [`DOCK_CLEAR`] at the
//! bottom, so a maximized window never meets the dock. A window's content
//! lies below its 40 px titlebar, inset 1 px at the sides and bottom; its
//! app draws there into a [`ui::Ui`] in the current theme, clipped to it.
//!
//! # Apps
//!
//! - The [`Registry`] makes apps by name; unknown names open nothing.
//!   [`Shell::new`] opens `welcome`, 680 x 480, centered.
//! - Apps hear [`AppEvent::Resized`] before drawing at a new size and once
//!   more after that frame, [`AppEvent::Focus`] when their window gains or
//!   loses the focus, and a tick at each [`Input::Tick`]. Pointer
//!   coordinates in app events are relative to the content rect.
//! - After each app event its [`ui::Request`]s are carried out: opening
//!   `launcher` shows the launcher, a theme request switches the theme, the
//!   lazy fonts are fetched once.
//!
//! # Pointer
//!
//! Button 0 focuses the window it presses. A titlebar drags the window
//! (from 4 px of travel; a maximized or snapped one comes back to its
//! normal size under the pointer, at the same fraction of its width); two
//! presses within 350 ms toggle maximize. Edges (6 px) and corners (14 px)
//! resize. Dropped with the pointer within 6 px of the left or right screen
//! edge, a window snaps to that half; of the top edge, it maximizes; within
//! 24 px of two edges, it snaps to that quarter. Buttons act on release over
//! the button pressed. In content a press sends [`AppEvent::PointerDown`]
//! with the topmost hit of the window's last frame, and a release over the
//! same [`ui::Sense::Click`] hit sends [`AppEvent::Click`]. The wheel goes
//! to the window under the pointer.
//!
//! # Keys
//!
//! Key-down events only. `mod` is Alt or Meta, without Ctrl.
//!
//! | keys | action |
//! |---|---|
//! | mod+Space | show or hide the launcher |
//! | mod+Enter | open a terminal |
//! | mod+Q | close the focused window |
//! | mod+Up | maximize or restore the focused window |
//! | mod+Down | restore a maximized window, else minimize it |
//! | mod+Left, mod+Right | snap the focused window to that half |
//! | mod+Backquote, mod+Shift+Backquote | focus the next, the previous window |
//!
//! While the launcher shows, other keys are its own. Otherwise they go to
//! the focused app as [`AppEvent::Key`], consumed, except that the paste
//! keys (Ctrl+V, Ctrl+Shift+V, Meta+V, Shift+Insert, which the platform
//! leaves to the browser) are neither sent nor consumed (the text comes as
//! [`Input::Text`]), and F5, F12, Ctrl+R, Ctrl+Shift+R and Ctrl+Shift+I are
//! consumed only if the app wants text input.
//!
//! # Motion
//!
//! Every change eases out (CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`): windows
//! open (fade and grow from 96%, 180 ms), close (140 ms), minimize into
//! their dock tile and come back (220 ms), and move between rects when they
//! maximize, restore or snap (200 ms); dock tiles lift under the pointer
//! (120 ms), the launcher fades in (160 ms) and themes crossfade (200 ms).
//! Drags and resizes follow the pointer. Times come from
//! [`Shell::set_now`]. While anything moves, [`Shell::draw`] asks for the
//! next frame; otherwise no frame is asked for.
//!
//! ```
//! use shell::{Input, Key, Mods, Shell};
//! use ui::{App, AppEvent, Cx, Ui};
//!
//! struct Hello;
//! impl App for Hello {
//!     fn title(&self) -> String { "Hello".into() }
//!     fn draw(&mut self, ui: &mut Ui<'_>) { ui.label("hi"); }
//!     fn event(&mut self, _: AppEvent, _: &mut Cx<'_>) -> bool { false }
//! }
//! let font = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf");
//! let text = ui::TextSystem::new(std::fs::read(font).unwrap()).unwrap();
//! let registry: shell::Registry =
//!     Box::new(|name| (name == "terminal").then(|| Box::new(Hello) as Box<dyn App>));
//! let mut desk = Shell::new(1280.0, 800.0, text, vfs::Vfs::new(), registry, "Dawn");
//! assert!(desk.wm().layout().is_empty()); // only "terminal" is known
//! let r = desk.input(Input::Key { key: Key::Enter, mods: Mods { alt: true, ..Mods::default() } });
//! assert!(r.consumed && r.redraw && r.animating && desk.wm().layout().len() == 1);
//! let mut list = gfx::DrawList::new();
//! assert!(desk.draw(&mut list)); // the window fades in: more frames to come
//! assert_eq!(desk.theme_name(), "Dawn");
//! ```

#![forbid(unsafe_code)]

mod bar;
mod chrome;
mod desktop;
mod dock;
mod keys;
mod launcher;
mod motion;

pub use host::{Cursor, Effect, Input, LocalTime, Registry, Response};
pub use ui::{Key, Mods};

use desktop::{Grab, Target};
use gfx::{DrawList, Rgba};
use host::motion::Themes;
use host::{Ask, Host};
use ui::{AppEvent, TextSystem, Theme, WidgetId};
use vfs::Vfs;
use wm::{Rect, WinId, Wm};

/// Height of the top bar.
pub const BAR_H: f32 = 32.0;
/// What the work area leaves free at the bottom of the screen: the dock
/// and its margins.
pub const DOCK_CLEAR: f32 = 84.0;
/// The app [`Shell::new`] opens, and its window size.
const STARTUP: (&str, (i32, i32)) = ("welcome", (680, 480));
/// How long a theme crossfade takes.
const THEME_MS: f32 = 200.0;

/// A widget of an app: its window and id.
type Widget = (WinId, WidgetId);

/// Everything whose change means a new frame, but the clock.
#[derive(PartialEq)]
struct Visuals {
    wm: u64,
    buttons: [Option<Target>; 2],
    widgets: [Option<Widget>; 2],
    wins: usize,
    theme: &'static str,
    launcher: (bool, usize, usize, usize),
    zone: Option<host::frame::Zone>,
}

/// The desktop: a window manager, the apps in its windows, and the dock,
/// bar, launcher, chrome, motion and bindings around them.
pub struct Shell {
    /// The wm, the text system, the filesystem and the apps.
    host: Host,
    /// Effects from outside a [`Response`], for [`Shell::take_effects`].
    pending: Vec<Effect>,
    /// Screen width and height.
    size: (f32, f32),
    /// Where the pointer is; `None` once it leaves.
    pointer: Option<(f32, f32)>,
    /// What is under the pointer, and the button pressed but not released.
    hover: Option<Target>,
    armed: Option<Target>,
    /// The app widget under the pointer, and the one pressed and held.
    app_hover: Option<Widget>,
    app_press: Option<Widget>,
    /// A window being moved or resized.
    grab: Option<Grab>,
    /// The last press on a titlebar, for double clicks: window and time.
    last_title: Option<(WinId, f64)>,
    /// The focus and the want of text input, as last reported.
    ime: Option<(Option<WinId>, bool)>,
    /// The cursor, as last reported.
    cursor: Cursor,
    /// The time, from the last tick.
    clock: Option<LocalTime>,
    theme: Themes,
    dock: Vec<dock::Item>,
    launcher: launcher::Launcher,
    motion: motion::Motion,
    /// Whether this input's wm changes follow the pointer, not animate.
    instant: bool,
    /// A layer drawn, then replayed scaled and faded.
    scratch: DrawList,
}

impl Shell {
    /// A desktop of `w` x `h` logical pixels in the theme named `theme`
    /// (the first of [`ui::THEMES`] if none is), with the startup app open; see
    /// the crate docs. Effects the app causes here wait for
    /// [`Shell::take_effects`].
    pub fn new(
        w: f32,
        h: f32,
        text: TextSystem,
        vfs: Vfs,
        registry: Registry,
        theme: &str,
    ) -> Shell {
        let size = (coord(w).max(0.0), coord(h).max(0.0));
        let mut shell = Shell {
            host: Host::new(Wm::new(work_area(size)), text, vfs, registry),
            pending: Vec::new(),
            size,
            pointer: None,
            hover: None,
            armed: None,
            app_hover: None,
            app_press: None,
            grab: None,
            last_title: None,
            ime: None,
            cursor: Cursor::Default,
            clock: None,
            theme: Themes::new(theme),
            dock: Vec::new(),
            launcher: launcher::Launcher::default(),
            motion: motion::Motion::default(),
            instant: false,
            scratch: DrawList::new(),
        };
        let mut out = Response::default();
        shell.host.open(STARTUP.0, Some(STARTUP.1), &mut out);
        shell.settle(&mut out);
        shell.pending = out.effects;
        shell
    }

    /// The window manager, read-only: change it through [`Shell::input`].
    pub fn wm(&self) -> &Wm {
        self.host.wm()
    }

    /// The filesystem the apps share.
    pub fn vfs(&self) -> &Vfs {
        self.host.vfs()
    }

    /// The text system, for the platform to upload the atlas's dirty rows.
    pub fn text_mut(&mut self) -> &mut TextSystem {
        self.host.text_mut()
    }

    /// Sets the device pixel ratio text and hairlines are drawn for.
    pub fn set_dpr(&mut self, dpr: f32) {
        self.host.set_dpr(dpr);
    }

    /// Sets the page clock (monotonic milliseconds; ignored unless finite)
    /// that animations and apps run on, with no event or frame. Call it
    /// before each input and frame.
    pub fn set_now(&mut self, now_ms: f64) {
        self.host.set_now(now_ms);
    }

    /// The name of the current theme, for the platform to keep.
    pub fn theme_name(&self) -> &'static str {
        self.theme.current().name
    }

    /// The frame's clear color: the theme's base.
    pub fn clear_color(&self) -> Rgba {
        self.theme.current().base
    }

    /// The effects that arose outside a [`Response`] (while drawing, or in
    /// [`Shell::new`]), oldest first, leaving none. Call it after every frame
    /// and event. A response also carries, first, those queued before it, so
    /// no effect is handed out twice.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.pending)
    }

    /// Hands over the bytes (or the error) of an [`Effect::Fetch`]. Fonts
    /// are added as fallbacks in the order they were asked for, each once
    /// those before it arrived or failed.
    pub fn fetched(&mut self, id: u32, got: Result<Vec<u8>, String>) -> Response {
        let (mut out, before) = (Response::default(), self.visuals());
        self.host.fetched(id, got, &mut out);
        self.finish(before, &mut out);
        out
    }

    /// Handles one event; see the crate docs for the bindings and rules.
    pub fn input(&mut self, input: Input) -> Response {
        let mut out = Response::default();
        let before = self.visuals();
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
        match input {
            Input::Key { key, mods } => self.key(key, mods, &mut out),
            Input::Text(s) if self.launcher.open => {
                self.launcher.search.type_text(&s);
                out.consumed = true;
            }
            Input::Text(s) => {
                if let Some(win) = self.host.focused_app().filter(|_| !s.is_empty()) {
                    self.host.deliver(win, AppEvent::Text(s), &mut out);
                    out.consumed = true;
                }
            }
            Input::PointerMove { .. } => self.drag_to(),
            Input::PointerDown { button: 0, .. } => self.press(&mut out),
            Input::PointerUp { button, .. } => self.release(button == 0, &mut out),
            Input::PointerDown { .. } | Input::PointerLeave => {}
            Input::Wheel { x, y, dy } => self.wheel(coord(x), coord(y), dy, &mut out),
            Input::Resize { w, h } => {
                self.size = (coord(w).max(0.0), coord(h).max(0.0));
                self.host.apply(wm::Cmd::SetArea(work_area(self.size)));
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

    /// Clears `list`, then draws the desktop: the wallpaper, the windows
    /// bottom to top with their apps' content, the dock, the top bar, the
    /// launcher and tooltips. If the glyph atlas was reset midway, the frame
    /// is drawn once more; so it is, once, if an app retold its size after
    /// drawing asks to redraw. Returns whether an animation runs, so the
    /// next frame is wanted. Effects the apps cause wait for
    /// [`Shell::take_effects`].
    pub fn draw(&mut self, list: &mut DrawList) -> bool {
        let mut out = Response::default();
        self.instant = false;
        self.settle(&mut out);
        let now = self.now();
        self.arm(now);
        let theme = self.theme.at(now);
        for _ in 0..2 {
            for _ in 0..2 {
                list.clear();
                self.paint(list, &theme, now);
                if !self.host.text_mut().take_atlas_reset() {
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

    /// Every layer, bottom to top.
    fn paint(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let screen = gfx::RectF::new(0.0, 0.0, self.size.0, self.size.1);
        theme.draw_backdrop(list, screen);
        self.draw_windows(list, theme, now);
        self.draw_dock(list, theme, now);
        self.draw_bar(list, theme);
        self.draw_launcher(list, theme, now);
        self.draw_tooltip(list, theme, now);
    }

    /// The page clock.
    fn now(&self) -> f64 {
        self.host.now_ms()
    }

    /// Brings the apps up to date with the wm, then the shell with the apps:
    /// their asks, the dock and the motion.
    fn settle(&mut self, out: &mut Response) {
        self.host.settle(out);
        for ask in self.host.take_asks() {
            match ask {
                Ask::Launcher if !self.launcher.open => self.show_launcher(),
                Ask::Launcher => {}
                Ask::Theme(name) => _ = self.theme.set(&name, self.host.now_ms(), THEME_MS),
            }
        }
        let live =
            |w: Option<Widget>, h: &Host| w.filter(|w| h.win(w.0).is_some_and(|a| !a.closing()));
        self.app_hover = live(self.app_hover, &self.host);
        self.app_press = live(self.app_press, &self.host);
        self.refresh_dock();
        self.sync();
    }

    fn visuals(&self) -> Visuals {
        let button = |t: Option<Target>| t.filter(|t| t.is_button());
        let l = &self.launcher.search;
        Visuals {
            wm: self.host.wm().state_hash(),
            buttons: [button(self.hover), self.armed],
            widgets: [self.app_hover, self.app_press],
            wins: self.host.wins().len(),
            theme: self.theme.current().name,
            launcher: (self.launcher.open, l.query.len(), l.sel, l.first),
            zone: self.grab.and_then(Grab::zone),
        }
    }

    /// Ends every event: settles apps, recomputes what is under the
    /// pointer, and fills in `redraw`, `text_input`, `cursor`, `animating`
    /// and the pending effects.
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
        if cursor != std::mem::replace(&mut self.cursor, cursor) {
            out.cursor = Some(cursor);
        }
        out.animating = self.animating();
        out.redraw |= out.animating || self.visuals() != before;
        let mut effects = std::mem::take(&mut self.pending);
        effects.append(&mut out.effects);
        out.effects = effects;
        let focus = self.host.focused_app();
        let app = focus.and_then(|w| self.host.win(w));
        let wants = self.launcher.open || app.is_some_and(|w| w.app.wants_text_input());
        if self.ime != Some((focus, wants)) {
            self.ime = Some((focus, wants));
            out.text_input = Some(wants);
        }
    }
}

/// A finite coordinate within the wm's range; NaN and infinities become 0.
/// Not `clamp`: its panic path links in float formatting.
fn coord(v: f32) -> f32 {
    let max = wm::MAX_COORD as f32;
    let v = if v.is_finite() { v } else { 0.0 };
    v.max(-max).min(max)
}

/// The wm's area for a screen of `(w, h)`: below the bar, above the dock.
fn work_area((w, h): (f32, f32)) -> Rect {
    let h = (h - BAR_H - DOCK_CLEAR).max(0.0);
    Rect::new(0, BAR_H as i32, w.round() as i32, h.round() as i32)
}

#[cfg(test)]
mod tests;
