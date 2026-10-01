//! The launcher: a glass panel over a veiled desktop with a search field,
//! the built-in apps as tiles and the `.app` files as a list. Typing
//! filters both; the arrows move the selection, Enter or a click opens.

use gfx::{DrawList, RectF};
use host::layout::{FIELD_H, PANEL_PAD as PAD, Panel, ROW_H};
use host::motion::{Tween, Vis, replay};
use host::paint::{ICON as TILE, cap_baseline, draw_icon, magnifier, px, sheen};
use host::search::Search;
use ui::{FontId, TextStyle, Theme};

use crate::desktop::Target;
use crate::{Response, Shell};

/// The built-in apps the grid offers, in order (those the registry knows).
const BUILTIN: [&str; 5] = ["terminal", "studio", "settings", "welcome", "about"];
const RADIUS: f32 = 20.0;
const FIELD_SIZE: f32 = 18.0;
/// Where the query starts in the field, after the search glyph.
const QUERY_X: f32 = 44.0;
const ROW_ICON: f32 = 24.0;
const OPEN_MS: f32 = 160.0;
const CLOSE_MS: f32 = 120.0;
/// The veil's alpha over the desktop, and the panel's scale as it opens.
const VEIL: f32 = 90.0;
const FROM_SCALE: f32 = 0.98;

/// The launcher: whether it shows (and takes the keys), how shown it is
/// (0 to 1, as it fades), and its search.
pub(crate) struct Launcher {
    pub open: bool,
    pub t: Tween<f32>,
    pub search: Search,
}

impl Default for Launcher {
    fn default() -> Launcher {
        Launcher { open: false, t: Tween::new(0.0), search: Search::default() }
    }
}

impl Shell {
    /// The launcher's panel as it is laid out now.
    pub(crate) fn panel(&self) -> Panel {
        Panel::new(self.size, self.launcher.search.tiles.len())
    }

    /// The result under `(x, y)`, the bare panel, or the veil around it.
    pub(crate) fn launcher_hit(&self, x: f32, y: f32) -> Target {
        let l = &self.launcher.search;
        match self.panel().at(x, y, l.first, l.rows.len()) {
            Some(Some(k)) => Target::Item(k),
            Some(None) => Target::Panel,
            None => Target::Veil,
        }
    }

    /// Shows the launcher, its query empty: the built-in apps the registry
    /// knows, then every `.app` file directly in `/apps` and the guest's home.
    pub(crate) fn show_launcher(&mut self) {
        let items = self.host.entries(&BUILTIN);
        let (now, l) = (self.now(), &mut self.launcher);
        (l.search, l.open) = (Search::new(items), true);
        l.t.to(1.0, now, OPEN_MS);
        (self.grab, self.armed) = (None, None);
    }

    /// Hides the launcher, fading.
    pub(crate) fn hide_launcher(&mut self) {
        let now = self.now();
        self.launcher.open = false;
        self.launcher.t.to(0.0, now, CLOSE_MS);
    }

    pub(crate) fn toggle_launcher(&mut self) {
        match self.launcher.open {
            true => self.hide_launcher(),
            false => self.show_launcher(),
        }
    }

    /// Opens result `k` in a new window and hides the launcher.
    pub(crate) fn launch(&mut self, k: usize, out: &mut Response) {
        if let Some(name) = self.launcher.search.get(k).map(|e| e.name.clone()) {
            self.hide_launcher();
            self.host.open(&name, None, out);
        }
    }

    /// The veil, then the panel, fading in and growing from 98%.
    pub(crate) fn draw_launcher(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let t = self.launcher.t.value(now);
        if t <= 0.0 {
            return;
        }
        let screen = RectF::new(0.0, 0.0, self.size.0, self.size.1);
        list.fill(screen, 0.0, theme.base.with_alpha((VEIL * t).round() as u8));
        let p = self.panel();
        if p.rect.w <= 0.0 || p.rect.h <= 0.0 {
            return;
        }
        self.launcher.search.fit = p.fit().max(1);
        if t >= 1.0 {
            return self.draw_panel(list, theme, p);
        }
        let mut layer = std::mem::take(&mut self.scratch);
        layer.clear();
        self.draw_panel(&mut layer, theme, p);
        let s = FROM_SCALE + (1.0 - FROM_SCALE) * t;
        replay(list, &layer, Vis { s, a: t, ..Vis::at(p.rect) }.xform());
        self.scratch = layer;
    }

