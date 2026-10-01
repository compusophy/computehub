//! The compusophyOS desktop shell: the panel, window chrome and bindings
//! around a [`host::Host`], which runs one [`ui::App`] per window. Pure Rust,
//! no browser.
//!
//! [`Shell`] owns the host, and through it the [`wm::Wm`] (changed only
//! through [`wm::Wm::apply`]), the [`ui::TextSystem`], the [`vfs::Vfs`] and
//! the apps. It turns [`Input`] into wm commands and [`ui::AppEvent`]s,
//! draws into a [`gfx::DrawList`], and hands back what only the platform can
//! do as [`Effect`]s.
//!
//! # Screen and windows
//!
//! Logical pixels, origin top-left; non-finite sizes and positions count as
//! 0. The panel ([`theme::PANEL_H`] tall, over everything) holds the
//! launcher and terminal buttons, the workspace indicators and a clock; the
//! wm gets the rest. A window's content rect lies below its titlebar, inset
//! 1 px from its border; its app draws there into a [`ui::Ui`] clipped to
//! it, and the title (ellipsized) sits in the titlebar.
//!
//! # Apps
//!
//! - The [`Registry`] makes apps by name; unknown names open nothing.
//!   [`Shell::new`] opens `welcome`, `terminal` and `/apps/counter.app`
//!   tiled, then focuses welcome. There is one `launcher` at a time: opening
//!   it again focuses it.
//! - A floating window gets its app's [`ui::App::preferred_size`] as its
//!   content size, centered.
//! - Apps get [`AppEvent::Resized`] before drawing at a new size, and once
//!   more after that frame (or the first at a new pixel ratio), so what the
//!   frame changed (a terminal's grid) goes out at once (and the frame is
//!   drawn again if one asks); and [`AppEvent::Focus`] when their window
//!   gains or loses the focus. Pointer coordinates in app events are
//!   relative to the content rect. Apps on hidden workspaces ask for no
//!   frames. The clock apps see is the last [`Input::Tick`]'s or
//!   [`Shell::set_now`]'s.
//! - After each app event its [`ui::Request`]s are carried out. A socket
//!   belongs to the window that opened it (`ws://127.0.0.1:<port>/` only)
//!   and closes with it; the lazy fonts are fetched once. Apps see the
//!   latest pairing, and [`Shell::set_pairing`] opens a terminal with it.
//!
//! # Bindings
//!
//! Key-down events only. `mod` is Alt or Meta; directions are the arrows or
//! H/J/K/L.
//!
//! | keys | action |
//! |---|---|
//! | mod+Enter, mod+Shift+Enter | open a tiled, a floating terminal |
//! | mod+Space | open (or focus) the launcher |
//! | mod+Q | close the focused window |
//! | mod+F, mod+O | toggle floating, flip the split's orientation |
//! | mod+direction | focus that way |
//! | mod+Shift+direction | swap the focused tile that way |
//! | mod+Ctrl+direction | move that edge of the focused tile by 40 px |
//! | mod+1..4, mod+Shift+1..4 | switch to, move the focused window to, a workspace |
//!
//! Other keys go to the focused app as [`AppEvent::Key`], consumed, except
//! that the paste keys (Ctrl+V, Ctrl+Shift+V, Meta+V, Shift+Insert, which
//! the platform leaves to the browser) are neither sent nor consumed (the
//! text comes as [`Input::Text`]), and F5, F12, Ctrl+R, Ctrl+Shift+R and
//! Ctrl+Shift+I are consumed only if the app wants text input.
//!
//! Pointer button 0 focuses the window it presses, drags a floating window
//! by its titlebar (kept below the panel), and clicks buttons on release
//! over the button pressed. In content it sends [`AppEvent::PointerDown`]
//! with the topmost hit of the window's last frame, and a release over the
//! same [`ui::Sense::Click`] hit sends [`AppEvent::Click`]. Any release ends
//! a drag or press: browsers report only a chord's last release. The wheel
//! goes to the window under the pointer.
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
//! let mut desk = Shell::new(1280.0, 800.0, text, vfs::Vfs::new(), registry, None);
//! assert_eq!(desk.wm().layout().len(), 1); // only "terminal" is known
//! let r = desk.input(Input::Key { key: Key::Enter, mods: Mods { alt: true, ..Mods::default() } });
//! assert!(r.consumed && r.redraw && desk.wm().layout().len() == 2);
//! desk.draw(&mut gfx::DrawList::new());
//! ```

#![forbid(unsafe_code)]

mod bindings;
mod chrome;

