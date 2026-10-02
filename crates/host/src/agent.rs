//! The AI's eyes and hands, for the overlay: the Assistant's window over the desktop, held under
//! [`OVERLAY`] outside the wm (it is never in the dock, the stack or a window list).
//!
//! - **Eyes.** [`Host::scene`] draws each shown window again at rest into a recording list and
//!   keeps its hits, the text it shows (clipped) and its widgets' marks, with the wm's facts, the
//!   theme and the home screen's apps; the whole at most [`SCENE`] bytes as sent, so an answer
//!   always reaches the overlay. An idle desktop pays nothing for it.
//! - **Hands.** An [`Act`] goes the way a person's pointer and keys go: a click raises the window,
//!   tells it its focus, then presses and releases the widget's middle (`PointerDown`, then
//!   `Click` for a button); a window verb is the title bar's control; opening an app is the home
//!   screen's tile. Each act flashes what it touched ([`Agent::flash`]).
//! - **Settling.** An act is answered by one [`Event::Acted`] once no window is
//!   [`ui::App::busy`] or [`SETTLE_MS`] passed (a wait: once its time passed), with the scene
//!   then; the platform's timer is armed for the deadline, and again whenever it fires before it
//!   (the page keeps the sooner of two). One act at a time; only the overlay may act
//!   ([`acted::REFUSED`] otherwise) and nothing acts on it. A task ending (the overlay's status
//!   no longer working, or the person taking over) drops the act settling for it.

use std::mem;

use gfx::{DrawList, RectF};
use ui::uiwire::scene::{self, Scene, state};
use ui::uiwire::{Act, Event, Request, acted, mods};
use ui::{AppEvent, Key, Mods, Sense, UiState};
use wm::{Cmd, State, WinId};

use crate::{Call, Effect, Host, Response, Win, app_of, content_rect, rectf};

/// The overlay's id: the wm hands out ids from 1.
pub const OVERLAY: WinId = WinId(0);
/// The overlay's app; opening it summons the overlay ([`Agent::summon`]), never a window.
pub const ASSISTANT: &str = "assistant";
/// How long an act waits for busy windows, and how long its flash shows.
pub const SETTLE_MS: f64 = 1500.0;
pub const FLASH_MS: f64 = 600.0;
/// What a window's scene keeps at most: hits (and marks), runs, bytes of text; a title's and a
/// mark's value's bytes; and the whole scene's, as sent.
const HITS: usize = 300;
const RUNS: usize = 400;
const TEXT: usize = 16 << 10;
const NAME: usize = 256;
const VALUE: usize = 512;
pub const SCENE: usize = 64 << 10;

/// The act settling: its id and code, its deadline, whether only the deadline ends it (a
/// wait), and whether the timer is armed for it.
struct Pending {
    id: u32,
    code: u16,
    until: f64,
    wait: bool,
    armed: bool,
}

/// The overlay's state in the host: acts asked for, the one settling, and what the shell shows.
#[derive(Default)]
pub struct Agent {
    acts: Vec<(WinId, u32, Act)>,
    pending: Option<Pending>,
    stepping: bool,
    /// Whether the overlay works on a task ([`Request::Status`]).
    pub working: bool,
    /// What the last act touched (a widget, or a window's frame) and when.
    pub flash: Option<(RectF, f64)>,
    /// An app asked for the Assistant: the shell opens the overlay.
    pub summon: bool,
    /// The home screen's apps and the screen's size, as the shell has them, for the scene.
    pub apps: Vec<String>,
    pub screen: (f32, f32),
    /// Where the overlay is laid out while it shows.
    pub shown: Option<RectF>,
}

impl Host {
    /// Makes the overlay's app if it does not run, or again if what it ran failed; whether it
    /// runs.
    pub fn open_overlay(&mut self) -> bool {
        if self.win(OVERLAY).is_some_and(|w| w.app.ended()) {
            self.drop_overlay();
        }
        if self.win(OVERLAY).is_none() {
            let Some(app) = (self.registry)(ASSISTANT) else { return false };
            let (name, hits, sizes) = (ASSISTANT.to_string(), Vec::new(), [None; 2]);
            self.wins.push(Win { id: OVERLAY, app, name, hits, hits_at: RectF::default(), sizes });
        }
        true
    }

