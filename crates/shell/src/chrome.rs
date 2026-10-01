//! Drawing: each window's frame around the content its app draws (through
//! the host), and the panel across the top (launcher and terminal buttons at
//! the left, workspaces centered, the clock at the right).

use gfx::{DrawList, Icon, RectF};
use ui::{FontId, TextStyle};
use wm::Placement;

use crate::theme::*;
use crate::{PANEL_BTN, PANEL_SPACING, Shell, Target};
use crate::{baseline, rectf, title_buttons};

/// Where a title starts from the window's left edge, and the space kept
/// before the titlebar buttons (or the right edge).
const TITLE_X: f32 = 12.0;
const TITLE_GAP: f32 = 8.0;
const TITLE_STYLE: TextStyle = TextStyle::new(FontId::SansBold, 13.0, TEXT_BRIGHT);
/// The terminal button's label and the clock.
const PROMPT: TextStyle = TextStyle::new(FontId::Mono, 13.0, ICON);
const CLOCK: TextStyle = TextStyle::new(FontId::Sans, 13.0, ICON);
/// The clock's distance from the right edge.
const CLOCK_MARGIN: f32 = 12.0;

impl Shell {
    /// Shadow, body, titlebar, the app's content, border, title, buttons.
    pub(crate) fn draw_window(&mut self, list: &mut DrawList, p: &Placement) {
        let r = rectf(p.rect);
        let (blur, alpha, drop) = if p.floating {
            (SHADOW_BLUR * 1.6, SHADOW.3.saturating_add(50), 6.0)
        } else {
            (SHADOW_BLUR, SHADOW.3, 2.0)
        };
        let shadow = RectF { y: r.y + drop, ..r };
        list.shadow(shadow, RADIUS, blur, SHADOW.with_alpha(alpha));
        list.fill(r, RADIUS, WINDOW);
        if r.h >= crate::CHROME_MIN_H {
            let bar = [TITLEBAR, TITLEBAR_FOCUSED][usize::from(p.focused)];
            list.fill(RectF { h: TITLEBAR_H, ..r }, RADIUS, bar);
            // Square the strip's lower corners where it meets the body.
            let half = TITLEBAR_H / 2.0;
            list.fill(RectF::new(r.x, r.y + half, r.w, half), 0.0, bar);
        }
        self.draw_content(list, p);
        let width = if p.focused { 2.0 } else { 1.5 };
        let color = if p.focused { BORDER_FOCUSED } else { BORDER };
        list.border(r, RADIUS, width, color);
        if r.h >= crate::CHROME_MIN_H {
            self.draw_title(list, r, p);
        }
        let ink = if p.focused { ICON } else { ICON_DIM };
        for (b, hit) in title_buttons(r, p.win).into_iter().flatten() {
            self.highlight(list, b, hit, crate::TITLE_BTN / 2.0);
            let icon = [Icon::Square, Icon::Cross][usize::from(matches!(hit, Target::Close(_)))];
            list.icon(b.inset(4.0), icon, 1.5, ink);
        }
    }

    /// The content well, then the app draws into it and leaves its hits.
    fn draw_content(&mut self, list: &mut DrawList, p: &Placement) {
        let pick = |on: Option<crate::Widget>| on.filter(|h| h.0 == p.win).map(|h| h.1);
        let (hover, pressed) = (pick(self.app_hover), pick(self.app_press));
        self.host.draw_content(list, p, hover, pressed);
    }

    /// The app's title after [`TITLE_X`], cut with an ellipsis before the
    /// buttons.
    fn draw_title(&mut self, list: &mut DrawList, r: RectF, p: &Placement) {
        let Some(w) = self.host.win(p.win) else {
            return;
        };
        let title = w.app.title();
        let x = r.x + TITLE_X;
        let end = match title_buttons(r, p.win) {
            Some([(float, _), _]) => float.x,
            None => r.x + r.w,
        };
        let room = end - TITLE_GAP - x;
        if room <= 0.0 {
            return;
        }
        let color = if p.focused { TEXT_BRIGHT } else { TEXT_DIM };
        let style = TITLE_STYLE.with_color(color);
        let text = self.host.text_mut();
        let shown = text.ellipsize(&title, style, room);
        let base = baseline(text, r.y, TITLEBAR_H, style);
        list.push_clip(RectF::new(x, r.y, room, TITLEBAR_H));
        text.draw_text(list, x, base, &shown, style);
        list.pop_clip();
    }

    /// The panel's buttons: the launcher and the terminal at the left, then
    /// one slot per workspace, centered.
    pub(crate) fn panel_targets(&self) -> Vec<(RectF, Target)> {
        let step = PANEL_BTN + PANEL_SPACING;
        let slot = |x| RectF::new(x, (PANEL_H - PANEL_BTN) / 2.0, PANEL_BTN, PANEL_BTN);
        let mut out = vec![
            (slot(PANEL_SPACING), Target::Launcher),
            (slot(PANEL_SPACING + step), Target::Terminal),
        ];
        let n = self.host.wm().workspace_count();
        let x0 = ((self.size.0 - (n as f32 * step - PANEL_SPACING)) / 2.0).round();
        out.extend((0..n).map(|i| (slot(x0 + i as f32 * step), Target::Workspace(i))));
        out
    }

    /// The bar, its buttons, the workspace indicators (the active one an
    /// accent pill, the rest dots, brighter where windows are open) and the
    /// clock once a tick has set it.
    pub(crate) fn draw_panel(&mut self, list: &mut DrawList) {
        let w = self.size.0;
        list.fill(RectF::new(0.0, 0.0, w, PANEL_H), 0.0, PANEL);
        list.fill(RectF::new(0.0, PANEL_H - 1.0, w, 1.0), 0.0, BORDER);
        let active = self.host.wm().active_workspace();
        for (r, hit) in self.panel_targets() {
            self.highlight(list, r, hit, 8.0);
            match hit {
                Target::Launcher => list.icon(r.inset(6.0), Icon::Grid, 1.5, ICON),
                Target::Terminal => {
                    let text = self.host.text_mut();
                    let tw = text.measure(">_", PROMPT);
                    let x = text.snap(r.x + (r.w - tw) / 2.0);
                    let base = baseline(text, r.y, r.h, PROMPT);
                    text.draw_text(list, x, base, ">_", PROMPT);
                }
                Target::Workspace(i) => {
                    let empty = self.host.wm().layout_of(i).is_empty();
                    let dim = ICON_DIM.with_alpha(if empty { 150 } else { 255 });
                    let dot_w = if i == active { 20.0 } else { 8.0 };
                    let color = if i == active { ACCENT } else { dim };
                    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    let dot = RectF::new(cx - dot_w / 2.0, cy - 4.0, dot_w, 8.0);
                    list.fill(dot, 4.0, color);
                }
                _ => {}
            }
        }
        if let Some(m) = self.clock {
            let d = |n: u32| char::from(b'0' + (n % 10) as u8);
            let s = String::from_iter([d(m / 600), d(m / 60), ':', d(m / 10 % 6), d(m)]);
            let text = self.host.text_mut();
            let tw = text.measure(&s, CLOCK);
            let x = text.snap(w - CLOCK_MARGIN - tw);
            let base = baseline(text, 0.0, PANEL_H, CLOCK);
            text.draw_text(list, x, base, &s, CLOCK);
        }
    }
}