pub use host::{Effect, Registry, Response};
pub use ui::theme;
pub use ui::{Key, Mods, WsEvent};

use gfx::{DrawList, RectF, Rgba};
use host::{CHROME_MIN_H, Host, content_rect, rectf};
use theme::*;
use ui::{AppEvent, Pairing, SocketId, TextStyle, TextSystem, WidgetId};
use vfs::Vfs;
use wm::{Cmd, Gaps, Rect, WinId, Wm};

// Panel buttons and workspace slots: their side, and the space before the
// first and between each. Titlebar buttons: their side, the close button's
// distance from the right edge, and the space between them.
const PANEL_BTN: f32 = 28.0;
const PANEL_SPACING: f32 = 4.0;
const TITLE_BTN: f32 = 22.0;
const TITLE_MARGIN: f32 = 6.0;
const TITLE_SPACING: f32 = 4.0;
/// Windows narrower than this get no titlebar buttons.
const CHROME_MIN_W: f32 = 2.0 * (TITLE_BTN + TITLE_MARGIN) + TITLE_SPACING;
/// Pixels one resize binding moves an edge.
const RESIZE_PX: i32 = 40;
/// The apps [`Shell::new`] opens, tiled, in order; the first gets the focus.
const STARTUP: [&str; 3] = ["welcome", "terminal", "/apps/counter.app"];

/// One platform event. Positions and sizes are logical pixels; pointer
/// button 0 is the primary button.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A key went down (repeats too), by its physical position.
    Key { key: Key, mods: Mods },
    /// Text typed, pasted or composed by an IME.
    Text(String),
    /// The pointer moved.
    PointerMove { x: f32, y: f32 },
    /// A pointer button went down.
    PointerDown { x: f32, y: f32, button: u8 },
    /// A pointer button went up.
    PointerUp { x: f32, y: f32, button: u8 },
    /// The pointer left the canvas.
    PointerLeave,
    /// The wheel turned at `(x, y)`; positive `dy` scrolls down.
    Wheel { x: f32, y: f32, dy: f32 },
    /// The canvas has a new size.
    Resize { w: f32, h: f32 },
    /// Time passed: `minutes` since local midnight (for the clock) and the
    /// page clock in milliseconds (for apps).
    Tick { minutes: u32, now_ms: f64 },
    /// Something happened on the socket an [`Effect::WsOpen`] named `id`.
    Ws { id: u32, ev: WsEvent },
}

/// What lies under a point: the bare panel, a panel button, a titlebar
/// button, a titlebar, or a window body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Panel,
    Launcher,
    Terminal,
    Workspace(usize),
    Close(WinId),
    Float(WinId),
    Title(WinId),
    Body(WinId),
}

impl Target {
    fn is_button(self) -> bool {
        !matches!(self, Target::Panel | Target::Title(_) | Target::Body(_))
    }

    /// The window this target belongs to.
    fn win(self) -> Option<WinId> {
        match self {
            Target::Close(w) | Target::Float(w) | Target::Title(w) | Target::Body(w) => Some(w),
            _ => None,
        }
    }
}

/// A widget of an app: its window and id.
type Widget = (WinId, WidgetId);
/// Everything whose change means a new frame.
type Visuals = (u64, [Option<Target>; 2], [Option<Widget>; 2]);

/// The desktop: a window manager, the apps in its windows, and the panel,
/// chrome and bindings around them.
pub struct Shell {
    /// The wm, the text system, the filesystem and the apps.
    host: Host,
    /// Effects from outside a [`Response`], for [`Shell::take_effects`].
    pending: Vec<Effect>,
    /// Screen width and height.
    size: (f32, f32),
    /// Where the pointer is; `None` once it leaves.
    pointer: Option<(f32, f32)>,
    /// The button under the pointer, and the one pressed but not released.
    hover: Option<Target>,
    armed: Option<Target>,
    /// The app widget under the pointer, and the one pressed and held.
    app_hover: Option<Widget>,
    app_press: Option<Widget>,
    /// The floating window dragged, and the grab's offset from its corner.
    drag: Option<(WinId, f32, f32)>,
    /// The focus and its want of text input, as last reported.
    ime: Option<(Option<WinId>, bool)>,
    /// Minutes since midnight, from the last tick.
    clock: Option<u32>,
}

