//! The compusophyOS app host, no browser: the [`wm::Wm`] and one [`ui::App`] per window, the
//! apps of the home screen ([`Host::home`]), windows held by the pointer ([`grab`]), and the pure
//! parts the shell builds on ([`motion`], [`frame`], [`paint`]).
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
//! animates it away, until [`Host::reap`] drops it and ends its processes. The Assistant is never
//! a window: it is the overlay, the AI that uses the desktop as a person does ([`agent`]).

#![forbid(unsafe_code)]

pub mod agent;
pub mod frame;
pub mod grab;
pub mod motion;
pub mod paint;

use std::mem;

pub use agent::{ASSISTANT, OVERLAY};
use gfx::{DrawList, RectF};
use kernel::Kernel;
use motion::Themes;
use ui::icon::Mark;
use ui::{AiStatus, AppEvent, AppIcon, Cx, Key, Mods, Request, TextSystem, Theme, Ui, UiState};
use vfs::Vfs;
use wm::{Cmd, Outcome, Rect, Snap, State, WinId, Wm};

/// The screen before a keyboard shortened it; each free window then, its rect then, and where
/// the last squeeze left it.
type Kept = ((f32, f32), Vec<(WinId, Rect, Rect)>);

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
/// store a preference ([`ui::Request::Pref`]) in the page's storage, send
/// feedback ([`ui::Request::Feedback`]), sign out (the person's own act, from
/// the desktop's menu: no app, and not the Assistant, can ask for it), or erase all the device
/// keeps ([`ui::Request::Reset`], never from a window the Assistant acted on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Fetch { id: u32, url: String },
    Kernel(kernel::Effect),
    Pref { key: String, value: String },
    Feedback { kind: String, text: String, context: bool },
    SignOut,
    Reset,
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

/// After an input: draw, `preventDefault`, text input and cursor (if changed), effects, animate,
/// and whether a finger lifted from a gesture (a scroll, a wander or a long press), not a tap; a
/// long press on a window's content that opened no menu is still a tap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub redraw: bool,
    pub consumed: bool,
    pub text_input: Option<bool>,
    pub effects: Vec<Effect>,
    pub cursor: Option<Cursor>,
    pub animating: bool,
    pub gesture: bool,
}

/// An app in a window, its name and its hit regions from the last frame.
pub struct Win {
    pub id: WinId,
    pub app: Box<dyn ui::App>,
    pub name: String,
    pub hits: Vec<ui::Hit>,
    /// The content rect `hits` were laid out in.
    hits_at: RectF,
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
    /// Whether the backdrop's grain lives, as apps see it in [`Cx::grain`]; the shell sets it
    /// from the page, a [`ui::GRAIN`] preference updates it.
    pub grain: bool,
    /// The windows maximized only because the work area is narrow, with their snap and normal
    /// rect from before, put back when it widens.
    forced: Vec<(WinId, Option<Snap>, Rect)>,
    /// What a keyboard squeezed, to put back when it goes.
    kept: Option<Kept>,
    /// The overlay's acts and status, and what the shell shows of it.
    pub agent: agent::Agent,
}

impl Host {
    #[rustfmt::skip]
    pub fn new(wm: Wm, text: TextSystem, vfs: Vfs, registry: Registry, theme: &str) -> Host {
        let (wins, fonts, icons, theme) = (Vec::new(), Vec::new(), Vec::new(), Themes::new(theme));
        let (now_ms, themes, focus) = (0.0, Vec::new(), None);
        let (ai, forced) = (AiStatus::default(), Vec::new());
        Host { generation: vfs.generation(), kernel: Kernel::new(), wm, text, vfs, registry, wins,
            now_ms, theme, themes, fonts, focus, icons, ai, grain: true, forced,
            agent: Default::default(), kept: None }
    }

    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// Applies `cmd` to the wm (a stale id changes nothing); a closing window's app hears first.
    /// A work area turning narrow first notes the windows it will maximize, as they are.
    pub fn apply(&mut self, cmd: Cmd) {
        if matches!(cmd, Cmd::SetArea(a) if a.w < NARROW) && !self.narrow() {
            self.force();
        }
        _ = self.close_then(cmd, &mut Response::default());
    }

