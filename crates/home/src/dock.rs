//! The bottom strip: the AI button, always at the screen's bottom center, where the early iPad's
//! home button was; above it, in a row of its own and only while it holds anything, the dock: one
//! centered glass shelf of tiles, the apps the person keeps there first ([`PREF`]; none at first),
//! then the other running apps, a hairline between the two groups, a dot under each running app.
//! On a narrow screen the tiles shrink evenly to fit between 13 px margins. The work area leaves
//! the button's row free, and the dock's while it shows ([`clear`]).

use gfx::{DrawList, RectF};
use host::paint::{faded, px, sheen};
use ui::icon::{Glyph, PHI};
use ui::{AppIcon, TextSystem, Theme};

/// The preference that keeps the favorites (the registry names, joined by commas).
pub const PREF: &str = "dock";
/// The AI button's side, its space above the screen's bottom, and the gap above it (and above
/// the dock).
pub const BUTTON: f32 = 64.0;
pub const BOTTOM: f32 = 13.0;
pub const GAP: f32 = 8.0;
/// A tile's side, the shelf's padding around its tiles (below them 12, for the dots), and the
/// dock's row (a full shelf's height); the space between tiles, and between the groups (the
/// hairline's); the least margin beside the shelf; its corner radius; the running dot.
pub const TILE: f32 = 44.0;
const PAD: f32 = 8.0;
pub const ROW: f32 = TILE + PAD + 12.0;
const SPACE: f32 = 8.0;
const SEP: f32 = 17.0;
const MARGIN: f32 = 13.0;
const RADIUS: f32 = 18.0;
const DOT: f32 = 4.0;
/// The overlay: a card's widest, a pill's widest and height, the gutters it keeps from the
/// screen's sides ([`Strip::overlay`]).
pub const CARD_W: f32 = 560.0;
pub const PILL_W: f32 = 420.0;
pub const PILL_H: f32 = 52.0;
const GUTTER: f32 = 16.0;

/// What the work area leaves free at the bottom: the button's row (the space under it and the
/// gap above), and the dock's row and its gap while the dock shows (`docked`).
pub fn clear(docked: bool) -> f32 {
    BOTTOM + BUTTON + GAP + if docked { ROW + GAP } else { 0.0 }
}

/// The favorites a stored preference names (each once); none if none is stored.
pub fn favorites(stored: Option<&str>) -> Vec<String> {
    crate::names(stored.unwrap_or(""))
}

/// Adds `name` to the dock (`keep`, last) or removes it; whether `favs` changed. An empty name
/// or one with a comma is never added.
pub fn pin(favs: &mut Vec<String>, name: &str, keep: bool) -> bool {
    match (favs.iter().position(|f| f == name), keep) {
        (None, true) if !name.is_empty() && !name.contains(',') => favs.push(name.to_string()),
        (Some(i), false) => _ = favs.remove(i),
        _ => return false,
    }
    true
}

/// One app on the dock as drawn: its icon (a `.app` file's sigil, if one), how lifted it is (0 to
/// 1, while hovered), whether it runs and whether one of its windows has the focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub icon: AppIcon,
    pub sigil: Option<u32>,
    pub lift: f32,
    pub running: bool,
    pub focused: bool,
}

/// What lies at a point of the strip: the AI button, tile `i` (favorites first), or the bare
/// shelf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Button,
    Tile(usize),
    Shelf,
}

/// Where the strip's parts sit: the AI button; the dock's shelf (0 wide while the dock is empty:
/// nothing drawn, nothing hit), its tiles' side, each tile's left edge (favorites first), and
/// where the hairline between the groups is while both show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub button: RectF,
    pub shelf: RectF,
    pub tile: f32,
    pub xs: Vec<f32>,
    pub sep: Option<f32>,
}

impl Strip {
    /// The strip on a `w` x `h` screen for `favs` favorites and `others` other running apps:
    /// the button centered, the shelf centered above it, its tiles [`TILE`] px but as small as
    /// its margins on the screen need (never off it). On whole px.
    pub fn new(favs: usize, others: usize, (w, h): (f32, f32)) -> Strip {
        let x = (w / 2.0 - BUTTON / 2.0).round();
        let button = RectF::new(x, h - BOTTOM - BUTTON, BUTTON, BUTTON);
        let (n, both) = (favs + others, favs > 0 && others > 0);
        if n == 0 {
            return Strip { button, ..Strip::default() };
        }
        let extra = if both { SEP - SPACE } else { 0.0 };
        let fixed = 2.0 * PAD + (n - 1) as f32 * SPACE + extra;
        // Constant bounds: clamp's panic path folds away.
        let tile = ((w - 2.0 * MARGIN - fixed) / n as f32).floor().clamp(1.0, TILE);
        let (sw, sh) = (fixed + n as f32 * tile, tile + PAD + 12.0);
        let shelf = RectF::new(((w - sw) / 2.0).round(), button.y - GAP - sh, sw, sh);
        let mut xs = Vec::new();
        for i in 0..n {
            let past = if i >= favs { extra } else { 0.0 };
            xs.push(shelf.x + PAD + i as f32 * (tile + SPACE) + past);
        }
        let sep = both.then(|| xs[favs] - (SEP + 1.0) / 2.0);
        Strip { button, shelf, tile, xs, sep }
    }

