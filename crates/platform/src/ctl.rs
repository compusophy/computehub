//! [`Ctl`]: what an app asks of the page while it handles an event or draws.

use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::JsCast;

/// The app's handle on the page, lent to each [`crate::App`] call. Requests queue as [`Effect`]s
/// and apply in order right after the app returns, still inside the DOM event that caused them
/// (phones need that user activation to show a keyboard); results come back as events. Reads are
/// live; natively the clocks read 0 and [`LocalTime::EPOCH`], and storage (local and session)
/// holds only the handle's queued writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ctl {
    pub(crate) effects: Vec<Effect>,
}

/// One queued request of a [`Ctl`], named for the method that queues it (`Wake` for `wake_in`).
/// `RequestFrame` and `Cursor` (a CSS keyword; the last wins) queue at most once per handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    TextInput(bool),
    Fetch { id: u32, url: String },
    RequestFrame,
    Cursor(&'static str),
    Store { key: String, value: String },
    Spawn { pid: u32, sab: bool },
    Start { pid: u32, msg: Vec<u8>, program: Load },
    Send { pid: u32, msg: Vec<u8> },
    Reply { pid: u32, errno: u16, data: Vec<u8> },
    Word { pid: u32, index: u32, value: i32 },
    Kill(u32),
    Wake(u32),
    FrameIn(u32),
    Stream { id: u32, url: String, headers: Vec<(&'static str, String)>, body: Vec<u8> },
    Abort(u32),
    Remove(String),
    Erase(String),
    Session { key: String, value: Option<String> },
    Reload,
    InputMode(bool),
    Derive { id: u32, pin: Vec<u8>, salt: [u8; 16], iterations: u32 },
    Link { id: u32, cert: String, offer: Option<String> },
    Accept { id: u32, answer: String },
    LinkSend { id: u32, data: Vec<u8> },
    Unlink(u32),
    Estimate,
}

/// A load the browser timed ([`Ctl::timings`]): its URL (empty for the page itself), then its
/// `startTime` and `responseEnd` (ms from navigation start), `transferSize` and
/// `encodedBodySize` (bytes); -1 for what the browser does not give.
pub type Timing = (String, [f64; 4]);

/// The program a Start carries ([`Ctl::start`]): none (homed), its bytes,
/// or a page-relative URL the worker fetches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    None,
    Bytes(Vec<u8>),
    Url(String),
}

/// A local date and time to the minute, as `Date` reports it in the user's
/// time zone: month `1..=12`, weekday `0..=6` from Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

impl LocalTime {
    /// 1970-01-01 00:00, a Thursday: what [`Ctl::local_time`] reads natively.
    pub const EPOCH: LocalTime =
        LocalTime { year: 1970, month: 1, day: 1, weekday: 4, hour: 0, minute: 0 };

    /// From `Date`'s getters (`getMonth` counts from 0), each clamped into its field's range.
    pub(crate) fn from_js(y: u32, month0: u32, day: u32, wd: u32, h: u32, m: u32) -> LocalTime {
        let byte = |v: u32, lo: u32, hi: u32| v.clamp(lo, hi) as u8;
        LocalTime {
            year: y.min(u32::from(u16::MAX)) as u16,
            month: byte(month0, 0, 11) + 1,
            day: byte(day, 1, 31),
            weekday: byte(wd, 0, 6),
            hour: byte(h, 0, 23),
            minute: byte(m, 0, 59),
        }
    }

    pub(crate) fn of(d: &js_sys::Date) -> LocalTime {
        let (y, mo, day) = (d.get_full_year(), d.get_month(), d.get_date());
        LocalTime::from_js(y, mo, day, d.get_day(), d.get_hours(), d.get_minutes())
    }
}

/// [`Ctl`] methods that queue one effect each: `doc name(args) => effect;`.
macro_rules! queue {
    ($($(#[$doc:meta])* $f:ident($($a:ident: $t:ty),*) => $fx:expr;)+) => {
        $($(#[$doc])* pub fn $f(&mut self, $($a: $t),*) { self.effects.push($fx); })+
    };
}

impl Ctl {
    /// The requests queued so far, oldest first.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    queue! {
        /// Focuses or blurs the hidden `<textarea>`: while focused, unprevented
        /// printable keys, pastes and IME input arrive as [`crate::Event::Text`].
        /// `true` while focused blurs first, bringing back a dismissed keyboard.
        set_text_input(active: bool) => Effect::TextInput(active);
        /// Fetches `url`, which must be same-origin relative (else it fails without a request); the
        /// 2xx body or an error such as `"HTTP 404"` arrives as [`crate::Event::Fetched`].
        fetch(id: u32, url: &str) => Effect::Fetch { id, url: url.to_owned() };
        /// Stores `value` under `key` in `localStorage`; failures are ignored.
        storage_set(key: &str, value: &str) =>
            Effect::Store { key: key.to_owned(), value: value.to_owned() };
        /// Starts a module Worker (`cpu/worker.js`) for `pid`, with a SAB if `sab`, which needs
        /// [`Ctl::isolated`] (the kernel asks only then): its [`crate::Event::Proc`] and
        /// [`crate::Event::ProcError`] follow. The calls below ignore unknown pids.
        spawn(pid: u32, sab: bool) => Effect::Spawn { pid, sab };
        /// Posts `[sab | null, msg, program]` to the worker of `pid`.
        start(pid: u32, msg: Vec<u8>, program: Load) => Effect::Start { pid, msg, program };
        /// Posts `msg` as a Uint8Array to the worker of `pid`.
        send(pid: u32, msg: Vec<u8>) => Effect::Send { pid, msg };
        /// Answers the blocked worker of `pid` through its SAB: the payload (at
        /// most 64 KiB), LEN and ERRNO, then STATE = 1 and notify.
        reply(pid: u32, errno: u16, data: Vec<u8>) => Effect::Reply { pid, errno, data };
        /// Stores `value` in SAB word `index` (0 to 15) of `pid`, then notifies.
        word(pid: u32, index: u32, value: i32) => Effect::Word { pid, index, value };
        /// Terminates the worker of `pid` and drops its callbacks.
        kill(pid: u32) => Effect::Kill(pid);
        /// Arms the one-shot timer, replacing it: [`crate::Event::Wake`] in `ms`.
        wake_in(ms: u32) => Effect::Wake(ms);
        /// Asks for one frame in `ms` (a timer, then [`Ctl::request_frame`]), replacing the last
        /// such timer: a slow animation's next frame, with no frame loop between. A hidden page
        /// draws no frame until it shows again.
        frame_in(ms: u32) => Effect::FrameIn(ms);
        /// POSTs `body` with `headers` to `url` (any: the caller vouches for it), or GETs it
        /// when `body` is empty; the body streams back as [`crate::Event::Chunk`]s, then a
        /// [`crate::Event::StreamEnd`].
        stream(id: u32, url: &str, headers: Vec<(&'static str, String)>, body: Vec<u8>) =>
            Effect::Stream { id, url: url.to_owned(), headers, body };
        /// Aborts stream `id`, which then says nothing more.
        abort(id: u32) => Effect::Abort(id);
        /// Removes `key` from `localStorage`; failures are ignored.
        storage_remove(key: &str) => Effect::Remove(key.to_owned());
        /// Removes every key that starts with `prefix` from `localStorage` and from
        /// `sessionStorage`; failures are ignored.
        storage_erase(prefix: &str) => Effect::Erase(prefix.to_owned());
        /// Stores `value` under `key` in `sessionStorage` (this tab's, which outlives its reloads),
        /// or removes it (`None`); failures are ignored.
        session_set(key: &str, value: Option<&str>) =>
            Effect::Session { key: key.to_owned(), value: value.map(str::to_owned) };
        /// Reloads the page (`location.reload()`), after what was queued before it.
        reload() => Effect::Reload;
        /// Sets the hidden textarea's `inputmode`: `numeric` (a phone's number pad), or none.
        input_mode(numeric: bool) => Effect::InputMode(numeric);
        /// Derives 32 bytes from `pin` by PBKDF2-HMAC-SHA-256 with `salt` and `iterations`, through
        /// WebCrypto: they, or why not (no `crypto.subtle` on an insecure page), arrive as
        /// [`crate::Event::Derived`] with `id`.
        derive(id: u32, pin: Vec<u8>, salt: [u8; 16], iterations: u32) =>
            Effect::Derive { id, pin, salt, iterations };
        /// Makes link `id` to another tab (`crate::link`): an offer, or with `offer` the answer to
        /// it. Its description arrives as [`crate::Event::Signal`] for the other tab, then
        /// [`crate::Event::Linked`] once its channel opens, or [`crate::Event::Unlinked`].
        /// `cert` is where the page's certificate is kept (the first link's says).
        link(id: u32, cert: &str, offer: Option<String>) =>
            Effect::Link { id, cert: cert.to_owned(), offer };
        /// The other tab's answer to link `id`'s offer.
        accept(id: u32, answer: String) => Effect::Accept { id, answer };
        /// Sends `data` on link `id` (dropped unless its channel is open), as one message.
        link_send(id: u32, data: Vec<u8>) => Effect::LinkSend { id, data };
        /// Closes link `id`, which sends no Unlinked.
        unlink(id: u32) => Effect::Unlink(id);
        /// Asks for the page's storage quota: [`crate::Event::Estimated`].
        estimate() => Effect::Estimate;
    }

    /// Asks for one more frame; from a frame, exactly one after it.
    pub fn request_frame(&mut self) {
        if !self.effects.iter().any(|e| matches!(e, Effect::RequestFrame)) {
            self.effects.push(Effect::RequestFrame);
        }
    }

    /// Shows the CSS cursor `css` over the canvas; the style is written only when it changes.
    pub fn set_cursor(&mut self, css: &'static str) {
        self.effects.retain(|e| !matches!(e, Effect::Cursor(_)));
        self.effects.push(Effect::Cursor(css));
    }

    /// Stores `value` under `key` in `localStorage` now, not queued: whether it took it (not when
    /// full or blocked). Natively there is no storage, and it says yes.
    pub fn storage_put(&mut self, key: &str, value: &str) -> bool {
        !cfg!(target_arch = "wasm32")
            || crate::io::storage()
                .is_some_and(|st| st.set_item(&crate::mount::keyed(key), value).is_ok())
    }

    /// `localStorage[key]`, newest queued write (or removal) first; `None` when absent or
    /// storage is unavailable.
    pub fn storage_get(&self, key: &str) -> Option<String> {
        let queued = self.effects.iter().rev().find_map(|e| match e {
            Effect::Store { key: k, value } if k == key => Some(Some(value.clone())),
            Effect::Remove(k) if k == key => Some(None),
            Effect::Erase(p) if key.starts_with(p.as_str()) => Some(None),
            _ => None,
        });
        if queued.is_some() || !cfg!(target_arch = "wasm32") {
            return queued.flatten();
        }
        crate::io::storage()?.get_item(&crate::mount::keyed(key)).ok().flatten()
    }

    /// `sessionStorage[key]`, newest queued write first; `None` when absent or unavailable.
    pub fn session_get(&self, key: &str) -> Option<String> {
        let queued = self.effects.iter().rev().find_map(|e| match e {
            Effect::Session { key: k, value } if k == key => Some(value.clone()),
            Effect::Erase(p) if key.starts_with(p.as_str()) => Some(None),
            _ => None,
        });
        if queued.is_some() || !cfg!(target_arch = "wasm32") {
            return queued.flatten();
        }
        crate::io::session()?.get_item(&crate::mount::keyed(key)).ok().flatten()
    }

    /// The page's loads as the browser timed them: the page itself, then each resource; none
    /// natively.
    pub fn timings(&self) -> Vec<Timing> {
        let mut out = Vec::new();
        let page = cfg!(target_arch = "wasm32").then(crate::window).flatten();
        let Some(p) = page.and_then(|w| w.performance()) else { return out };
        for ty in ["navigation", "resource"] {
            for e in p.get_entries_by_type(ty).iter() {
                let get = |k: &str| Reflect::get(&e, &k.into()).unwrap_or_default();
                let num = |k: &str| get(k).as_f64().unwrap_or(-1.0);
                let name = if ty == "resource" { get("name").as_string() } else { None };
                let at = ["startTime", "responseEnd", "transferSize", "encodedBodySize"].map(num);
                out.push((name.unwrap_or_default(), at));
            }
        }
        out
    }

    /// Fills `out` from `crypto.getRandomValues`; whether it could (natively it cannot).
    pub fn random(&self, out: &mut [u8]) -> bool {
        let page = cfg!(target_arch = "wasm32").then(crate::window).flatten();
        let Some(w) = page else { return false };
        let crypto = Reflect::get(&w, &"crypto".into()).unwrap_or_default();
        let get = Reflect::get(&crypto, &"getRandomValues".into()).ok();
        let a = Uint8Array::new_with_length(out.len() as u32);
        let got = get.and_then(|f| f.dyn_into::<Function>().ok()?.call1(&crypto, &a).ok());
        out.iter_mut().zip(crate::bytes(&a)).for_each(|(o, b)| *o = b);
        got.is_some()
    }

    /// Each worker's meters by pid, read where it keeps them (no message): the ms it ran (its
    /// compile and the run so far too; wrapping), KB of its wasm memory as of its last wait, and
    /// 1 while it runs (else 0). None for a worker with no SAB, and natively.
    pub fn proc_stats(&self) -> Vec<(u32, [u32; 3])> {
        crate::proc::stats()
    }

    /// `performance.now()`: monotonic milliseconds since the page started.
    pub fn monotonic_ms(&self) -> f64 {
        if !cfg!(target_arch = "wasm32") {
            return 0.0;
        }
        crate::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now())
    }

    /// The local time now, as [`crate::Event::Tick`] reports it.
    pub fn local_time(&self) -> LocalTime {
        if !cfg!(target_arch = "wasm32") {
            return LocalTime::EPOCH;
        }
        LocalTime::of(&js_sys::Date::new_0())
    }

    /// The local time's offset from UTC now, in minutes east (UTC+2 is 120); 0 natively.
    pub fn utc_offset(&self) -> i32 {
        if !cfg!(target_arch = "wasm32") {
            return 0;
        }
        let west = js_sys::Date::new_0().get_timezone_offset();
        if west.is_finite() { -(west as i32) } else { 0 }
    }

    /// Whether the person asks for reduced motion (`prefers-reduced-motion: reduce`); false
    /// natively.
    pub fn reduced_motion(&self) -> bool {
        let query = "(prefers-reduced-motion: reduce)";
        cfg!(target_arch = "wasm32")
            && crate::window()
                .and_then(|w| w.match_media(query).ok().flatten())
                .is_some_and(|m| m.matches())
    }

    /// Whether the page is cross-origin isolated, so workers can share
    /// memory (`crossOriginIsolated`); false natively.
    pub fn isolated(&self) -> bool {
        window_says("crossOriginIsolated")
    }

    /// Whether the page is a secure context (`isSecureContext`: https, or localhost), as
    /// WebCrypto's [`Ctl::derive`] needs; false natively.
    pub fn secure(&self) -> bool {
        window_says("isSecureContext")
    }
}

/// Whether `window[key]` is truthy; false natively.
fn window_says(key: &str) -> bool {
    cfg!(target_arch = "wasm32")
        && crate::window()
            .is_some_and(|w| Reflect::get(&w, &key.into()).is_ok_and(|v| v.is_truthy()))
}

/// Whether `url` can only resolve against the page's origin: no scheme (a
/// `:` before the first `/`, `?` or `#`), no leading `//` or space, and no
/// backslash or control character, which parsers turn into slashes or strip.
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