impl Shell {
    /// A desktop of `w` x `h` logical pixels with the startup apps open; see
    /// the crate docs. Effects the apps cause here wait for
    /// [`Shell::take_effects`].
    pub fn new(
        w: f32,
        h: f32,
        text: TextSystem,
        vfs: Vfs,
        registry: Registry,
        pairing: Option<Pairing>,
    ) -> Shell {
        let size = (coord(w).max(0.0), coord(h).max(0.0));
        let mut gaps = Gaps::default();
        [gaps.outer, gaps.inner] = [GAP; 2];
        let wm = Wm::new(wm_area(size), gaps, WORKSPACES);
        let mut shell = Shell {
            host: Host::new(wm, text, vfs, registry, pairing),
            pending: Vec::new(),
            size,
            pointer: None,
            hover: None,
            armed: None,
            app_hover: None,
            app_press: None,
            drag: None,
            ime: None,
            clock: None,
        };
        let mut out = Response::default();
        for name in STARTUP {
            shell.host.open(name, false, &mut out);
        }
        let first = shell.host.wins().iter().find(|w| w.name == STARTUP[0]);
        if let Some(win) = first.map(|w| w.id) {
            shell.host.apply(Cmd::Focus(win));
        }
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

    /// Sets the device pixel ratio text is rasterized for.
    pub fn set_dpr(&mut self, dpr: f32) {
        self.host.set_dpr(dpr);
    }

    /// Sets the page clock apps see (unless not finite), with no event or
    /// frame. Call it before each input and frame: ticks come once a minute,
    /// and timeouts (a terminal's synchronized-output hold) need time.
    pub fn set_now(&mut self, now_ms: f64) {
        self.host.set_now(now_ms);
    }

    /// The effects that arose outside a [`Response`] (while drawing, or in
    /// [`Shell::new`]), oldest first, leaving none. Call it after every frame
    /// and event. A response also carries, first, those queued before it, so
    /// no effect is handed out twice.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.pending)
    }

