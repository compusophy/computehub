//! The launcher's panel, anchored above the everything bar: the Ask row while there is a query,
//! the apps as a grid of tiles, then the files as a list, as tall as they need up to the room
//! above the bar; a full-width sheet on a narrow screen.

use gfx::{DrawList, RectF};
use host::paint::{cap_baseline, px, sheen};
use host::search::Search;
use ui::icon::Glyph;
use ui::{FontId, TILE_H, TILE_W, TextStyle, TextSystem, Theme};

/// The panel's largest width, padding, corner radius and gap above the bar; the grid's columns
/// and line gap; a row's height (the Ask row's too), a row's icon and a tile's.
const MAX_W: f32 = 600.0;
const PAD: f32 = 13.0;
const RADIUS: f32 = 21.0;
const GAP: f32 = 8.0;
const COLS: usize = 4;
const LINE_GAP: f32 = 8.0;
pub const ROW_H: f32 = 44.0;
const ROW_ICON: f32 = 24.0;
const TILE: f32 = 44.0;

/// The panel on screen, with the Ask row (`ask`) and `tiles` app tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub rect: RectF,
    pub ask: bool,
    pub tiles: usize,
}

impl Panel {
    /// The panel above `field` on a screen `w` wide whose work area starts at `top`, for the Ask
    /// row (if `ask`), `tiles` tiles and `rows` rows.
    pub fn new(w: f32, field: RectF, top: f32, ask: bool, tiles: usize, rows: usize) -> Panel {
        let pw = if crate::narrow(w) { w } else { (w - 32.0).min(MAX_W) }.max(0.0);
        let rect = RectF::new(((w - pw) / 2.0).round(), 0.0, pw, 0.0);
        let mut p = Panel { rect, ask, tiles };
        let need = p.list_top(rows) + rows as f32 * ROW_H + PAD;
        let bottom = field.y - GAP;
        let h = need.min(bottom - top - GAP).max(0.0).round();
        (p.rect.y, p.rect.h) = (bottom - h, h);
        p
    }

    fn inner_w(&self) -> f32 {
        (self.rect.w - 2.0 * PAD).max(0.0)
    }

    pub fn cols(&self) -> usize {
        (self.inner_w() / TILE_W).clamp(1.0, COLS as f32) as usize
    }

    /// The Ask row (where it would be, if there is none).
    pub fn ask_row(&self) -> RectF {
        RectF::new(self.rect.x + PAD, self.rect.y + PAD, self.inner_w(), ROW_H)
    }

    fn grid_top(&self) -> f32 {
        self.rect.y + PAD + if self.ask { ROW_H + LINE_GAP } else { 0.0 }
    }

    pub fn tile(&self, k: usize) -> RectF {
        let (n, top) = (self.cols(), self.grid_top());
        let cell = self.inner_w() / n as f32;
        let x = self.rect.x + PAD + (k % n) as f32 * cell + (cell - TILE_W) / 2.0;
        RectF::new(x.round(), top + (k / n) as f32 * (TILE_H + LINE_GAP), TILE_W, TILE_H)
    }

    /// Where the list starts below the grid, when it has `rows`.
    fn list_top(&self, rows: usize) -> f32 {
        let (lines, top) = (self.tiles.div_ceil(self.cols()) as f32, self.grid_top());
        let grid = lines * (TILE_H + LINE_GAP) - LINE_GAP;
        match () {
            _ if lines > 0.0 => top + grid + if rows > 0 { PAD } else { 0.0 },
            _ if self.ask && rows == 0 => top - LINE_GAP,
            _ => top,
        }
    }

    /// Row `j` of the list on screen.
    pub fn row(&self, j: usize) -> RectF {
        let y = self.list_top(1) + j as f32 * ROW_H;
        RectF::new(self.rect.x + PAD, y, self.inner_w(), ROW_H)
    }

    /// How many rows fit.
    pub fn fit(&self) -> usize {
        ((self.rect.y + self.rect.h - PAD - self.list_top(1)) / ROW_H).max(0.0) as usize
    }