    /// Notes each shown window not maximized (once): its snap and normal rect.
    fn force(&mut self) {
        for p in self.wm.layout().into_iter().filter(|p| p.state != State::Maximized) {
            if !self.forced.iter().any(|f| f.0 == p.win) {
                let normal = self.wm.normal_rect(p.win).unwrap_or(p.rect);
                self.forced.push((p.win, p.snap, normal));
            }
        }
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

    /// The screen went from `from` to `to`, its work area now `area`. A keyboard that shortens
    /// the page (the width kept, while the person is `typing`) squeezes the free windows only
    /// until the page is that tall again: they come back then, but for those the person moved or
    /// resized meanwhile, which stay where they put them.
    pub fn resize(&mut self, (from, to): ((f32, f32), (f32, f32)), area: Rect, typing: bool) {
        let free = |p: &wm::Placement| p.state == State::Normal && p.snap.is_none();
        if self.kept.is_none() && typing && to.0 == from.0 && to.1 < from.1 {
            let wins = self.wm.layout().into_iter().filter(free).map(|p| (p.win, p.rect, p.rect));
            self.kept = Some((from, wins.collect()));
        }
        // Each kept window must still be free and where the last squeeze left it.
        let layout = self.wm.layout();
        let left = |&(win, _, at): &(WinId, Rect, Rect)| {
            layout.iter().any(|p| p.win == win && free(p) && p.rect == at)
        };
        if let Some((_, wins)) = &mut self.kept {
            wins.retain(left);
        }
        self.apply(Cmd::SetArea(area));
        let Some((full, mut wins)) = self.kept.take() else { return };
        if to.0 != full.0 || to.1 < full.1 {
            // Still short, it waits (noting where each window is now); a new width (a phone
            // turned) forgets them.
            let layout = self.wm.layout();
            for w in &mut wins {
                w.2 = layout.iter().find(|p| p.win == w.0).map_or(w.2, |p| p.rect);
            }
            self.kept = (to.0 == full.0).then_some((full, wins));
            return;
        }
        for (win, rect, _) in wins {
            self.apply(Cmd::Resize { win, rect });
        }
    }

    /// Whether the work area is a phone's: every window maximized, none moved.
    pub fn narrow(&self) -> bool {
        self.wm.area().w < NARROW
    }

    /// Opens `name`, if the registry knows it, in a new focused window placed as the crate docs
    /// say, `size` (window px) if given; the Assistant summons the overlay instead.
    pub fn open(&mut self, name: &str, size: Option<(i32, i32)>, out: &mut Response) {
        if name == ASSISTANT {
            self.agent.summon = true;
            return;
        }
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
        self.wins.push(Win { id, app, name, hits, hits_at: RectF::default(), sizes: [None; 2] });
        out.redraw = true;
    }

    /// Drops the app of `win` once its window is closed, killing what it ran.
    pub fn reap(&mut self, win: WinId) {
        if self.wm.normal_rect(win).is_none() && win != OVERLAY {
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
            self.agent_step(out);
        }
    }

    /// `f` for each window by index (fewer boot bytes than a Vec; apps only add windows).
    fn each(&mut self, out: &mut Response, mut f: impl FnMut(&mut Host, WinId, &mut Response)) {
        for i in 0..self.wins.len() {
            let Some(win) = self.wins.get(i).map(|w| w.id) else { break };
            f(self, win, out);
        }
    }

    /// Hands `call` to the app of `win` if open (or the overlay's), owning what it spawns; does
    /// what it asked.
    fn call(&mut self, win: WinId, call: Call<'_>, out: &mut Response) {
        let overlay = win == OVERLAY;
        let shown = self.wm.layout().iter().any(|p| p.win == win);
        let shown = shown || overlay && self.agent.shown.is_some();
        let rect = self.wm.normal_rect(win);
        let open = rect.is_some() || overlay;
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win && open) else { return };
        self.kernel.set_owner(win.0);
        let mut cx = Cx::new(&mut self.vfs, &mut self.kernel, self.now_ms);
        (cx.ai, cx.grain) = (self.ai.clone(), self.grain);
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
            if !(self.live(win) || overlay && self.win(win).is_some()) {
                break;
            }
            match request {
                Request::Open { name, .. } => self.open(&name, None, out),
                Request::CloseSelf if overlay => self.drop_overlay(),
                Request::CloseSelf => out.redraw |= self.close_then(Cmd::Close(win), out),
                Request::Agent(req) => self.agent_request(win, req),
                Request::LoadFallbackFonts => self.load_fonts(out),
                Request::SetTheme(name) => self.themes.push(name),
                Request::Pref { key, value } => {
                    if key == ui::AI_MODEL {
                        self.ai.model = value.clone();
                    }
                    if key == ui::GRAIN {
                        self.grain = value != "off";
                    }
                    out.effects.push(Effect::Pref { key, value });
                }
                Request::Feedback { kind, text, context } => {
                    out.effects.push(Effect::Feedback { kind, text, context });
                }
                Request::Reset if !self.agent.touched.contains(&win) => {
                    out.effects.push(Effect::Reset);
                }
                Request::Reset => {}
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
            KernelIn::Wake | KernelIn::Hidden => {
                self.kernel.wake();
                self.woken();
            }
        }
        self.pump(out);
    }

