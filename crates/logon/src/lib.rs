//! compusophyOS's start, drawn natively in the boot (no program, no worker): the peaceful welcome
//! a new tab opens on, and its sign-in to the desktop.
//!
//! - **The welcome is the first frame.** It costs less than the desktop's would: no /home is
//!   put back and no shell is made until someone signs in.
//! - **The mark is art.** compusophy's mark comes in once from the center out (618 ms, as 365
//!   round fills: no atlas), never labeled as progress. The name follows when its font lands.
//! - **The record is truth** ([`record`]): under the column a hairline holds one segment per
//!   stage of the real start, at its measured times, gaps kept; its caption says
//!   `Loading fonts…`, then `Ready in 412 ms · 218 KB`. A tap (or Tab, then Enter) opens a card
//!   of the stages. Nothing is simulated or held back for show, and waiting draws no frames.
//! - **Sign in.** A first visit says hello and starts with **Start**; a return starts with a
//!   tap, Enter or Space. The welcome then flies its mark to the bar's (220 ms) and fades off
//!   the desktop. A reload of a signed-in tab skips it ([`SESSION`]).
//!
//! Pure: no browser, no clock. `os` hands in the page clock and storage reads, and carries out
//! what comes back ([`Out`]). Logical pixels, origin top-left; every color from the theme of
//! the profile that signed in last.

#![forbid(unsafe_code)]

pub mod record;

use gfx::{DrawList, RectF};
use host::motion::{Vis, ease, replay};
use host::paint::{cap_baseline, faded, px, sheen};
use host::{Input, LocalTime, Response};
use record::Record;
use ui::icon::{PHI, REVEAL_MS, mark_dots};
use ui::{FontId, Key, TextStyle, TextSystem, Theme};

/// The device's mark that a welcome said hello (`localStorage`), and the tab's session
/// (`sessionStorage`): the profile it signed in to, which a reload goes straight back to.
pub const SEEN: &str = "compusophy.seen";
pub const SESSION: &str = "compusophy.session";

/// The clock band (the bar's height), the record's hairline above the bottom, a target's
/// least side, a small gap.
const BAND: f32 = 44.0;
const TRACK: f32 = 55.0;
const TOUCH: f32 = 44.0;
const GAP: f32 = 13.0;
/// The mark's sides, largest first (the house's Fibonacci sizes), the gap under it, the
/// name's line and the gap under that.
const SIDES: [f32; 4] = [233.0, 144.0, 89.0, 55.0];
const UNDER_MARK: f32 = 34.0;
const NAME_H: f32 = 41.0;
const UNDER_NAME: f32 = 21.0;
/// A fade in; sign-in's flight, the welcome's content and backdrop fading off the desktop
/// within it, the mark crossfading into the bar's at its end; the living grain's step (ms).
const FADE: f64 = 233.0;
const FLIGHT: f64 = 220.0;
const CONTENT_OUT: f64 = 140.0;
const BACKDROP_OUT: f64 = 180.0;
const CROSS: f64 = 55.0;
const GRAIN_MS: f64 = 125.0;

const NAME: &str = "compusophy";
const LINE: &str = "a computer in your browser";
const STAYS: &str = "Your files stay in this browser, on this device.";
const CLICK: &str = "Click or press Enter to start";
const TAP: &str = "Tap to start";
const CARD_TITLE: &str = "How this start went";
const CARD_FOOT: &str = "Measured by this browser. Sizes are what crossed the network.";

/// What the welcome asks of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Sign in to profile `id`: put back its home, read its preferences, make the desktop.
    SignIn(u32),
    /// Store the value under the `localStorage` key.
    Set(String, String),
    /// Keep the tab's [`SESSION`]: a profile a reload goes straight back to.
    Session(String),
}