    /// Pairs the desktop with a node: stores `p` for every app's [`ui::Cx`],
    /// then opens a tiled terminal, focused, which connects with it.
    pub fn set_pairing(&mut self, p: Pairing) -> Response {
        let (mut out, before) = (Response::default(), self.visuals());
        self.host.set_pairing(p);
        self.host.open("terminal", false, &mut out);
        self.finish(before, &mut out);
        out
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

    /// The frame's clear color, [`theme::BG`].
    pub fn clear_color(&self) -> Rgba {
        BG
    }

    /// Handles one event; see the crate docs for the bindings and rules.
    pub fn input(&mut self, input: Input) -> Response {
        let mut out = Response::default();
        let before = self.visuals();
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
                self.host.apply(Cmd::SetArea(wm_area(self.size)));
                out.redraw = true;
            }
            Input::Tick { minutes, now_ms } => self.tick(minutes, now_ms, &mut out),
            Input::Ws { id, ev } => self.host.ws(SocketId(id), ev, &mut out),
        }
        self.finish(before, &mut out);
        out
    }

    /// Clears `list`, then draws every window of the active workspace
    /// bottom to top, each with its app's content, and the panel over them.
    /// The desktop background is [`Shell::clear_color`], not an instance.
    /// If the glyph atlas was reset midway, the frame is drawn once more; so
    /// it is, once, if an app retold its size after drawing asks to redraw
    /// (a frame cannot ask for the next). Effects the apps cause wait for
    /// [`Shell::take_effects`].
    pub fn draw(&mut self, list: &mut DrawList) {
        let mut out = Response::default();
        for _ in 0..2 {
            self.settle(&mut out);
            for _ in 0..2 {
                list.clear();
                for p in self.host.wm().layout() {
                    self.draw_window(list, &p);
                }
                self.draw_panel(list);
                if !self.host.text_mut().take_atlas_reset() {
                    break;
                }
            }
            out.redraw = false;
            self.host.redrawn(&mut out);
            if !out.redraw {
                break;
            }
        }
        self.prune();
        self.pending.append(&mut out.effects);
    }

    /// Brings the apps up to date with the wm ([`Host::settle`]).
    fn settle(&mut self, out: &mut Response) {
        self.host.settle(out);
        self.prune();
    }

    /// Forgets the hovered and pressed widgets of apps that are gone.
    fn prune(&mut self) {
        let live = |w: Option<Widget>| w.filter(|w| self.host.win(w.0).is_some());
        (self.app_hover, self.app_press) = (live(self.app_hover), live(self.app_press));
    }

    fn visuals(&self) -> Visuals {
        let (s, hash) = (self, self.host.wm().state_hash());
        (hash, [s.hover, s.armed], [s.app_hover, s.app_press])
    }

    /// Ends every event: settles apps, recomputes hover, and fills in
    /// `redraw`, `text_input` and the pending effects.
    fn finish(&mut self, before: Visuals, out: &mut Response) {
        self.settle(out);
        let under = match (self.drag, self.pointer) {
            (None, Some((x, y))) => self.hit(x, y).map(|t| (t, x, y)),
            _ => None,
        };
        self.hover = under.map(|u| u.0).filter(|t| t.is_button());
        self.app_hover = under.and_then(|(t, x, y)| match t {
            Target::Body(win) => Some((win, self.widget_at(win, x, y)?.id)),
            _ => None,
        });
        out.redraw |= self.visuals() != before;
        let mut effects = std::mem::take(&mut self.pending);
        effects.append(&mut out.effects);
        out.effects = effects;
        let focus = self.host.focused_app();
        let app = focus.and_then(|w| self.host.win(w));
        let wants = app.is_some_and(|w| w.app.wants_text_input());
        if self.ime != Some((focus, wants)) {
            self.ime = Some((focus, wants));
            out.text_input = Some(wants);
        }
    }

    /// The minute changed: redraw the clock. Every app gets the tick.
    fn tick(&mut self, minutes: u32, now_ms: f64, out: &mut Response) {
        let minutes = minutes % (24 * 60);
        out.redraw |= self.clock.replace(minutes) != Some(minutes);
        self.host.tick(now_ms, out);
    }

    /// What is under `(x, y)`: the panel first, then windows top to bottom.
    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if (0.0..PANEL_H).contains(&y) {
            let targets = self.panel_targets();
            let on = targets.iter().find(|t| t.0.contains(x, y));
            return Some(on.map_or(Target::Panel, |t| t.1));
        }
        let layout = self.host.wm().layout();
        let p = layout.iter().rev().find(|p| rectf(p.rect).contains(x, y))?;
        let r = rectf(p.rect);
        let mut buttons = title_buttons(r, p.win).into_iter().flatten();
        Some(match buttons.find(|b| b.0.contains(x, y)) {
            Some((_, hit)) => hit,
            None if y < r.y + TITLEBAR_H => Target::Title(p.win),
            None => Target::Body(p.win),
        })
    }

    /// The topmost hit of `win`'s last frame at `(x, y)`, if that is inside
    /// its content.
    fn widget_at(&self, win: WinId, x: f32, y: f32) -> Option<ui::Hit> {
        let inside = self.content_of(win)?.contains(x, y);
        let w = self.host.win(win).filter(|_| inside)?;
        ui::hit_test(&w.hits, x, y)
    }

    /// The content rect of `win` if it is on the active workspace.
    fn content_of(&self, win: WinId) -> Option<RectF> {
        let layout = self.host.wm().layout();
        let p = layout.iter().find(|p| p.win == win)?;
        Some(content_rect(rectf(p.rect)))
    }

    /// The hover wash behind a button, doubled while it is pressed.
    fn highlight(&self, list: &mut DrawList, r: RectF, hit: Target, radius: f32) {
        if self.hover == Some(hit) {
            let pressed = u8::from(self.armed == Some(hit));
            let a = HOVER.3.saturating_mul(1 + pressed);
            list.fill(r, radius, HOVER.with_alpha(a));
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

/// The wm's area for a screen of `(w, h)`: everything below the panel.
fn wm_area((w, h): (f32, f32)) -> Rect {
    let h = (h - PANEL_H).max(0.0);
    Rect::new(0, PANEL_H as i32, w.round() as i32, h.round() as i32)
}

/// A window's float and close buttons, or `None` if it is too small.
fn title_buttons(r: RectF, win: WinId) -> Option<[(RectF, Target); 2]> {
    if r.w < CHROME_MIN_W || r.h < CHROME_MIN_H {
        return None;
    }
    let y = r.y + (TITLEBAR_H - TITLE_BTN) / 2.0;
    let x = r.x + r.w - TITLE_MARGIN - TITLE_BTN;
    let close = RectF::new(x, y, TITLE_BTN, TITLE_BTN);
    let float = RectF::new(x - TITLE_SPACING - TITLE_BTN, y, TITLE_BTN, TITLE_BTN);
    Some([(float, Target::Float(win)), (close, Target::Close(win))])
}

/// The baseline that centers a line of `style` in `h` px from `top`.
fn baseline(text: &TextSystem, top: f32, h: f32, style: TextStyle) -> f32 {
    let (a, d) = (text.ascent(style), text.descent(style));
    text.snap(top + (h - a - d) / 2.0 + a)
}

#[cfg(test)]
mod tests;
