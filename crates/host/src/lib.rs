//! The compusophyOS app host: the window manager and the apps in its
//! windows, one [`ui::App`] per window. Split from `shell`, which draws the
//! panel and window chrome around it and binds keys and the pointer. Pure
//! Rust, no browser.
//!
//! [`Host`] owns the [`wm::Wm`] (changed only through [`wm::Wm::apply`]),
//! the [`ui::TextSystem`], the [`vfs::Vfs`] and the apps. It opens and closes
//! windows, delivers [`ui::AppEvent`]s, carries out the [`ui::Request`]s the
//! apps make, and hands back what only the platform can do as [`Effect`]s in
//! a [`Response`].

#![forbid(unsafe_code)]

use std::mem;

use gfx::{DrawList, RectF};
use ui::theme::{RADIUS, TITLEBAR_H, WINDOW};
use ui::{AppEvent, Cx, Pairing, Request, SocketId, TextSystem, Ui, UiState, WidgetId, WsEvent};
use vfs::Vfs;
use wm::{Cmd, FLOAT_MIN, Outcome, Placement, Rect, WinId, Wm};

/// Makes the app for a window from its name: a built-in such as
/// `"terminal"` or `"launcher"`, or a `.app` path. `None` for a name it does
/// not know, which then opens nothing.
pub type Registry = Box<dyn Fn(&str) -> Option<Box<dyn ui::App>>>;

/// The lazy fonts: fetched on the first [`Request::LoadFallbackFonts`] and
/// added in this order.
const FONT_URLS: [&str; 2] = ["fonts/symbols-a.ttf", "fonts/symbols-b.ttf"];
/// The one app that opens at most once.
const LAUNCHER: &str = "launcher";
/// Rounds of focus and resize events before [`Host::settle`] gives up; the
/// rest waits for the next event or frame.
const SETTLE_PASSES: usize = 8;
/// Windows shorter than this get no titlebar, title, content or buttons.
pub const CHROME_MIN_H: f32 = TITLEBAR_H + RADIUS;

/// Something only the platform can do. Ids are unique across all effects:
/// socket ids are the [`SocketId`]s apps hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Open a WebSocket; report what happens as `shell::Input::Ws` with `id`.
    WsOpen { id: u32, url: String },
    /// Send one binary message.
    WsSend { id: u32, bytes: Vec<u8> },
    /// Close the socket. No more events are routed for it.
    WsClose { id: u32 },
    /// Fetch `url` (relative to the page) and hand the bytes to
    /// `shell::Shell::fetched` ([`Host::fetched`]) with `id`.
    Fetch { id: u32, url: String },
}

/// What the platform should do after an input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// The screen changed: draw a new frame. Set when the wm, a hovered or
    /// pressed button or widget, the clock or the fonts changed, when the
    /// event handler of an app on the active workspace asked, and on resize.
    pub redraw: bool,
    /// The shell used the event: the platform should `preventDefault` it.
    /// Set for every binding (even one the wm answers with `Noop`), every
    /// key or text an app takes, the wheel over a window, and every pointer
    /// move, press and release.
    pub consumed: bool,
    /// Whether the focused app wants text input, when that or the focus
    /// changed: focus or blur the platform's text element to match.
    pub text_input: Option<bool>,
    /// What to do, in order.
    pub effects: Vec<Effect>,
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
}

/// A lazy font on its way, by its fetch id.
enum Load {
    Pending,
    Ready(Vec<u8>),
    Done,
}

/// The window manager and the apps in its windows, and what they share:
/// the text system, the filesystem, the node pairing, the page clock, and
/// the sockets and fonts they asked for.
pub struct Host {
    wm: Wm,
    text: TextSystem,
    vfs: Vfs,
    registry: Registry,
    pairing: Option<Pairing>,
    /// The apps, by window id (ids only grow, so pushing keeps them sorted).
    wins: Vec<Win>,
    /// Open sockets and the windows that own them.
    sockets: Vec<(SocketId, WinId)>,
    /// The next socket or fetch id.
    next_id: u32,
    /// The lazy fonts, once asked for.
    fonts: Vec<(u32, Load)>,
    /// The window that last got `Focus(true)`.
    focus: Option<WinId>,
    now_ms: f64,
}

impl Host {
    /// A host over `wm` with no apps yet; `registry` makes them by name.
    pub fn new(
        wm: Wm,
        text: TextSystem,
        vfs: Vfs,
        registry: Registry,
        pairing: Option<Pairing>,
    ) -> Host {
        Host {
            wm,
            text,
            vfs,
            registry,
            pairing,
            wins: Vec::new(),
            sockets: Vec::new(),
            next_id: 1,
            fonts: Vec::new(),
            focus: None,
            now_ms: 0.0,
        }
    }