/// The profile a tab's [`SESSION`] (`session`, as stored) goes straight back to on a reload,
/// if it still may (`get` reads `localStorage`).
pub fn session(session: Option<&str>, get: &dyn Fn(&str) -> Option<String>) -> Option<u32> {
    let _ = get;
    (session == Some("0")).then_some(0)
}

/// What the welcome shows: a first visit's hello, or the return's `Tap to start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Hello,
    Ready,
}

/// What a press lands on: Start, the record's band, its card, or anywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Start,
    Record,
    Card,
    Back,
}

/// The welcome: the screen, the theme and grain of the profile that signed in last, whether
/// a welcome said hello on this device, what shows, the clock; when it first drew (the
/// reveal's clock) and when someone signed in (the flight's); the pointer (over, pressed, a
/// finger's), the keys' focus (the record), the card; what was drawn where, last frame.
#[derive(Debug)]
pub struct Logon {
    size: (f32, f32),
    theme: &'static Theme,
    grain: bool,
    seen: bool,
    state: State,
    clock: Option<LocalTime>,
    start: Option<f64>,
    leaving: Option<f64>,
    reduced: bool,
    hover: Option<Target>,
    press: Option<Target>,
    finger: bool,
    focus: Option<Target>,
    card: bool,
    hits: Vec<(RectF, Target)>,
}

/// Where the column lands: the mark's square (none if it does not fit), the name's line, the
/// block's top, whether the record shows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    mark: Option<RectF>,
    name: Option<f32>,
    block: f32,
    record: bool,
}

impl Logon {
    /// The welcome for a screen `size`, reading `localStorage` through `get`: a first visit
    /// (no welcome said hello, no files kept) says hello.
    pub fn new(size: (f32, f32), get: &dyn Fn(&str) -> Option<String>) -> Logon {
        let seen = get(SEEN).is_some();
        let state =
            if seen || get("compusophy.home").is_some() { State::Ready } else { State::Hello };
        Logon {
            size,
            theme: ui::theme(&get("compusophy.theme").unwrap_or_default()),
            grain: get("compusophy.grain").as_deref() != Some("off"),
            seen,
            state,
            clock: None,
            start: None,
            leaving: None,
            reduced: false,
            hover: None,
            press: None,
            finger: false,
            focus: None,
            card: false,
            hits: Vec::new(),
        }
    }

