//! The compusophyOS app host, no browser: the [`wm::Wm`] and one [`ui::App`] per window, and the
//! pure parts the shell builds on ([`motion`], [`frame`], [`search`], [`paint`]).
//!
//! Where a window opens ([`Host::open`]): on a narrow work area (under [`NARROW`] px) maximized,
//! as every window there stays; else a [`ui::App::compact`] app at its preferred size, centered,
//! and any other large (0.85 of the work area, cascaded), maximized if no other window shows. A
//! window opened maximized restores to that large size.
//!
//! [`Host`] changes the wm only through [`Host::apply`], delivers
//! [`ui::AppEvent`]s and carries out the apps' [`ui::Request`]s; what only the
//! platform can do comes back as [`Effect`]s in a [`Response`]. GUI process
//! frames go to every app's [`ui::App::frame`], never the platform. A closing
//! window's app hears [`ui::App::closing`], then stays, deaf, while the shell
//! animates it away, until [`Host::reap`] drops it and ends its processes.

#![forbid(unsafe_code)]

pub mod frame;
pub mod motion;
pub mod paint;
pub mod search;

use std::mem;

use gfx::{DrawList, RectF};
use kernel::Kernel;
use motion::Themes;
use ui::{AiStatus, AppEvent, AppIcon, Cx, Key, Mods, Request, TextSystem, Theme, Ui, UiState};
use vfs::Vfs;
use wm::{Cmd, Outcome, Rect, State, WinId, Wm};

/// Makes the app for a window by name (`"terminal"`, a `.app` path, ...).
pub type Registry = Box<dyn Fn(&str) -> Option<Box<dyn ui::App>>>;

/// The lazy fonts, fetched as ids 1 and 2 and added in this order.
const FONT_URLS: [&str; 2] = ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"];
/// How long a theme crossfade takes, and the height of a window's titlebar.
pub const THEME_MS: f32 = 200.0;
pub const TITLEBAR_H: f32 = wm::TITLE_H as f32;
/// Work areas narrower than this (logical px) are phones'.
pub const NARROW: i32 = 720;

/// One platform event, in logical pixels: a key down by position (repeats too), text typed,
/// pasted or composed, the pointer (button 0 primary, 2 secondary; `touch` for a finger), the
/// wheel (`dy` > 0 down), size, time.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Key { key: Key, mods: Mods },
    Text(String),
    PointerMove { x: f32, y: f32 },
    PointerDown { x: f32, y: f32, button: u8, touch: bool },
    PointerUp { x: f32, y: f32, button: u8 },
    PointerLeave,
    Wheel { x: f32, y: f32, dy: f32 },
    Resize { w: f32, h: f32 },
    Tick { time: LocalTime },
}

/// A local time to the minute: `month` 1..=12, `weekday` 0..=6 from Sunday, `hour` 0..=23.
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
        date.extend((self.day >= 10).then(|| digit(self.day / 10)));
        date.extend([digit(self.day), ' ']);
        date + MONTHS[usize::from(self.month.clamp(1, 12) - 1)]
    }

    /// The time of day, 24-hour: `"14:32"`.
    pub fn clock(&self) -> String {
        let (h, m) = (self.hour, self.minute);
        String::from_iter([digit(h / 10), digit(h), ':', digit(m / 10), digit(m)])
    }
}

/// Something only the platform can do: fetch `url` (page-relative) for
/// [`Host::fetched`] with `id`, what the kernel asked for (workers, timer),
/// store a preference ([`ui::Request::Pref`]) in the page's storage, or send
/// feedback ([`ui::Request::Feedback`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Fetch { id: u32, url: String },
    Kernel(kernel::Effect),
    Pref { key: String, value: String },
    Feedback { kind: String, text: String, context: bool },
}

/// For the kernel: a worker's message or failure, the one-shot timer, the page hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelIn {
    Msg { pid: u32, msg: Vec<u8> },
    Error { pid: u32 },
    Wake,
    Hidden,
}

