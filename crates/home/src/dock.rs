//! The bottom row, one tile high along the screen's bottom: at its left, left-aligned, the dock,
//! the person's own: the apps they keep there ([`PREF`]; none at first), in their order, then
//! the other running apps past a hairline, a dot under each running one; at the bottom-right
//! corner, alone, the Assistant's tile, as the home grid shows it, which shows and hides the
//! overlay ([`Strip::overlay`], anchored to it). A kept tile moves as a home screen icon does
//! (a mouse drags it past 4 px; a finger held on it picks it up, then drags it past 8 px, or
//! lifted unmoved opens its menu); icons carried onto the row open a gap under the pointer,
//! where the dock keeps their apps when they drop. On a narrow screen the dock's tiles shrink
//! evenly to fit beside the Assistant. The work area leaves the row free ([`ROW`]).

use gfx::{DrawList, RectF};
use host::paint::{faded, px};
use host::{ASSISTANT, Effect};
use ui::{AppIcon, TextSystem, Theme};

use crate::grid::Carry;

/// The preference that keeps the favorites (the registry names, joined by commas).
pub const PREF: &str = "dock";
/// A tile's side; the space above the row's tiles, below them (the dots'), and beside its ends;
/// the row's height, which the work area leaves free.
pub const TILE: f32 = 44.0;
const TOP: f32 = 8.0;
const BOTTOM: f32 = 10.0;
const MARGIN: f32 = 10.0;
pub const ROW: f32 = TOP + TILE + BOTTOM;
/// The space between tiles, between the groups (the hairline's), and between the dock and the
/// Assistant; the running dot; how much larger a carried tile is.
const SPACE: f32 = 8.0;
const SEP: f32 = 17.0;
const APART: f32 = 16.0;
const DOT: f32 = 4.0;
const LIFT: f32 = 0.08;
/// The overlay: a card's widest, a pill's widest and height, the gap it keeps above the
/// Assistant (and below the bar), the gutters it keeps from the screen's sides.
pub const CARD_W: f32 = 560.0;
pub const PILL_W: f32 = 420.0;
pub const PILL_H: f32 = 52.0;
const GAP: f32 = 8.0;
const GUTTER: f32 = 16.0;

/// The favorites a stored preference names (each once, never the Assistant, which has its own
/// corner); none if none is stored.
pub fn favorites(stored: Option<&str>) -> Vec<String> {
    let mut favs = crate::names(stored.unwrap_or(""));
    favs.retain(|f| f != ASSISTANT);
    favs
}

/// Whether the dock may keep `name`: never an empty name, one with a comma, or the Assistant.
fn fits(name: &str) -> bool {
    !name.is_empty() && !name.contains(',') && name != ASSISTANT
}

/// Adds `name` to the dock (`keep`, last) or removes it; whether `favs` changed. One that does
/// not fit is never added.
pub fn pin(favs: &mut Vec<String>, name: &str, keep: bool) -> bool {
    match (favs.iter().position(|f| f == name), keep) {
        (None, true) if fits(name) => favs.push(name.to_string()),
        (Some(i), false) => _ = favs.remove(i),
        _ => return false,
    }
    true
}

/// A tile as drawn: its icon (a `.app` file's sigil, if one), where it shows (sliding to its
/// place), how lifted it is (0 to 1, while hovered), its dot's opacity (0: none; a working
/// Assistant's beats) and whether the dot is the focus's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub icon: AppIcon,
    pub sigil: Option<u32>,
    pub r: RectF,
    pub lift: f32,
    pub dot: f32,
    pub focused: bool,
}

impl Look {
    /// The tile, raised as it lifts, over its dot: the accent's if focused, else dim.
    pub fn draw(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme) {
        let (r, icon) = (self.r, (self.icon, self.sigil));
        crate::tile(list, text, RectF { y: r.y - 2.0 * self.lift, ..r }, icon, theme);
        if self.dot > 0.0 {
            let (x, y) = (text.snap(r.x + (r.w - DOT) / 2.0), text.snap(r.y + r.h + 3.0));
            let ink = if self.focused { theme.accent } else { theme.text_dim };
            list.fill(RectF::new(x, y, DOT, DOT), DOT / 2.0, faded(ink, self.dot));
        }
    }

    /// The tile carried: larger, shadowed, at its rect.
    pub fn draw_carried(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme) {
        let r = self.r.inset(-self.r.w * LIFT / 2.0);
        list.shadow_offset(r, r.w / 6.0, 21.0, 8.0, theme.shadow);
        crate::tile(list, text, r, (self.icon, self.sigil), theme);
    }
}

/// What lies at a point of the row: the Assistant, or the dock's tile `i` (favorites first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Assistant,
    Tile(usize),
}

/// Where the row's parts sit: its top, the Assistant's tile, the dock's tiles' side and top,
/// each place's left edge (favorites first), where the hairline between the groups is while
/// both show, and a gap among the places (its first, how many) that no tile takes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub top: f32,
    pub assistant: RectF,
    pub tile: f32,
    pub y: f32,
    pub xs: Vec<f32>,
    pub sep: Option<f32>,
    pub gap: (usize, usize),
}

