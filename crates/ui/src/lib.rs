//! The compusophyOS UI toolkit: text on a glyph atlas (the `text` crate,
//! re-exported), runtime [`Theme`]s, an immediate-mode widget builder and
//! the [`App`] trait every window hosts. Pure Rust, no browser.
//!
//! A frame: the shell calls [`App::draw`] with a [`Ui`] over the window's
//! content rect, collecting [`Hit`] regions and instances (and builds it again
//! if [`TextSystem::take_atlas_reset`] says the atlas was cleared midway).
//! Input comes back as [`AppEvent`]s routed by last frame's hits
//! ([`hit_test`]); [`App::event`] returns whether to redraw, and an app asks
//! for anything outside itself through its [`Cx`].

#![forbid(unsafe_code)]

pub mod theme;
mod widgets;

pub use gfx::Rgba;
pub use text::{ATLAS_SIZE, FontId, MAX_FALLBACKS, TextStyle, TextSystem};
pub use theme::{Glow, THEMES, Theme, theme};
pub use widgets::{BUTTON_H, CARD_PAD, FIELD_H, PAD, RADIUS_LG, RADIUS_SM};
pub use widgets::{Hit, Sense, Ui, UiState, WidgetId, hit_test};
pub use widgets::{SPACING, SPACING_LG, SPACING_MD, TILE_H, TILE_ICON, TILE_W};

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
    /// Its icon on launchers, docks and tiles.
    fn icon(&self) -> AppIcon {
        AppIcon::default()
    }
}

/// An app's tile ([`Ui::tile`]): one to three chars (empty: the name's first
/// letter) in white on a gradient of `hue`, such as `">_"` for a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppIcon {
    pub glyph: &'static str,
    pub hue: Rgba,
}

impl Default for AppIcon {
    /// No glyph on slate gray.
    fn default() -> AppIcon {
        AppIcon { glyph: "", hue: Rgba::hex(0x64748b) }
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
}

/// A key by its physical position (`KeyboardEvent.code`); text comes
/// separately as [`AppEvent::Text`]. `Enter` includes numpad Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
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
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

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
}

/// What an app can reach while handling an event: the filesystem, the clock
/// (milliseconds on the page clock) and requests to the shell.
#[derive(Debug)]
pub struct Cx<'a> {
    pub vfs: &'a mut vfs::Vfs,
    pub now_ms: f64,
    requests: Vec<Request>,
}

impl<'a> Cx<'a> {
    pub fn new(vfs: &'a mut vfs::Vfs, now_ms: f64) -> Cx<'a> {
        Cx { vfs, now_ms, requests: Vec::new() }
    }

    /// Opens an app (a registry name or a `.app` path) in a new tiled window.
    pub fn open(&mut self, name: &str) {
        self.open_as(name, false);
    }

    /// Opens an app in a new floating window.
    pub fn open_floating(&mut self, name: &str) {
        self.open_as(name, true);
    }

    fn open_as(&mut self, name: &str, floating: bool) {
        self.requests.push(Request::Open { name: name.to_string(), floating });
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

    /// The requests so far, oldest first, leaving none.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }
}

#[cfg(test)]
mod tests;