    /// Time passed: every app hears it (`all`, the minute), or each shown one that animates (a
    /// frame).
    pub fn tick(&mut self, all: bool, out: &mut Response) {
        let (now_ms, shown) = (self.now_ms, self.wm.layout());
        self.each(out, |h, win, out| {
            let animates = || h.win(win).is_some_and(|w| w.app.frame_in(now_ms) == Some(0));
            if all || shown.iter().any(|p| p.win == win) && animates() {
                h.deliver(win, AppEvent::Tick { now_ms }, out);
            }
        });
    }

    /// Maximizes what shows on a narrow work area (once it widens, puts back those it did and
    /// that are still maximized), tells apps focus changes and new content sizes, then switches
    /// to themes they asked for.
    pub fn settle(&mut self, out: &mut Response) {
        if self.narrow() {
            self.force();
            for p in self.wm.layout().into_iter().filter(|p| p.state != State::Maximized) {
                _ = self.wm.apply(Cmd::Maximize(p.win));
            }
        } else {
            for (win, snap, rect) in mem::take(&mut self.forced) {
                if self.wm.layout().iter().any(|p| p.win == win && p.state == State::Maximized) {
                    _ = self.wm.apply(Cmd::Restore(win));
                    _ = self.wm.apply(Cmd::Resize { win, rect });
                    if let Some(snap) = snap {
                        _ = self.wm.apply(Cmd::SnapTo { win, snap });
                    }
                }
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
        (w.hits_at, _) = (layout, w.hits.clear());
        if layout.w > 0.0 && layout.h > 0.0 {
            list.push_clip(clip);
            w.app.draw(&mut Ui::new(list, &mut self.text, layout, &mut w.hits, state, theme));
            list.pop_clip();
        }
    }

    /// Lays `win`'s hits out at `layout` (drawing nowhere) unless the last frame did: a press
    /// during a window's motion, or before its first frame at rest, finds what is there now.
    pub fn fresh_hits(&mut self, win: WinId, layout: RectF, theme: &Theme, state: UiState) {
        if self.win(win).is_some_and(|w| w.hits_at != layout) {
            self.draw_content(&mut DrawList::new(), win, [layout; 2], theme, state);
        }
    }

    /// The icon of the app `name`, running or not; a `.app` file's on its name's hue.
    pub fn icon(&mut self, name: &str) -> Option<AppIcon> {
        let icon = match self.wins.iter().find(|w| w.name == name) {
            Some(w) => Some(w.app.icon()),
            None => match self.icons.iter().find(|i| i.0 == name) {
                Some(known) => known.1,
                None => {
                    let icon = (self.registry)(name).map(|app| app.icon());
                    self.icons.push((name.to_string(), icon));
                    icon
                }
            },
        };
        let hue = |icon: AppIcon| match sigil(name) {
            Some(seed) => AppIcon { hue: ui::theme::app_tint(seed), ..icon },
            None => icon,
        };
        icon.map(hue)
    }

    /// The open windows of `name`: those opened as `name`, and as `name:<arg>` (Studio on a
    /// file, Files in a folder), which the dock shows as one app.
    pub fn windows_of(&self, name: &str) -> Vec<WinId> {
        let of = |w: &&Win| w.name == name || app_of(&w.name) == name;
        self.wins.iter().filter(of).filter(|w| self.live(w.id)).map(|w| w.id).collect()
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
            let app = app_of(&w.name);
            if !names.iter().any(|n| n == app) {
                names.push(app.to_string());
            }
        }
        let apps = names.into_iter().filter_map(|n| Some((self.icon(&n)?, self.windows_of(&n), n)));
        apps.map(|(icon, wins, n)| (n, icon, wins)).collect()
    }

    /// The apps of the home screen: those of `apps` the registry knows, then the person's own,
    /// the `.app` files in `~/apps`.
    pub fn home(&mut self, apps: &[&str]) -> Vec<Entry> {
        let mut out = Vec::new();
        for &name in apps {
            if let Some(icon) = self.icon(name) {
                let (name, label) = (name.to_string(), app_label(name));
                out.push(Entry { name, label, icon, mark: None });
            }
        }
        let dir = [Vfs::HOME, "/apps"].concat();
        for e in self.vfs.list(&dir).unwrap_or_default() {
            if !e.is_dir && e.name.ends_with(".app") {
                let name = [&dir, "/", &e.name].concat();
                let (seed, glyph) = (sigil(&name).unwrap_or(0), ui::icon::Glyph::Window);
                let icon = AppIcon { glyph, hue: ui::theme::app_tint(seed) };
                out.push(Entry { label: app_label(&name), mark: self.mark(&name), name, icon });
            }
        }
        out
    }

    /// What a `.app` file's tile shows: the icon its header draws ([`ui::icon::made`]), else its
    /// name's sigil; none for another app.
    pub fn mark(&self, name: &str) -> Option<Mark> {
        let seed = sigil(name)?;
        let made = self.vfs.read(name).ok().and_then(ui::icon::made::read);
        Some(made.map_or(Mark::Sigil(seed), Mark::Made))
    }
}

/// An app on the home screen: its registry name or `.app` path, what it is called, its icon,
/// and a `.app` file's mark (its own icon, or its [`sigil`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub label: String,
    pub icon: AppIcon,
    pub mark: Option<Mark>,
}

