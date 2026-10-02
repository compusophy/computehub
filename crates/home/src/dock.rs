//! The bottom strip: the AI button at its center, where the early iPad's home button was (aside
//! only as far as a phone's full dock needs), and the dock in two wings beside it, glass shelves of
//! tiles: the person's favorites (kept as the [`PREF`] preference, none at first) to its left, the
//! other running apps to its right, a dot under each running app. The strip's height and place on
//! the screen's bottom never change, so neither does the work area.

use gfx::{DrawList, RectF};
use host::paint::{faded, px, sheen};
use ui::icon::{Glyph, PHI};
use ui::{AppIcon, TextSystem, Theme};

/// The preference that keeps the favorites (the registry names, joined by commas).
pub const PREF: &str = "dock";
/// The strip's height (a full-size wing's), its space above the screen's bottom and the gap
/// above it; the AI button's side.
pub const H: f32 = 64.0;
pub const BOTTOM: f32 = 13.0;
pub const GAP: f32 = 8.0;
pub const BUTTON: f32 = 56.0;
/// A tile's side, and the least it shrinks to beside a centered button (a finger's); a wing's
/// padding around its tiles (below them, 12 for the dots); the space between tiles; the air
/// between the button and a wing, and between a wing and the screen's edge; a wing's corner
/// radius; the running dot.
pub const TILE: f32 = 44.0;
const MIN_TILE: f32 = 32.0;
const PAD: f32 = 8.0;
const SPACE: f32 = 8.0;
const AIR: f32 = 13.0;
const EDGE: f32 = 5.0;
const RADIUS: f32 = 18.0;
const DOT: f32 = 4.0;

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

/// What lies at a point of the strip: the AI button, tile `i` (favorites first), or a bare wing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Button,
    Tile(usize),
    Wing,
}

/// Where the strip's parts sit: the AI button, the wings (left, right; empty ones 0 wide), the
/// tiles' side, and each tile's left edge, the favorites' then the others'.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub button: RectF,
    pub wings: [RectF; 2],
    pub tile: f32,
    pub xs: Vec<f32>,
}

impl Strip {
    /// The strip on a `w` x `h` screen, for `favs` favorites and `others` other running apps:
    /// the button centered, each wing growing out from it; tiles shrink until the fuller wing
    /// fits beside the button. Past 32 px (a phone's full dock) the button slides off center as
    /// far as the wings need to stay on the screen, and only a dock too long for the screen even
    /// so shrinks its tiles further: every tile stays in reach.
    pub fn new(favs: usize, others: usize, (w, h): (f32, f32)) -> Strip {
        let (y, cx) = (h - BOTTOM - H, (w / 2.0).round());
        let wide = |k: usize, tile: f32| match k {
            0 => 0.0,
            k => 2.0 * PAD + k as f32 * tile + (k - 1) as f32 * SPACE,
        };
        let n = favs.max(others).max(1);
        let beside = (cx - BUTTON / 2.0 - AIR - EDGE - wide(n, 0.0)) / n as f32;
        let side = |k: usize, tile: f32| if k > 0 { AIR + wide(k, tile) } else { 0.0 };
        let across = w - 2.0 * EDGE - BUTTON - side(favs, 0.0) - side(others, 0.0);
        let all = across / (favs + others).max(1) as f32;
        let tile = if beside >= MIN_TILE { beside.min(TILE) } else { all.clamp(1.0, MIN_TILE) };
        let tile = tile.floor();
        // Not clamp: its panic path links in float formatting.
        let (lo, hi) = (EDGE + side(favs, tile), w - EDGE - BUTTON - side(others, tile));
        let bx = (cx - BUTTON / 2.0).min(hi).max(lo);
        let button = RectF::new(bx, y + (H - BUTTON) / 2.0, BUTTON, BUTTON);
        let wh = tile + PAD + 12.0;
        let wy = y + ((H - wh) / 2.0).round();
        let left = RectF::new(button.x - AIR - wide(favs, tile), wy, wide(favs, tile), wh);
        let right = RectF::new(button.x + BUTTON + AIR, wy, wide(others, tile), wh);
        let mut xs = Vec::new();
        for (wing, k) in [(left, favs), (right, others)] {
            for i in 0..k {
                xs.push(wing.x + PAD + i as f32 * (tile + SPACE));
            }
        }
        Strip { button, wings: [left, right], tile, xs }
    }

    /// Tile `i`'s rect.
    pub fn tile(&self, i: usize) -> RectF {
        let x = self.xs.get(i).copied().unwrap_or(self.button.x);
        RectF::new(x, self.wings[0].y + PAD, self.tile, self.tile)
    }

    /// What is at `(x, y)`; a tile takes half the space beside it.
    pub fn at(&self, x: f32, y: f32) -> Option<Spot> {
        if self.button.contains(x, y) {
            return Some(Spot::Button);
        }
        let half = SPACE / 2.0;
        let wing = self.wings.iter().any(|w| w.w > 0.0 && w.contains(x, y));
        let tile = self.xs.iter().position(|&s| x >= s - half && x < s + self.tile + half);
        wing.then(|| tile.map_or(Spot::Wing, Spot::Tile))
    }

    /// The wings (those with tiles) and their tiles, lifted while hovered, each over a dot while
    /// its app runs: the accent's if it has the focus.
    pub fn draw(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme, apps: &[Look]) {
        let line = px(text, 1.0);
        for &d in self.wings.iter().filter(|w| w.w > 0.0) {
            list.shadow_offset(d, RADIUS, 34.0, 8.0, theme.shadow);
            list.fill(d, RADIUS, theme.glass);
            list.border(d, RADIUS, line, theme.border);
            sheen(list, d, RADIUS, line, theme.highlight);
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
