//! [`Ctl`]: what an app asks of the page while it handles an event or draws.

/// The app's handle on the page, lent to each [`crate::App`] call. Requests
/// queue as [`Effect`]s and apply in order right after the app returns, still
/// inside the DOM event that caused them (phones need that user activation to
/// show a keyboard); results come back as events. Reads are live; natively
/// the clocks read 0 and [`LocalTime::EPOCH`], and storage holds only the
/// handle's queued writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ctl {
    pub(crate) effects: Vec<Effect>,
}

/// One queued request of a [`Ctl`], named for the method that queues it.
/// `RequestFrame` and `Cursor` (a CSS keyword; the last wins) queue at most
/// once per handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    TextInput(bool),
    Fetch { id: u32, url: String },
    RequestFrame,
    Cursor(&'static str),
    Store { key: String, value: String },
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

    /// From `Date`'s getters (`getMonth` counts from 0), each clamped into
    /// its field's range.
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

impl Ctl {
    /// The requests queued so far, oldest first.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// Focuses or blurs the hidden `<textarea>`: while focused, unprevented
    /// printable keys, pastes and IME input arrive as [`crate::Event::Text`].
    /// `true` while focused blurs first, bringing back a dismissed keyboard.
    pub fn set_text_input(&mut self, active: bool) {
        self.effects.push(Effect::TextInput(active));
    }

    /// Fetches `url`, which must be same-origin relative (else it fails
    /// without a request); the 2xx body or an error such as `"HTTP 404"`
    /// arrives as [`crate::Event::Fetched`].
    pub fn fetch(&mut self, id: u32, url: &str) {
        self.effects.push(Effect::Fetch { id, url: url.to_owned() });
    }

    /// Asks for one more frame; from a frame, exactly one after it.
    pub fn request_frame(&mut self) {
        if !self.effects.contains(&Effect::RequestFrame) {
            self.effects.push(Effect::RequestFrame);
        }
    }

    /// Shows the CSS cursor `css` over the canvas; the style is written only
    /// when it changes.
    pub fn set_cursor(&mut self, css: &'static str) {
        self.effects.retain(|e| !matches!(e, Effect::Cursor(_)));
        self.effects.push(Effect::Cursor(css));
    }

    /// `localStorage[key]`, newest queued write first; `None` when absent or
    /// storage is unavailable.
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

    /// Stores `value` under `key` in `localStorage`; failures are ignored.
    pub fn storage_set(&mut self, key: &str, value: &str) {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.effects.push(Effect::Store { key, value });
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
