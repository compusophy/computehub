//! The compusophyOS app host: the window manager and the apps in its
//! windows, one [`ui::App`] per window, plus the pure parts the desktop
//! shell builds on: [`motion`] (tweens, the ease-out curve, replaying a
//! draw list scaled and faded, theme crossfades), [`frame`] (window
//! controls, resize edges, snap zones), [`layout`] (where the dock and
//! the launcher sit), [`search`] (the launcher's query and results) and
//! [`paint`] (shared drawing helpers). Pure Rust, no browser.
//!
//! [`Host`] owns the [`wm::Wm`] (changed only through [`Host::apply`]), the
//! [`ui::TextSystem`], the [`vfs::Vfs`] and the apps. It opens and closes
//! windows, delivers [`ui::AppEvent`]s, carries out the [`ui::Request`]s the
//! apps make, and hands back what only the platform can do as [`Effect`]s in
//! a [`Response`]. What only the shell can do (the launcher, the theme)
//! waits as an [`Ask`] for [`Host::take_asks`].
//!
//! A closed window's app stays, [`Win::closing`] and deaf to events, until
//! the shell has animated it away and calls [`Host::reap`].

#![forbid(unsafe_code)]

pub mod frame;
pub mod layout;
pub mod motion;
pub mod paint;
pub mod search;

use std::mem;

use gfx::{DrawList, RectF};
use ui::{AppEvent, AppIcon, Cx, Key, Mods, Request, TextSystem, Theme, Ui, UiState};
use vfs::Vfs;
use wm::{Cmd, Outcome, Rect, WinId, Wm};

/// Makes the app for a window from its name: a built-in such as
/// `"terminal"`, or a `.app` path. `None` for a name it does not know, which
/// then opens nothing.
pub type Registry = Box<dyn Fn(&str) -> Option<Box<dyn ui::App>>>;

/// The lazy fonts: fetched on the first [`Request::LoadFallbackFonts`] and
/// added in this order.
const FONT_URLS: [&str; 2] = ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"];
/// Rounds of focus and resize events before [`Host::settle`] gives up; the
/// rest waits for the next event or frame.
const SETTLE_PASSES: usize = 8;
/// Height of a window's titlebar: the wm's [`wm::TITLE_H`].
pub const TITLEBAR_H: f32 = wm::TITLE_H as f32;

/// One platform event for the desktop shell. Positions and sizes are
/// logical pixels; pointer button 0 is the primary button.
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
    /// The minute changed (or the date): the time for the top bar's clock.
    Tick { time: LocalTime },
}

/// A local date and time of day, to the minute, for the clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    /// The full year, such as 2026.
    pub year: u16,
    /// The month, `1..=12`.
    pub month: u8,
    /// The day of the month, `1..=31`.
    pub day: u8,
    /// The day of the week, `0..=6`, 0 being Sunday.
    pub weekday: u8,
    /// The hour, `0..=23`.
    pub hour: u8,
    /// The minute, `0..=59`.
    pub minute: u8,
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

impl LocalTime {
    /// The date as the top bar shows it: `"Wed 1 Oct"`.
    pub fn date(&self) -> String {
        let digit = |n: u8| char::from(b'0' + n % 10);
        let mut date = String::from(DAYS[usize::from(self.weekday % 7)]);
        date.push(' ');
        if self.day >= 10 {
            date.push(digit(self.day / 10));
        }
        date.extend([digit(self.day), ' ']);
        date.push_str(MONTHS[usize::from(self.month.clamp(1, 12) - 1)]);
        date
    }

    /// The time of day, 24-hour: `"14:32"`.
    pub fn clock(&self) -> String {
        let digit = |n: u8| char::from(b'0' + n % 10);
        let (h, m) = (self.hour, self.minute);
        String::from_iter([digit(h / 10), digit(h), ':', digit(m / 10), digit(m)])
    }
}

/// Something only the platform can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Fetch `url` (relative to the page) and hand the bytes back to
    /// [`Host::fetched`] (through the shell) with `id`.
    Fetch { id: u32, url: String },
}

/// The pointer's look over the desktop; the platform maps each to its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    /// The arrow.
    #[default]
    Default,
    /// The I-beam, over text an app edits.
    Text,
    /// An open hand, over a titlebar.
    Grab,
    /// A closed hand, while a window moves.
    Grabbing,
    /// Resizing left and right.
    EwResize,
    /// Resizing up and down.
    NsResize,
    /// Resizing along the top-left to bottom-right diagonal.
    NwseResize,
    /// Resizing along the top-right to bottom-left diagonal.
    NeswResize,
}

