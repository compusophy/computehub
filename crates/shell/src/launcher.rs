//! The launcher: the everything bar's focus and query, and the panel of results above it over a
//! veiled desktop. Typing into the bar opens the panel; the arrows move the selection, Enter or
//! a click opens an app or asks the Assistant.

use gfx::{DrawList, RectF};
use home::panel::{Panel, ROW_H};
use host::motion::{Tween, Vis, replay};
use host::search::Search;
use ui::Theme;

use crate::desktop::Target;
use crate::{BAR_H, Response, Shell};

/// The apps the grid offers, in order (those the registry knows).
const BUILTIN: [&str; 8] =
    ["studio", "assistant", "terminal", "files", "settings", "welcome", "about", "feedback"];
const OPEN_MS: f32 = 160.0;
const CLOSE_MS: f32 = 120.0;
/// The veil's alpha, and the panel's scale as it opens.
const VEIL: f32 = 90.0;
const FROM_SCALE: f32 = 0.98;

/// Whether the panel shows, whether the bar takes the keys, how shown the panel is (0 to 1, as
/// it fades), the search, and wheel or finger travel short of a whole row.
#[derive(Default)]
pub(crate) struct Launcher {
    pub open: bool,
    pub focus: bool,
    pub t: Tween<f32>,
    pub search: Search,
    pub travel: f32,
}

impl Shell {
    pub(crate) fn panel(&self) -> Panel {
        let (s, field) = (&self.launcher.search, home::field::rect(self.size));
        Panel::new(self.size.0, field, BAR_H, s.ask(), s.tiles.len(), s.rows.len())
    }

    pub(crate) fn launcher_hit(&self, x: f32, y: f32) -> Target {
        let l = &self.launcher.search;
        match self.panel().at(x, y, l.first, l.rows.len()) {
            Some(Some(k)) => Target::Item(k),
            Some(None) => Target::Panel,
            None => Target::Veil,
        }
    }

    /// Gives the bar the keys with a fresh search, and asks for text input again: a tap on the
    /// bar brings back a phone keyboard put away.
    pub(crate) fn focus_field(&mut self) {
        if !self.launcher.focus {
            let items = self.host.entries(&BUILTIN);
            (self.launcher.search, self.launcher.focus) = (Search::new(items), true);
        }
        self.ime = None;
    }

    /// Shows the panel, the bar focused.
    pub(crate) fn show_launcher(&mut self) {
        self.focus_field();
        let (now, l) = (self.host.now_ms, &mut self.launcher);
        l.open = true;
        l.t.to(1.0, now, OPEN_MS);
        (self.grab, self.armed) = (None, None);
    }

    /// Hides the panel and takes the keys (and the query) from the bar.
    pub(crate) fn hide_launcher(&mut self) {
        let l = &mut self.launcher;
        (l.open, l.focus) = (false, false);
        l.t.to(0.0, self.host.now_ms, CLOSE_MS);
    }

    pub(crate) fn toggle_launcher(&mut self) {
        if self.launcher.open { self.hide_launcher() } else { self.show_launcher() }
    }

    /// Text typed into the bar; any shows the panel.
    pub(crate) fn typed(&mut self, s: &str) {
        self.launcher.search.type_text(s);
        if !self.launcher.open && !self.launcher.search.query.is_empty() {
            self.show_launcher();
        }
    }

    /// Opens result `k` or asks the Assistant (the Ask row), and hides the launcher.
    pub(crate) fn launch(&mut self, k: usize, out: &mut Response) {
        let s = &self.launcher.search;
        let (name, query) = (s.get(k).map(|e| e.name.clone()), s.query.clone());
        if name.is_none() && !(s.ask() && k == 0) {
            return;
        }
        self.hide_launcher();
        match name {
            Some(name) => self.host.open(&name, None, out),
            None => self.host.ask(&query, out),
        }
    }

    /// Scrolls the list by `dy` px, a row at a time.
    pub(crate) fn scroll_launcher(&mut self, dy: f32) {
        let l = &mut self.launcher;
        l.travel += dy;
        let rows = (l.travel / ROW_H).trunc();
        l.travel -= rows * ROW_H;
        l.search.scroll(rows as isize);
    }

    /// The veil, then the panel, fading in and growing.
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
        let hot = match self.hover {
            Some(Target::Item(k)) => Some((k, self.armed == self.hover)),
            _ => None,
        };
        let (s, text) = (&self.launcher.search, &mut self.host.text);
        if t >= 1.0 {
            return p.draw(list, text, theme, s, hot);
        }
        let mut layer = std::mem::take(&mut self.scratch);
        layer.clear();
        p.draw(&mut layer, text, theme, s, hot);
        let s = FROM_SCALE + (1.0 - FROM_SCALE) * t;
        replay(list, &layer, Vis { s, a: t, ..Vis::at(p.rect) });
        self.scratch = layer;
    }

    /// The everything bar, showing the query while it has the keys.
    pub(crate) fn draw_field(&mut self, list: &mut DrawList, theme: &Theme) {
        let (l, r) = (&self.launcher, home::field::rect(self.size));
        let hover = (self.hover == Some(Target::Field)).then_some(self.armed == self.hover);
        let query = if l.focus { l.search.query.as_str() } else { "" };
        home::field::draw(list, &mut self.host.text, theme, r, query, l.focus, hover);
    }
}
