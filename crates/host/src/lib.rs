//! The compusophyOS app host: the [`wm::Wm`] and one [`ui::App`] per window,
//! plus the pure parts the desktop shell builds on: [`motion`], window
//! [`frame`] geometry, the dock and launcher [`layout`], the launcher's
//! [`search`] and [`paint`] helpers. No browser.
//!
//! [`Host`] changes the wm only through [`Host::apply`], delivers
//! [`ui::AppEvent`]s and carries out the apps' [`ui::Request`]s; what only the
//! platform can do comes back as [`Effect`]s in a [`Response`]. An app whose
//! window the wm closed stays, deaf, until the shell has animated it away and
//! calls [`Host::reap`].

#![forbid(unsafe_code)]

pub mod frame;
pub mod layout;
pub mod motion;
pub mod paint;
pub mod search;

use std::mem;

use gfx::{DrawList, RectF};
use motion::Themes;
use ui::{AppEvent, AppIcon, Cx, Key, Mods, Request, TextSystem, Theme, Ui, UiState};
use vfs::Vfs;
use wm::{Cmd, Outcome, Rect, WinId, Wm};

/// Makes the app for a window by name (`"terminal"`, a `.app` path, ...).
pub type Registry = Box<dyn Fn(&str) -> Option<Box<dyn ui::App>>>;

/// The lazy fonts, fetched as ids 1 and 2 and added in this order.
const FONT_URLS: [&str; 2] = ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"];
/// How long a theme crossfade takes, and the height of a window's titlebar.
pub const THEME_MS: f32 = 200.0;
pub const TITLEBAR_H: f32 = wm::TITLE_H as f32;

/// One platform event, in logical pixels: a key down (repeats too) by its
/// position, text typed or pasted or composed, the pointer (button 0 the
/// primary), the wheel (`dy` > 0 scrolls down), a new size, a new minute.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Key { key: Key, mods: Mods },
    Text(String),
    PointerMove { x: f32, y: f32 },
    PointerDown { x: f32, y: f32, button: u8 },
    PointerUp { x: f32, y: f32, button: u8 },
    PointerLeave,
    Wheel { x: f32, y: f32, dy: f32 },
    Resize { w: f32, h: f32 },
    Tick { time: LocalTime },
}

/// A local date and time to the minute: `month` 1..=12, `weekday` 0..=6
/// from Sunday, `hour` 0..=23.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn digit(n: u8) -> char {
    char::from(b'0' + n % 10)
}

impl LocalTime {
    /// The date as the top bar shows it: `"Wed 1 Oct"`.
    pub fn date(&self) -> String {
        let mut date = [DAYS[usize::from(self.weekday % 7)], " "].concat();
        if self.day >= 10 {
            date.push(digit(self.day / 10));
        }
        date.extend([digit(self.day), ' ']);
        date + MONTHS[usize::from(self.month.clamp(1, 12) - 1)]
    }

    /// The time of day, 24-hour: `"14:32"`.
    pub fn clock(&self) -> String {
        let (h, m) = (self.hour, self.minute);
        String::from_iter([digit(h / 10), digit(h), ':', digit(m / 10), digit(m)])
    }
}

/// Something only the platform can do: fetch `url` (relative to the page)
/// and hand the result to [`Host::fetched`] with `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Fetch { id: u32, url: String },
}

/// The pointer's look: the arrow, an I-beam over text, an open hand over a
/// titlebar (closed while a window moves), and the resize arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Default,
    Text,
    Grab,
    Grabbing,
    EwResize,
    NsResize,
    NwseResize,
    NeswResize,
}

/// What the platform should do after an input: draw, `preventDefault`, set
/// text input and the cursor (when changed), carry out effects, animate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub redraw: bool,
    pub consumed: bool,
    pub text_input: Option<bool>,
    pub effects: Vec<Effect>,
    pub cursor: Option<Cursor>,
    pub animating: bool,
}

/// An app in a window, its name and its hit regions from the last frame.
pub struct Win {
    pub id: WinId,
    pub app: Box<dyn ui::App>,
    pub name: String,
    pub hits: Vec<ui::Hit>,
    /// The content size it was last told, and the one it last drew at.
    sizes: [Option<(f32, f32)>; 2],
}

enum Load {
    Pending,
    Ready(Vec<u8>),
    Done,
}

/// The window manager and the apps in its windows (closing ones too, by
/// id), and what they share: text, files, the page clock (ms), the theme.
pub struct Host {
    wm: Wm,
    pub text: TextSystem,
    pub vfs: Vfs,
    registry: Registry,
    pub wins: Vec<Win>,
    pub now_ms: f64,
    pub theme: Themes,
    /// An app asked for the launcher (opened `"launcher"`).
    pub launcher: bool,
    /// Themes apps asked for, switched to when the apps settle.
    themes: Vec<String>,
    /// The lazy fonts, by fetch id less one.
    fonts: Vec<Load>,
    /// The window that last got `Focus(true)`.
    focus: Option<WinId>,
    /// Icons of apps by name, as the registry made them.
    icons: Vec<(String, Option<AppIcon>)>,
}