/// What the platform should do after an input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// The screen changed: draw a new frame.
    pub redraw: bool,
    /// The event was used: the platform should `preventDefault` it.
    pub consumed: bool,
    /// Whether keys and text should reach the desktop, when that or the
    /// focus changed: focus or blur the platform's text element to match.
    pub text_input: Option<bool>,
    /// What to do, in order.
    pub effects: Vec<Effect>,
    /// The pointer's new look, when it changed.
    pub cursor: Option<Cursor>,
    /// An animation runs: frames are wanted until a frame says otherwise.
    pub animating: bool,
}

/// A request an app made that only the shell can carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Open the launcher (an app opened `"launcher"`).
    Launcher,
    /// Switch to the theme with this name ([`Request::SetTheme`]).
    Theme(String),
}

/// An app in a window.
pub struct Win {
    /// Its window.
    pub id: WinId,
    /// The app.
    pub app: Box<dyn ui::App>,
    /// The name it was opened by.
    pub name: String,
    /// Its hit regions from the last frame, in screen coordinates.
    pub hits: Vec<ui::Hit>,
    /// The content size it was last told, and the one it last drew at
    /// (`None` again when the device pixel ratio changes).
    sizes: [Option<(f32, f32)>; 2],
    closing: bool,
}

impl Win {
    /// Whether its window is closed and it only waits for [`Host::reap`].
    pub fn closing(&self) -> bool {
        self.closing
    }
}

/// A lazy font on its way, by its fetch id.
enum Load {
    Pending,
    Ready(Vec<u8>),
    Done,
}

/// The window manager and the apps in its windows, and what they share:
/// the text system, the filesystem, the page clock, and the fonts they
/// asked for.
pub struct Host {
    wm: Wm,
    text: TextSystem,
    vfs: Vfs,
    registry: Registry,
    /// The apps, by window id (ids only grow, so pushing keeps them sorted).
    wins: Vec<Win>,
    /// The next fetch id.
    next_id: u32,
    /// The lazy fonts, once asked for.
    fonts: Vec<(u32, Load)>,
    /// The window that last got `Focus(true)`.
    focus: Option<WinId>,
    now_ms: f64,
    asks: Vec<Ask>,
    /// Icons of apps by name, as their registry entry made them.
    icons: Vec<(String, Option<AppIcon>)>,
}

impl Host {
    /// A host over `wm` with no apps yet; `registry` makes them by name.
    pub fn new(wm: Wm, text: TextSystem, vfs: Vfs, registry: Registry) -> Host {
        Host {
            wm,
            text,
            vfs,
            registry,
            wins: Vec::new(),
            next_id: 1,
            fonts: Vec::new(),
            focus: None,
            now_ms: 0.0,
            asks: Vec::new(),
            icons: Vec::new(),
        }
    }

    /// The window manager, read-only: change it through [`Host::apply`].
    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// Applies `cmd` to the wm; whether that changed anything. Errors leave
    /// the wm untouched, and a stale id is not worth reporting.
    pub fn apply(&mut self, cmd: Cmd) -> bool {
        matches!(self.wm.apply(cmd), Ok(Outcome::Changed | Outcome::Opened(_)))
    }

    /// The filesystem the apps share.
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    /// The text system the apps draw with.
    pub fn text(&self) -> &TextSystem {
        &self.text
    }

    /// The text system, to draw and measure with.
    pub fn text_mut(&mut self) -> &mut TextSystem {
        &mut self.text
    }

    /// Sets the device pixel ratio text is rasterized for; a new one retells
    /// every app its size after its next frame ([`Host::redrawn`]).
    pub fn set_dpr(&mut self, dpr: f32) {
        let old = self.text.dpr();
        self.text.set_dpr(dpr);
        if self.text.dpr() != old {
            self.wins.iter_mut().for_each(|w| w.sizes[1] = None);
        }
    }

    /// The apps, by window id, closing ones included.
    pub fn wins(&self) -> &[Win] {
        &self.wins
    }

    /// The app in window `id`, even if closing.
    pub fn win(&self, id: WinId) -> Option<&Win> {
        self.wins.iter().find(|w| w.id == id)
    }

