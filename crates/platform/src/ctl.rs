//! [`Ctl`]: what an app asks of the page while it handles an event or draws.

/// The app's handle on the page, lent to [`crate::App::event`] and
/// [`crate::App::frame`].
///
/// Requests (text input, sockets, fetches, clearing the hash) are queued as
/// [`Effect`]s and applied, in order, right after the app returns, while the
/// DOM event that caused them is still being handled: a request made in a
/// pointer or key handler keeps that event's user activation, which phones
/// need before they show a keyboard. Results come back later as events.
///
/// Reads (`location_hash`, `now_ms`, `local_minutes`) are live. Outside the
/// browser (native tests) they return `""`, `0.0` and `0`.
///
/// Native tests can drive an app with [`Ctl::new`] and check
/// [`Ctl::effects`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ctl {
    effects: Vec<Effect>,
}

/// One queued request of a [`Ctl`], as [`Ctl::effects`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// [`Ctl::set_text_input`].
    TextInput(bool),
    /// [`Ctl::ws_open`].
    WsOpen { id: u32, url: String },
    /// [`Ctl::ws_send`].
    WsSend { id: u32, bytes: Vec<u8> },
    /// [`Ctl::ws_close`].
    WsClose { id: u32 },
    /// [`Ctl::fetch`].
    Fetch { id: u32, url: String },
    /// [`Ctl::clear_location_hash`].
    ClearHash,
    /// [`Ctl::guard_unload`].
    GuardUnload(bool),
}

impl Ctl {
    /// A handle with nothing queued.
    pub fn new() -> Ctl {
        Ctl::default()
    }

    /// The requests queued so far, oldest first.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub(crate) fn into_effects(self) -> Vec<Effect> {
        self.effects
    }

    /// Focuses (`true`) or blurs (`false`) the page's hidden `<textarea>`.
    /// While it has focus, printable keys the app does not prevent, pastes
    /// and IME input arrive as [`crate::Event::Text`], and phones show their
    /// keyboard (when asked from a pointer or key handler). Asking for
    /// `true` while already focused blurs and refocuses, which brings back
    /// a keyboard the user dismissed.
    pub fn set_text_input(&mut self, active: bool) {
        self.effects.push(Effect::TextInput(active));
    }

    /// Opens WebSocket `id` (the app picks ids) to `url`, with binary
    /// messages as bytes. An open socket with the same id is closed first
    /// and reports nothing more. Events arrive as [`crate::Event::Ws`]: `Open`,
    /// `Data` per message, then `Error` (maybe) and `Closed` once. A `url`
    /// the browser refuses reports `Error` and `Closed { code: 1006, .. }`.
    pub fn ws_open(&mut self, id: u32, url: &str) {
        let url = url.to_owned();
        self.effects.push(Effect::WsOpen { id, url });
    }

    /// Sends `bytes` as one binary message on socket `id`. Sends before
    /// `Open` are queued and flushed on open, in order; sends to an unknown,
    /// closing or closed socket are dropped.
    pub fn ws_send(&mut self, id: u32, bytes: &[u8]) {
        let bytes = bytes.to_vec();
        self.effects.push(Effect::WsSend { id, bytes });
    }

    /// Closes socket `id` (code 1000). It reports nothing more, not even
    /// `Closed`, and `id` is free for [`Ctl::ws_open`] at once.
    pub fn ws_close(&mut self, id: u32) {
        self.effects.push(Effect::WsClose { id });
    }

    /// Fetches `url`, which must be relative to the page (same origin): a
    /// URL with a scheme, a leading `//`, a backslash or a control character
    /// fails without a request. The body arrives as
    /// [`crate::Event::Fetched`] `{ id, result }`: the bytes of a 2xx
    /// response, else an error message (`"HTTP 404"`, a network error).
    pub fn fetch(&mut self, id: u32, url: &str) {
        let url = url.to_owned();
        self.effects.push(Effect::Fetch { id, url });
    }

    /// `location.hash` without the `#`, as the URL has it (percent-encoded).
    /// [`crate::Event::HashChange`] reports when it changes.
    pub fn location_hash(&self) -> String {
        if !cfg!(target_arch = "wasm32") {
            return String::new();
        }
        web_sys::window().map_or_else(String::new, |w| hash_of(&w))
    }

    /// Removes the hash from the address bar with `history.replaceState`,
    /// keeping the path, query and history state; no `hashchange` fires.
    pub fn clear_location_hash(&mut self) {
        self.effects.push(Effect::ClearHash);
    }

    /// While `on`, leaving the page (closing or reloading the tab, or
    /// following a link) asks the user first: a `beforeunload` listener
    /// calls `preventDefault`, and the browser shows its own "Leave site?"
    /// prompt (only once the user has interacted with the page). Ask for it
    /// while leaving would lose something, such as a live shell, and drop it
    /// after: the listener exists only while asked for, so the page can
    /// still enter the back/forward cache otherwise.
    pub fn guard_unload(&mut self, on: bool) {
        self.effects.push(Effect::GuardUnload(on));
    }

    /// Milliseconds since the page started (`performance.now()`): monotonic,
    /// for measuring intervals, not the time of day.
    pub fn now_ms(&self) -> f64 {
        if !cfg!(target_arch = "wasm32") {
            return 0.0;
        }
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now())
    }

    /// Minutes since local midnight, `0..1440`: the time of day that
    /// [`crate::Event::Tick`] also reports.
    pub fn local_minutes(&self) -> u32 {
        if !cfg!(target_arch = "wasm32") {
            return 0;
        }
        let d = js_sys::Date::new_0();
        d.get_hours() * 60 + d.get_minutes()
    }
}

/// `window.location.hash` without its `#`; `""` if the browser throws.
pub(crate) fn hash_of(w: &web_sys::Window) -> String {
    let hash = w.location().hash().unwrap_or_default();
    strip_hash(&hash).to_owned()
}

/// `hash` without its leading `#`.
pub(crate) fn strip_hash(hash: &str) -> &str {
    hash.strip_prefix('#').unwrap_or(hash)
}

/// Whether `url` is a relative reference that can only resolve against the
/// page's own origin: no scheme (a `:` before the first `/`, `?` or `#`),
/// no leading `//` or space, and no backslash or ASCII control character,
/// which URL parsers turn into slashes or strip (so `/\host` and `/\t/host`
/// would reach another host).
pub(crate) fn is_relative_url(url: &str) -> bool {
    if url.starts_with("//") || url.starts_with(' ') {
        return false;
    }
    let mut head = true;
    for &b in url.as_bytes() {
        match b {
            b'\\' | 0..=0x1f | 0x7f => return false,
            b'/' | b'?' | b'#' => head = false,
            b':' if head => return false,
            _ => {}
        }
    }
    true
}
