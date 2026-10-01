//! Window chrome: the shadows, body, outline, title and controls around the
//! content an app draws; windows drawn moving (through a replayed layer);
//! and the snap preview.

use gfx::{DrawList, RectF};
use host::frame::{CTL, controls};
use host::motion::replay;
use host::paint::{cap_baseline, control_glyph, faded, px, sheen};
use host::{TITLEBAR_H, content_rect};
use ui::{FontId, TextStyle, Theme, UiState};
use wm::{State, WinId};

use crate::Shell;
use crate::desktop::Target;

const RADIUS: f32 = 12.0;
const TITLE_X: f32 = 16.0;
const TITLE_SIZE: f32 = 13.0;
/// Space between the title and the controls.
const TITLE_GAP: f32 = 12.0;

impl Shell {
    /// The shown windows bottom to top (the snap preview just under the
    /// top one, the one being dragged), then those leaving (closing, or
    /// flying into the dock) over them.
    pub(crate) fn draw_windows(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let layout = self.host.wm().layout();
        let mut order: Vec<(WinId, bool)> = layout.iter().map(|p| (p.win, p.focused)).collect();
        for (win, t) in &self.motion.wins {
            if !layout.iter().any(|p| p.win == *win) && t.value(now).a > 0.0 {
                order.push((*win, false));
            }
        }
        let top = layout.last().map(|p| p.win);
        for (win, focused) in order {
            if Some(win) == top {
                self.draw_preview(list, theme, now);
            }
            let Some(t) = self.motion.wins.iter().find(|w| w.0 == win).map(|w| w.1) else {
                continue;
            };
            let (vis, full) = (t.value(now), t.target().rect);
            let maximized = layout.iter().any(|p| p.win == win && p.state == State::Maximized);
            let at = (win, focused, maximized);
            if vis.is_plain() {
                self.draw_window(list, theme, at, [vis.rect, full]);
            } else if vis.a > 0.0 {
                let mut layer = std::mem::take(&mut self.scratch);
                layer.clear();
                self.draw_window(&mut layer, theme, at, [vis.rect, full]);
                replay(list, &layer, vis.xform());
                self.scratch = layer;
            }
        }
    }

    /// Window `win` at `r`, `focused` and `maximized` or not: two shadows
    /// (lighter when unfocused), the body, the app's content (laid out at
    /// the size of `full`, where the window is heading, and clipped to
    /// `r`'s), the outline and its lit top edge, the title and controls.
    fn draw_window(
        &mut self,
        list: &mut DrawList,
        theme: &Theme,
        at: (WinId, bool, bool),
        r: [RectF; 2],
    ) {
        let ((win, focused, maximized), [r, full]) = (at, r);
        let k = if focused { 1.0 } else { 0.6 };
        list.shadow_offset(r, RADIUS, 40.0, 16.0, faded(theme.shadow, k));
        list.shadow_offset(r, RADIUS, 6.0, 2.0, faded(theme.shadow, k / 2.0));
        // The theme's surface is a tint: over the base it is solid, so no
        // window shows through another.
        list.fill(r, RADIUS, theme.base);
        list.fill(r, RADIUS, theme.surface);
        let (c, cf) = (content_rect(r), content_rect(full));
        let pick = |w: Option<crate::Widget>| w.filter(|w| w.0 == win).map(|w| w.1);
        let (hover, pressed) = (pick(self.app_hover), pick(self.app_press));
        let state = UiState { hover, pressed, focused, now_ms: self.now() };
        let layout = RectF { w: cf.w, h: cf.h, ..c };
        self.host.draw_content(list, win, [layout, c], theme, state);
        let line = px(self.host.text(), 1.0);
        list.border(r, RADIUS, line, theme.border);
        sheen(list, r, RADIUS, line, theme.highlight);
        if r.h < TITLEBAR_H {
            return;
        }
        let ctl = controls(r);
        // The title, cut with an ellipsis before the controls.
        let end = ctl.map_or(r.x + r.w - TITLE_X, |c| c[0].x - TITLE_GAP);
        let room = RectF::new(r.x + TITLE_X, r.y, end - r.x - TITLE_X, TITLEBAR_H);
        let color = if focused { theme.text } else { theme.text_dim };
        let style = TextStyle::new(FontId::SansBold, TITLE_SIZE, color);
        if let Some(title) = self.host.win(win).map(|w| w.app.title()).filter(|_| room.w > 0.0) {
            let text = self.host.text_mut();
            let (shown, base) = (
                text.ellipsize(&title, style, room.w),
                cap_baseline(text, r.y, TITLEBAR_H, TITLE_SIZE),
            );
            list.push_clip(room);
            text.draw_text(list, text.snap(room.x), base, &shown, style);
            list.pop_clip();
        }
        let hot = matches!(self.hover, Some(Target::Min(w) | Target::Max(w) | Target::Close(w)) if w == win);
        let ink = theme.surface.with_alpha(255);
        for (i, c) in ctl.into_iter().flatten().enumerate() {
            let fill = if hot && i == 2 { theme.danger } else { theme.text_faint };
            list.fill(c, CTL / 2.0, fill);
            if hot {
                control_glyph(list, self.host.text(), c, i, maximized, [fill, ink]);
            }
        }
    }

    /// Where a dragged window would snap: an accent wash with a fine accent
    /// outline, fading and growing as it comes and goes.
    fn draw_preview(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let v = self.motion.preview.value(now);
        if v.a > 0.0 {
            let line = px(self.host.text(), 1.0);
            list.fill(v.rect, RADIUS, faded(theme.accent.with_alpha(40), v.a));
            list.border(v.rect, RADIUS, line, faded(theme.accent.with_alpha(90), v.a));
        }
    }
}
