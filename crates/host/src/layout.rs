//! Where the desktop's fixed parts sit on the screen: the dock's shelf and
//! tiles, and the launcher's panel with its search field, grid of app
//! tiles and list of files. Pure geometry.

use gfx::RectF;
use ui::{TILE_H, TILE_W};

use crate::paint::ICON;

/// The dock's height, its distance from the screen's bottom, the padding
/// around its tiles and the space between them.
pub const DOCK_H: f32 = 60.0;
pub const DOCK_MARGIN: f32 = 12.0;
pub const DOCK_PAD: f32 = 8.0;
pub const DOCK_GAP: f32 = 10.0;
/// The launcher panel's largest size, its least distance from the screen's
/// edges, its padding, and the search field's, the list rows' and the
/// grid's sizes.
pub const PANEL_W: f32 = 600.0;
pub const PANEL_H: f32 = 440.0;
pub const PANEL_MARGIN: f32 = 24.0;
pub const PANEL_PAD: f32 = 16.0;
pub const FIELD_H: f32 = 48.0;
pub const ROW_H: f32 = 40.0;
pub const COLS: usize = 4;
const LINE_GAP: f32 = 8.0;

/// The shelf of a dock of `n` apps on a `w` x `h` screen: centered, just
/// wide enough, [`DOCK_MARGIN`] above the bottom.
pub fn dock(n: usize, (w, h): (f32, f32)) -> RectF {
    let n = n as f32;
    let dw = 2.0 * DOCK_PAD + n * ICON + (n - 1.0).max(0.0) * DOCK_GAP;
    RectF::new(((w - dw) / 2.0).round(), h - DOCK_MARGIN - DOCK_H, dw, DOCK_H)
}

/// Where tile `i` of that dock rests.
pub fn dock_tile(n: usize, screen: (f32, f32), i: usize) -> RectF {
    let d = dock(n, screen);
    RectF::new(d.x + DOCK_PAD + i as f32 * (ICON + DOCK_GAP), d.y + DOCK_PAD, ICON, ICON)
}

/// What of that dock is at `(x, y)`: `Some(Some(i))` tile `i` (neighbors
/// split the gap between them), `Some(None)` the bare shelf, `None` off it.
pub fn dock_at(n: usize, screen: (f32, f32), x: f32, y: f32) -> Option<Option<usize>> {
    let d = dock(n, screen);
    if n == 0 || !d.contains(x, y) {
        return None;
    }
    let i = ((x - d.x - DOCK_PAD + DOCK_GAP / 2.0) / (ICON + DOCK_GAP)).floor();
    Some((i >= 0.0 && (i as usize) < n).then_some(i as usize))
}

/// The launcher's panel on a screen, holding `tiles` app tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    /// The panel: [`PANEL_W`] x [`PANEL_H`] at most, centered.
    pub rect: RectF,
    /// How many tiles its grid holds.
    pub tiles: usize,
}

impl Panel {
    /// The panel on a `w` x `h` screen, at least [`PANEL_MARGIN`] from its
    /// edges.
    pub fn new((w, h): (f32, f32), tiles: usize) -> Panel {
        let m = 2.0 * PANEL_MARGIN;
        let (pw, ph) = ((w - m).clamp(0.0, PANEL_W), (h - m).clamp(0.0, PANEL_H));
        Panel {
            rect: RectF::new(((w - pw) / 2.0).round(), ((h - ph) / 2.0).round(), pw, ph),
            tiles,
        }
    }

    fn inner_w(&self) -> f32 {
        (self.rect.w - 2.0 * PANEL_PAD).max(0.0)
    }

    /// How many tile columns fit: 1 to [`COLS`].
    pub fn cols(&self) -> usize {
        (self.inner_w() / TILE_W).clamp(1.0, COLS as f32) as usize
    }

    /// The search field across the top.
    pub fn field(&self) -> RectF {
        let p = self.rect;
        RectF::new(p.x + PANEL_PAD, p.y + PANEL_PAD, self.inner_w(), FIELD_H)
    }

    /// Tile `k` of the grid under the field, centered in its column.
    pub fn tile(&self, k: usize) -> RectF {
        let (p, n) = (self.rect, self.cols());
        let cell = self.inner_w() / n as f32;
        let x = p.x + PANEL_PAD + (k % n) as f32 * cell + (cell - TILE_W) / 2.0;
        let y = p.y + 2.0 * PANEL_PAD + FIELD_H + (k / n) as f32 * (TILE_H + LINE_GAP);
        RectF::new(x.round(), y, TILE_W, TILE_H)
    }

    /// Where the list starts: under the grid, or the field when no tiles.
    pub fn list_top(&self) -> f32 {
        let lines = self.tiles.div_ceil(self.cols()) as f32;
        let grid =
            if lines > 0.0 { lines * (TILE_H + LINE_GAP) - LINE_GAP + PANEL_PAD } else { 0.0 };
        self.rect.y + 2.0 * PANEL_PAD + FIELD_H + grid
    }

    /// Row `j` of the list, counted from the first on screen.
    pub fn row(&self, j: usize) -> RectF {
        let y = self.list_top() + j as f32 * ROW_H;
        RectF::new(self.rect.x + PANEL_PAD, y, self.inner_w(), ROW_H)
    }

    /// How many rows fit under the grid.
    pub fn fit(&self) -> usize {
        ((self.rect.y + self.rect.h - PANEL_PAD - self.list_top()) / ROW_H).max(0.0) as usize
    }

    /// What is at `(x, y)` when the list of `rows` rows shows from row
    /// `first`: `None` off the panel, `Some(None)` its bare glass,
    /// `Some(Some(k))` result `k` (the tiles, then the rows).
    pub fn at(&self, x: f32, y: f32, first: usize, rows: usize) -> Option<Option<usize>> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let tile = (0..self.tiles).find(|&k| self.tile(k).contains(x, y));
        let shown = rows.saturating_sub(first).min(self.fit());
        let row = (0..shown).find(|&j| self.row(j).contains(x, y)).map(|j| self.tiles + first + j);
        Some(tile.or(row))
    }
}