impl Host {
    pub fn new(wm: Wm, text: TextSystem, vfs: Vfs, registry: Registry, theme: &str) -> Host {
        let (wins, fonts, icons, theme) = (Vec::new(), Vec::new(), Vec::new(), Themes::new(theme));
        let (now_ms, launcher, themes, focus) = (0.0, false, Vec::new(), None);
        Host { wm, text, vfs, registry, wins, now_ms, theme, launcher, themes, fonts, focus, icons }
    }

    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// Applies `cmd` to the wm; an error (a stale id) leaves it untouched.
    pub fn apply(&mut self, cmd: Cmd) {
        _ = self.wm.apply(cmd);
    }

    /// Sets the device pixel ratio; a new one retells apps their sizes.
    pub fn set_dpr(&mut self, dpr: f32) {
        let old = self.text.dpr();
        self.text.set_dpr(dpr);
        if self.text.dpr() != old {
            self.wins.iter_mut().for_each(|w| w.sizes[1] = None);
        }
    }

    pub fn win(&self, id: WinId) -> Option<&Win> {
        self.wins.iter().find(|w| w.id == id)
    }

    /// Whether window `id` is open, with an app in it.
    pub fn live(&self, id: WinId) -> bool {
        self.wm.normal_rect(id).is_some() && self.win(id).is_some()
    }

    pub fn focused_app(&self) -> Option<WinId> {
        self.wm.focused().filter(|&w| self.live(w))
    }

    /// Opens `name`, if the registry knows it, in a new focused window of
    /// `size`, else of the app's preferred content size, else the wm's default.
    pub fn open(&mut self, name: &str, size: Option<(i32, i32)>, out: &mut Response) {
        let Some(app) = (self.registry)(name) else {
            return;
        };
        let size = size.or_else(|| app.preferred_size().and_then(window_size));
        if let Ok(Outcome::Opened(id)) = self.wm.apply(Cmd::Open { size }) {
            let (name, hits) = (name.to_string(), Vec::new());
            self.wins.push(Win { id, app, name, hits, sizes: [None; 2] });
            out.redraw = true;
        }
    }

    /// Drops the app of `win` once its window is closed.
    pub fn reap(&mut self, win: WinId) {
        if self.wm.normal_rect(win).is_none() {
            self.wins.retain(|w| w.id != win);
        }
    }