    /// Shows the overlay laid out in `layout`, telling it a new size (`None`: hidden; its program
    /// runs on, finishing its task).
    pub fn place_overlay(&mut self, layout: Option<RectF>, out: &mut Response) {
        self.agent.shown = layout;
        let Some(r) = layout else { return };
        let w = self.wins.iter_mut().find(|w| w.id == OVERLAY);
        if w.is_some_and(|w| w.sizes[0].replace((r.w, r.h)) != Some((r.w, r.h))) {
            self.deliver(OVERLAY, AppEvent::Resized { w: r.w, h: r.h }, out);
        }
    }

    /// The person took over: a working overlay hears [`Event::Halt`], its act settling dropped.
    pub fn halt(&mut self, out: &mut Response) {
        if mem::take(&mut self.agent.working) {
            self.agent.pending = None;
            self.deliver(OVERLAY, AppEvent::Agent(Event::Halt), out);
        }
    }

    /// The overlay's program ended: it goes, and so does what it asked.
    pub(crate) fn drop_overlay(&mut self) {
        self.wins.retain(|w| w.id != OVERLAY);
        self.kernel.kill_owned(OVERLAY.0);
        let a = &mut self.agent;
        (a.acts, a.pending, a.working, a.shown) = (Vec::new(), None, false, None);
    }

    /// What window `win`'s app asked of the agent: an act, or the overlay's status.
    pub(crate) fn agent_request(&mut self, win: WinId, req: Request) {
        match req {
            // Bytes off the wire were checked; an app's own may be no act, which is answered
            // as malformed (no theme has no name).
            Request::Act { id, act } => {
                let act = Act::decode(&act).unwrap_or(Act::Theme { name: String::new() });
                self.agent.acts.push((win, id, act));
            }
            Request::Status { working } if win == OVERLAY => {
                self.agent.working = working;
                // The task is over: so is the act it waited for.
                self.agent.pending = self.agent.pending.take().filter(|_| working);
            }
            _ => {}
        }
    }

    /// Runs the acts asked for, then answers the one settling once it settled (module docs).
    pub(crate) fn agent_step(&mut self, out: &mut Response) {
        if mem::replace(&mut self.agent.stepping, true) {
            return;
        }
        while !self.agent.acts.is_empty() {
            let (win, id, act) = self.agent.acts.remove(0);
            if win != OVERLAY || self.agent.pending.is_some() {
                let code = if win == OVERLAY { acted::IN_FLIGHT } else { acted::REFUSED };
                self.answer(win, (id, code), Vec::new(), out);
                continue;
            }
            let wait =
                if let Act::Wait { ms } = act { Some(f64::from(ms.min(5000))) } else { None };
            let code = self.act(act, out);
            let (until, wait) = (self.now_ms + wait.unwrap_or(SETTLE_MS), wait.is_some());
            self.agent.pending = Some(Pending { id, code, until, wait, armed: false });
        }
        if self.agent.pending.is_some() {
            self.settle(out);
        }
        let busy = self.wins.iter().any(|w| self.live(w.id) && w.app.busy());
        let now = self.now_ms;
        if let Some(p) = self.agent.pending.take_if(|p| now >= p.until || !p.wait && !busy) {
            let code = if p.code == acted::OK && busy { acted::BUSY } else { p.code };
            let scene = self.scene().encode();
            self.answer(OVERLAY, (p.id, code), scene, out);
        }
        if let Some(p) = self.agent.pending.as_mut().filter(|p| !p.armed) {
            p.armed = true;
            let ms = (p.until - now).max(0.0) as u32 + 1;
            out.effects.push(Effect::Kernel(kernel::Effect::Wake { ms }));
        }
        self.agent.stepping = false;
    }

    /// The platform's timer fired, perhaps a sooner one in place of the act's (the page keeps
    /// the sooner of two): the act's deadline is asked for again.
    pub(crate) fn woken(&mut self) {
        if let Some(p) = &mut self.agent.pending {
            p.armed = false;
        }
    }

    /// Tells window `win` act `id` settled with `code` and the screen `scene`.
    fn answer(&mut self, win: WinId, (id, code): (u32, u16), scene: Vec<u8>, out: &mut Response) {
        let acted = Event::Acted { id, code, note: String::new(), scene };
        self.call(win, Call::Event(AppEvent::Agent(acted)), out);
    }