    fn live(&self, id: WinId) -> bool {
        self.win(id).is_some_and(|w| !w.closing)
    }

    /// The focused window, if an app lives in it.
    pub fn focused_app(&self) -> Option<WinId> {
        self.wm.focused().filter(|&w| self.live(w))
    }

    /// Opens `name` in a new window, focused, unless the registry does not
    /// know it. The window is `size` (whole window, titlebar included), else
    /// its app's [`ui::App::preferred_size`] as content, else the wm's
    /// default; the wm centers and cascades it.
    pub fn open(
        &mut self,
        name: &str,
        size: Option<(i32, i32)>,
        out: &mut Response,
    ) -> Option<WinId> {
        let app = (self.registry)(name)?;
        let size = size.or_else(|| app.preferred_size().and_then(window_size));
        let Ok(Outcome::Opened(id)) = self.wm.apply(Cmd::Open { size }) else {
            return None;
        };
        let name = name.to_string();
        let (hits, sizes, closing) = (Vec::new(), [None; 2], false);
        self.wins.push(Win { id, app, name, hits, sizes, closing });
        out.redraw = true;
        Some(id)
    }

    /// Closes `win`; its app stays, closing, until [`Host::reap`].
    pub fn close(&mut self, win: WinId, out: &mut Response) {
        self.apply(Cmd::Close(win));
        self.mark_closed(out);
    }

    /// Marks the apps whose windows the wm no longer has as closing.
    fn mark_closed(&mut self, out: &mut Response) {
        for w in self.wins.iter_mut().filter(|w| !w.closing) {
            if self.wm.normal_rect(w.id).is_none() {
                (w.closing, out.redraw) = (true, true);
            }
        }
    }

    /// Drops the app of `win` if it is closing.
    pub fn reap(&mut self, win: WinId) {
        self.wins.retain(|w| !(w.id == win && w.closing));
    }