    /// The window manager, read-only: change it through [`Host::apply`].
    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// Applies `cmd` to the wm. Errors leave the wm untouched, and a stale
    /// id is not worth reporting.
    pub fn apply(&mut self, cmd: Cmd) {
        let _ = self.wm.apply(cmd);
    }

    /// The filesystem the apps share.
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    /// The text system the apps draw with.
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

    /// Stores `p` for every app's [`ui::Cx`].
    pub fn set_pairing(&mut self, p: Pairing) {
        self.pairing = Some(p);
    }

    /// The apps, by window id.
    pub fn wins(&self) -> &[Win] {
        &self.wins
    }

    /// The app in window `id`.
    pub fn win(&self, id: WinId) -> Option<&Win> {
        self.wins.iter().find(|w| w.id == id)
    }

    /// The focused window, if an app lives in it.
    pub fn focused_app(&self) -> Option<WinId> {
        self.wm.focused().filter(|&w| self.win(w).is_some())
    }

    /// Opens `name` in a new window, unless the registry does not know it;
    /// a second launcher focuses the first instead. A floating window gets
    /// its app's [`ui::App::preferred_size`] as its content size, centered.
    pub fn open(&mut self, name: &str, floating: bool, out: &mut Response) {
        let open = self.wins.iter().find(|w| w.name == name);
        if let Some(win) = open.filter(|_| name == LAUNCHER).map(|w| w.id) {
            self.apply(Cmd::Focus(win));
            return;
        }
        let Some(app) = (self.registry)(name) else {
            return;
        };
        let Ok(Outcome::Opened(win)) = self.wm.apply(Cmd::Open { floating }) else {
            return;
        };
        let rect = app.preferred_size().and_then(|s| self.centered(s));
        if let (true, Some(rect)) = (floating, rect) {
            self.apply(Cmd::SetFloatRect { win, rect });
        }
        self.wins.push(Win {
            id: win,
            app,
            name: name.to_string(),
            hits: Vec::new(),
            sizes: [None; 2],
        });
        out.redraw = true;
    }

    /// A window rect whose content is `w` x `h`, centered in the wm area and
    /// no bigger than it.
    fn centered(&self, (w, h): (f32, f32)) -> Option<Rect> {
        if !(w.is_finite() && h.is_finite()) {
            return None;
        }
        let a = self.wm.area();
        let fit = |len: f32, max: i32| (len.round() as i32).clamp(FLOAT_MIN, max.max(FLOAT_MIN));
        let (w, h) = (fit(w + 2.0, a.w), fit(h + TITLEBAR_H + 1.0, a.h));
        Some(Rect::new(a.x + (a.w - w) / 2, a.y + (a.h - h) / 2, w, h))
    }

    /// Closes `win` and drops its app.
    pub fn close(&mut self, win: WinId, out: &mut Response) {
        self.apply(Cmd::Close(win));
        self.forget(win, out);
    }

    /// Drops the app of `win`, once the wm has no such window, and closes
    /// its sockets.
    fn forget(&mut self, win: WinId, out: &mut Response) {
        let i = self.wins.iter().position(|w| w.id == win);
        let Some(i) = i.filter(|_| self.wm.workspace_of(win).is_none()) else {
            return;
        };
        self.wins.remove(i);
        let ids = self.sockets.iter().filter(|s| s.1 == win).map(|s| s.0.0);
        out.effects.extend(ids.map(|id| Effect::WsClose { id }));
        self.sockets.retain(|s| s.1 != win);
        out.redraw = true;
    }

    /// Hands `ev` to the app of `win`, then carries out what it asked for.
    /// A hidden app's redraw is moot: switching to it draws it anyway.
    pub fn deliver(&mut self, win: WinId, ev: AppEvent, out: &mut Response) {
        let shown = self.wm.workspace_of(win) == Some(self.wm.active_workspace());
        let Some(w) = self.wins.iter_mut().find(|w| w.id == win) else {
            return;
        };
        let mut cx = Cx::new(&mut self.vfs, self.now_ms, self.pairing, &mut self.next_id);
        out.redraw |= w.app.event(ev, &mut cx) && shown;
        for request in cx.take_requests() {
            self.request(win, request, out);
        }
    }

