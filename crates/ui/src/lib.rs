//! The compusophyOS UI toolkit: text on a glyph atlas (the `text` crate,
//! re-exported), runtime [`Theme`]s, an immediate-mode widget builder and
//! the [`App`] trait every window hosts. Pure Rust, no browser.
//!
//! A frame: the shell calls [`App::draw`] with a [`Ui`] over the window's content rect, collecting
//! [`Hit`] regions and instances (and builds it again if [`TextSystem::take_atlas_reset`] says the
//! atlas was cleared midway). Input comes back as [`AppEvent`]s routed by last frame's hits
//! ([`hit_test`]); [`App::event`] returns whether to redraw, and an app asks for anything outside
//! itself through its [`Cx`]. [`Code`] is the code editor widget; [`icon`] draws vector icons.

#![forbid(unsafe_code)]

mod code;
pub mod icon;
pub mod theme;
mod widgets;

pub use code::{CODE_MAX, Code, Span, push_num};
pub use gfx::Rgba;
pub use kernel;
pub use text::{ATLAS_SIZE, Editor, FontId, MAX_FALLBACKS, TextStyle, TextSystem};
pub use theme::{Glow, IconStyle, THEMES, Theme, theme};
pub use widgets::{BUTTON_H, CARD_PAD, FIELD_H, PAD, RADIUS_LG, RADIUS_SM};
pub use widgets::{Hit, Sense, Ui, UiState, WidgetId, button_width, hit_test};
pub use widgets::{SPACING, SPACING_LG, SPACING_MD};

/// What every window hosts.
pub trait App {
    /// The titlebar text.
    fn title(&self) -> String;
    /// Builds this frame's content.
    fn draw(&mut self, ui: &mut Ui<'_>);
    /// Handles one event; returns whether the window must redraw.
    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool;
    /// Whether keys and text should reach this app (a focused field or a terminal).
    fn wants_text_input(&self) -> bool {
        false
    }
    /// The content size it would like for a floating window, in logical pixels.
    fn preferred_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// Its icon on the home screen, the dock and tiles.
    fn icon(&self) -> AppIcon {
        AppIcon::default()
    }
    /// Whether it is a small card (Welcome, About, Settings): on a wide screen it opens at its
    /// preferred size, centered, never maximized as main apps' first windows are.
    fn compact(&self) -> bool {
        false
    }
    /// Whether it animates at `now_ms` (the page clock of [`UiState::now_ms`]): the shell draws
    /// frames while a shown app does, and only then, so an idle desktop draws none.
    fn animating(&self, now_ms: f64) -> bool {
        let _ = now_ms;
        false
    }
    /// GUI process `pid` drew `frame` (uiwire bytes, unchecked). Every app hears every frame and
    /// takes only its own process's; returns whether to redraw.
    fn frame(&mut self, pid: u32, frame: &[u8], cx: &mut Cx<'_>) -> bool {
        let _ = (pid, frame, cx);
        false
    }
    /// The window is closing; the processes it owns end soon after.
    fn closing(&mut self, cx: &mut Cx<'_>) {
        let _ = cx;
    }
}

/// An app's icon: a vector glyph on a tile tinted by `hue` as the theme
/// paints tiles ([`icon::tile`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppIcon {
    pub glyph: icon::Glyph,
    pub hue: Rgba,
}

impl Default for AppIcon {
    /// A window on slate gray.
    fn default() -> AppIcon {
        AppIcon { glyph: icon::Glyph::Window, hue: Rgba::hex(0x64748b) }
    }
}

/// Input and notifications for an app, in logical pixels relative to its
/// content rect's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// A [`Sense::Click`] widget was pressed and released.
    Click(WidgetId),
    /// The primary button went down, over the topmost hit `id` if any.
    PointerDown { x: f32, y: f32, id: Option<WidgetId> },
    /// A key went down while the window was focused.
    Key { key: Key, mods: Mods },
    /// Text typed, pasted or composed by an IME.
    Text(String),
    /// The wheel turned over the window: positive `dy` scrolls down.
    Wheel { x: f32, y: f32, dy: f32 },
    /// The window gained (`true`) or lost keyboard focus.
    Focus(bool),
    /// The content rect is now `w` x `h`.
    Resized { w: f32, h: f32 },
    /// Time passed (milliseconds on the page clock), for apps that animate.
    Tick { now_ms: f64 },
    /// A process this window owns has console output, exited or changed
    /// mode, or homed has a new note: see [`Cx::kernel`].
    Io,
    /// A prompt for the Assistant, as if typed and sent.
    Ask(String),
}

/// A key by its physical position (`KeyboardEvent.code`); text comes
/// separately as [`AppEvent::Text`]. `Enter` includes numpad Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[rustfmt::skip]
pub enum Key {
    Enter, Escape, Backspace, Delete, Tab, Space, Left, Right, Up, Down, Home, End, PageUp,
    PageDown, Insert,
    /// A function key, `F(1)` to `F(24)`.
    F(u8),
    /// A letter key as a lowercase ASCII letter, or a digit key (top row or
    /// numpad) as an ASCII digit.
    Char(char),
    Other,
}

