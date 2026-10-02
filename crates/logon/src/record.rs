//! The record of the real start: the stages the browser timed, from navigation start, as the
//! hairline under the welcome's mark, its caption and its card. Nothing is simulated: a stage
//! shows once it is over, at its measured times, gaps kept (between the page and its first
//! script the browser was parsing).
//!
//! | stage | from | to | bytes |
//! |---|---|---|---|
//! | Page | navigation start | the page's `responseEnd` | the page's |
//! | compusophyOS | the first `startTime` of `os.js`, `os_bg.wasm` | the last `responseEnd` | theirs |
//! | Start | `os_bg.wasm`'s `responseEnd` | the first frame | none (the 100 ms budget) |
//! | Fonts | the first frame | the last deferred font landed | theirs |
//!
//! Sizes are `encodedBodySize` where something crossed the network (`transferSize` > 0); a value
//! the browser does not give is `—`. Numbers are integer text, no float formatting.

use host::motion::ease;

/// A load the browser timed: its URL (empty for the page), then `startTime` and `responseEnd`
/// (ms from navigation start), `transferSize` and `encodedBodySize` (bytes); -1 when not given.
pub type Timing = (String, [f64; 4]);

/// The stages' names, in order.
pub const STAGES: [&str; 4] = ["Page", "compusophyOS", "Start", "Fonts"];
/// The first frame's budget after the wasm arrives, in ms; how long a new segment fades in.
pub const BUDGET: f64 = 100.0;
pub const FADE_MS: f64 = 233.0;

/// What the start is known to have been: the timings as last read, when the first frame was
/// drawn, each deferred font (Inter SemiBold, JetBrains Mono) as it landed (loaded or not,
/// and when), and the line's scale before the fonts' segment rescaled it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    timings: Vec<Timing>,
    pub first: Option<f64>,
    pub fonts: [Option<(bool, f64)>; 2],
}

/// A stage over: its index in [`STAGES`], from and to (ms from navigation start), its bytes
/// over the network (`None`: from the cache, or not a download), when it was learned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub i: usize,
    pub from: f64,
    pub to: f64,
    pub bytes: Option<f64>,
    pub learned: f64,
}

impl Record {
    /// The first frame was drawn at `now`; the timings then.
    pub fn first(&mut self, now: f64, timings: Vec<Timing>) {
        (self.first, self.timings) = (Some(now), timings);
    }

    /// Deferred font `i` landed at `now` (`ok`: it loaded), with the timings then; once both
    /// have, the start's note for the report ring: `boot ready <ms> ff <ms> net <KB>`.
    pub fn font(&mut self, i: usize, ok: bool, now: f64, timings: Vec<Timing>) -> Option<String> {
        if let Some(f) = self.fonts.get_mut(i) {
            *f = Some((ok, now));
        }
        self.timings = timings;
        let start = self.stages().into_iter().find(|s| s.i == 2).map_or(-1.0, |s| s.to - s.from);
        let mut n = String::from("boot ready ");
        for (v, then) in [(self.ready(), " ff "), (start, " net "), (self.net() / 1024.0, "")] {
            push(&mut n, v);
            n.push_str(then);
        }
        self.done().then_some(n)
    }

    /// Whether every deferred font landed.
    pub fn done(&self) -> bool {
        self.fonts.iter().all(Option::is_some)
    }