impl Strip {
    /// The row on a `w` x `h` screen for `favs` favorites and `others` other running apps: the
    /// Assistant [`TILE`] px in the bottom-right corner, the dock's tiles from the left, as
    /// large but as small as the room beside it needs (never off the screen). On whole px.
    pub fn new(favs: usize, others: usize, (w, h): (f32, f32)) -> Strip {
        let top = h - ROW;
        let assistant = RectF::new((w - MARGIN - TILE).round(), top + TOP, TILE, TILE);
        let (n, both) = (favs + others, favs > 0 && others > 0);
        let extra = if both { SEP - SPACE } else { 0.0 };
        let room = assistant.x - APART - MARGIN - n.saturating_sub(1) as f32 * SPACE - extra;
        // Constant bounds: clamp's panic path folds away.
        let tile = (room / n.max(1) as f32).floor().clamp(1.0, TILE);
        let y = top + TOP + ((TILE - tile) / 2.0).round();
        let past = |i: usize| if i >= favs { extra } else { 0.0 };
        let xs: Vec<f32> = (0..n).map(|i| MARGIN + i as f32 * (tile + SPACE) + past(i)).collect();
        let sep = xs.get(favs).filter(|_| both).map(|x| x - (SEP + 1.0) / 2.0);
        Strip { top, assistant, tile, y, xs, sep, gap: (0, 0) }
    }

    /// Where the overlay (the Assistant over the desktop) shows on a `w` x `h` screen whose top
    /// `bar` px are the bar's: above the Assistant, its right edge the Assistant's, a card
    /// [`CARD_W`] wide at most and 60% of the screen tall at most (a phone's: a sheet out to
    /// the row's first tile, half the screen tall at most); while it works (`pill`), [`PILL_H`]
    /// tall and [`PILL_W`] wide at most. Inside 16 px gutters (a phone's: the row's own). On
    /// whole px.
    pub fn overlay(&self, (w, h): (f32, f32), bar: f32, pill: bool) -> RectF {
        let (bottom, narrow) = (self.assistant.y - GAP, crate::narrow(w));
        let right = self.assistant.x + self.assistant.w;
        let ow = (right - if narrow { MARGIN } else { GUTTER }).min(match (pill, narrow) {
            (true, _) => PILL_W,
            (false, true) => w,
            (false, false) => CARD_W,
        });
        let oh = if pill { PILL_H } else { h * if narrow { 0.5 } else { 0.6 } };
        let (ow, oh) = (ow.max(0.0), oh.min(bottom - bar - GAP).max(0.0));
        RectF::new((right - ow).round(), (bottom - oh).round(), ow.round(), oh.round())
    }

    /// Tile `i`'s rect, past the gap (past the last, the Assistant's).
    pub fn tile(&self, i: usize) -> RectF {
        let at = |&x: &f32| RectF::new(x, self.y, self.tile, self.tile);
        let (g, k) = self.gap;
        self.xs.get(if i < g { i } else { i + k }).map_or(self.assistant, at)
    }

    /// What is at `(x, y)`: in the row, the Assistant out to the screen's corner, a tile with
    /// half the space beside it (in the gap, none).
    pub fn at(&self, x: f32, y: f32) -> Option<Spot> {
        let ((g, k), half) = (self.gap, SPACE / 2.0);
        if y < self.top {
            return None;
        }
        if x >= self.assistant.x - half {
            return Some(Spot::Assistant);
        }
        let place = self.xs.iter().position(|&s| x >= s - half && x < s + self.tile + half);
        let tile = place.filter(|&p| p < g || p >= g + k);
        tile.map(|p| Spot::Tile(if p < g { p } else { p - k }))
    }

    /// The slot among `kept` places for a tile carried with its middle at `x`.
    pub fn slot(&self, x: f32, kept: usize) -> usize {
        // `as` saturates: NaN and what is left of the first are 0.
        let i = ((x - MARGIN + SPACE / 2.0) / (self.tile + SPACE)) as usize;
        i.min(kept.saturating_sub(1))
    }

    /// The hairline between the groups, then the tiles `looks` (the Assistant's among them).
    pub fn draw(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme, looks: &[Look]) {
        if let Some(x) = self.sep {
            let k = (self.tile / 4.0).round();
            let hair = RectF::new(text.snap(x), self.y + k, px(text, 1.0), self.tile - 2.0 * k);
            list.fill(hair, 0.0, theme.border);
        }
        for look in looks {
            look.draw(list, text, theme);
        }
    }
}

/// Moves `items[from]` to `to`, those between shifting toward its place (no further than the
/// end).
pub fn shift<T>(items: &mut [T], from: usize, to: usize) {
    let mut i = from;
    while i < to && i + 1 < items.len() {
        items.swap(i, i + 1);
        i += 1;
    }
    while i > to && i < items.len() {
        items.swap(i - 1, i);
        i -= 1;
    }
}

