//! How the welcome looks: the column (the mark, the name, the state's block) laid out for the
//! screen, the record under it, the card, a circle's menu; and over the desktop, sign-in's
//! flight or the card that files could not be kept.

use gfx::{DrawList, Icon, RectF, Rgba};
use home::menu::{Item, Menu};
use host::motion::{Vis, ease, replay};
use host::paint::{cap_baseline, faded, px, sheen};
use ui::icon::{PHI, REVEAL_MS, face, mark_dots, sin};
use ui::{FontId, TextStyle, TextSystem};

use crate::record::{self, Record};
use crate::{Act, HOLD_MS, Logon, State, Target};

/// The clock band (the bar's height), the record's hairline above the bottom, a target's
/// least side, a small gap, the column's widest.
pub(crate) const BAND: f32 = 44.0;
pub(crate) const TRACK: f32 = 55.0;
pub(crate) const TOUCH: f32 = 44.0;
const GAP: f32 = 13.0;
const COLUMN: f32 = 466.0;
/// The mark's sides, largest first (Fibonacci): every measure of the column above the block
/// follows from the side by powers of φ ([`head`]); the name's least size; Inter's cap height
/// in ems.
const SIDES: [f32; 3] = [144.0, 89.0, 55.0];
const NAME_LEAST: f32 = 21.0;
const CAP: f32 = 0.727;
/// A circle's side (a person's face, Add): 1/φ² of the largest mark, and a touch target.
const CIRCLE: f32 = 55.0;
/// A label's line, a line of prose, a circle's label under it, a PIN's dot.
const LINE_H: f32 = 15.0;
const PROSE_H: f32 = 20.0;
const LABEL: f32 = 8.0 + LINE_H;
const DOT: f32 = 13.0;
/// A fade in; sign-in's flight, the welcome's content and backdrop fading off the desktop
/// within it, the mark crossfading into the bar's at its end; the living grain's step (ms).
pub(crate) const FADE: f64 = 233.0;
pub(crate) const FLIGHT: f64 = 220.0;
const CONTENT_OUT: f64 = 140.0;
const BACKDROP_OUT: f64 = 180.0;
const CROSS: f64 = 55.0;
const GRAIN_MS: f64 = 125.0;

const NAME: &str = "compusophy";
const CARD_TITLE: &str = "How this start went";
const CARD_FOOT: &str = "Measured by this browser. Sizes are what crossed the network.";
const ASK: &str = "A PIN keeps a casual tap out; it is not a lock. Your files are not encrypted, \
and anyone with this device and browser can read them. The PIN never leaves this browser, so \
don't reuse one from anywhere else. Forget it and this profile can only be removed, with its \
files.";
const UNKEPT_TITLE: &str = "Your files could not be kept.";
const UNKEPT: &str = "This browser's storage is full or blocked, so changes since they were last \
kept are lost if you sign out now.";

/// How a button looks: the main action, a plain one, a destructive one.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Main,
    Plain,
    Danger,
}

/// Where the column lands: the mark's square (none if it does not fit), the name's baseline
/// and size, the block's top, whether the record shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    pub(crate) mark: Option<RectF>,
    pub(crate) name: Option<(f32, f32)>,
    pub(crate) block: f32,
    pub(crate) record: bool,
}

/// What a mark of side `s` sets above the block, each a power of φ of it: the name's size
/// (s/φ³, [`NAME_LEAST`] at least), the gap from the mark to the name's capitals (s/φ³ too: the
/// two are one sign), and from the name's baseline to the block, φ² that (the sign and the
/// choice under it, apart).
fn head(s: f32) -> (f32, f32, f32) {
    let cube = PHI * PHI * PHI;
    let (size, gap) = ((s / cube).round().max(NAME_LEAST), (s / cube).round());
    (size, gap, (gap * PHI * PHI).round())
}

impl Logon {
    /// The circles' side and gap (1/φ of the side; a phone's 21), and how many fit in a row.
    fn ring(&self) -> (f32, f32, usize) {
        let narrow = home::narrow(self.size.0);
        let (c, gap) = (CIRCLE, if narrow { 21.0 } else { 34.0 });
        let room = (self.size.0 - 32.0).min(COLUMN);
        (c, gap, (((room + gap) / (c + gap)) as usize).max(1))
    }