/// The pointer: arrow, I-beam on text, open hand on a titlebar (closed while moving), resizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Cursor {
    #[default] Default, Text, Grab, Grabbing, EwResize, NsResize, NwseResize, NeswResize,
}

/// After an input: draw, `preventDefault`, text input and cursor (if changed), effects, animate.
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

/// What an app is handed: an event, a process's frame, or its window closing.
enum Call<'a> {
    Event(AppEvent),
    Frame(u32, &'a [u8]),
    Closing,
}

/// The window manager and the apps in its windows (closing ones too, by id),
/// and what they share: text, files, the kernel, the page clock (ms), the theme.
pub struct Host {
    wm: Wm,
    pub text: TextSystem,
    pub vfs: Vfs,
    pub kernel: Kernel,
    /// The [`Vfs::generation`] the kernel last heard of.
    generation: u64,
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
    /// What apps see in [`Cx::ai`]: os sets it; an [`ui::AI_MODEL`] preference updates it.
    pub ai: AiStatus,
}

impl Host {
    #[rustfmt::skip]
    pub fn new(wm: Wm, text: TextSystem, vfs: Vfs, registry: Registry, theme: &str) -> Host {
        let (wins, fonts, icons, theme) = (Vec::new(), Vec::new(), Vec::new(), Themes::new(theme));
        let (now_ms, launcher, themes, focus) = (0.0, false, Vec::new(), None);
        let ai = AiStatus::default();
        Host { generation: vfs.generation(), kernel: Kernel::new(), wm, text, vfs, registry, wins,
            now_ms, theme, launcher, themes, fonts, focus, icons, ai }
    }

    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// Applies `cmd` to the wm (a stale id changes nothing); a closing window's app hears first.
    pub fn apply(&mut self, cmd: Cmd) {
        _ = self.close_then(cmd, &mut Response::default());
    }