    /// The live window `win` names (0: the focused one).
    fn target(&self, win: u32) -> Option<WinId> {
        let w = if win == 0 { self.focused_app() } else { Some(WinId(win)) };
        w.filter(|&w| self.live(w))
    }

    /// Does `act` as a person would; its code.
    fn act(&mut self, act: Act, out: &mut Response) -> u16 {
        match act {
            Act::Wait { .. } => {}
            Act::Open { name } if name == ASSISTANT => return acted::REFUSED,
            Act::Open { name } => {
                if self.icon(&name).is_none() {
                    return acted::UNKNOWN_APP;
                }
                self.show(&name, out);
                self.settle(out);
                self.flash_window(self.focused_app());
            }
            Act::Theme { name } => {
                if !ui::THEMES.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
                    return acted::MALFORMED;
                }
                self.themes.push(name);
            }
            Act::Window { win, op } => {
                let Some(w) = self.target(win) else { return acted::GONE };
                self.flash_window(Some(w));
                let cmd = [Cmd::Focus, Cmd::Close, Cmd::Minimize, Cmd::Maximize, Cmd::Restore];
                self.apply(cmd[op as usize](w));
            }
            act => return self.touch(act, out),
        }
        acted::OK
    }

    /// A click, typing, a key or the wheel into a window, raised and focused first; its code.
    fn touch(&mut self, act: Act, out: &mut Response) -> u16 {
        let (win, id) = match act {
            Act::Click { win, id } | Act::Type { win, id, .. } | Act::Scroll { win, id, .. } => {
                (win, id)
            }
            Act::Key { win, .. } => (win, 0),
            _ => (0, 0),
        };
        let Some(w) = self.target(win) else { return acted::GONE };
        self.apply(Cmd::Focus(w));
        self.settle(out);
        let layout = self.wm.layout();
        let Some(c) = layout.iter().find(|p| p.win == w).map(|p| content_rect(rectf(p.rect)))
        else {
            return acted::GONE;
        };
        // Its hits as it shows now, as a press during motion finds them.
        let (now, theme) = (self.now_ms, self.theme.at(self.now_ms));
        let state = UiState { focused: true, now_ms: now, ..UiState::default() };
        self.draw_content(&mut DrawList::new(), w, [c, c], &theme, state);
        let hit = self.win(w).and_then(|x| x.hits.iter().rev().find(|h| h.id.0 == id).copied());
        let middle = |r: RectF| (r.x + r.w / 2.0 - c.x, r.y + r.h / 2.0 - c.y);
        let ev = match act {
            Act::Key { code, mods: m, .. } => {
                let key = Key::from_code(&code);
                if key == Key::Other {
                    return acted::MALFORMED;
                }
                let b = |bit| m & bit != 0;
                let mods = Mods {
                    shift: b(mods::SHIFT),
                    ctrl: b(mods::CTRL),
                    alt: b(mods::ALT),
                    meta: b(mods::META),
                };
                self.agent.flash = Some((c, now));
                AppEvent::Key { key, mods }
            }
            Act::Scroll { dy, .. } => {
                let r = match hit {
                    _ if !(-3000..=3000).contains(&dy) => return acted::MALFORMED,
                    _ if id == 0 => c,
                    Some(h) => h.rect,
                    None => return acted::OFF_SCREEN,
                };
                let (x, y) = middle(r);
                self.agent.flash = Some((r, now));
                AppEvent::Wheel { x, y, dy: f32::from(dy) }
            }
            act => {
                let Some(h) = hit else { return acted::OFF_SCREEN };
                let typed = if let Act::Type { text, submit, .. } = act {
                    Some((text, submit))
                } else {
                    None
                };
                if typed.is_some() && h.sense != Sense::Text {
                    return acted::NOT_TEXT;
                }
                let (x, y) = middle(h.rect);
                self.agent.flash = Some((h.rect, now));
                self.deliver(w, AppEvent::PointerDown { x, y, id: Some(h.id) }, out);
                match typed {
                    Some((text, submit)) => {
                        self.deliver(w, AppEvent::Text(text), out);
                        if !submit {
                            return acted::OK;
                        }
                        AppEvent::Key { key: Key::Enter, mods: Mods::default() }
                    }
                    None if h.sense == Sense::Click => AppEvent::Click(h.id),
                    None => return acted::OK,
                }
            }
        };
        self.deliver(w, ev, out);
        acted::OK
    }

    /// Flashes the frame of window `w`, if it shows.
    fn flash_window(&mut self, w: Option<WinId>) {
        let at = self.wm.layout().into_iter().find(|p| Some(p.win) == w).map(|p| rectf(p.rect));
        self.agent.flash = at.map(|r| (r, self.now_ms)).or(self.agent.flash);
    }

    /// The screen as the overlay reads it: see the module docs. Windows top first, then the
    /// minimized ones (their titles alone).
    pub fn scene(&mut self) -> Scene {
        let (now, layout) = (self.now_ms, self.wm.layout());
        let theme = self.theme.at(now);
        let name = self.theme.current().name.to_string();
        // The bytes left as sent: the head, the apps, then each window while it fits.
        let mut room = SCENE - 20 - name.len();
        let fit = |a: &&String| take(&mut room, 4 + a.len());
        let apps: Vec<String> = self.agent.apps.iter().take_while(fit).cloned().collect();
        let mut wins = Vec::new();
        for p in layout.iter().rev() {
            let c = content_rect(rectf(p.rect));
            let (mut list, focused) = (DrawList::recording(), p.focused);
            let ui = UiState { focused, now_ms: now, ..UiState::default() };
            self.draw_content(&mut list, p.win, [c, c], &theme, ui);
            let sem = list.take_sem().unwrap_or_default();
            let st = match (p.state, p.snap) {
                (State::Maximized, _) => state::MAX,
                (_, Some(_)) => state::SNAPPED,
                _ => state::FREE,
            };
            let mut w = self.swin(p.win, rectf(p.rect), st);
            if !take(&mut room, 33 + w.app.len() + w.title.len()) {
                break;
            }
            let hits = self.win(p.win).map_or(&[][..], |x| &x.hits);
            let hit =
                |h: &ui::Hit| scene::Hit { id: h.id.0, sense: h.sense as u8, rect: px(h.rect) };
            let fit = |_: &&ui::Hit| take(&mut room, 13);
            w.hits = hits.iter().take(HITS).take_while(fit).map(hit).collect();
            let mark = |m: gfx::Mark| {
                let (id, role, flags, value) = (m.id, m.role, m.flags, clip(m.value, VALUE));
                scene::Mark { id, role, flags, value }
            };
            let fit = |m: &scene::Mark| take(&mut room, 10 + m.value.len());
            w.marks = sem.marks.into_iter().take(HITS).map(mark).take_while(fit).collect();
            let mut text = TEXT;
            let fit =
                |r: &gfx::Run| take(&mut text, r.text.len()) && take(&mut room, 12 + r.text.len());
            let run = |r: gfx::Run| scene::Run { rect: px(r.rect), text: r.text };
            w.runs = sem.runs.into_iter().take(RUNS).take_while(fit).map(run).collect();
            wins.push(w);
        }
        for (id, _) in self.wm.windows().into_iter().filter(|w| w.1 == State::Minimized) {
            let r = self.wm.normal_rect(id).map_or(RectF::default(), rectf);
            let w = self.swin(id, r, state::MIN);
            if take(&mut room, 33 + w.app.len() + w.title.len()) {
                wins.push(w);
            }
        }
        let ((w, h), focus) = (self.agent.screen, self.focused_app().map_or(0, |w| w.0));
        Scene { w: w as u16, h: h as u16, theme: name, focus, apps, wins }
    }

    /// A scene's window `id` at `r` in `state`, with its app and title.
    fn swin(&self, id: WinId, r: RectF, state: u8) -> scene::Win {
        let w = self.win(id);
        let app = w.map_or("", |w| app_of(&w.name)).to_string();
        let title = clip(w.map(|w| w.app.title()).unwrap_or_default(), NAME);
        let app = clip(app, NAME);
        scene::Win { id: id.0, app, title, rect: px(r), state, ..scene::Win::default() }
    }
}

/// Takes `n` bytes of `room`, if it holds them.
fn take(room: &mut usize, n: usize) -> bool {
    room.checked_sub(n).map(|r| *room = r).is_some()
}

/// `s` cut to `max` bytes at most, on a char boundary.
fn clip(mut s: String, max: usize) -> String {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    s.truncate(end);
    s
}

/// `r` in whole logical px, as the scene has rects (`as` saturates).
fn px(r: RectF) -> scene::Rect {
    [r.x, r.y, r.w, r.h].map(|v| v.round() as i16)
}