    /// Hands `ev` to the app of `win` if open, then carries out its requests.
    pub fn deliver(&mut self, win: WinId, ev: AppEvent, out: &mut Response) {
        let shown = self.wm.layout().iter().any(|p| p.win == win);
        let open = self.wm.normal_rect(win).is_some();
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win && open) else {
            return;
        };
        let mut cx = Cx::new(&mut self.vfs, self.now_ms);
        out.redraw |= w.app.event(ev, &mut cx) && shown;
        for request in cx.take_requests() {
            // Nothing more once the app closed itself.
            if !self.live(win) {
                break;
            }
            match request {
                Request::Open { name, .. } if name == "launcher" => self.launcher = true,
                Request::Open { name, .. } => self.open(&name, None, out),
                Request::CloseSelf => out.redraw |= self.wm.apply(Cmd::Close(win)).is_ok(),
                Request::LoadFallbackFonts => self.load_fonts(out),
                Request::SetTheme(name) => self.themes.push(name),
            }
        }
    }

    fn load_fonts(&mut self, out: &mut Response) {
        if self.fonts.is_empty() && self.text.fallback_count() == 0 {
            for (id, url) in (1..).zip(FONT_URLS) {
                self.fonts.push(Load::Pending);
                out.effects.push(Effect::Fetch { id, url: url.to_string() });
            }
        }
    }

    /// The result of an [`Effect::Fetch`]: fonts become fallbacks in the order
    /// asked for, once those before them arrived or failed.
    pub fn fetched(&mut self, id: u32, got: Result<Vec<u8>, String>, out: &mut Response) {
        let slot = self.fonts.get_mut((id as usize).wrapping_sub(1));
        if let Some(f) = slot.filter(|f| matches!(f, Load::Pending)) {
            *f = got.map_or(Load::Done, Load::Ready);
        }
        for f in &mut self.fonts {
            if matches!(f, Load::Pending) {
                break;
            }
            if let Load::Ready(bytes) = mem::replace(f, Load::Done) {
                out.redraw |= self.text.add_fallback(bytes).is_ok();
            }
        }
    }

    pub fn tick(&mut self, out: &mut Response) {
        let now_ms = self.now_ms;
        for win in self.wins.iter().map(|w| w.id).collect::<Vec<_>>() {
            self.deliver(win, AppEvent::Tick { now_ms }, out);
        }
    }

    /// Tells the apps focus changes and new content sizes, then switches the
    /// themes they asked for.
    pub fn settle(&mut self, out: &mut Response) {
        for _ in 0..8 {
            let focused = self.focused_app();
            let calm = focused == self.focus;
            if !calm {
                if let Some(old) = mem::replace(&mut self.focus, focused) {
                    self.deliver(old, AppEvent::Focus(false), out);
                }
                if let Some(new) = focused {
                    self.deliver(new, AppEvent::Focus(true), out);
                }
            }
            if !self.retell(0, out) && calm {
                break;
            }
        }
        for name in mem::take(&mut self.themes) {
            self.theme.set(&name, self.now_ms, THEME_MS);
        }
    }

    /// After a frame, retells apps that drew at a size (or dpr) new to them.
    pub fn redrawn(&mut self, out: &mut Response) {
        self.retell(1, out);
    }

    /// Tells each shown app its content size if `sizes[k]` differs.
    fn retell(&mut self, k: usize, out: &mut Response) -> bool {
        let mut told = false;
        for p in self.wm.layout() {
            let c = content_rect(rectf(p.rect));
            let w = self.wins.iter_mut().find(|w| w.id == p.win);
            let new = w.is_some_and(|w| w.sizes[k].replace((c.w, c.h)) != Some((c.w, c.h)));
            if new && (k == 0 || c.w > 0.0 && c.h > 0.0) {
                told = true;
                self.deliver(p.win, AppEvent::Resized { w: c.w, h: c.h }, out);
            }
        }
        told
    }

    /// The app of `win` draws, laid out in `layout`, clipped to `clip`.
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
        if layout.w > 0.0 && layout.h > 0.0 {
            list.push_clip(clip);
            w.app.draw(&mut Ui::new(list, &mut self.text, layout, &mut w.hits, state, theme));
            list.pop_clip();
        }
    }

    /// The icon of the app `name`, running or not.
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

    pub fn windows_of(&self, name: &str) -> Vec<WinId> {
        let mine = self.wins.iter().filter(|w| w.name == name && self.live(w.id));
        mine.map(|w| w.id).collect()
    }

    /// Shows the newest window of `name` (on top, focused), else opens it.
    pub fn show(&mut self, name: &str, out: &mut Response) {
        match self.windows_of(name).last() {
            Some(&win) => self.apply(Cmd::Focus(win)),
            None => self.open(name, None, out),
        }
    }

    /// A dock click: minimizes the app's focused window, else shows its top
    /// or newest one, else opens it.
    pub fn toggle(&mut self, name: &str, out: &mut Response) {
        let mine = self.windows_of(name);
        let focused = self.wm.focused().filter(|w| mine.contains(w));
        let shown = self.wm.layout().iter().rev().map(|p| p.win).find(|w| mine.contains(w));
        match (focused, shown.or(mine.last().copied())) {
            (Some(win), _) => self.apply(Cmd::Minimize(win)),
            (None, Some(win)) => self.apply(Cmd::Focus(win)),
            (None, None) => self.open(name, None, out),
        }
    }

    /// The dock's apps (`pinned`, then the others running) and their windows.
    pub fn dock_apps(&mut self, pinned: &[&str]) -> Vec<(String, AppIcon, Vec<WinId>)> {
        let mut names: Vec<String> = pinned.iter().map(|n| n.to_string()).collect();
        for w in self.wins.iter().filter(|w| self.live(w.id)) {
            if !names.contains(&w.name) {
                names.push(w.name.clone());
            }
        }
        let apps = names.into_iter().filter_map(|n| Some((self.icon(&n)?, self.windows_of(&n), n)));
        apps.map(|(icon, wins, n)| (n, icon, wins)).collect()
    }

    /// The launcher's entries: `apps` as tiles, then the `.app` files in
    /// `/apps` and the guest's home (`~`).
    pub fn entries(&mut self, apps: &[&str]) -> Vec<search::Entry> {
        let mut out = Vec::new();
        for &n in apps {
            if let Some(icon) = self.icon(n) {
                let (name, label) = (n.to_string(), app_label(n));
                out.push(search::Entry { name, label, icon, place: None });
            }
        }
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

pub fn rectf(r: Rect) -> RectF {
    RectF::from_i32(r.x, r.y, r.w, r.h)
}

/// Where a window at `r` shows its app: below the titlebar, inset 1 px.
pub fn content_rect(r: RectF) -> RectF {
    let (w, h) = ((r.w - 2.0).max(0.0), (r.h - TITLEBAR_H - 1.0).max(0.0));
    RectF::new(r.x + 1.0, r.y + TITLEBAR_H, w, h)
}

fn window_size((w, h): (f32, f32)) -> Option<(i32, i32)> {
    let side =
        |v: f32| (v.is_finite() && v >= 0.0).then(|| v.round().min(wm::MAX_COORD as f32) as i32);
    Some((side(w + 2.0)?, side(h + TITLEBAR_H + 1.0)?))
}

/// What a person calls the app `name`: `"/apps/counter.app"` is `"Counter"`.
pub fn app_label(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let mut chars = base.strip_suffix(".app").unwrap_or(base).chars();
    chars.next().map(search::upper).into_iter().chain(chars).collect()
}

#[cfg(test)]
mod tests;