    /// The strip's top: the shelf's while the dock shows, else the button's.
    pub fn top(&self) -> f32 {
        if self.shelf.w > 0.0 { self.shelf.y } else { self.button.y }
    }

    /// Where the overlay (the Assistant over the desktop) shows on a `w` x `h` screen whose top
    /// `bar` px are the bar's: above the strip, a card [`CARD_W`] wide at most and 60% of the
    /// screen tall at most, centered as the button is (a phone's: a sheet across but its 16 px
    /// gutters, half the screen tall at most); while it works (`pill`), [`PILL_H`] tall and
    /// [`PILL_W`] wide at most. On whole px.
    pub fn overlay(&self, (w, h): (f32, f32), bar: f32, pill: bool) -> RectF {
        let (bottom, narrow) = (self.top() - GAP, crate::narrow(w));
        let ow = (w - 2.0 * GUTTER).min(match (pill, narrow) {
            (true, _) => PILL_W,
            (false, true) => w,
            (false, false) => CARD_W,
        });
        let oh = if pill { PILL_H } else { h * if narrow { 0.5 } else { 0.6 } };
        let oh = oh.min(bottom - bar - GAP).max(0.0);
        let x = ((w - ow) / 2.0).max(GUTTER);
        RectF::new(x.round(), (bottom - oh).round(), ow.round(), oh.round())
    }

    /// Tile `i`'s rect (past the last, the button's).
    pub fn tile(&self, i: usize) -> RectF {
        let at = |&x: &f32| RectF::new(x, self.shelf.y + PAD, self.tile, self.tile);
        self.xs.get(i).map_or(self.button, at)
    }

    /// What is at `(x, y)`; a tile takes half the space beside it.
    pub fn at(&self, x: f32, y: f32) -> Option<Spot> {
        if self.button.contains(x, y) {
            return Some(Spot::Button);
        }
        let half = SPACE / 2.0;
        let tile = self.xs.iter().position(|&s| x >= s - half && x < s + self.tile + half);
        let shelf = self.shelf.w > 0.0 && self.shelf.contains(x, y);
        shelf.then(|| tile.map_or(Spot::Shelf, Spot::Tile))
    }

    /// The shelf (while the dock shows) and its tiles, lifted while hovered, each over a dot
    /// while its app runs (the accent's if it has the focus); the hairline between the groups.
    pub fn draw(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme, apps: &[Look]) {
        let (d, line) = (self.shelf, px(text, 1.0));
        if d.w <= 0.0 {
            return;
        }
        list.shadow_offset(d, RADIUS, 34.0, 8.0, theme.shadow);
        list.fill(d, RADIUS, theme.glass);
        list.border(d, RADIUS, line, theme.border);
        sheen(list, d, RADIUS, line, theme.highlight);
        if let Some(x) = self.sep {
            let k = (self.tile / 4.0).round();
            let hair = RectF::new(text.snap(x), d.y + PAD + k, line, self.tile - 2.0 * k);
            list.fill(hair, 0.0, theme.border);
        }
        for (i, a) in apps.iter().enumerate() {
            let t = self.tile(i);
            let lifted = RectF { y: t.y - 2.0 * a.lift, ..t };
            crate::tile(list, text, lifted, (a.icon, a.sigil), theme);
            if a.running {
                let dot = RectF::new(t.x + (t.w - DOT) / 2.0, t.y + t.w + 3.0, DOT, DOT);
                list.fill(dot, DOT / 2.0, if a.focused { theme.accent } else { theme.text_dim });
            }
        }
    }

    /// The AI button: a round of glass (washed while hovered, `Some(held)`) and the ring around
    /// a dot, 1/φ² of its side, in the theme's ink.
    pub fn draw_button(
        &self,
        list: &mut DrawList,
        text: &mut TextSystem,
        theme: &Theme,
        hover: Option<bool>,
    ) {
        let (b, r, line) = (self.button, BUTTON / 2.0, px(text, 1.0));
        list.shadow_offset(b, r, 21.0, 5.0, faded(theme.shadow, 0.6));
        list.fill(b, r, theme.glass);
        if let Some(down) = hover {
            list.fill(b, r, theme.wash(down));
        }
        list.border(b, r, line, theme.border);
        sheen(list, b, r, line, theme.highlight);
        let side = (BUTTON / (PHI * PHI)).round();
        ui::icon::draw(list, text, b.inset((BUTTON - side) / 2.0), Glyph::Apps, theme.text);
    }
}
