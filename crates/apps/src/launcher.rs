//! The Launcher: a search box over the apps, in a floating window.

use gfx::RectF;
use ui::{App, AppEvent, Cx, FontId, Key, PAD, Sense, TextStyle, Ui, WidgetId, theme};

/// Row `k` of the filtered list is widget `ROW + k`; the field is 1.
const ROW: u32 = 100;
const ROW_H: f32 = 34.0;
const LABEL: TextStyle = TextStyle::new(FontId::Sans, 14.0, theme::TEXT_BRIGHT);
/// The built-in apps; [`Cx::open`] takes their names in lowercase.
const BUILTIN: [&str; 4] = ["Terminal", "Studio", "Welcome", "About"];

/// The app launcher: the built-in apps and every `*.app` in `/apps` and the
/// guest's home (by file name). Typing filters (a subsequence of the name,
/// ignoring ASCII case, earliest first match first), Up and Down
/// choose, Enter or a click opens the app and closes the launcher, Escape
/// closes it.
#[derive(Debug)]
pub struct Launcher {
    /// Label and what [`Cx::open`] gets: a registry name or a `.app` path.
    items: Vec<(String, String)>,
    query: String,
    /// Indices into `items` that match, best first.
    shown: Vec<usize>,
    /// The chosen row of `shown`, and the first row on screen.
    sel: usize,
    first: usize,
    /// Whether the `.app` files were listed (at the first event).
    loaded: bool,
}

/// Where `query` first matches `label` as a subsequence, ignoring ASCII
/// case: the char index of its first char, 0 for an empty query, `None` for
/// no match.
pub(crate) fn rank(query: &str, label: &str) -> Option<usize> {
    let mut hay = label.chars().enumerate();
    let mut first = None;
    for q in query.chars() {
        let (i, _) = hay.find(|&(_, h)| h.eq_ignore_ascii_case(&q))?;
        first.get_or_insert(i);
    }
    Some(first.unwrap_or(0))
}

/// A launcher showing the built-in apps.
impl Default for Launcher {
    fn default() -> Launcher {
        let items = BUILTIN.map(|l| (l.to_string(), l.to_ascii_lowercase())).into();
        let mut l = Launcher {
            items,
            query: String::new(),
            shown: Vec::new(),
            sel: 0,
            first: 0,
            loaded: false,
        };
        l.filter();
        l
    }
}

impl Launcher {
    /// Lists the matches by rank, then in order (an insertion sort: no sort code).
    fn filter(&mut self) {
        let mut ranked: Vec<(usize, usize)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            if let Some(r) = rank(&self.query, &item.0) {
                let at = ranked.iter().take_while(|e| e.0 <= r).count();
                ranked.insert(at, (r, i));
            }
        }
        self.shown = ranked.into_iter().map(|(_, i)| i).collect();
        (self.sel, self.first) = (0, 0);
    }

    /// Opens row `k` and closes the launcher.
    fn launch(&self, k: usize, cx: &mut Cx<'_>) {
        if let Some(&i) = self.shown.get(k) {
            cx.open(&self.items[i].1);
            cx.close_self();
        }
    }
}

impl App for Launcher {
    fn title(&self) -> String {
        "Launcher".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        ui.text_field(WidgetId(1), &self.query, true, "Search apps");
        let ((x, y), w, r) = (ui.cursor(), ui.width(), ui.rect());
        if self.shown.is_empty() {
            ui.small("No app matches.");
            return;
        }
        // Keep the chosen row on screen.
        let fit = ((r.y + r.h - PAD - y) / ROW_H).floor().max(1.0) as usize;
        let lowest = (self.sel + 1).saturating_sub(fit);
        self.first = self.first.min(self.sel).max(lowest);
        let hover = ui.state().hover;
        for (k, &i) in self.shown.iter().enumerate().skip(self.first).take(fit) {
            let row = RectF::new(x, y + (k - self.first) as f32 * ROW_H, w, ROW_H - 4.0);
            let id = WidgetId(ROW + k as u32);
            if k == self.sel {
                ui.fill(row, 8.0, theme::SELECTION);
            } else if hover == Some(id) {
                ui.fill(row, 8.0, theme::HOVER);
            }
            let ts = ui.text_system();
            let (a, d) = (ts.ascent(LABEL), ts.descent(LABEL));
            let base = ts.snap(row.y + (row.h - a - d) / 2.0 + a);
            ui.text(row.x + 10.0, base, &self.items[i].0, LABEL);
            ui.hit(id, row, Sense::Click);
        }
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        if !std::mem::replace(&mut self.loaded, true) {
            for path in guest::app_files(cx.vfs) {
                let name = path.rsplit('/').next().unwrap_or_default().to_string();
                self.items.push((name, path));
            }
            self.filter();
        }
        let (last, typed) = (self.shown.len().saturating_sub(1), self.query.len());
        match ev {
            AppEvent::Key { key, .. } => match key {
                Key::Up => self.sel = self.sel.saturating_sub(1),
                Key::Down => self.sel = (self.sel + 1).min(last),
                Key::Enter => self.launch(self.sel, cx),
                Key::Escape => cx.close_self(),
                Key::Backspace => _ = self.query.pop(),
                _ => return false,
            },
            AppEvent::Text(t) => self.query.extend(t.chars().filter(|c| !c.is_control())),
            AppEvent::Click(WidgetId(id)) if id >= ROW => self.launch((id - ROW) as usize, cx),
            AppEvent::Resized { .. } | AppEvent::Focus(_) => {}
            _ => return false,
        }
        if self.query.len() != typed {
            self.filter();
        }
        true
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((440.0, 380.0))
    }
}
