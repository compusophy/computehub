//! [`Ctl`]: what an app asks of the page while it handles an event or draws.

/// The app's handle on the page, lent to [`crate::App::event`] and
/// [`crate::App::frame`].
///
/// Requests (text input, fetches, frames, the cursor, storage writes) are
/// queued as [`Effect`]s and applied, in order, right after the app returns,
/// while the DOM event that caused them is still being handled: a request
/// made in a pointer or key handler keeps that event's user activation,
/// which phones need before they show a keyboard. Results come back later as
/// events.
///
/// Reads (the clocks, [`Ctl::storage_get`]) are live. Outside the browser
/// (native tests) the clocks read `0.0` and [`LocalTime::EPOCH`], and
/// storage holds nothing but this handle's own queued writes.
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
    /// [`Ctl::fetch`].
    Fetch { id: u32, url: String },
    /// [`Ctl::request_frame`]; queued at most once per handle.
    RequestFrame,
    /// [`Ctl::set_cursor`]; queued at most once per handle (the last wins).
    Cursor(Cursor),
    /// [`Ctl::storage_set`].
    Store { key: String, value: String },
}

/// The pointer's look over the canvas: one CSS `cursor` keyword each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    /// The arrow (`default`).
    #[default]
    Default,
    /// The hand over something clickable (`pointer`).
    Pointer,
    /// The I-beam over editable or selectable text (`text`).
    Text,
    /// Four arrows: something moves (`move`).
    Move,
    /// An open hand: something can be dragged (`grab`).
    Grab,
    /// A closed hand: something is being dragged (`grabbing`).
    Grabbing,
    /// Resize left and right (`ew-resize`).
    EwResize,
    /// Resize up and down (`ns-resize`).
    NsResize,
    /// Resize along the top-left to bottom-right diagonal (`nwse-resize`).
    NwseResize,
    /// Resize along the top-right to bottom-left diagonal (`nesw-resize`).
    NeswResize,
}

impl Cursor {
    /// The CSS `cursor` keyword.
    pub const fn css(self) -> &'static str {
        match self {
            Cursor::Default => "default",
            Cursor::Pointer => "pointer",
            Cursor::Text => "text",
            Cursor::Move => "move",
            Cursor::Grab => "grab",
            Cursor::Grabbing => "grabbing",
            Cursor::EwResize => "ew-resize",
            Cursor::NsResize => "ns-resize",
            Cursor::NwseResize => "nwse-resize",
            Cursor::NeswResize => "nesw-resize",
        }
    }
}

/// A local date and time of day, to the minute, as the browser's `Date`
/// reports it in the user's time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// The full year, such as 2026.
    pub year: u16,
    /// The month, `1..=12`.
    pub month: u8,
    /// The day of the month, `1..=31`.
    pub day: u8,
    /// The day of the week, `0..=6`, 0 being Sunday.
    pub weekday: u8,
    /// The hour, `0..=23`.
    pub hour: u8,
    /// The minute, `0..=59`.
    pub minute: u8,
}

impl LocalTime {
    /// 1970-01-01 00:00, a Thursday: what [`Ctl::local_time`] reads outside
    /// the browser, and the [`Default`].
    pub const EPOCH: LocalTime =
        LocalTime { year: 1970, month: 1, day: 1, weekday: 4, hour: 0, minute: 0 };

    /// Minutes since local midnight, `0..1440`.
    pub const fn minute_of_day(self) -> u32 {
        self.hour as u32 * 60 + self.minute as u32
    }

    /// From the values of `Date`'s getters (`getMonth` counts from 0), each
    /// clamped into its field's range.
    pub(crate) fn from_js(
        year: u32,
        month0: u32,
        day: u32,
        weekday: u32,
        hour: u32,
        minute: u32,
    ) -> LocalTime {
        let byte = |v: u32, lo: u32, hi: u32| v.clamp(lo, hi) as u8;
        LocalTime {
            year: year.min(u32::from(u16::MAX)) as u16,
            month: byte(month0, 0, 11) + 1,
            day: byte(day, 1, 31),
            weekday: byte(weekday, 0, 6),
            hour: byte(hour, 0, 23),
            minute: byte(minute, 0, 59),
        }
    }

