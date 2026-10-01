//! Welcome: the first screen.

use gfx::RectF;
use ui::{App, AppEvent, AppIcon, CARD_PAD, Cx, PAD, RADIUS_LG, Sense, Ui, WidgetId};

use crate::kit::{self, Scroll};

const TITLE: &str = "compusophy";
const SUBTITLE: &str = "a computer in your browser — nothing installed, nothing leaves this tab.";
const HINT: &str = "Alt+Space opens the launcher · drag windows to the edges to snap";
/// The cards, widgets 1 to 3: the app to open, its title, a line, its icon.
const CARDS: [(&str, &str, &str, AppIcon); 3] = [
    ("terminal", "Terminal", "a shell and your files", kit::TERMINAL),
    ("studio", "Studio", "build apps in applang", kit::STUDIO),
    ("settings", "Settings", "themes and more", kit::SETTINGS),
];
/// The widest content, and the narrowest card three across (else stacked).
const MAX_W: f32 = 680.0;
const CARD_MIN: f32 = 168.0;
/// Between cards; a card's icon side.
const GAP: f32 = 12.0;
const ICON: f32 = 36.0;
/// Between a card's icon and its text, and its name and line.
const ICON_GAP: f32 = 14.0;
const NAME_GAP: f32 = 2.0;
/// Room kept for the arrow a card shows under the pointer, when stacked.
const ARROW_ROOM: f32 = 20.0;
const ARROW: &str = "→";

/// The first screen: a title, a line, three cards that open Terminal, Studio
/// and Settings (stacked when narrow) and a hint, centered a little above the
/// middle; a window too short scrolls with the wheel.
#[derive(Debug, Default)]
pub struct Welcome {
    scroll: Scroll,
}

impl App for Welcome {
    fn title(&self) -> String {
        "Welcome".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        let side = if r.w >= 560.0 { 40.0 } else { PAD };
        let w = (r.w - 2.0 * side).clamp(0.0, MAX_W).floor();
        let x = ui.text_system().snap(r.x + (r.w - w) / 2.0);
        let three = w >= 3.0 * CARD_MIN + 2.0 * GAP;
        let (title, sub) = (t.title(), t.body().with_color(t.text_dim));
        let (name, small) = (t.subheading(), t.small());
        let ts = ui.text_system();
        let (title_l, sub_l, hint_l) =
            (ts.wrap(TITLE, title, w), ts.wrap(SUBTITLE, sub, w), ts.wrap(HINT, small, w));
        // Cards: as tall as the one with the most lines.
        let cw = if three { ((w - 2.0 * GAP) / 3.0).floor() } else { w };
        let text_w = cw - 2.0 * CARD_PAD - if three { 0.0 } else { ICON + ICON_GAP + ARROW_ROOM };
        let descs: Vec<Vec<&str>> = CARDS.iter().map(|c| ts.wrap(c.2, small, text_w)).collect();
        let most = descs.iter().map(Vec::len).max().unwrap_or(1) as f32;
        let (name_h, small_h) = (ts.line_height(name), ts.line_height(small));
        let text_h = name_h + NAME_GAP + most * small_h;
        let ch = match three {
            true => 2.0 * CARD_PAD + ICON + ICON_GAP + text_h,
            false => 2.0 * CARD_PAD + text_h.max(ICON),
        };
        let cards_h = if three { ch } else { 3.0 * ch + 2.0 * GAP };
        let title_h = title_l.len() as f32 * ts.line_height(title);
        let sub_h = sub_l.len() as f32 * ts.line_height(sub);
        let hint_h = hint_l.len() as f32 * small_h;
        let total = title_h + 6.0 + sub_h + 28.0 + cards_h + 24.0 + hint_h;
        self.scroll.measure(total + 2.0 * side, r.h);
        let top = r.y + side.max((r.h - total) * 0.45) - self.scroll.y;
        let mut y = ts.snap(top);
        y += kit::lines(ui, &title_l, title, (x, w), y, true) + 6.0;
        y += kit::lines(ui, &sub_l, sub, (x, w), y, true) + 28.0;
        for (i, card) in CARDS.iter().enumerate() {
            let at = match three {
                true => RectF::new(x + i as f32 * (cw + GAP), y, cw, ch),
                false => RectF::new(x, y + i as f32 * (ch + GAP), cw, ch),
            };
            let rect = kit::snapped(ui, at);
            self.card(ui, WidgetId(i as u32 + 1), rect, (card, &descs[i]), three);
        }
        y += cards_h + 24.0;
        kit::lines(ui, &hint_l, small, (x, w), y, true);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Click(WidgetId(i @ 1..=3)) => cx.open(CARDS[i as usize - 1].0),
            AppEvent::Wheel { dy, .. } => return self.scroll.wheel(dy),
            AppEvent::Resized { .. } => {}
            _ => return false,
        }
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((720.0, 420.0))
    }

    fn icon(&self) -> AppIcon {
        kit::WELCOME
    }
}

/// A card and its description's lines.
type Card<'c> = (&'c (&'c str, &'c str, &'c str, AppIcon), &'c [&'c str]);

impl Welcome {
    /// One card: its icon above (`tall`) or beside its text; an arrow on hover.
    fn card(&self, ui: &mut Ui<'_>, id: WidgetId, rect: RectF, card: Card<'_>, tall: bool) {
        let ((_, name, _, icon), desc) = card;
        let t = ui.theme();
        let (hover, down) = kit::pointer(ui, id);
        let (fill, edge) = kit::card_colors(t, hover, down);
        kit::raised(ui, rect, RADIUS_LG, fill, edge);
        let (style, small) = (t.subheading(), t.small());
        let ts = ui.text_system();
        let (name_h, small_h) = (ts.line_height(style), ts.line_height(small));
        let text_h = name_h + NAME_GAP + desc.len() as f32 * small_h;
        let (icon_at, tx, ty) = match tall {
            true => {
                let at = RectF::new(rect.x + CARD_PAD, rect.y + CARD_PAD, ICON, ICON);
                (at, at.x, at.y + ICON + ICON_GAP)
            }
            false => {
                let at = RectF::new(rect.x + CARD_PAD, rect.y + (rect.h - ICON) / 2.0, ICON, ICON);
                (at, at.x + ICON + ICON_GAP, rect.y + (rect.h - text_h) / 2.0)
            }
        };
        let icon_r = kit::snapped(ui, icon_at);
        kit::icon(ui, icon_r, *icon);
        let ty = ui.text_system().snap(ty);
        let w = rect.x + rect.w - tx;
        kit::lines(ui, &[*name], style, (tx, w), ty, false);
        kit::lines(ui, desc, small, (tx, w), ty + name_h + NAME_GAP, false);
        if hover {
            let arrow = t.body().with_color(t.text_dim);
            let aw = ui.text_system().measure(ARROW, arrow);
            let base = kit::cap_base(ui, icon_r.y, icon_r.h, arrow);
            ui.text(rect.x + rect.w - CARD_PAD - aw, base, ARROW, arrow);
        }
        ui.hit(id, rect, Sense::Click);
    }
}