/// The seed of the sigil a `.app` file shows for a glyph (and of its hue): FNV-1a of its file
/// name, for a window name that is a `.app` path (`"studio:<path>"` is Studio's).
pub fn sigil(name: &str) -> Option<u32> {
    let file = name.rsplit('/').next().filter(|f| f.ends_with(".app") && app_of(name) == name)?;
    let fnv = |h: u32, b: u8| (h ^ u32::from(b)).wrapping_mul(16_777_619);
    Some(file.bytes().fold(2_166_136_261, fnv))
}

/// The app a window name opens: `"studio"` for `"studio:/apps/x.app"`, else the name (a path
/// is its own app).
pub fn app_of(name: &str) -> &str {
    name.split_once(':').filter(|(app, _)| !app.contains('/')).map_or(name, |(app, _)| app)
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
    chars.next().map(upper).into_iter().chain(chars).collect()
}

/// `c` in upper case if a small letter with a Latin-1 capital (not `ß`, `ÿ`). Not
/// `to_uppercase`: core's Unicode tables cost the boot kilobytes.
pub fn upper(c: char) -> char {
    match c {
        'a'..='z' | 'à'..='ö' | 'ø'..='þ' => char::from(c as u8 - 32),
        _ => c,
    }
}

#[cfg(test)]
mod tests;