    /// Hands `ev` to the app of `win` (not a closing one), then carries out
    /// what it asked for. A minimized app's redraw is moot.
    pub fn deliver(&mut self, win: WinId, ev: AppEvent, out: &mut Response) {
        let shown = self.wm.layout().iter().any(|p| p.win == win);
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win && !w.closing) else {
            return;
        };
        let mut cx = Cx::new(&mut self.vfs, self.now_ms);
        out.redraw |= w.app.event(ev, &mut cx) && shown;
        for request in cx.take_requests() {
            self.request(win, request, out);
        }
    }

    /// Carries out one request of the app in `from`; nothing once that
    /// window is closed (it closed itself earlier in the same batch).
    fn request(&mut self, from: WinId, request: Request, out: &mut Response) {
        if !self.live(from) {
            return;
        }
        match request {
            Request::Open { name, .. } if name == "launcher" => self.asks.push(Ask::Launcher),
            Request::Open { name, .. } => _ = self.open(&name, None, out),
            Request::CloseSelf => self.close(from, out),
            Request::LoadFallbackFonts => self.load_fonts(out),
            Request::SetTheme(name) => self.asks.push(Ask::Theme(name)),
        }
    }

    /// The requests only the shell can carry out, oldest first, leaving none.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        mem::take(&mut self.asks)
    }

    /// Asks for the lazy fonts, once, unless fallbacks are already loaded.
    fn load_fonts(&mut self, out: &mut Response) {
        if !self.fonts.is_empty() || self.text.fallback_count() > 0 {
            return;
        }
        for url in FONT_URLS {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            self.fonts.push((id, Load::Pending));
            out.effects.push(Effect::Fetch { id, url: url.to_string() });
        }
    }

    /// Hands over the bytes (or the error) of an [`Effect::Fetch`]. Fonts
    /// are added as fallbacks in the order they were asked for, each once
    /// those before it arrived or failed; adding one sets `out.redraw`.
    pub fn fetched(&mut self, id: u32, got: Result<Vec<u8>, String>, out: &mut Response) {
        let slot = self.fonts.iter_mut().find(|f| f.0 == id);
        if let Some(f) = slot.filter(|f| matches!(f.1, Load::Pending)) {
            f.1 = got.map_or(Load::Done, Load::Ready);
        }
        for f in &mut self.fonts {
            match mem::replace(&mut f.1, Load::Done) {
                Load::Pending => {
                    f.1 = Load::Pending;
                    break;
                }
                Load::Ready(bytes) => out.redraw |= self.text.add_fallback(bytes).is_ok(),
                Load::Done => {}
            }
        }
    }

    /// Sets the page clock for every app's [`ui::Cx`] and frame, unless
    /// `now_ms` is not finite.
    pub fn set_now(&mut self, now_ms: f64) {
        if now_ms.is_finite() {
            self.now_ms = now_ms;
        }
    }

    /// The page clock, in milliseconds.
    pub fn now_ms(&self) -> f64 {
        self.now_ms
    }

    /// Time passed: every app gets a tick at the page clock.
    pub fn tick(&mut self, out: &mut Response) {
        let wins: Vec<WinId> = self.wins.iter().map(|w| w.id).collect();
        for win in wins {
            let now_ms = self.now_ms;
            self.deliver(win, AppEvent::Tick { now_ms }, out);
        }
    }

    /// Brings the apps up to date with the wm: marks apps whose windows are
    /// gone as closing, then sends focus changes and new content sizes,
    /// until nothing changes (or 8 rounds).
    pub fn settle(&mut self, out: &mut Response) {
        for _ in 0..SETTLE_PASSES {
            self.mark_closed(out);
            let mut calm = true;
            let focused = self.focused_app();
            if focused != self.focus {
                calm = false;
                if let Some(old) = mem::replace(&mut self.focus, focused) {
                    self.deliver(old, AppEvent::Focus(false), out);
                }
                if let Some(new) = focused {
                    self.deliver(new, AppEvent::Focus(true), out);
                }
            }
            for p in self.wm.layout() {
                let c = content_rect(rectf(p.rect));
                let w = self.wins.iter_mut().find(|w| w.id == p.win);
                if w.is_some_and(|w| w.sizes[0].replace((c.w, c.h)) != Some((c.w, c.h))) {
                    calm = false;
                    self.deliver(p.win, AppEvent::Resized { w: c.w, h: c.h }, out);
                }
            }
            if calm {
                break;
            }
        }
    }

    /// After a frame: each app on screen that drew at a size (or dpr) new to
    /// it hears [`AppEvent::Resized`] again; `out.redraw` says whether one
    /// asked to draw again (what it drew may already be stale).
    pub fn redrawn(&mut self, out: &mut Response) {
        for p in self.wm.layout() {
            let c = content_rect(rectf(p.rect));
            let w = self.wins.iter_mut().find(|w| w.id == p.win);
            let new = w.is_some_and(|w| w.sizes[1].replace((c.w, c.h)) != Some((c.w, c.h)));
            if new && c.w > 0.0 && c.h > 0.0 {
                self.deliver(p.win, AppEvent::Resized { w: c.w, h: c.h }, out);
            }
        }
    }

    /// The app of `win` draws its content laid out in `layout`, showing what
    /// falls inside `clip`, in `theme`; its hits replace the last frame's.
    pub fn draw_content(
        &mut self,
        list: &mut DrawList,
        win: WinId,
        [layout, clip]: [RectF; 2],
        theme: &Theme,
        state: UiState,
    ) {
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win) else {
            return;
        };
        w.hits.clear();
        if layout.w <= 0.0 || layout.h <= 0.0 {
            return;
        }
        list.push_clip(clip);
        w.app.draw(&mut Ui::new(list, &mut self.text, layout, &mut w.hits, state, theme));
        list.pop_clip();
    }

    /// The icon of the app `name`: a running one's, else what its registry
    /// entry says (made once and remembered); `None` if the registry does
    /// not know it.
    pub fn icon(&mut self, name: &str) -> Option<AppIcon> {
        if let Some(w) = self.wins.iter().find(|w| w.name == name) {
            return Some(w.app.icon());
        }
        if let Some(known) = self.icons.iter().find(|i| i.0 == name) {
            return known.1;
        }
        let icon = (self.registry)(name).map(|app| app.icon());
        self.icons.push((name.to_string(), icon));
        icon
    }

    /// The open (not closing) windows of the app `name`, oldest first.
    pub fn windows_of(&self, name: &str) -> Vec<WinId> {
        self.wins.iter().filter(|w| w.name == name && !w.closing).map(|w| w.id).collect()
    }

    /// Shows the newest window of `name` (on top, focused, minimized or
    /// not), else opens it.
    pub fn show(&mut self, name: &str, out: &mut Response) {
        match self.windows_of(name).last() {
            Some(&win) => _ = self.apply(Cmd::Focus(win)),
            None => _ = self.open(name, None, out),
        }
    }

    /// What a click on the app `name` in a dock does: minimizes its focused
    /// window, else shows its topmost shown window or its newest, else
    /// opens it.
    pub fn toggle(&mut self, name: &str, out: &mut Response) {
        let mine = self.windows_of(name);
        let focused = self.wm.focused().filter(|w| mine.contains(w));
        let shown = self.wm.layout().iter().rev().map(|p| p.win).find(|w| mine.contains(w));
        match (focused, shown.or(mine.last().copied())) {
            (Some(win), _) => _ = self.apply(Cmd::Minimize(win)),
            (None, Some(win)) => _ = self.apply(Cmd::Focus(win)),
            (None, None) => _ = self.open(name, None, out),
        }
    }

    /// The apps a dock shows, with their icons and open windows: each of
    /// `pinned` the registry knows, then every other app with an open
    /// window, in the order they opened.
    pub fn dock_apps(&mut self, pinned: &[&str]) -> Vec<(String, AppIcon, Vec<WinId>)> {
        let mut names: Vec<String> = pinned.iter().map(|n| n.to_string()).collect();
        for w in self.wins.iter().filter(|w| !w.closing) {
            if !names.contains(&w.name) {
                names.push(w.name.clone());
            }
        }
        let known = names.into_iter().filter_map(|n| Some((self.icon(&n)?, n)));
        let apps: Vec<(AppIcon, String)> = known.collect();
        apps.into_iter().map(|(icon, n)| (n.clone(), icon, self.windows_of(&n))).collect()
    }

    /// What a launcher offers: each of `apps` the registry knows, as a
    /// tile, then every `.app` file directly in `/apps` and the guest's
    /// home (shown as `~`), each in a hue of its own.
    pub fn entries(&mut self, apps: &[&str]) -> Vec<search::Entry> {
        let tile = |(name, icon): (&str, AppIcon)| {
            let (label, name) = (app_label(name), name.to_string());
            search::Entry { name, label, icon, place: None }
        };
        let known: Vec<(&str, AppIcon)> =
            apps.iter().filter_map(|&n| Some((n, self.icon(n)?))).collect();
        let mut out: Vec<search::Entry> = known.into_iter().map(tile).collect();
        for (dir, shown) in [("/apps", "/apps"), (Vfs::HOME, "~")] {
            for e in self.vfs.list(dir).unwrap_or_default() {
                if e.is_dir || !e.name.ends_with(".app") {
                    continue;
                }
                let fnv = |h: u32, b: u8| (h ^ u32::from(b)).wrapping_mul(16_777_619);
                let hue = ui::theme::app_tint(e.name.bytes().fold(2_166_136_261, fnv));
                let (name, place) = ([dir, "/", &e.name].concat(), [shown, "/", &e.name].concat());
                let icon = AppIcon { glyph: "", hue };
                out.push(search::Entry { label: app_label(&name), name, icon, place: Some(place) });
            }
        }
        out
    }
}