    /// The panel, the field, then the results or word that there are none.
    fn draw_panel(&mut self, list: &mut DrawList, theme: &Theme, p: Panel) {
        let (r, line) = (p.rect, px(self.host.text(), 1.0));
        // Solid: nothing blurs what is under it, so nothing may show
        // through the results.
        list.shadow_offset(r, RADIUS, 60.0, 24.0, theme.shadow);
        list.fill(r, RADIUS, theme.base);
        list.fill(r, RADIUS, theme.surface);
        list.border(r, RADIUS, line, theme.border);
        sheen(list, r, RADIUS, line, theme.highlight);
        list.push_clip(r);
        self.draw_field(list, theme, p.field());
        let l = &self.launcher.search;
        let (tiles, rows, first) = (l.tiles.clone(), l.rows.clone(), l.first);
        for (k, &i) in tiles.iter().enumerate() {
            self.draw_result(list, theme, k, i, p.tile(k));
        }
        for (j, (n, &i)) in rows.iter().enumerate().skip(first).take(p.fit()).enumerate() {
            self.draw_result(list, theme, tiles.len() + n, i, p.row(j));
        }
        let body = TextStyle::new(FontId::Sans, 14.0, theme.text_dim);
        let text = self.host.text_mut();
        if !tiles.is_empty() && !rows.is_empty() {
            let y = text.snap(p.list_top() - PAD / 2.0);
            list.fill(RectF::new(r.x + PAD, y, r.w - 2.0 * PAD, line), 0.0, theme.border);
        }
        if tiles.is_empty() && rows.is_empty() {
            let msg = "No results";
            let w = text.measure(msg, body);
            let x = text.snap(r.x + (r.w - w) / 2.0);
            text.draw_text(list, x, cap_baseline(text, p.list_top(), ROW_H, 14.0), msg, body);
        }
        list.pop_clip();
    }

    /// The search field: a well with an accent search glyph, the query (or
    /// a faint placeholder) and an accent caret, the end kept in view.
    fn draw_field(&mut self, list: &mut DrawList, theme: &Theme, f: RectF) {
        let text = self.host.text_mut();
        let line = px(text, 1.0);
        list.fill(f, 12.0, theme.wash(false));
        list.border(f, 12.0, line, theme.border);
        magnifier(list, (f.x + 21.0, f.y + FIELD_H / 2.0 - 1.0), theme.accent);
        let style = TextStyle::new(FontId::Sans, FIELD_SIZE, theme.text);
        let q = &self.launcher.search.query;
        let room = RectF::new(f.x + QUERY_X, f.y, (f.w - QUERY_X - PAD).max(0.0), f.h);
        let (base, tw) = (cap_baseline(text, f.y, f.h, FIELD_SIZE), text.measure(q, style));
        let caret_w = px(text, 1.5);
        let x = text.snap(room.x + (room.w - caret_w - tw).min(0.0));
        list.push_clip(room);
        if q.is_empty() {
            let hint = style.with_color(theme.text_faint);
            text.draw_text(list, room.x + 4.0, base, "Search apps and files", hint);
        } else {
            text.draw_text(list, x, base, q, style);
        }
        let (a, d) = (text.ascent(style), text.descent(style));
        list.fill(RectF::new(text.snap(x + tw), base - a, caret_w, a + d), 0.0, theme.accent);
        list.pop_clip();
    }

    /// Result `k` (item `i`) in `r`: a tile (icon over its label) or a row
    /// (icon, label, and where the file lives), washed under the pointer and
    /// ringed in the accent when selected.
    fn draw_result(&mut self, list: &mut DrawList, theme: &Theme, k: usize, i: usize, r: RectF) {
        let (target, l) = (Some(Target::Item(k)), &self.launcher.search);
        let e = &l.items[i];
        let text = self.host.text_mut();
        let radius = if e.place.is_none() { 12.0 } else { 10.0 };
        if self.hover == target {
            list.fill(r, radius, theme.wash(self.armed == target));
        }
        if l.sel == k {
            list.border(r, radius, px(text, 1.5), theme.accent);
        }
        let label = TextStyle::new(FontId::Sans, 12.0, theme.text);
        let Some(place) = &e.place else {
            let icon = RectF::new(r.x + (r.w - TILE) / 2.0, r.y + 8.0, TILE, TILE);
            draw_icon(list, text, icon, e.icon, &e.label, theme.shadow);
            let name = text.ellipsize(&e.label, label, r.w - 8.0);
            let w = text.measure(&name, label);
            let x = text.snap(r.x + (r.w - w) / 2.0);
            let base = cap_baseline(text, icon.y + TILE + 8.0, 16.0, 12.0);
            text.draw_text(list, x, base, &name, label);
            return;
        };
        let icon = RectF::new(r.x + 10.0, r.y + (r.h - ROW_ICON) / 2.0, ROW_ICON, ROW_ICON);
        draw_icon(list, text, icon, e.icon, &e.label, theme.shadow);
        let body = TextStyle::new(FontId::Sans, 14.0, theme.text);
        let x = r.x + 10.0 + ROW_ICON + 12.0;
        let lw = text.draw_text(list, x, cap_baseline(text, r.y, r.h, 14.0), &e.label, body);
        let dim = label.with_color(theme.text_dim);
        let room = r.x + r.w - 12.0 - (x + lw + 24.0);
        let place = text.ellipsize(place, dim, room.max(0.0));
        let (pw, base) = (text.measure(&place, dim), cap_baseline(text, r.y, r.h, 12.0));
        text.draw_text(list, text.snap(r.x + r.w - 12.0 - pw), base, &place, dim);
    }
}