    /// The circles' rows' height.
    fn rows_h(&self) -> f32 {
        let (c, _, per) = self.ring();
        let rows = self.circles().div_ceil(per) as f32;
        rows * (c + LABEL) + (rows - 1.0) * GAP
    }

    /// The prose of a removal to confirm, in lines at most the column wide.
    fn prose<'a>(&self, text: &mut TextSystem, s: &'a str) -> Vec<&'a str> {
        let style = TextStyle::new(FontId::Sans, 14.0, self.theme().text);
        text.wrap(s, style, (self.size.0 - 32.0).min(320.0))
    }

    /// What a removal asks: `Remove kai and their files (1.2 MB) from this browser? ...`.
    fn removal(&self, id: u32) -> (String, String) {
        let name = self.list.get(id).map_or("", |p| p.name.as_str());
        let mut size = String::new();
        match self.files {
            b if b < 1 << 20 => record::kb(&mut size, b as f64),
            b => {
                let tenths = b * 10 / (1 << 20);
                ui::push_num(&mut size, tenths / 10);
                size.push('.');
                ui::push_num(&mut size, tenths % 10);
                size.push_str(" MB");
            }
        }
        let ask = [
            "Remove ",
            name,
            " and their files (",
            &size,
            ") from this browser? This cannot \
be undone.",
        ];
        (ask.concat(), ["Remove ", name].concat())
    }

    /// The block under the name: its height (with the line under it, when something is said).
    pub(crate) fn block_h(&self, text: &mut TextSystem) -> f32 {
        let note = if self.note.is_some() { GAP + LINE_H } else { 0.0 };
        match self.state {
            State::Pick => self.rows_h() + note,
            State::Name(_) => TOUCH + GAP + TOUCH + GAP + LINE_H,
            State::Confirm(id) => {
                let (ask, _) = self.removal(id);
                PROSE_H * self.prose(text, &ask).len() as f32 + 21.0 + TOUCH + note
            }
            State::Ask => PROSE_H * self.prose(text, ASK).len() as f32 + 21.0 + TOUCH + note,
            State::NewPin(_) => DOT + GAP + LINE_H + GAP + TOUCH,
            State::Pin(..) => self.ring().0 + LABEL + 21.0 + DOT + GAP + LINE_H,
            State::Unkept => 0.0,
        }
    }

    /// Where the column goes for the block: the mark is the largest of [`SIDES`] at most 1/φ²
    /// of the screen's shorter side that lets the column fit between the clock band and the
    /// record (if none, no mark; if the column still does not fit, no name: the block always
    /// shows), the name and the gaps by [`head`]; the column at the golden section of the room
    /// it leaves, φ times as much under it as over it. The record shows if the block fits above
    /// it, and not while a name is typed (a phone's keyboard needs the room).
    pub(crate) fn layout(&self, text: &mut TextSystem) -> Layout {
        let ((w, h), block) = (self.size, self.block_h(text));
        let typing = matches!(self.state, State::Name(_) | State::NewPin(_) | State::Pin(..));
        let record = !typing && block <= h - TRACK - GAP - BAND;
        // A pixel clear of the record's band: snapped to device pixels, the two never touch.
        let bottom = h - if record { TRACK + GAP + 1.0 } else { GAP };
        let room = bottom - BAND;
        let tall = |s: f32| {
            let (size, over, under) = head(s);
            over + (CAP * size).round() + under
        };
        let fits = |s: f32| s <= w.min(h) / (PHI * PHI) && s + tall(s) + block <= room;
        let side = SIDES.into_iter().find(|&s| fits(s));
        let least = SIDES[SIDES.len() - 1];
        let (size, over, under) = head(side.unwrap_or(least));
        let name = side.is_some() || tall(least) - over + block <= room;
        let column = side.map_or(0.0, |s| s + over)
            + if name { (CAP * size).round() + under } else { 0.0 }
            + block;
        let top = (BAND + (room - column).max(0.0) / (PHI * PHI)).round();
        let mark = side.map(|s| RectF::new(((w - s) / 2.0).round(), top, s, s));
        let base = name.then(|| top + side.map_or(0.0, |s| s + over) + (CAP * size).round());
        Layout {
            mark,
            name: base.map(|b| (b, size)),
            block: base.map_or(top, |b| b + under),
            record,
        }
    }

    /// The living grain's pattern at `now` (a new one each [`GRAIN_MS`] while it lives: the
    /// profile's `grain` on, motion not reduced, a theme with grain), and when the next is due.
    fn grain(&self, now: f64) -> (u32, Option<u32>) {
        if !self.grain || self.reduced || self.theme().grain == 0 {
            return (0, None);
        }
        let next = GRAIN_MS * (now / GRAIN_MS).floor() + GRAIN_MS - now;
        ((now / GRAIN_MS) as u32 % 4096 + 1, Some((next.ceil() as u32).max(1)))
    }

    /// The welcome on the whole screen at `now` (motion `reduced`), `rec` the start as known:
    /// when it wants its next frame (0: at once) if ever, only while something moves (and by
    /// the grain's timer, or to judge a held circle). A circle held 500 ms opens its menu.
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        (now, reduced): (f64, bool),
    ) -> Option<u32> {
        self.reduced = reduced;
        let start = *self.start.get_or_insert(now);
        let held = match (self.press, self.down) {
            (Some(Target::Circle(i)), Some((t, at))) if i < self.list.list.len() => {
                Some((i, t, at))
            }
            _ => None,
        };
        if let Some((i, _, at)) = held.filter(|h| now - h.1 >= HOLD_MS) {
            (self.press, self.down) = (None, None);
            self.ask_menu(i, at);
        }
        if let Some((id, at)) = self.asked.take() {
            let pinned = self.list.get(id).is_some_and(|p| p.pin.is_some());
            let menu = Menu::new(&items(pinned), at, self.size, self.finger, text);
            self.menu = Some((id, menu));
        }
        list.clear();
        self.hits.clear();
        let (seed, grain) = self.grain(now);
        let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
        self.theme().draw_backdrop(list, screen, seed as f32);
        let ms = if reduced { f64::NAN } else { now - start };
        if let Some(m) = self.paint(list, text, rec, now) {
            mark_dots(list, m, text.dpr(), ms, self.theme().text);
        }
        if self.card {
            self.paint_card(list, text, rec);
        }
        if let Some((_, menu)) = &self.menu {
            menu.draw(list, text, self.theme(), self.press.is_some());
        }
        let fading = |since: Option<f64>| since.is_some_and(|t| now - t < FADE);
        let name = fading(rec.fonts[0].map(|f| f.1));
        let stage = rec.stages().iter().any(|s| fading(Some(s.learned)));
        let shaking = self.shook.is_some_and(|s| now - s < FADE);
        let moving = !reduced && (now - start < REVEAL_MS || name || stage || shaking);
        let hold =
            held.filter(|h| now - h.1 < HOLD_MS).map(|h| (HOLD_MS - (now - h.1)).ceil() as u32);
        if moving { Some(0) } else { [grain, hold].into_iter().flatten().min() }
    }

    /// Over the desktop: the card that files could not be kept; or, once someone signed in,
    /// the welcome fading off it (its content in 140 ms, its backdrop in 180) and the mark
    /// flying to the bar's (220), into whose glyph it crossfades over the last 55 ms. When it
    /// wants its next frame.
    pub fn layer(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        rec: &Record,
        now: f64,
    ) -> Option<u32> {
        let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
        if self.state == State::Unkept {
            self.hits.clear();
            self.paint_unkept(list, text, screen);
            return None;
        }
        let t = now - self.leaving?;
        if t >= FLIGHT {
            return None;
        }
        let out = |ms: f64| 1.0 - ease((t / ms) as f32);
        let mut layer = DrawList::new();
        self.theme().draw_backdrop(&mut layer, screen, 0.0);
        replay(list, &layer, Vis { a: out(BACKDROP_OUT), ..Vis::at(screen) });
        layer.clear();
        let mark = self.paint(&mut layer, text, rec, now);
        replay(list, &layer, Vis { a: out(CONTENT_OUT), ..Vis::at(screen) });
        if let Some(m) = mark {
            layer.clear();
            mark_dots(&mut layer, m, text.dpr(), f64::NAN, self.theme().text);
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
        let (t, (w, h)) = (self.theme(), self.size);
        let lay = self.layout(text);
        if let Some(time) = self.clock {
            home::bar::draw_clock(list, text, t, w, time);
        }
        // The name, in SemiBold once it lands (Regular if it failed), fading in.
        if let (Some((base, size)), Some((ok, at))) = (lay.name, rec.fonts[0]) {
            let font = if ok { FontId::SansBold } else { FontId::Sans };
            let a = if self.reduced { 1.0 } else { ease(((now - at) / FADE) as f32) };
            let style = TextStyle::new(font, size, faded(t.text, a));
            centered(list, text, (w / 2.0, text.snap(base)), NAME, style);
        }
        let (mut y, cx) = (lay.block, w / 2.0);
        let small = TextStyle::new(FontId::Sans, 12.0, t.text_dim);
        match self.state {
            State::Pick => y = self.paint_circles(list, text, y),
            State::Name(id) => {
                self.field(list, text, y);
                y += TOUCH + GAP;
                let next = if id.is_some() { "Save" } else { "Next" };
                let row =
                    [("Cancel", Look::Plain, Target::Cancel), (next, Look::Main, Target::Next)];
                self.buttons(list, text, y, &row);
                y += TOUCH + GAP;
            }
            State::Confirm(id) => {
                let (ask, remove) = self.removal(id);
                let body = TextStyle::new(FontId::Sans, 14.0, t.text);
                for l in self.prose(text, &ask) {
                    centered(list, text, (cx, cap_baseline(text, y, PROSE_H, 14.0)), l, body);
                    y += PROSE_H;
                }
                y += 21.0;
                let row = [
                    ("Cancel", Look::Plain, Target::Cancel),
                    (&remove, Look::Danger, Target::Remove),
                ];
                self.buttons(list, text, y, &row);
                y += TOUCH + GAP;
            }
            State::Ask => {
                let body = TextStyle::new(FontId::Sans, 14.0, t.text_dim);
                for l in self.prose(text, ASK) {
                    centered(list, text, (cx, cap_baseline(text, y, PROSE_H, 14.0)), l, body);
                    y += PROSE_H;
                }
                y += 21.0;
                let row = [
                    ("Skip", Look::Plain, Target::Skip),
                    ("Set a PIN", Look::Main, Target::SetPin),
                ];
                self.buttons(list, text, y, &row);
                y += TOUCH + GAP;
            }
            State::NewPin(_) => {
                self.dots(list, text, y, now);
                y += DOT + GAP;
                // The note between the dots and the buttons: Next only for a first entry.
                let row =
                    [("Cancel", Look::Plain, Target::Cancel), ("Next", Look::Main, Target::Next)];
                let n = if self.first.is_empty() { 2 } else { 1 };
                self.buttons(list, text, y + LINE_H + GAP, &row[..n]);
            }
            State::Pin(id, _) => {
                let i = self.list.list.iter().position(|p| p.id == id).unwrap_or(0);
                let c = self.ring().0;
                let r = RectF::new(text.snap((w - c) / 2.0), y, c, c);
                self.circle(list, text, r, i);
                y += c + LABEL + 21.0;
                self.dots(list, text, y, now);
                y += DOT + GAP;
            }
            State::Unkept => {}
        }
        if let Some((note, error)) = self.note {
            let style = small.with_color(if error { t.danger } else { t.text_dim });
            let y = if self.state == State::Pick { y + GAP } else { y };
            centered(list, text, (cx, cap_baseline(text, y, LINE_H, 12.0)), note, style);
        }
        if lay.record {
            self.paint_record(list, text, rec, (now, h), lay.mark.map_or(89.0, |m| m.w));
        }
        lay.mark
    }

    /// The circles from `y`, in rows centered under the name: each profile's face (its name
    /// under it), then Add, a ring around a plus; the focused one's ring in the accent. Where
    /// they end.
    fn paint_circles(&mut self, list: &mut DrawList, text: &mut TextSystem, y: f32) -> f32 {
        let (c, gap, per) = self.ring();
        let n = self.circles();
        for i in 0..n {
            let (row, col) = (i / per, i % per);
            let in_row = per.min(n - row * per) as f32;
            let x0 = self.size.0 / 2.0 - (in_row * c + (in_row - 1.0) * gap) / 2.0;
            let top = y + row as f32 * (c + LABEL + GAP);
            let r = RectF::new(text.snap(x0 + col as f32 * (c + gap)), text.snap(top), c, c);
            self.circle(list, text, r, i);
        }
        y + self.rows_h()
    }

    /// Circle `i` in the square `r`: a profile's face, or Add (a ring around a plus), its ring a
    /// pixel wide: the accent if focused, brighter under the pointer, else faint; its name under
    /// it.
    fn circle(&mut self, list: &mut DrawList, text: &mut TextSystem, r: RectF, i: usize) {
        let (t, (c, gap, _)) = (self.theme(), self.ring());
        let label = TextStyle::new(FontId::Sans, 13.0, t.text_dim);
        let lit = match () {
            _ if self.focus == i => t.accent,
            _ if self.hover == Some(Target::Circle(i)) => t.text_dim,
            _ => t.text_faint,
        };
        let p = self.list.list.get(i);
        face(list, text, r, p.map_or(0, |p| p.face), [lit, t.text]);
        let name = match p {
            Some(p) => p.name.as_str(),
            None => {
                list.icon(r.inset(c * 0.36), Icon::Plus, px(text, 1.0), t.text_dim);
                "Add"
            }
        };
        let shown = text.ellipsize(name, label, c + gap - 4.0);
        let base = cap_baseline(text, r.y + c + 8.0, LINE_H, 13.0);
        let style = if self.focus == i { label.with_color(t.text) } else { label };
        centered(list, text, (r.x + c / 2.0, base), &shown, style);
        self.hits.push((RectF::new(r.x, r.y, c, c + LABEL), Target::Circle(i)));
    }

    /// A PIN's dots from `y`, one per digit (a new PIN's grows from four to eight), those typed
    /// filled: dim while it is checked; swinging in the danger color as a wrong one clears.
    /// They bring back a dismissed keyboard.
    fn dots(&mut self, list: &mut DrawList, text: &mut TextSystem, y: f32, now: f64) {
        let t = self.theme();
        let n = match self.state {
            State::Pin(id, _) => self.pin(id).map_or(4, |p| p.len),
            _ if self.first.is_empty() => self.typed.len().clamp(4, 8),
            _ => self.first.len(),
        };
        let filled = if self.awaited.is_some() { n } else { self.typed.len() };
        // Three damped half-swings, 8 px at first.
        let p = self.shook.map_or(1.0, |s| ((now - s) / FADE) as f32);
        let swing = !self.reduced && (0.0..1.0).contains(&p);
        let dx = if swing { 8.0 * (1.0 - p) * sin(3.0 * core::f32::consts::PI * p) } else { 0.0 };
        let ink = match () {
            _ if swing => t.danger,
            _ if self.awaited.is_some() => t.text_faint,
            _ => t.text,
        };
        let x0 = (self.size.0 - n as f32 * (DOT + 21.0) + 21.0) / 2.0 + dx;
        for i in 0..n {
            let r = RectF::new(text.snap(x0 + i as f32 * (DOT + 21.0)), text.snap(y), DOT, DOT);
            match i < filled {
                true => list.fill(r, DOT / 2.0, ink),
                false => {
                    list.border(r, DOT / 2.0, px(text, 1.5), if swing { ink } else { t.text_faint })
                }
            }
        }
        let room = (self.size.0 - 32.0).min(COLUMN);
        let hit = RectF::new(((self.size.0 - room) / 2.0).round(), y - 15.0, room, TOUCH);
        self.hits.push((hit, Target::Field));
    }

    /// A ring 2 px wide, 3 px out from `r` (corner `radius`), in `color`.
    fn ring_at(&self, list: &mut DrawList, text: &TextSystem, r: RectF, radius: f32, color: Rgba) {
        let (line, out) = (px(text, 2.0), px(text, 3.0));
        list.border(r.inset(-(out + line)), radius + out + line, line, color);
    }

    /// The keys' focus ring around `r` if `at` has it and the keys moved it there.
    fn focus_ring(&self, list: &mut DrawList, text: &TextSystem, r: RectF, at: Target) {
        let focused = match (self.state, at) {
            (State::Pick, Target::Record) => self.focus + 1 == self.stops(),
            _ => false,
        };
        if focused && self.keyed {
            self.ring_at(list, text, r, (r.h / 2.0).min(13.0), self.theme().accent);
        }
    }

    /// The name field from `y`: a sunken line across the column with what is typed (or a faint
    /// `Name`) and the caret, its ring the accent while typing.
    fn field(&mut self, list: &mut DrawList, text: &mut TextSystem, y: f32) {
        let t = self.theme();
        let fw = (self.size.0 - 32.0).min(280.0);
        let r = RectF::new(text.snap((self.size.0 - fw) / 2.0), y, text.snap(fw), TOUCH);
        list.fill(r, 8.0, t.surface_lo);
        list.border(r, 8.0, px(text, 1.5), if self.ime { t.accent } else { t.border });
        let style = TextStyle::new(FontId::Sans, 15.0, t.text);
        let (inner, base) = (r.inset(13.0), cap_baseline(text, r.y, r.h, 15.0));
        let tw = text.measure(&self.typed, style);
        let x = inner.x + (inner.w - tw - 2.0).min(0.0);
        list.push_clip(RectF::new(inner.x, r.y, inner.w, r.h));
        if self.typed.is_empty() {
            text.draw_text(list, text.snap(inner.x), base, "Name", style.with_color(t.text_faint));
        } else {
            text.draw_text(list, text.snap(x), base, &self.typed, style);
        }
        let (a, d) = (text.ascent(style), text.descent(style));
        list.fill(RectF::new(text.snap(x + tw), base - a, px(text, 1.5), a + d), 0.0, t.accent);
        list.pop_clip();
        self.hits.push((r, Target::Field));
    }

    /// A row of buttons from `y`, centered: each as wide as its label needs (89 px at least),
    /// 13 apart, lighter under the pointer and while held.
    fn buttons(
        &mut self,
        list: &mut DrawList,
        text: &mut TextSystem,
        y: f32,
        row: &[(&str, Look, Target)],
    ) {
        let t = self.theme();
        let style = TextStyle::new(FontId::Sans, 15.0, t.text);
        // A lone button is 144 px at least, one of a row 89.
        let least = if row.len() == 1 { 144.0 } else { 89.0 };
        let widths: Vec<f32> = row
            .iter()
            .map(|b| {
                let w = (text.measure(b.0, style) + 42.0).max(least);
                text.snap(w)
            })
            .collect();
        let total = widths.iter().sum::<f32>() + GAP * (row.len() - 1) as f32;
        let mut x = text.snap((self.size.0 - total) / 2.0);
        for (&(label, look, at), &bw) in row.iter().zip(&widths) {
            let r = RectF::new(x, y, bw, TOUCH);
            let lift = [0.0, 0.12, 0.24]
                [usize::from(self.hover == Some(at)) + usize::from(self.press == Some(at))];
            let (fill, ink) = match look {
                Look::Main => (ui::theme::mix(t.accent, t.accent_text, lift), t.accent_text),
                Look::Danger => (ui::theme::mix(t.danger, t.accent_text, lift), t.accent_text),
                Look::Plain => (ui::theme::mix(t.surface_hi, t.text, lift / 3.0), t.text),
            };
            list.fill(r, r.h / 2.0, fill);
            if look == Look::Plain {
                list.border(r, r.h / 2.0, px(text, 1.0), t.border);
            }
            sheen(list, r, r.h / 2.0, px(text, 1.0), t.highlight.with_alpha(t.highlight.3.min(40)));
            let base = cap_baseline(text, r.y, r.h, 15.0);
            centered(list, text, (r.x + r.w / 2.0, base), label, style.with_color(ink));
            self.focus_ring(list, text, r, at);
            self.hits.push((r, at));
            x += bw + GAP;
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
        let (t, w) = (self.theme(), self.size.0);
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
        centered(list, text, (w / 2.0, cap_baseline(text, y + GAP, LINE_H, 12.0)), &caption, small);
        let bw = tw.max(cw) + 2.0 * GAP;
        let band = RectF::new(text.snap((w - bw) / 2.0), y - GAP, text.snap(bw), TOUCH);
        self.focus_ring(list, text, band, Target::Record);
        self.hits.push((band, Target::Record));
    }

    /// A panel from the home screen's menus' recipe in `r`.
    fn panel(&self, list: &mut DrawList, text: &TextSystem, r: RectF) {
        let (t, line) = (self.theme(), px(text, 1.0));
        list.shadow_offset(r, 13.0, 34.0, 13.0, t.shadow);
        list.fill(r, 13.0, t.base);
        list.fill(r, 13.0, t.surface);
        list.border(r, 13.0, line, t.border);
        sheen(list, r, 13.0, line, t.highlight);
    }

    /// The record's card, 13 px above the hairline: each stage's size and time, then Ready's,
    /// and where the numbers come from.
    fn paint_card(&mut self, list: &mut DrawList, text: &mut TextSystem, rec: &Record) {
        let (t, (w, h)) = (self.theme(), self.size);
        let cw = 280.0f32.min(w - 32.0);
        let small = TextStyle::new(FontId::Sans, 12.0, t.text_dim);
        let foot = text.wrap(CARD_FOOT, small, cw - 2.0 * GAP);
        let rows = rec.rows();
        let ch = 2.0 * GAP + 21.0 * (rows.len() + 1) as f32 + 8.0 + LINE_H * foot.len() as f32;
        let y = text.snap((h - TRACK - GAP - ch).max(BAND));
        let r = RectF::new(text.snap((w - cw) / 2.0), y, text.snap(cw), text.snap(ch));
        self.panel(list, text, r);
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
            text.draw_text(list, text.snap(x), cap_baseline(text, base, LINE_H, 12.0), l, small);
            base += LINE_H;
        }
        self.hits.push((r, Target::Card));
    }

    /// Over the desktop, dimmed: the card that files could not be kept, with Stay and Sign out
    /// anyway.
    fn paint_unkept(&mut self, list: &mut DrawList, text: &mut TextSystem, screen: RectF) {
        let t = self.theme();
        list.fill(screen, 0.0, t.shadow.with_alpha(t.shadow.3 / 2));
        let cw = (screen.w - 32.0).min(360.0);
        let body = TextStyle::new(FontId::Sans, 14.0, t.text_dim);
        let lines = text.wrap(UNKEPT, body, cw - 2.0 * 21.0);
        let ch = 21.0 + PROSE_H + 8.0 + PROSE_H * lines.len() as f32 + 21.0 + TOUCH + 21.0;
        let y = text.snap((screen.h * 0.382 - ch / 2.0).max(GAP));
        let r = RectF::new(text.snap((screen.w - cw) / 2.0), y, text.snap(cw), text.snap(ch));
        self.panel(list, text, r);
        let title = TextStyle::new(FontId::SansBold, 15.0, t.text);
        let mut base = cap_baseline(text, y + 21.0, PROSE_H, 15.0);
        text.draw_text(list, text.snap(r.x + 21.0), base, UNKEPT_TITLE, title);
        base += 8.0;
        for l in lines {
            base += PROSE_H;
            text.draw_text(list, text.snap(r.x + 21.0), base, l, body);
        }
        self.hits.push((r, Target::Card));
        let row = [
            ("Stay", Look::Plain, Target::Stay),
            ("Sign out anyway", Look::Danger, Target::Anyway),
        ];
        self.buttons(list, text, r.y + r.h - 21.0 - TOUCH, &row);
    }

    /// The keys' stops: the circles, then the record.
    pub(crate) fn stops(&self) -> usize {
        match self.state {
            State::Pick => self.circles() + 1,
            _ => 0,
        }
    }
}

/// A circle's menu, for a profile with a PIN (`pinned`) or without.
fn items(pinned: bool) -> Vec<Item<Act>> {
    let mut items = vec![("Rename\u{2026}", "", Some(Act::Rename))];
    match pinned {
        true => items.extend([
            ("Change PIN\u{2026}", "", Some(Act::SetPin)),
            ("Remove PIN", "", Some(Act::Unpin)),
        ]),
        false => items.push(("Set a PIN\u{2026}", "", Some(Act::SetPin))),
    }
    items.push(("Remove profile\u{2026}", "", Some(Act::Remove)));
    items
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