/// The dock as it behaves: the person's favorites (kept as [`PREF`]), the places in them of
/// those that show (the registry knows them), where the row's parts sit, a kept tile pressed
/// or carried, as a home screen icon is ([`Carry`]: its place among those shown, and its
/// slot), and the apps whose icons are carried in over the row that it would keep (its gap
/// open for them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dock {
    pub favs: Vec<String>,
    pub kept: Vec<usize>,
    pub strip: Strip,
    pub carry: Option<Carry>,
    pub incoming: Vec<String>,
}

impl Dock {
    /// A dock with the favorites a stored preference names.
    pub fn new(stored: Option<&str>) -> Dock {
        Dock { favs: favorites(stored), ..Dock::default() }
    }

    /// Lays the row out on a `size` screen for `n` tiles, first the favorites at `kept` (their
    /// places in the favorites, as they show), then the other running apps. Apps carried in
    /// (`over`: their names, and the pointer's x) that it would keep (none kept already) open a
    /// gap as wide as they are among the kept tiles, at the slot under the pointer. A tile
    /// carried that is no longer kept is put down.
    pub fn layout(&mut self, kept: Vec<usize>, n: usize, size: (f32, f32), over: (&[String], f32)) {
        let favs = &self.favs;
        self.incoming = over.0.iter().filter(|a| fits(a) && !favs.contains(a)).cloned().collect();
        let k = self.incoming.len();
        self.strip = Strip::new(kept.len() + k, n.saturating_sub(kept.len()), size);
        if k > 0 {
            self.strip.gap = (self.strip.slot(over.1, kept.len() + 1), k);
        }
        self.carry = self.carry.take().filter(|c| c.lead < kept.len());
        self.kept = kept;
    }

    /// Keeps the apps carried in, if any, where their gap shows (the preference to `fx`).
    pub fn take_in(&mut self, fx: &mut Vec<Effect>) {
        if self.incoming.is_empty() {
            return;
        }
        let at = self.kept.get(self.strip.gap.0).map_or(self.favs.len(), |&f| f);
        for (k, name) in std::mem::take(&mut self.incoming).into_iter().enumerate() {
            self.favs.insert(at + k, name);
        }
        self.save(fx);
    }

    /// Where the tile carried goes, from its place to its slot: the tiles show so ([`shift`]).
    pub fn moving(&self) -> Option<(usize, usize)> {
        self.carry.as_ref().map(|c| (c.lead, c.slot))
    }

    /// Adds `name` to the dock or removes it, keeping the favorites (their preference to `fx`)
    /// if they changed.
    pub fn keep(&mut self, name: &str, keep: bool, fx: &mut Vec<Effect>) {
        if pin(&mut self.favs, name, keep) {
            self.save(fx);
        }
    }

    fn save(&self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Pref { key: PREF.to_string(), value: crate::joined(&self.favs) });
    }

    /// Tile `i` pressed at `at` (a finger's, which held it, if `touch`: lifted at once), if a
    /// kept one: carried once it travels.
    pub fn pick(&self, i: usize, at: (f32, f32), touch: bool) -> Option<Carry> {
        let (r, icons) = (self.strip.tile(i), vec![i]);
        let off = (at.0 - r.x, at.1 - r.y);
        let c =
            Carry { icons, lead: i, from: at, off, touch, lifted: touch, moved: false, slot: i };
        (i < self.kept.len()).then_some(c)
    }

    /// The pointer moved to `at`: a carried tile follows it once it traveled far enough, and the
    /// slot under it opens. Whether it just started to move (a press is no click then).
    pub fn carry_to(&mut self, at: Option<(f32, f32)>) -> bool {
        let (Some(at), Some(c)) = (at, &mut self.carry) else { return false };
        let start = c.travel(at);
        if c.moved {
            c.slot = self.strip.slot(at.0 - c.off.0 + self.strip.tile / 2.0, self.kept.len());
        }
        start
    }

    /// Where the carried tile shows with the pointer at `at`, while lifted: its slot and its
    /// rect, held where it was pressed.
    pub fn carried(&self, at: Option<(f32, f32)>) -> Option<(usize, RectF)> {
        let (c, at) = (self.carry.as_ref().filter(|c| c.lifted)?, at?);
        let side = self.strip.tile;
        Some((c.slot, RectF::new(at.0 - c.off.0, at.1 - c.off.1, side, side)))
    }

    /// Puts a carried tile down: if `keep` and it moved to a new slot, its favorite moves to the
    /// place of the one there, the others between shifting (the preference to `fx`); else it
    /// goes back.
    pub fn drop(&mut self, keep: bool, fx: &mut Vec<Effect>) {
        let Some(c) = self.carry.take().filter(|c| keep && c.moved && c.slot != c.lead) else {
            return;
        };
        if let (Some(&from), Some(&to)) = (self.kept.get(c.lead), self.kept.get(c.slot)) {
            shift(&mut self.favs, from, to);
            self.save(fx);
        }
    }
}
