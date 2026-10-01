//! The dock: the favorites (the person's, kept as the [`PREF`] preference), then the other
//! running apps after a hairline, then the Apps button after another, as tiles on a glass shelf
//! centered above the everything bar, a dot under each running app.

use gfx::{DrawList, RectF};
use host::paint::{faded, px, sheen};
use ui::icon::{Glyph, PHI};
use ui::{AppIcon, TextSystem, Theme};

/// The preference that keeps the favorites (the registry names, joined by commas), and the
/// favorites a new desktop starts with.
pub const PREF: &str = "dock";
pub const DEFAULT: &str = "studio,assistant,terminal,files,settings";
/// The shelf's height with full-size tiles, and the gap above it (and above the bar).
pub const H: f32 = 64.0;
pub const GAP: f32 = 8.0;
/// A tile's side, and the least it shrinks to on a narrow screen; the shelf's padding above the
/// tiles (below them, 12 for the dots); the space between tiles and between groups (a hairline
/// in its middle); the shelf's corner radius; the running dot's side.
pub const TILE: f32 = 44.0;
const MIN_TILE: f32 = 28.0;
const PAD: f32 = 8.0;
const SPACE: f32 = 8.0;
const GROUP: f32 = 21.0;
const RADIUS: f32 = 18.0;
const DOT: f32 = 4.0;

/// The favorites a stored preference names (each once), or [`DEFAULT`]'s if none is stored.
pub fn favorites(stored: Option<&str>) -> Vec<String> {
    let mut favs = Vec::new();
    for name in stored.unwrap_or(DEFAULT).split(',') {
        pin(&mut favs, name, true);
    }
    favs
}

/// Keeps `name` in the dock (`keep`, last) or removes it; whether `favs` changed. An empty name
/// or one with a comma is never kept.
pub fn pin(favs: &mut Vec<String>, name: &str, keep: bool) -> bool {
    match (favs.iter().position(|f| f == name), keep) {
        (None, true) if !name.is_empty() && !name.contains(',') => favs.push(name.to_string()),
        (Some(i), false) => _ = favs.remove(i),
        _ => return false,
    }
    true
}

/// One app on the dock as drawn: its icon, how lifted it is (0 to 1, while hovered), whether it
/// runs and whether one of its windows has the focus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub icon: AppIcon,
    pub lift: f32,
    pub running: bool,
    pub focused: bool,
}

/// Where the dock's parts sit: the shelf, the tiles' side, each slot's left edge (the apps, then
/// the Apps button, last) and the hairlines' x.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shelf {
    pub rect: RectF,
    pub tile: f32,
    pub xs: Vec<f32>,
    pub lines: Vec<f32>,
}

impl Shelf {
    /// The dock of `favs` favorites, `others` other running apps and the Apps button, centered on
    /// a screen `w` wide with its top at `y`; tiles shrink to fit a narrow screen.
    pub fn new(favs: usize, others: usize, w: f32, y: f32) -> Shelf {
        let n = favs + others + 1;
        // Groups start at the first other app and at the Apps button.
        let group = |i: usize| i > 0 && (i == favs && others > 0 || i == n - 1);
        let gaps: f32 = (1..n).map(|i| if group(i) { GROUP } else { SPACE }).sum();
        let room = (w - 2.0 * (GAP + PAD) - gaps) / n as f32;
        let tile = room.clamp(MIN_TILE, TILE).floor();
        let width = 2.0 * PAD + gaps + n as f32 * tile;
        let left = ((w - width) / 2.0).round();
        let (mut x, mut xs, mut lines) = (left + PAD, Vec::new(), Vec::new());
        for i in 0..n {
            if group(i) {
                lines.push(x + GROUP / 2.0);
            }
            x += if group(i) {
                GROUP
            } else if i > 0 {
                SPACE
            } else {
                0.0
            };
            xs.push(x);
            x += tile;
        }
        Shelf { rect: RectF::new(left, y, width, tile + PAD + 12.0), tile, xs, lines }
    }

    /// Slot `i`'s tile (the last slot is the Apps button).
    pub fn tile(&self, i: usize) -> RectF {
        let x = self.xs.get(i).copied().unwrap_or(self.rect.x);
        RectF::new(x, self.rect.y + PAD, self.tile, self.tile)
    }

    /// The slot at `(x, y)`, with half the space between tiles; `Some(None)` on the bare shelf.
    pub fn at(&self, x: f32, y: f32) -> Option<Option<usize>> {
        let (t, half) = (self.tile, SPACE / 2.0);
        let slot = self.xs.iter().position(|&s| x >= s - half && x < s + t + half);
        self.rect.contains(x, y).then_some(slot)
    }

    /// The glass, the hairlines, the apps' tiles (lifted) with their dots, then the Apps button
    /// (washed while `button` is hovered, `Some(held)`).
    pub fn draw(
        &self,
        list: &mut DrawList,
        text: &mut TextSystem,
        theme: &Theme,
        apps: &[Look],
        button: Option<bool>,
    ) {
        let (d, line) = (self.rect, px(text, 1.0));
        list.shadow_offset(d, RADIUS, 34.0, 8.0, theme.shadow);
        list.fill(d, RADIUS, theme.glass);
        list.border(d, RADIUS, line, theme.border);
        sheen(list, d, RADIUS, line, theme.highlight);
        for &x in &self.lines {
            let r = RectF::new(text.snap(x), d.y + PAD + 8.0, line, self.tile - 16.0);
            list.fill(r, 0.0, theme.border);
        }
        for (i, a) in apps.iter().enumerate() {
            let t = self.tile(i);
            let lifted = RectF { y: t.y - 2.0 * a.lift, ..t };
            ui::icon::tile(list, text, lifted, a.icon.glyph, a.icon.hue, theme);
            if a.running {
                let dot = RectF::new(t.x + (t.w - DOT) / 2.0, t.y + t.w + 3.0, DOT, DOT);
                list.fill(dot, DOT / 2.0, if a.focused { theme.accent } else { theme.text_dim });
            }
        }
        // A ghost tile: the glyph as large as on a tile, a wash where the tile would be.
        let b = self.tile(self.xs.len().saturating_sub(1));
        if let Some(down) = button {
            list.fill(b, b.w / PHI.powi(3), theme.wash(down));
        }
        let ink = faded(theme.text, if button.is_some() { 1.0 } else { 0.8 });
        ui::icon::draw(list, text, b.inset(b.w * (1.0 - 1.0 / PHI) / 2.0), Glyph::Apps, ink);
    }
}
