//! The compusophyOS UI toolkit: text on a glyph atlas (the `text` crate,
//! re-exported here), an immediate-mode widget builder, the palette, and the
//! [`App`] trait every window hosts.
//!
//! Pure Rust with no browser: the shell builds a [`Ui`] per window per
//! frame, the app draws into it, and the platform uploads the resulting
//! [`gfx::DrawList`] and the [`TextSystem`]'s atlas.
//!
//! # The frame
//!
//! 1. The shell calls [`App::draw`] with a [`Ui`] over the window's content
//!    rect, collecting [`Hit`] regions and glyph instances.
//! 2. If [`TextSystem::take_atlas_reset`] is then true, it builds the frame
//!    once more (the atlas was cleared midway).
//! 3. Input becomes [`AppEvent`]s: the shell hit-tests the pointer against
//!    last frame's hits ([`hit_test`]) and passes the focused window its keys
//!    and text. [`App::event`] returns whether to redraw.
//! 4. An app asks for things outside itself (windows, sockets, fonts)
//!    through its [`Cx`]; the shell drains [`Cx::take_requests`].
//!
//! # Example
//!
//! ```
//! use ui::{Cx, Request, SocketId, parse_pairing};
//!
//! let pairing = parse_pairing(&format!("#token={}&node=8123", "ab".repeat(32))).unwrap();
//! assert_eq!((pairing.port, pairing.token), (8123, [0xab; 32]));
//! let (mut fs, mut next_socket) = (vfs::Vfs::new(), 7);
//! let mut cx = Cx::new(&mut fs, 0.0, Some(pairing), &mut next_socket);
//! let s = cx.connect(pairing.port);
//! cx.send(s, b"hi".to_vec());
//! assert_eq!(cx.take_requests(), [
//!     Request::Connect { socket: SocketId(7), port: 8123 },
//!     Request::Send { socket: s, bytes: b"hi".to_vec() },
//! ]);
//! ```

#![forbid(unsafe_code)]

pub mod theme;
mod widgets;

use std::fmt;

pub use text::{ATLAS_SIZE, FontId, MAX_FALLBACKS, TextStyle, TextSystem};
pub use widgets::{BUTTON_H, FIELD_H, Hit, PAD, SPACING, Sense, Ui, UiState, WidgetId, hit_test};

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
    /// Something happened on a socket the app opened with [`Cx::connect`].
    Ws { socket: SocketId, ev: WsEvent },
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

/// A WebSocket an app opened, numbered by the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SocketId(pub u32);

/// What happened on a socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsEvent {
    /// It connected.
    Open,
    /// A message arrived (a text message as its UTF-8 bytes).
    Data(Vec<u8>),
    /// It closed with this code and reason; the socket is gone.
    Closed { code: u16, reason: String },
    /// It failed; a [`WsEvent::Closed`] follows.
    Error,
}

/// How to reach the local node: the page was opened with
/// `#node=<port>&token=<64 hex digits>`. `Debug` hides the token.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Pairing {
    /// The node's WebSocket port on 127.0.0.1.
    pub port: u16,
    /// The shared secret the node expects.
    pub token: [u8; 32],
}

impl Pairing {
    /// The token as 64 lowercase hex digits.
    pub fn token_hex(&self) -> String {
        self.token.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for Pairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pairing {{ port: {}, token: <hidden> }}", self.port)
    }
}

/// Reads a pairing from a URL fragment (a leading `#` is optional): `&`
/// separated `key=value` pairs in any order, needing `node` (a decimal port
/// in 1..=65535) and `token` (exactly 64 hex digits, either case). Other
/// keys are ignored; a bad or repeated `node` or `token` gives `None`.
pub fn parse_pairing(fragment: &str) -> Option<Pairing> {
    let s = fragment.strip_prefix('#').unwrap_or(fragment);
    let (mut port, mut token) = (None, None);
    for pair in s.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "node" if port.is_none() => port = Some(parse_port(v)?),
            "token" if token.is_none() => token = Some(parse_token(v)?),
            "node" | "token" => return None,
            _ => {}
        }
    }
    let (port, token) = (port?, token?);
    Some(Pairing { port, token })
}

fn parse_port(v: &str) -> Option<u16> {
    if v.is_empty() || v.len() > 5 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let p: u32 = v.parse().ok()?;
    u16::try_from(p).ok().filter(|&p| p != 0)
}

fn parse_token(v: &str) -> Option<[u8; 32]> {
    let v = v.as_bytes();
    if v.len() != 64 {
        return None;
    }
    let nibble = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0; 32];
    for (o, pair) in out.iter_mut().zip(v.chunks_exact(2)) {
        *o = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

/// Something an app asked the shell for, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open an app (a registry name or a `.app` path) in a new window,
    /// floating or tiled.
    Open { name: String, floating: bool },
    /// Close the asking app's window.
    CloseSelf,
    /// Open `ws://127.0.0.1:<port>/` as `socket` (the id [`Cx::connect`]
    /// returned).
    Connect { socket: SocketId, port: u16 },
    /// Send one binary message.
    Send { socket: SocketId, bytes: Vec<u8> },
    /// Close a socket.
    CloseSocket(SocketId),
    /// Fetch the lazy fallback fonts and add them to the [`TextSystem`].
    LoadFallbackFonts,
}

/// What an app can reach while handling an event: the filesystem, the clock,
/// the node pairing, and requests to the shell.
#[derive(Debug)]
pub struct Cx<'a> {
    /// The filesystem.
    pub vfs: &'a mut vfs::Vfs,
    /// Milliseconds on the page clock.
    pub now_ms: f64,
    /// The local node, when the page was opened paired with one.
    pub pairing: Option<Pairing>,
    next_socket: &'a mut u32,
    requests: Vec<Request>,
}

impl<'a> Cx<'a> {
    /// A context over `vfs`. `next_socket` is the shell's socket counter:
    /// [`Cx::connect`] takes its value and increments it.
    pub fn new(
        vfs: &'a mut vfs::Vfs,
        now_ms: f64,
        pairing: Option<Pairing>,
        next_socket: &'a mut u32,
    ) -> Cx<'a> {
        Cx { vfs, now_ms, pairing, next_socket, requests: Vec::new() }
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

    /// Opens a WebSocket to `ws://127.0.0.1:<port>/`; apps cannot name any
    /// other host. Events arrive as [`AppEvent::Ws`] with the returned id.
    pub fn connect(&mut self, port: u16) -> SocketId {
        let socket = SocketId(*self.next_socket);
        *self.next_socket = self.next_socket.wrapping_add(1);
        self.requests.push(Request::Connect { socket, port });
        socket
    }

    /// Sends one binary message on `s`.
    pub fn send(&mut self, s: SocketId, bytes: Vec<u8>) {
        self.requests.push(Request::Send { socket: s, bytes });
    }

    /// Closes `s`.
    pub fn close_socket(&mut self, s: SocketId) {
        self.requests.push(Request::CloseSocket(s));
    }

    /// Asks for the lazy fallback fonts (symbols a terminal needs).
    pub fn load_fallback_fonts(&mut self) {
        self.requests.push(Request::LoadFallbackFonts);
    }

    /// The requests so far, oldest first, leaving none.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }
}

#[cfg(test)]
mod tests;