    /// Result `k` (the Ask row, the tiles, then the rows) at `(x, y)` while the list shows from
    /// row `first` of `rows`; `Some(None)` on the bare panel.
    pub fn at(&self, x: f32, y: f32, first: usize, rows: usize) -> Option<Option<usize>> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let skip = usize::from(self.ask);
        let ask = (self.ask && self.ask_row().contains(x, y)).then_some(0);
        let tile = (0..self.tiles).find(|&k| self.tile(k).contains(x, y)).map(|k| k + skip);
        let shown = rows.saturating_sub(first).min(self.fit());
        let row = (0..shown).find(|&j| self.row(j).contains(x, y));
        Some(ask.or(tile).or(row.map(|j| skip + self.tiles + first + j)))
    }

    /// The (solid) panel and `s`'s results; `hot` is the result under the pointer (and whether
    /// it is held).
    pub fn draw(
        &self,
        list: &mut DrawList,
        text: &mut TextSystem,
        theme: &Theme,
        s: &Search,
        hot: Option<(usize, bool)>,
    ) {
        let (r, line) = (self.rect, px(text, 1.0));
        list.shadow_offset(r, RADIUS, 55.0, 21.0, theme.shadow);
        list.fill(r, RADIUS, theme.base);
        list.fill(r, RADIUS, theme.surface);
        list.border(r, RADIUS, line, theme.border);
        sheen(list, r, RADIUS, line, theme.highlight);
        list.push_clip(r);
        let mark = |list: &mut DrawList, text: &TextSystem, k: usize, r: RectF, radius: f32| {
            if let Some((_, down)) = hot.filter(|h| h.0 == k) {
                list.fill(r, radius, theme.wash(down));
            }
            if s.sel == k {
                list.border(r, radius, px(text, 1.5), theme.accent);
            }
        };
        let body = TextStyle::new(FontId::Sans, 14.0, theme.text);
        if self.ask {
            let a = self.ask_row();
            mark(list, text, 0, a, 13.0);
            let g = RectF::new(a.x + 13.0, a.y + (ROW_H - 20.0) / 2.0, 20.0, 20.0);
            ui::icon::draw(list, text, g, Glyph::Assistant, theme.accent);
            let label = ["Ask the Assistant \u{2014} \u{201c}", &s.query, "\u{201d}"].concat();
            let x = g.x + 20.0 + 13.0;
            let label = text.ellipsize(&label, body, a.x + a.w - 13.0 - x);
            text.draw_text(list, x, cap_baseline(text, a.y, ROW_H, 14.0), &label, body);
        }
        let skip = usize::from(self.ask);
        let small = TextStyle::new(FontId::Sans, 12.0, theme.text);
        for (k, &i) in s.tiles.iter().enumerate() {
            let (t, e) = (self.tile(k), &s.items[i]);
            mark(list, text, k + skip, t, 13.0);
            let icon = RectF::new(t.x + (t.w - TILE) / 2.0, t.y + 8.0, TILE, TILE);
            ui::icon::tile(list, text, icon, e.icon.glyph, e.icon.hue, theme);
            let name = text.ellipsize(&e.label, small, t.w - 8.0);
            let nw = text.measure(&name, small);
            let x = text.snap(t.x + (t.w - nw) / 2.0);
            text.draw_text(
                list,
                x,
                cap_baseline(text, icon.y + TILE + 8.0, 16.0, 12.0),
                &name,
                small,
            );
        }
        if !s.tiles.is_empty() && !s.rows.is_empty() {
            let y = text.snap(self.list_top(1) - PAD / 2.0);
            list.fill(RectF::new(r.x + PAD, y, r.w - 2.0 * PAD, line), 0.0, theme.border);
        }
        let dim = small.with_color(theme.text_dim);
        let first = skip + s.tiles.len() + s.first;
        for (j, &i) in s.rows.iter().skip(s.first).take(self.fit()).enumerate() {
            let (row, e) = (self.row(j), &s.items[i]);
            mark(list, text, first + j, row, 10.0);
            let icon =
                RectF::new(row.x + 10.0, row.y + (ROW_H - ROW_ICON) / 2.0, ROW_ICON, ROW_ICON);
            ui::icon::tile(list, text, icon, e.icon.glyph, e.icon.hue, theme);
            let x = icon.x + ROW_ICON + 13.0;
            let lw =
                text.draw_text(list, x, cap_baseline(text, row.y, ROW_H, 14.0), &e.label, body);
            let place = e.place.as_deref().unwrap_or("");
            let room = row.x + row.w - 13.0 - (x + lw + 21.0);
            let place = text.ellipsize(place, dim, room.max(0.0));
            let pw = text.measure(&place, dim);
            let at = text.snap(row.x + row.w - 13.0 - pw);
            text.draw_text(list, at, cap_baseline(text, row.y, ROW_H, 12.0), &place, dim);
        }
        list.pop_clip();
    }
}
