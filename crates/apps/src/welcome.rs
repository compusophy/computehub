//! Welcome: the first screen.

use gfx::RectF;
use ui::icon::PHI;
use ui::{App, AppEvent, AppIcon, Cx, FontId, TextStyle, Ui, WidgetId};

use crate::kit::{self, ROW_H, Row, Scroll};

const TITLE: &str = "compusophy";
const LINE: &str = "a computer in your browser \u{2014} free AI, nothing to install.";
const HINT: &str = "Type anything in the bar below to open an app or ask the Assistant.";
/// The apps, rows 1 to 7: the name to open, what it is called, what it is for, its icon.
const APPS: [(&str, &str, &str, AppIcon); 7] = [
    ("studio", "Studio", "build apps with AI", kit::STUDIO),
    ("assistant", "Assistant", "ask anything", kit::ASSISTANT),
    ("terminal", "Terminal", "a shell and your files", kit::TERMINAL),
    ("files", "Files", "your home folder", kit::FILES),
    ("settings", "Settings", "themes, AI, privacy", kit::SETTINGS),
    ("about", "About", "what this is", kit::ABOUT),
    ("feedback", "Feedback", "tell compusophy what to fix", kit::FEEDBACK),
];
/// The column's widest, the mark's largest side, and the title's size.
const MAX_W: f32 = 466.0;
const MARK_MAX: f32 = 144.0;
const TITLE_SIZE: f32 = 34.0;

/// The first screen: the mark (1/φ of the window's shorter side, at most 144 px) revealed ring by
/// ring when it first draws, the name, a line, how the everything bar works, and the apps as a
/// list, each row opening its app. A short window scrolls.
#[derive(Debug, Default)]
pub struct Welcome {
    scroll: Scroll,
    /// The page clock at the first draw: the reveal's start.
    start: Option<f64>,
}

impl App for Welcome {
    fn title(&self) -> String {
        "Welcome".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        let (x, w) = kit::column(ui, r, MAX_W);
        let now = ui.state().now_ms;
        let since = now - *self.start.get_or_insert(now);
        let side = (r.w.min(r.h) / PHI).min(MARK_MAX).floor().max(0.0);
        let title = TextStyle::new(FontId::SansBold, TITLE_SIZE, t.text);
        let (line, hint) = (t.body().with_color(t.text_dim), t.small());
        let ts = ui.text_system();
        let (line_l, hint_l) = (ts.wrap(LINE, line, w), ts.wrap(HINT, hint, w));
        let text_h = ts.line_height(title)
            + 8.0
            + line_l.len() as f32 * ts.line_height(line)
            + 13.0
            + hint_l.len() as f32 * ts.line_height(hint);
        let total = side + 34.0 + text_h + 34.0 + APPS.len() as f32 * ROW_H;
        let top = 34.0f32.max((r.h - total) / 2.0);
        self.scroll.measure(total + top + 34.0, r.h);
        let mut y = ts.snap(r.y + top - self.scroll.y);
        let at = ui.snapped(RectF::new(r.x + (r.w - side) / 2.0, y, side, side));
        kit::mark(ui, at, since);
        y += side + 34.0;
        y += kit::lines(ui, &[TITLE], title, (x, w), y, true) + 8.0;
        y += kit::lines(ui, &line_l, line, (x, w), y, true) + 13.0;
        y += kit::lines(ui, &hint_l, hint, (x, w), y, true) + 34.0;
        for (i, (_, name, what, icon)) in APPS.iter().enumerate() {
            let row = ui.snapped(RectF::new(x, y + i as f32 * ROW_H, w, ROW_H));
            let item = Row { icon: *icon, name, line: what, aside: "", more: true };
            kit::row(ui, WidgetId(i as u32 + 1), row, &item);
        }
        self.scroll.thumb(ui, r);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Click(WidgetId(i @ 1..=7)) => cx.open(APPS[i as usize - 1].0),
            AppEvent::Wheel { dy, .. } => return self.scroll.wheel(dy),
            AppEvent::Resized { .. } => {}
            _ => return false,
        }
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((520.0, 768.0))
    }

    fn icon(&self) -> AppIcon {
        kit::WELCOME
    }

    fn compact(&self) -> bool {
        true
    }

    fn animating(&self, now_ms: f64) -> bool {
        self.start.is_some_and(|s| now_ms - s < kit::REVEAL_MS)
    }
}