/// A wm rect in draw-list coordinates.
pub fn rectf(r: Rect) -> RectF {
    RectF::from_i32(r.x, r.y, r.w, r.h)
}

/// Where a window of rect `r` shows its app: below the titlebar, inset 1 px
/// from the border at the sides and bottom.
pub fn content_rect(r: RectF) -> RectF {
    let (w, h) = ((r.w - 2.0).max(0.0), (r.h - TITLEBAR_H - 1.0).max(0.0));
    RectF::new(r.x + 1.0, r.y + TITLEBAR_H, w, h)
}

/// The window size around content of `w` x `h`, unless not finite.
fn window_size((w, h): (f32, f32)) -> Option<(i32, i32)> {
    let side =
        |v: f32| (v.is_finite() && v >= 0.0).then(|| v.round().min(wm::MAX_COORD as f32) as i32);
    Some((side(w + 2.0)?, side(h + TITLEBAR_H + 1.0)?))
}

/// What a person calls the app `name`: the last part of a path without its
/// `.app`, first letter capitalized ([`search::upper`]):
/// `"/apps/counter.app"` is `"Counter"`.
pub fn app_label(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let stem = base.strip_suffix(".app").unwrap_or(base);
    let mut chars = stem.chars();
    chars.next().map(search::upper).into_iter().chain(chars).collect()
}

#[cfg(test)]
mod tests;