    /// The timings of the loads whose URL ends in one of `ends` (the page's: `""`).
    fn loads<'a>(&'a self, ends: &'a [&str]) -> impl Iterator<Item = &'a [f64; 4]> + 'a {
        let page = |n: &str| n.is_empty() && ends.contains(&"");
        let of = move |n: &str| page(n) || ends.iter().any(|e| !e.is_empty() && n.ends_with(e));
        self.timings.iter().filter(move |t| of(&t.0)).map(|t| &t.1)
    }

    /// What crossed the network for `loads`: `None` if nothing did (the cache) or the browser
    /// says nothing of it.
    fn bytes<'a>(loads: impl Iterator<Item = &'a [f64; 4]>) -> Option<f64> {
        let sum = loads.filter(|t| t[2] > 0.0 && t[3] >= 0.0).map(|t| t[3]).sum::<f64>();
        (sum > 0.0).then_some(sum)
    }

    /// The stages over, in order (each needs what the browser gave for it).
    pub fn stages(&self) -> Vec<Stage> {
        let Some(first) = self.first else { return Vec::new() };
        let mut out = Vec::new();
        let span = |ends: &[&str]| {
            let (mut from, mut to) = (f64::INFINITY, f64::NEG_INFINITY);
            for t in self.loads(ends) {
                (from, to) = (from.min(t[0]), to.max(t[1]));
            }
            (from >= 0.0 && to >= from).then(|| (from, to, Record::bytes(self.loads(ends))))
        };
        if let Some((_, to, bytes)) = span(&[""]) {
            out.push(Stage { i: 0, from: 0.0, to, bytes, learned: first });
        }
        if let Some((from, to, bytes)) = span(&["/os.js", "/os_bg.wasm"]) {
            out.push(Stage { i: 1, from, to, bytes, learned: first });
        }
        if let Some((_, wasm, _)) = span(&["/os_bg.wasm"]).filter(|s| s.1 <= first) {
            out.push(Stage { i: 2, from: wasm, to: first, bytes: None, learned: first });
        }
        if let (true, Some(last)) = (self.done(), self.fonts_end()) {
            let bytes =
                Record::bytes(self.loads(&["/Inter-SemiBold.ttf", "/JetBrainsMono-Regular.ttf"]));
            out.push(Stage { i: 3, from: first, to: last, bytes, learned: last });
        }
        out
    }

    /// When the last deferred font landed, once any has.
    fn fonts_end(&self) -> Option<f64> {
        self.fonts.iter().flatten().map(|f| f.1).reduce(f64::max)
    }

    /// Navigation start to the last end known: the first frame, or the fonts' landing.
    pub fn ready(&self) -> f64 {
        let last = if self.done() { self.fonts_end() } else { None };
        last.or(self.first).unwrap_or(0.0)
    }

    /// The bytes every timed load moved over the network.
    pub fn net(&self) -> f64 {
        Record::bytes(self.timings.iter().map(|t| &t.1)).unwrap_or(0.0)
    }

    /// The line under the hairline: `Loading fonts…` until they land, then
    /// `Ready in 412 ms · 218 KB` (`· from cache`, `· fonts did not load`; the time alone with
    /// no timings).
    pub fn caption(&self) -> String {
        if self.first.is_none() {
            return String::new();
        }
        if !self.done() {
            return "Loading fonts\u{2026}".into();
        }
        let mut out = String::from("Ready in ");
        ms(&mut out, self.ready());
        if self.fonts.iter().flatten().any(|f| !f.0) {
            out.push_str(" \u{b7} fonts did not load");
        } else if self.net() > 0.0 {
            out.push_str(" \u{b7} ");
            kb(&mut out, self.net());
        } else if !self.timings.is_empty() {
            out.push_str(" \u{b7} from cache");
        }
        out
    }

    /// The card's rows: each stage's name, size and time (`—` where unknown; `cache` for a
    /// load that crossed no network), then Ready's.
    pub fn rows(&self) -> Vec<[String; 3]> {
        let (stages, mut rows) = (self.stages(), Vec::new());
        for (i, name) in STAGES.into_iter().enumerate() {
            let mut row = [String::from(name), "\u{2014}".into(), "\u{2014}".into()];
            if let Some(s) = stages.iter().find(|s| s.i == i) {
                row[2].clear();
                ms(&mut row[2], s.to - s.from);
                row[1] = if i == 2 { "budget 100".into() } else { "cache".into() };
                if let Some(b) = s.bytes.filter(|_| i != 2) {
                    row[1].clear();
                    kb(&mut row[1], b);
                }
            }
            rows.push(row);
        }
        let mut ready = [String::from("Ready"), String::new(), String::new()];
        ms(&mut ready[2], self.ready());
        rows.push(ready);
        rows
    }

    /// The hairline's scale at `now`: the last end, moving from the first frame's to the
    /// fonts' over [`FADE_MS`] once they land.
    pub fn scale(&self, now: f64) -> f64 {
        let first = self.first.unwrap_or(0.0);
        match self.stages().iter().find(|s| s.i == 3) {
            Some(f) => first + (f.to - first) * f64::from(ease(((now - f.to) / FADE_MS) as f32)),
            None => first,
        }
        .max(1.0)
    }
}

/// The note sign-in leaves in the report ring: `home <KB> <ms>`, the /home put back and how
/// long that took.
pub fn home_note(bytes: usize, ms: f64) -> String {
    let mut n = String::from("home ");
    push(&mut n, bytes as f64 / 1024.0);
    n.push(' ');
    push(&mut n, ms);
    n
}

/// Appends `v` (ms) as `412 ms`, or from a second `1.21 s`.
pub fn ms(out: &mut String, v: f64) {
    let v = v.clamp(0.0, 1e9).round() as usize;
    if v < 1000 {
        ui::push_num(out, v);
        return out.push_str(" ms");
    }
    let c = v / 10;
    ui::push_num(out, c / 100);
    out.push('.');
    out.push(char::from(b'0' + (c / 10 % 10) as u8));
    out.push(char::from(b'0' + (c % 10) as u8));
    out.push_str(" s");
}

/// Appends `bytes` as KB: `7.3 KB` below ten, `218 KB` from there.
pub fn kb(out: &mut String, bytes: f64) {
    let tenths = (bytes.clamp(0.0, 1e12) / 102.4).round() as usize;
    if tenths < 100 {
        ui::push_num(out, tenths / 10);
        out.push('.');
        ui::push_num(out, tenths % 10);
    } else {
        ui::push_num(out, (tenths + 5) / 10);
    }
    out.push_str(" KB");
}

/// Appends `v` rounded, or `-` if it is unknown (negative).
fn push(out: &mut String, v: f64) {
    if v < 0.0 {
        return out.push('-');
    }
    ui::push_num(out, v.min(1e9).round() as usize);
}