    /// Carries out one request of the app in `from`; nothing once that
    /// window is gone (it closed itself earlier in the same batch).
    fn request(&mut self, from: WinId, request: Request, out: &mut Response) {
        if self.win(from).is_none() {
            return;
        }
        let owned = |s: &Host, socket| s.sockets.contains(&(socket, from));
        match request {
            Request::Open { name, floating } => self.open(&name, floating, out),
            Request::CloseSelf => self.close(from, out),
            Request::Connect { socket, port } => {
                self.sockets.push((socket, from));
                let url = ["ws://127.0.0.1:", &port.to_string(), "/"].concat();
                out.effects.push(Effect::WsOpen { id: socket.0, url });
            }
            Request::Send { socket, bytes } if owned(self, socket) => {
                let id = socket.0;
                out.effects.push(Effect::WsSend { id, bytes });
            }
            Request::CloseSocket(socket) if owned(self, socket) => {
                self.sockets.retain(|s| s.0 != socket);
                out.effects.push(Effect::WsClose { id: socket.0 });
            }
            Request::LoadFallbackFonts => self.load_fonts(out),
            _ => {}
        }
    }

    /// Asks for the lazy fonts, once, unless fallbacks are already loaded.
    fn load_fonts(&mut self, out: &mut Response) {
        if !self.fonts.is_empty() || self.text.fallback_count() > 0 {
            return;
        }
        for url in FONT_URLS {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            let url = url.to_string();
            self.fonts.push((id, Load::Pending));
            out.effects.push(Effect::Fetch { id, url });
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

    /// Time passed: `now_ms` on the page clock (ignored unless finite).
    /// Every app gets the tick.
    pub fn tick(&mut self, now_ms: f64, out: &mut Response) {
        self.set_now(now_ms);
        let wins: Vec<WinId> = self.wins.iter().map(|w| w.id).collect();
        for win in wins {
            let now_ms = self.now_ms;
            self.deliver(win, AppEvent::Tick { now_ms }, out);
        }
    }

    /// A socket event goes to the window that opened the socket; a closed
    /// socket is forgotten.
    pub fn ws(&mut self, socket: SocketId, ev: WsEvent, out: &mut Response) {
        let owner = self.sockets.iter().find(|s| s.0 == socket).map(|s| s.1);
        if let WsEvent::Closed { .. } = ev {
            self.sockets.retain(|s| s.0 != socket);
        }
        if let Some(win) = owner {
            self.deliver(win, AppEvent::Ws { socket, ev }, out);
        }
    }

    /// Brings the apps up to date with the wm: drops apps whose windows are
    /// gone, then sends focus changes and new content sizes, until nothing
    /// changes (or 8 rounds).
    pub fn settle(&mut self, out: &mut Response) {
        for _ in 0..SETTLE_PASSES {
            let wins: Vec<WinId> = self.wins.iter().map(|w| w.id).collect();
            wins.into_iter().for_each(|win| self.forget(win, out));
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
            let every = (0..self.wm.workspace_count()).flat_map(|ws| self.wm.layout_of(ws));
            let rects: Vec<_> = every.map(|p| (p.win, p.rect)).collect();
            for (win, r) in rects {
                let c = content_rect(rectf(r));
                let w = self.wins.iter_mut().find(|w| w.id == win);
                if w.is_some_and(|w| w.sizes[0].replace((c.w, c.h)) != Some((c.w, c.h))) {
                    calm = false;
                    self.deliver(win, AppEvent::Resized { w: c.w, h: c.h }, out);
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

    /// The content well of window `p`, then its app draws into it and
    /// leaves its hits; `hover` and `pressed` are its widgets under the
    /// pointer and held down.
    pub fn draw_content(
        &mut self,
        list: &mut DrawList,
        p: &Placement,
        hover: Option<WidgetId>,
        pressed: Option<WidgetId>,
    ) {
        let c = content_rect(rectf(p.rect));
        let Some(w) = self.wins.iter_mut().find(|w| w.id == p.win) else {
            return;
        };
        w.hits.clear();
        if c.w <= 0.0 || c.h <= 0.0 {
            return;
        }
        // Rounded to meet the window's lower corners; the body under the
        // upper ones is the same color.
        list.fill(c, RADIUS - 1.0, WINDOW);
        let state = UiState { hover, pressed, focused: p.focused, now_ms: self.now_ms };
        let mut ui = Ui::new(list, &mut self.text, c, &mut w.hits, state);
        w.app.draw(&mut ui);
    }
}

/// A wm rect in draw-list coordinates.
pub fn rectf(r: Rect) -> RectF {
    RectF::from_i32(r.x, r.y, r.w, r.h)
}

/// Where a window of rect `r` shows its app: below the titlebar, inset 1 px
/// from the border; no height when the window gets no chrome.
pub fn content_rect(r: RectF) -> RectF {
    let h = [0.0, r.h - TITLEBAR_H - 1.0][usize::from(r.h >= CHROME_MIN_H)];
    RectF::new(r.x + 1.0, r.y + TITLEBAR_H, (r.w - 2.0).max(0.0), h)
}

#[cfg(test)]
mod tests;
