//! The compusophyOS UI toolkit: text on a glyph atlas (the `text` crate,
//! re-exported here), runtime [`Theme`]s, an immediate-mode widget builder,
//! and the [`App`] trait every window hosts.
//!
//! Pure Rust with no browser: the shell builds a [`Ui`] per window per
//! frame, the app draws into it, and the platform uploads the resulting
//! [`gfx::DrawList`] and the [`TextSystem`]'s atlas.
//!
//! # The frame
//!
//! 1. The shell calls [`App::draw`] with a [`Ui`] over the window's content
//!    rect in the current [`Theme`], collecting [`Hit`] regions and glyph
//!    instances.
//! 2. If [`TextSystem::take_atlas_reset`] is then true, it builds the frame
//!    once more (the atlas was cleared midway).
//! 3. Input becomes [`AppEvent`]s: the shell hit-tests the pointer against
//!    last frame's hits ([`hit_test`]) and passes the focused window its keys
//!    and text. [`App::event`] returns whether to redraw.
//! 4. An app asks for things outside itself (windows, fonts, the theme)
//!    through its [`Cx`]; the shell drains [`Cx::take_requests`].
//!
//! # Example
//!
//! ```
//! use ui::{Cx, Request};
//!
//! let mut fs = vfs::Vfs::new();
//! let mut cx = Cx::new(&mut fs, 0.0);
//! cx.open_floating("launcher");
//! cx.set_theme("Dawn");
//! assert_eq!(cx.take_requests(), [
//!     Request::Open { name: "launcher".into(), floating: true },
//!     Request::SetTheme("Dawn".into()),
//! ]);
//! assert_eq!(ui::theme("dawn").name, "Dawn");
//! assert_eq!(ui::theme("no such theme").name, "Midnight");
//! ```

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
    /// Whether keys and text should reach this app (it has a focused text
    /// field or is a terminal), so the shell keeps its text input focused.
    fn wants_text_input(&self) -> bool {
        false
    }
    /// The content size it would like for a floating window, in logical
    /// pixels.
    fn preferred_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// Its icon on launchers, docks and tiles.
    fn icon(&self) -> AppIcon {
        AppIcon::default()
    }
}

/// How an app looks on a tile ([`Ui::tile`]): a short glyph drawn in white
/// on a gradient of `hue`, such as `">_"` for a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppIcon {
    /// One to three chars; empty draws the first letter of the app's name.
    pub glyph: &'static str,
    /// The tile's color.
    pub hue: Rgba,
}

impl Default for AppIcon {
    /// No glyph on slate gray.
    fn default() -> AppIcon {
        AppIcon { glyph: "", hue: Rgba::hex(0x64748b) }
    }
}

/// Input and notifications for an app. Coordinates are logical pixels
/// relative to the top-left corner of the content rect (the [`Ui`] rect the
/// app last drew into), as the shell sends them.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    /// A [`Sense::Click`] widget was pressed and released.
    Click(WidgetId),
    /// The primary button went down at `(x, y)` (relative to the content
    /// rect's corner), over the topmost hit `id` if any.
    PointerDown { x: f32, y: f32, id: Option<WidgetId> },
    /// A key went down while the window was focused.
    Key { key: Key, mods: Mods },
    /// Text typed, pasted or composed by an IME.
    Text(String),
    /// The wheel turned with the pointer at `(x, y)` over the window:
    /// positive `dy` scrolls down, in logical pixels.
    Wheel { x: f32, y: f32, dy: f32 },
    /// The window gained (`true`) or lost keyboard focus.
    Focus(bool),
    /// The content rect is now `w` x `h`.
    Resized { w: f32, h: f32 },
    /// Time passed (milliseconds on the page clock); for apps that animate.
    Tick { now_ms: f64 },
}

/// A key by its physical position (`KeyboardEvent.code`); text comes
/// separately as [`AppEvent::Text`]. Named variants are the keys they name
/// (`Enter` includes numpad Enter; `Left` to `Down` are the arrows).
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
    /// Any other key.
    Other,
}

impl Key {
    /// The key for a `KeyboardEvent.code` value: `"KeyA"` is `Char('a')`,
    /// `"Digit7"` and `"Numpad7"` are `Char('7')`, `"ArrowLeft"` is `Left`,
    /// `"F5"` is `F(5)`, and anything unknown is `Other`.
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
    /// Open an app (a registry name or a `.app` path) in a new window,
    /// floating or tiled.
    Open { name: String, floating: bool },
    /// Close the asking app's window.
    CloseSelf,
    /// Fetch the lazy fallback fonts and add them to the [`TextSystem`].
    LoadFallbackFonts,
    /// Switch the desktop to the theme with this name (see [`theme()`]).
    SetTheme(String),
}

/// What an app can reach while handling an event: the filesystem, the clock,
/// and requests to the shell.
#[derive(Debug)]
pub struct Cx<'a> {
    /// The filesystem.
    pub vfs: &'a mut vfs::Vfs,
    /// Milliseconds on the page clock.
    pub now_ms: f64,
    requests: Vec<Request>,
}

impl<'a> Cx<'a> {
    /// A context over `vfs` at `now_ms` on the page clock.
    pub fn new(vfs: &'a mut vfs::Vfs, now_ms: f64) -> Cx<'a> {
        Cx { vfs, now_ms, requests: Vec::new() }
    }

    /// Opens an app (a registry name or a `.app` path) in a new tiled
    /// window.
    pub fn open(&mut self, name: &str) {
        self.open_as(name, false);
    }

    /// Opens an app in a new floating window.
    pub fn open_floating(&mut self, name: &str) {
        self.open_as(name, true);
    }

    fn open_as(&mut self, name: &str, floating: bool) {
        let name = name.to_string();
        self.requests.push(Request::Open { name, floating });
    }

    /// Closes this app's window.
    pub fn close_self(&mut self) {
        self.requests.push(Request::CloseSelf);
    }

    /// Asks for the lazy fallback fonts (symbols a terminal needs).
    pub fn load_fallback_fonts(&mut self) {
        self.requests.push(Request::LoadFallbackFonts);
    }

    /// Switches the desktop to the theme named `name` (see [`theme()`]).
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