impl Key {
    /// The key for a `KeyboardEvent.code`: `"KeyA"` is `Char('a')`, `"Digit7"`
    /// and `"Numpad7"` are `Char('7')`, `"F5"` is `F(5)`, unknown is `Other`.
    pub fn from_code(code: &str) -> Key {
        use Key::*;
        const CODES: &str = "Enter Escape Backspace Delete Tab Space ArrowLeft ArrowRight \
            ArrowUp ArrowDown Home End PageUp PageDown Insert NumpadEnter";
        const KEYS: [Key; 16] = [
            Enter, Escape, Backspace, Delete, Tab, Space, Left, Right, Up, Down, Home, End, PageUp,
            PageDown, Insert, Enter,
        ];
        if let Some(i) = CODES.split(' ').position(|c| c == code) {
            return KEYS[i];
        }
        // The one ASCII byte after `prefix`, if that is all there is.
        let last = |prefix| {
            code.strip_prefix(prefix).and_then(|t| (t.len() == 1).then(|| t.as_bytes()[0]))
        };
        match (last("Key"), last("Digit").or(last("Numpad"))) {
            (Some(c @ b'A'..=b'Z'), _) => return Key::Char(c.to_ascii_lowercase().into()),
            (_, Some(c @ b'0'..=b'9')) => return Key::Char(c.into()),
            _ => {}
        }
        match code.strip_prefix('F').map(|n| (n, n.parse::<u8>())) {
            Some((n, Ok(k @ 1..=24))) if !n.starts_with(['0', '+']) => Key::F(k),
            _ => Key::Other,
        }
    }
}

/// Modifier keys held during a key press: `alt` is also Option, `meta` is
/// Command or the Windows key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[rustfmt::skip]
pub struct Mods { pub shift: bool, pub ctrl: bool, pub alt: bool, pub meta: bool }

/// Something an app asked the shell for, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open an app (a registry name or a `.app` path) in a new window.
    Open { name: String, floating: bool },
    /// Close the asking app's window.
    CloseSelf,
    /// Fetch the lazy fallback fonts and add them to the [`TextSystem`].
    LoadFallbackFonts,
    /// Switch the desktop to the named theme (see [`theme()`]).
    SetTheme(String),
    /// Resize the asking app's window to this content size, in logical px.
    Size(u16, u16),
    /// Set the preference `key` to `value` in the page's storage (never the VFS), such as
    /// [`AI_MODEL`]; the page ignores keys it does not know.
    Pref { key: String, value: String },
    /// Send feedback the person typed: `kind` ("bug", "idea" or "love") and their `text`, with
    /// the desktop's context (build, device, windows, recent events) if `context`.
    Feedback { kind: String, text: String, context: bool },
}

/// The preference naming the model the AI answers with ([`AiStatus::model`]).
pub const AI_MODEL: &str = "ai.model";
/// The preference that is `"off"` when automatic error reports are ([`AiStatus::reports_off`]).
pub const REPORTS: &str = "reports";
/// The preference that is `"off"` when the backdrop's grain is still ([`Cx::grain`]).
pub const GRAIN: &str = "grain";

/// What the page tells apps: the model the AI answers with, and how its reports fare.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiStatus {
    pub model: String,
    /// Automatic error reports are off: the [`REPORTS`] preference.
    pub reports_off: bool,
    /// A report is waiting to be sent: the last one did not get through (offline, or the
    /// inbox is not set up yet).
    pub held: bool,
}

/// What an app can reach while handling an event: the filesystem, the kernel (processes it spawns
/// are owned by its window), the clock (milliseconds on the page clock), the AI settings, whether
/// the backdrop's grain lives (the [`GRAIN`] preference), and requests to the shell.
#[derive(Debug)]
pub struct Cx<'a> {
    pub vfs: &'a mut vfs::Vfs,
    pub kernel: &'a mut kernel::Kernel,
    pub now_ms: f64,
    pub ai: AiStatus,
    pub grain: bool,
    requests: Vec<Request>,
}

impl<'a> Cx<'a> {
    pub fn new(vfs: &'a mut vfs::Vfs, kernel: &'a mut kernel::Kernel, now_ms: f64) -> Cx<'a> {
        let (ai, requests) = (AiStatus::default(), Vec::new());
        Cx { vfs, kernel, now_ms, ai, grain: true, requests }
    }

    /// Sets a preference ([`Request::Pref`]); for [`AI_MODEL`] and [`REPORTS`], [`Cx::ai`]
    /// follows at once, as [`Cx::grain`] does for [`GRAIN`].
    pub fn pref(&mut self, key: &str, value: &str) {
        if key == AI_MODEL {
            self.ai.model = value.to_string();
        }
        if key == REPORTS {
            self.ai.reports_off = value == "off";
        }
        if key == GRAIN {
            self.grain = value != "off";
        }
        self.requests.push(Request::Pref { key: key.to_string(), value: value.to_string() });
    }

    /// Opens an app (a registry name or a `.app` path) in a new tiled window.
    pub fn open(&mut self, name: &str) {
        self.requests.push(Request::Open { name: name.to_string(), floating: false });
    }

    pub fn close_self(&mut self) {
        self.requests.push(Request::CloseSelf);
    }

    /// Asks for the lazy fallback fonts (symbols a terminal needs).
    pub fn load_fallback_fonts(&mut self) {
        self.requests.push(Request::LoadFallbackFonts);
    }

    pub fn set_theme(&mut self, name: &str) {
        self.requests.push(Request::SetTheme(name.to_string()));
    }

    /// Sends the feedback the person typed ([`Request::Feedback`]).
    pub fn feedback(&mut self, kind: &str, text: &str, context: bool) {
        let (kind, text) = (kind.to_string(), text.to_string());
        self.requests.push(Request::Feedback { kind, text, context });
    }

    /// Resizes the asking app's window to a `w` x `h` content area.
    pub fn set_size(&mut self, w: u16, h: u16) {
        self.requests.push(Request::Size(w, h));
    }

    /// The requests so far, oldest first, leaving none.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }
}

#[cfg(test)]
mod tests;