    fn close_then(&mut self, cmd: Cmd, out: &mut Response) -> bool {
        if let Cmd::Close(win) = cmd {
            self.call(win, Call::Closing, out);
        }
        self.wm.apply(cmd).is_ok()
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

    /// Whether the work area is a phone's: every window maximized, none moved.
    pub fn narrow(&self) -> bool {
        self.wm.area().w < NARROW
    }

    /// Opens `name`, if the registry knows it, in a new focused window placed as the crate docs
    /// say, `size` (window px) if given.
    pub fn open(&mut self, name: &str, size: Option<(i32, i32)>, out: &mut Response) {
        let Some(app) = (self.registry)(name) else { return };
        let (a, alone, compact) = (self.wm.area(), self.wm.layout().is_empty(), app.compact());
        let preferred = app.preferred_size().and_then(window_size).filter(|_| compact);
        let size = size.or(preferred).unwrap_or((a.w * 85 / 100, a.h * 85 / 100));
        let Ok(Outcome::Opened(id)) = self.wm.apply(Cmd::Open { size: Some(size) }) else { return };
        let r = self.wm.normal_rect(id).unwrap_or_default();
        let (x, y) = (a.x + (a.w - r.w) / 2, a.y + (a.h - r.h) / 2);
        if self.narrow() || alone && !compact {
            _ = self.wm.apply(Cmd::Maximize(id));
        } else if compact {
            _ = self.wm.apply(Cmd::Move { win: id, x, y });
        }
        let (name, hits) = (name.to_string(), Vec::new());
        self.wins.push(Win { id, app, name, hits, sizes: [None; 2] });
        out.redraw = true;
    }

    /// The everything bar asked `text`: shows the Assistant (opening it if need be), which hears
    /// it as [`AppEvent::Ask`].
    pub fn ask(&mut self, text: &str, out: &mut Response) {
        self.show("assistant", out);
        if let Some(&win) = self.windows_of("assistant").last() {
            self.deliver(win, AppEvent::Ask(text.to_string()), out);
        }
    }

    /// Drops the app of `win` once its window is closed, killing what it ran.
    pub fn reap(&mut self, win: WinId) {
        if self.wm.normal_rect(win).is_none() {
            self.wins.retain(|w| w.id != win);
            self.kernel.kill_owned(win.0);
        }
    }

    /// Hands `ev` to the app of `win` if open, does what it asked, then pumps.
    pub fn deliver(&mut self, win: WinId, ev: AppEvent, out: &mut Response) {
        self.call(win, Call::Event(ev), out);
        self.pump(out);
    }

    /// After an app event or kernel input, up to 4 rounds: `Io` to woken windows,
    /// a moved [`Vfs::generation`] told, frames to the apps, other effects out.
    pub fn pump(&mut self, out: &mut Response) {
        for _ in 0..4 {
            let woken = self.kernel.take_woken();
            self.each(out, |h, win, out| {
                if woken.iter().any(|&o| o == win.0 || o == u32::MAX) {
                    h.call(win, Call::Event(AppEvent::Io), out);
                }
            });
            if mem::replace(&mut self.generation, self.vfs.generation()) != self.generation {
                self.kernel.vfs_changed();
            }
            for e in self.kernel.take_effects() {
                match e {
                    kernel::Effect::Draw { pid, frame } => self.each(out, |h, win, out| {
                        h.call(win, Call::Frame(pid, &frame), out);
                    }),
                    e => out.effects.push(Effect::Kernel(e)),
                }
            }
        }
    }

    /// `f` for each window by index (fewer boot bytes than a Vec; apps only add windows).
    fn each(&mut self, out: &mut Response, mut f: impl FnMut(&mut Host, WinId, &mut Response)) {
        for i in 0..self.wins.len() {
            let Some(win) = self.wins.get(i).map(|w| w.id) else { break };
            f(self, win, out);
        }
    }

    /// Hands `call` to the app of `win` if open, owning what it spawns; does what it asked.
    fn call(&mut self, win: WinId, call: Call<'_>, out: &mut Response) {
        let shown = self.wm.layout().iter().any(|p| p.win == win);
        let rect = self.wm.normal_rect(win);
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win && rect.is_some()) else { return };
        self.kernel.set_owner(win.0);
        let mut cx = Cx::new(&mut self.vfs, &mut self.kernel, self.now_ms);
        cx.ai = self.ai.clone();
        let redraw = match call {
            Call::Event(ev) => w.app.event(ev, &mut cx),
            Call::Frame(pid, frame) => w.app.frame(pid, frame, &mut cx),
            Call::Closing => {
                w.app.closing(&mut cx);
                false
            }
        };
        out.redraw |= redraw && shown;
        for request in cx.take_requests() {
            // Nothing more once the app closed itself.
            if !self.live(win) {
                break;
            }
            match request {
                Request::Open { name, .. } if name == "launcher" => self.launcher = true,
                Request::Open { name, .. } => self.open(&name, None, out),
                Request::CloseSelf => out.redraw |= self.close_then(Cmd::Close(win), out),
                Request::LoadFallbackFonts => self.load_fonts(out),
                Request::SetTheme(name) => self.themes.push(name),
                Request::Pref { key, value } => {
                    if key == ui::AI_MODEL {
                        self.ai.model = value.clone();
                    }
                    out.effects.push(Effect::Pref { key, value });
                }
                Request::Feedback { kind, text, context } => {
                    out.effects.push(Effect::Feedback { kind, text, context });
                }
                Request::Size(w, h) => {
                    let (r, size) = (rect.unwrap_or_default(), window_size((w.into(), h.into())));
                    let rect = size.map(|(w, h)| Rect::new(r.x, r.y, w, h));
                    out.redraw |=
                        rect.is_some_and(|rect| self.wm.apply(Cmd::Resize { win, rect }).is_ok());
                }
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

    /// Hands the kernel what the platform heard from workers and timers; pumps.
    pub fn kernel_in(&mut self, ev: KernelIn, out: &mut Response) {
        match ev {
            KernelIn::Msg { pid, msg } => self.kernel.message(&mut self.vfs, pid, &msg),
            KernelIn::Error { pid } => self.kernel.failed(pid),
            KernelIn::Wake | KernelIn::Hidden => self.kernel.wake(),
        }
        self.pump(out);
    }

    pub fn tick(&mut self, out: &mut Response) {
        let now_ms = self.now_ms;
        for win in self.wins.iter().map(|w| w.id).collect::<Vec<_>>() {
            self.deliver(win, AppEvent::Tick { now_ms }, out);
        }
    }

    /// Maximizes what shows on a narrow work area, tells apps focus changes and new content
    /// sizes, then switches to themes they asked for.
    pub fn settle(&mut self, out: &mut Response) {
        if self.narrow() {
            for p in self.wm.layout().into_iter().filter(|p| p.state != State::Maximized) {
                _ = self.wm.apply(Cmd::Maximize(p.win));
            }
        }
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
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win) else { return };
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

    /// A dock click: minimizes the app's focused window, else shows its top or newest, or opens.
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

    /// The dock's apps (`pinned`, then the others running; those the registry knows) and their
    /// windows.
    pub fn dock_apps(&mut self, pinned: &[String]) -> Vec<(String, AppIcon, Vec<WinId>)> {
        let mut names = pinned.to_vec();
        for w in self.wins.iter().filter(|w| self.live(w.id)) {
            if !names.contains(&w.name) {
                names.push(w.name.clone());
            }
        }
        let apps = names.into_iter().filter_map(|n| Some((self.icon(&n)?, self.windows_of(&n), n)));
        apps.map(|(icon, wins, n)| (n, icon, wins)).collect()
    }

    /// The launcher's entries: `apps` as tiles (those the registry knows), then the `.app` files
    /// in `~/apps`, `~` and `/apps`.
    pub fn entries(&mut self, apps: &[&str]) -> Vec<search::Entry> {
        let mut out = Vec::new();
        self.entries_of(apps, &mut out);
        let mine = [Vfs::HOME, "/apps"].concat();
        for (dir, shown) in [(&*mine, "~/apps"), (Vfs::HOME, "~"), ("/apps", "/apps")] {
            self.app_files(dir, shown, &mut out);
        }
        out
    }

    /// The desktop's icons: Home (Files at `~`), Welcome, About and Feedback (those the registry
    /// knows), then the person's own apps: the `.app` files in `~/apps`.
    pub fn desktop(&mut self) -> Vec<search::Entry> {
        let mut out = Vec::new();
        self.entries_of(&["files", "welcome", "about", "feedback"], &mut out);
        if let Some(home) = out.first_mut().filter(|e| e.name == "files") {
            (home.label, home.icon.glyph) = ("Home".into(), ui::icon::Glyph::Home);
        }
        self.app_files(&[Vfs::HOME, "/apps"].concat(), "~/apps", &mut out);
        out
    }

    /// The apps of `names` the registry knows.
    fn entries_of(&mut self, names: &[&str], out: &mut Vec<search::Entry>) {
        for &name in names {
            if let Some(icon) = self.icon(name) {
                let (name, label) = (name.to_string(), app_label(name));
                out.push(search::Entry { name, label, icon, place: None });
            }
        }
    }

    /// The `.app` files in `dir` (shown as in `shown`), each a window on a hue of its name.
    fn app_files(&self, dir: &str, shown: &str, out: &mut Vec<search::Entry>) {
        for e in self.vfs.list(dir).unwrap_or_default() {
            if e.is_dir || !e.name.ends_with(".app") {
                continue;
            }
            let fnv = |h: u32, b: u8| (h ^ u32::from(b)).wrapping_mul(16_777_619);
            let hue = ui::theme::app_tint(e.name.bytes().fold(2_166_136_261, fnv));
            let (name, place) = ([dir, "/", &e.name].concat(), [shown, "/", &e.name].concat());
            let icon = AppIcon { glyph: ui::icon::Glyph::Window, hue };
            out.push(search::Entry { label: app_label(&name), name, icon, place: Some(place) });
        }
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