    /// The theme it is drawn in (whose base clears the frame).
    pub fn theme(&self) -> &'static Theme {
        self.theme
    }

    /// Whether someone signed in: the desktop takes the input, and [`Logon::layer`] draws the
    /// flight over it.
    pub fn leaving(&self) -> bool {
        self.leaving.is_some()
    }

    /// Whether the flight is over at `now`: the welcome can go.
    pub fn gone(&self, now: f64) -> bool {
        self.leaving.is_some_and(|t| now - t >= FLIGHT)
    }

    /// One input at `now` (page ms): what to draw and ask of the page.
    pub fn input(&mut self, input: &Input, now: f64) -> (Response, Vec<Out>) {
        let mut out = (Response::default(), Vec::new());
        let r = &mut out.0;
        match *input {
            Input::Resize { w, h } => {
                self.size = (finite(w), finite(h));
                r.redraw = true;
            }
            Input::Tick { time } => r.redraw = self.clock.replace(time) != Some(time),
            Input::PointerMove { x, y } => {
                let at = self.at(x, y);
                r.redraw = std::mem::replace(&mut self.hover, at) != at;
                r.consumed = true;
            }
            Input::PointerLeave => r.redraw = self.hover.take().is_some(),
            Input::PointerDown { x, y, button, touch } => {
                (self.finger, self.press) = (touch, self.at(x, y).filter(|_| button == 0));
                r.consumed = true;
            }
            Input::PointerUp { x, y, .. } => {
                let at = self.at(x, y);
                if let Some(t) = self.press.take().filter(|&t| Some(t) == at) {
                    self.act(t, now, &mut out.1);
                }
                (r.consumed, r.redraw) = (true, true);
            }
            Input::Key { key, .. } => {
                r.consumed = self.key(key, now, &mut out.1);
                r.redraw = r.consumed;
            }
            Input::Text(_) | Input::Wheel { .. } => {}
        }
        out
    }

    /// A key: Enter or Space does what has the focus (Start, signing in, the card), Tab moves
    /// the focus to the record and back, Escape closes the card; whether it was the welcome's.
    fn key(&mut self, key: Key, now: f64, outs: &mut Vec<Out>) -> bool {
        match key {
            Key::Enter | Key::Space => {
                let t = match (self.focus, self.state) {
                    (Some(t), _) => t,
                    (None, State::Hello) => Target::Start,
                    (None, State::Ready) => Target::Back,
                };
                self.act(t, now, outs);
            }
            Key::Tab => {
                self.focus = if self.focus.is_some() { None } else { Some(Target::Record) };
            }
            Key::Escape => (self.card, self.focus) = (false, None),
            _ => return false,
        }
        true
    }

    /// Does what a tap on `t` does: a tap anywhere outside an open card only closes it.
    fn act(&mut self, t: Target, now: f64, outs: &mut Vec<Out>) {
        if self.card && !matches!(t, Target::Card | Target::Record) {
            self.card = false;
            return;
        }
        match (t, self.state) {
            (Target::Record, _) => self.card = !self.card,
            (Target::Start, State::Hello) | (Target::Back, State::Ready) => {
                self.sign_in(0, now, outs)
            }
            _ => {}
        }
    }

    /// Signs in to profile `id`: the device has said hello, the tab keeps its session, and the
    /// flight begins.
    fn sign_in(&mut self, id: u32, now: f64, outs: &mut Vec<Out>) {
        if !self.seen {
            outs.push(Out::Set(SEEN.into(), "1".into()));
        }
        let mut n = String::new();
        ui::push_num(&mut n, id as usize);
        outs.extend([Out::Session(n), Out::SignIn(id)]);
        self.leaving = Some(if self.reduced { f64::NEG_INFINITY } else { now });
    }

    /// What is at `(x, y)` as last drawn.
    fn at(&self, x: f32, y: f32) -> Option<Target> {
        let hit = self.hits.iter().rev().find(|h| h.0.contains(x, y)).map(|h| h.1);
        Some(hit.unwrap_or(Target::Back))
    }

    /// The block under the name: its height.
    fn block_h(&self) -> f32 {
        match self.state {
            State::Hello => 17.0 + 21.0 + TOUCH + 21.0 + 15.0,
            State::Ready => 17.0,
        }
    }

    /// Where the column goes for the block: the mark is the largest of [`SIDES`] at most 1/φ²
    /// of the screen's shorter side that lets the column fit between the clock band and the
    /// record (if none, no mark; if the column still does not fit, no name: the block always
    /// shows), centered on the golden point (0.382 of the height), moved up only as far as the
    /// column needs. The record shows if the block fits above it.
    fn layout(&self) -> Layout {
        let ((w, h), block) = (self.size, self.block_h());
        let record = block <= h - TRACK - GAP - BAND;
        let bottom = h - if record { TRACK + GAP } else { GAP };
        let room = bottom - BAND;
        let head = UNDER_MARK + NAME_H + UNDER_NAME;
        let fits = |s: f32| s <= w.min(h) / (PHI * PHI) && s + head + block <= room;
        let side = SIDES.into_iter().find(|&s| fits(s));
        let name = side.is_some() || NAME_H + UNDER_NAME + block <= room;
        let column = side.map_or(0.0, |s| s + UNDER_MARK)
            + if name { NAME_H + UNDER_NAME } else { 0.0 }
            + block;
        let center = (0.382 * h).round();
        let top = side.map_or(center - column / 2.0, |s| center - s / 2.0);
        let top = top.min(bottom - column).max(BAND).round();
        let mark = side.map(|s| RectF::new(((w - s) / 2.0).round(), top, s, s));
        let name = name.then(|| top + side.map_or(0.0, |s| s + UNDER_MARK));
        Layout { mark, name, block: name.map_or(top, |n| n + NAME_H + UNDER_NAME), record }
    }

    /// The living grain's pattern at `now` (a new one each [`GRAIN_MS`] while it lives: the
    /// profile's `grain` on, motion not reduced, a theme with grain), and when the next is due.
    fn grain(&self, now: f64) -> (u32, Option<u32>) {
        if !self.grain || self.reduced || self.theme.grain == 0 {
            return (0, None);
        }
        let next = GRAIN_MS * (now / GRAIN_MS).floor() + GRAIN_MS - now;
        ((now / GRAIN_MS) as u32 % 4096 + 1, Some((next.ceil() as u32).max(1)))
    }

    /// The welcome on the whole screen at `now` (motion `reduced`), `rec` the start as known:
    /// when it wants its next frame (0: at once) if ever, only while something moves (and by
    /// the grain's timer).
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        (now, reduced): (f64, bool),
    ) -> Option<u32> {
        self.reduced = reduced;
        let start = *self.start.get_or_insert(now);
        list.clear();
        self.hits.clear();
        let (seed, grain) = self.grain(now);
        let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
        self.theme.draw_backdrop(list, screen, seed as f32);
        let ms = if reduced { f64::NAN } else { now - start };
        if let Some(m) = self.paint(list, text, rec, now) {
            mark_dots(list, m, text.dpr(), ms, self.theme.text);
        }
        if self.card {
            self.paint_card(list, text, rec);
        }
        let fading = |since: Option<f64>| since.is_some_and(|t| now - t < FADE);
        let name = fading(rec.fonts[0].map(|f| f.1));
        let stage = rec.stages().iter().any(|s| fading(Some(s.learned)));
        let moving = !reduced && (now - start < REVEAL_MS || name || stage);
        if moving { Some(0) } else { grain }
    }

    /// Over the desktop once someone signed in: the welcome fading off it (its content in 140
    /// ms, its backdrop in 180) and the mark flying to the bar's (220), into whose glyph it
    /// crossfades over the last 55 ms; when it wants its next frame.
    pub fn layer(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        now: f64,
    ) -> Option<u32> {
        let t = now - self.leaving?;
        if t >= FLIGHT {
            return None;
        }
        let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
        let out = |ms: f64| 1.0 - ease((t / ms) as f32);
        let mut layer = DrawList::new();
        self.theme.draw_backdrop(&mut layer, screen, 0.0);
        replay(list, &layer, Vis { a: out(BACKDROP_OUT), ..Vis::at(screen) });
        layer.clear();
        let mark = self.paint(&mut layer, text, rec, now);
        replay(list, &layer, Vis { a: out(CONTENT_OUT), ..Vis::at(screen) });
        if let Some(m) = mark {
            layer.clear();
            mark_dots(&mut layer, m, text.dpr(), f64::NAN, self.theme.text);
            let (to, e) = (home::bar::mark_rect(), ease((t / FLIGHT) as f32));
            let mid = |r: RectF| (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let ((x0, y0), (x1, y1)) = (mid(m), mid(to));
            let a = 1.0 - ((t - (FLIGHT - CROSS)) / CROSS).max(0.0) as f32;
            let s = 1.0 + (to.w / m.w - 1.0) * e;
            replay(list, &layer, Vis { rect: m, s, dx: (x1 - x0) * e, dy: (y1 - y0) * e, a });
        }
        Some(0)
    }

    /// The clock, the name, the block and the record (all but the backdrop, the mark and the
    /// card), registering what can be pressed; the mark's square.
    fn paint(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        now: f64,
    ) -> Option<RectF> {
        let (t, (w, h)) = (self.theme, self.size);
        let lay = self.layout();
        if let Some(time) = self.clock {
            home::bar::draw_clock(list, text, t, w, time);
        }
        // The name, in SemiBold once it lands (Regular if it failed), fading in.
        if let (Some(y), Some((ok, at))) = (lay.name, rec.fonts[0]) {
            let font = if ok { FontId::SansBold } else { FontId::Sans };
            let a = if self.reduced { 1.0 } else { ease(((now - at) / FADE) as f32) };
            let style = TextStyle::new(font, 34.0, faded(t.text, a));
            centered(list, text, (w / 2.0, cap_baseline(text, y, NAME_H, 34.0)), NAME, style);
        }
        let (y, cx) = (lay.block, w / 2.0);
        let dim = TextStyle::new(FontId::Sans, 14.0, t.text_dim);
        let small = TextStyle::new(FontId::Sans, 12.0, t.text_dim);
        match self.state {
            State::Hello => {
                centered(list, text, (cx, cap_baseline(text, y, 17.0, 14.0)), LINE, dim);
                let y = y + 17.0 + 21.0;
                let bw = (text.measure("Start", dim) + 2.0 * 34.0).max(144.0);
                let r = RectF::new(text.snap(cx - bw / 2.0), y, text.snap(bw), TOUCH);
                self.button(list, text, r, "Start", Target::Start);
                let y = y + TOUCH + 21.0;
                centered(list, text, (cx, cap_baseline(text, y, 15.0, 12.0)), STAYS, small);
            }
            State::Ready => {
                let hint = if self.finger || home::narrow(w) { TAP } else { CLICK };
                centered(list, text, (cx, cap_baseline(text, y, 17.0, 14.0)), hint, dim);
            }
        }
        if lay.record {
            self.paint_record(list, text, rec, (now, h), lay.mark.map_or(89.0, |m| m.w));
        }
        lay.mark
    }

    /// An accent button in `r` with `label`, lighter under the pointer and while held.
    fn button(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        r: RectF,
        label: &str,
        at: Target,
    ) {
        let t = self.theme;
        let lift = [0.0, 0.12, 0.24]
            [usize::from(self.hover == Some(at)) + usize::from(self.press == Some(at))];
        list.fill(r, r.h / 2.0, ui::theme::mix(t.accent, t.accent_text, lift));
        sheen(list, r, r.h / 2.0, px(text, 1.0), t.highlight.with_alpha(t.highlight.3.min(40)));
        let style = TextStyle::new(FontId::Sans, 15.0, t.accent_text);
        centered(list, text, (r.x + r.w / 2.0, cap_baseline(text, r.y, r.h, 15.0)), label, style);
        self.ring(list, text, r, at);
        self.hits.push((r, at));
    }

    /// The keys' focus ring around `r` if `at` has the focus: the accent, 2 px, 3 px out.
    fn ring(&self, list: &mut DrawList, text: &TextSystem, r: RectF, at: Target) {
        if self.focus == Some(at) {
            let (line, out) = (px(text, 2.0), px(text, 3.0));
            let o = r.inset(-(out + line));
            list.border(o, (r.h / 2.0).min(13.0) + out + line, line, self.theme.accent);
        }
    }

    /// The record: its track (as wide as the mark, 89 px without one) on the hairline
    /// [`TRACK`] px above the bottom, a segment per stage over (fading in as it is learned,
    /// placed on the scale of the last end), and its caption 13 px under it; the whole band
    /// opens the card.
    fn paint_record(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        (now, h): (f64, f32),
        tw: f32,
    ) {
        let (t, w) = (self.theme, self.size.0);
        let (y, hair, line) = (text.snap(h - TRACK), 1.0 / text.dpr(), px(text, 1.0));
        let x0 = text.snap((w - tw) / 2.0);
        list.fill(RectF::new(x0, y, tw, hair), 0.0, t.border);
        let scale = rec.scale(now);
        for s in rec.stages() {
            let a = if self.reduced { 1.0 } else { ease(((now - s.learned) / FADE) as f32) };
            let at = |v: f64| text.snap(x0 + (v.min(scale) / scale) as f32 * tw);
            let (l, r) = (at(s.from), at(s.to).max(at(s.from) + hair));
            let seg = RectF::new(l, text.snap(y + hair / 2.0 - line / 2.0), r - l, line);
            list.fill(seg, 0.0, faded(t.text_dim, a));
        }
        let caption = rec.caption();
        let small = TextStyle::new(FontId::Sans, 12.0, t.text_dim);
        let cw = text.measure(&caption, small);
        centered(list, text, (w / 2.0, cap_baseline(text, y + GAP, 15.0, 12.0)), &caption, small);
        let bw = tw.max(cw) + 2.0 * GAP;
        let band = RectF::new(text.snap((w - bw) / 2.0), y - GAP, text.snap(bw), TOUCH);
        self.ring(list, text, band, Target::Record);
        self.hits.push((band, Target::Record));
    }

    /// The record's card, 13 px above the hairline: each stage's size and time, then Ready's,
    /// and where the numbers come from (the panel of the home screen's menus).
    fn paint_card(&mut self, list: &mut DrawList, text: &mut TextSystem, rec: &Record) {
        let (t, (w, h)) = (self.theme, self.size);
        let cw = 280.0f32.min(w - 32.0);
        let small = TextStyle::new(FontId::Sans, 12.0, t.text_dim);
        let foot = text.wrap(CARD_FOOT, small, cw - 2.0 * GAP);
        let rows = rec.rows();
        let ch = 2.0 * GAP + 21.0 * (rows.len() + 1) as f32 + 8.0 + 15.0 * foot.len() as f32;
        let y = text.snap((h - TRACK - GAP - ch).max(BAND));
        let r = RectF::new(text.snap((w - cw) / 2.0), y, text.snap(cw), text.snap(ch));
        let line = px(text, 1.0);
        list.shadow_offset(r, 13.0, 34.0, 13.0, t.shadow);
        list.fill(r, 13.0, t.base);
        list.fill(r, 13.0, t.surface);
        list.border(r, 13.0, line, t.border);
        sheen(list, r, 13.0, line, t.highlight);
        let body = TextStyle::new(FontId::Sans, 13.0, t.text);
        let x = r.x + GAP;
        let title = cap_baseline(text, y + GAP, 21.0, 13.0);
        text.draw_text(list, text.snap(x), title, CARD_TITLE, body);
        // Start's size is its budget, in the danger color once spent.
        let spent = rec.stages().iter().any(|s| s.i == 2 && s.to - s.from > record::BUDGET);
        for (i, [name, size, time]) in rows.iter().enumerate() {
            let base = title + 21.0 * (i + 1) as f32;
            text.draw_text(list, text.snap(x), base, name, body.with_color(t.text_dim));
            let over = small.with_color(if i == 2 && spent { t.danger } else { t.text_dim });
            for (s, edge, style) in [(size, 0.6 * r.w, over), (time, r.w - GAP, body)] {
                let sw = text.measure(s, style);
                text.draw_text(list, text.snap(r.x + edge - sw), base, s, style);
            }
        }
        let mut base = y + GAP + 21.0 * (rows.len() + 1) as f32 + 8.0;
        for l in foot {
            text.draw_text(list, text.snap(x), cap_baseline(text, base, 15.0, 12.0), l, small);
            base += 15.0;
        }
        self.hits.push((r, Target::Card));
    }
}

/// `s` centered on `x`, on `baseline`.
fn centered(
    list: &mut DrawList,
    text: &mut TextSystem,
    (x, baseline): (f32, f32),
    s: &str,
    style: TextStyle,
) {
    let sw = text.measure(s, style);
    let at = text.snap(x - sw / 2.0);
    text.draw_text(list, at, baseline, s, style);
}

/// `v` if finite and positive, else 0.
fn finite(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests;