    /// The local time `d` holds.
    pub(crate) fn of(d: &js_sys::Date) -> LocalTime {
        let (y, mo, day) = (d.get_full_year(), d.get_month(), d.get_date());
        LocalTime::from_js(y, mo, day, d.get_day(), d.get_hours(), d.get_minutes())
    }
}

impl Default for LocalTime {
    fn default() -> LocalTime {
        LocalTime::EPOCH
    }
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

    /// Fetches `url`, which must be relative to the page (same origin): a
    /// URL with a scheme, a leading `//`, a backslash or a control character
    /// fails without a request. The body arrives as
    /// [`crate::Event::Fetched`] `{ id, result }`: the bytes of a 2xx
    /// response, else an error message (`"HTTP 404"`, a network error).
    pub fn fetch(&mut self, id: u32, url: &str) {
        let url = url.to_owned();
        self.effects.push(Effect::Fetch { id, url });
    }

    /// Asks for one more frame: [`crate::App::frame`] runs on the next
    /// animation frame. Called from [`crate::App::frame`], it schedules
    /// exactly one frame after this one, so an animation asks on every frame
    /// while it runs and frames stop the moment it stops asking. From
    /// [`crate::App::event`] it does what [`crate::Handled::redraw`] does.
    /// Asking twice in one call asks once.
    pub fn request_frame(&mut self) {
        if !self.effects.contains(&Effect::RequestFrame) {
            self.effects.push(Effect::RequestFrame);
        }
    }

    /// Shows `c` as the pointer over the canvas. The page writes the
    /// canvas's `style.cursor` only when the cursor changes, so setting the
    /// same one on every pointer move costs nothing; within one call, the
    /// last cursor set wins.
    pub fn set_cursor(&mut self, c: Cursor) {
        self.effects.retain(|e| !matches!(e, Effect::Cursor(_)));
        self.effects.push(Effect::Cursor(c));
    }

    /// The value `localStorage` holds for `key`, or `None` when there is
    /// none or storage is unavailable (blocked, or private browsing that
    /// throws). A write this handle has queued ([`Ctl::storage_set`]) is
    /// read back before it reaches the page.
    pub fn storage_get(&self, key: &str) -> Option<String> {
        let queued = self.effects.iter().rev().find_map(|e| match e {
            Effect::Store { key: k, value } if k == key => Some(value.clone()),
            _ => None,
        });
        if queued.is_some() || !cfg!(target_arch = "wasm32") {
            return queued;
        }
        crate::io::storage()?.get_item(key).ok().flatten()
    }

    /// Stores `value` under `key` in `localStorage`. A failure (storage
    /// blocked or full, private browsing that throws) is ignored: storage
    /// is a convenience, never a place the only copy of anything lives.
    pub fn storage_set(&mut self, key: &str, value: &str) {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.effects.push(Effect::Store { key, value });
    }

    /// Milliseconds since the page started (`performance.now()`): monotonic
    /// and sub-millisecond, for animation timing and intervals, never the
    /// time of day.
    pub fn monotonic_ms(&self) -> f64 {
        if !cfg!(target_arch = "wasm32") {
            return 0.0;
        }
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now())
    }

    /// The same clock as [`Ctl::monotonic_ms`].
    pub fn now_ms(&self) -> f64 {
        self.monotonic_ms()
    }

    /// The local date and time of day, to the minute: what
    /// [`crate::Event::Tick`] also reports.
    pub fn local_time(&self) -> LocalTime {
        if !cfg!(target_arch = "wasm32") {
            return LocalTime::EPOCH;
        }
        LocalTime::of(&js_sys::Date::new_0())
    }

    /// Minutes since local midnight, `0..1440`: [`Ctl::local_time`]'s
    /// [`LocalTime::minute_of_day`].
    pub fn local_minutes(&self) -> u32 {
        self.local_time().minute_of_day()
    }
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
