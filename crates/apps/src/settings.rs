//! Settings: the themes, the AI model, and what gets reported.

use gfx::RectF;
use ui::icon::Glyph;
use ui::{AI_MODEL, App, AppEvent, AppIcon, CARD_PAD, Cx, PAD, RADIUS_LG, RADIUS_SM, REPORTS};
use ui::{SPACING, Sense, THEMES, Theme, Ui, WidgetId};

use crate::kit::{self, Scroll};

const PAGES: [&str; 3] = ["Appearance", "AI", "Privacy"];
/// Widget ids: page `i`'s nav item is `NAV + i`, theme card `i` `THEME + i`, model `i` `MODEL + i`;
/// the reports switch and the feedback line.
const NAV: u32 = 1;
const THEME: u32 = 10;
const MODEL: u32 = 20;
const SWITCH: u32 = 30;
const FEEDBACK: u32 = 31;
/// The models on offer as (name, what it is best at, the value stored), the first the default.
const MODELS: [(&str, &str, &str); 2] =
    [("GLM 5.3", "best answers", "zai/glm-5.3"), ("GLM 5.3 Flash", "fastest", "zai/glm-5.3-flash")];
const NOTE: &str = "AI is free while compusophy is in beta, powered by GLM 5.3. Your prompts go \
to the model through compusophy's server and are not stored there.";
const REPORT: &str = "Send error reports automatically";
const HOLDS: &str = "When something breaks (a program fails, the AI's server errs, or \
compusophyOS itself crashes) a short report goes to compusophy: what failed, the build, your \
browser and screen size, the theme, the apps open and the last 50 events, such as \u{201c}ai \
503\u{201d}. Never your files, your prompts or anything you typed.";
const TYPED: &str = "Feedback you write always sends: you choose what it says, and when.";
/// The nav column's width, and the narrowest window with one (else tabs).
const NAV_W: f32 = 172.0;
const WIDE: f32 = 520.0;
const ITEM_H: f32 = 32.0;
/// The content's left margin beside the nav column.
const MARGIN: f32 = 24.0;
/// Theme cards: narrowest, widest, gap, preview inset, name band.
const CARD_MIN: f32 = 150.0;
const CARD_MAX: f32 = 216.0;
const CARD_GAP: f32 = 16.0;
const INSET: f32 = 6.0;
const NAME_H: f32 = 34.0;
/// Model rows: height and widest (the Privacy page's rows too).
const ROW_H: f32 = 56.0;
const ROW_MAX: f32 = 440.0;
const CHECK: &str = "✓";

/// Settings: Appearance (each of [`THEMES`] as a miniature desktop, the default first; a click
/// applies it), AI (a note on the free AI, and the models as rows; a click picks one, which the
/// host stores) and Privacy (the automatic reports switch, what a report holds, a way to send
/// feedback), by nav column or, narrow, tabs; tall pages scroll.
#[derive(Debug, Default)]
pub struct Settings {
    /// The page shown, an index into [`PAGES`].
    pub(crate) page: usize,
    scroll: Scroll,
    /// The AI model as [`Cx::ai`] last said.
    pub(crate) model: String,
    /// Whether automatic reports are off, as last set here or told by the host, and what the
    /// host last told (it hears a change of ours only from the page, later).
    pub(crate) reports_off: bool,
    told: Option<bool>,
}

impl App for Settings {
    fn title(&self) -> String {
        "Settings".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        let (view, left) = if r.w >= WIDE { (self.nav(ui), MARGIN) } else { (self.tabs(ui), PAD) };
        ui.push_clip(view);
        let top = ui.text_system().snap(view.y + PAD - self.scroll.y);
        ui.set_cursor(view.x + left, top);
        let bottom = match self.page {
            0 => appearance(ui),
            1 => self.ai_page(ui),
            _ => self.privacy(ui),
        };
        ui.pop_clip();
        self.scroll.measure(bottom - top + 2.0 * PAD, view.h);
        self.scroll.thumb(ui, view);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let fresh = self.model != cx.ai.model || self.told != Some(cx.ai.reports_off);
        self.model.clone_from(&cx.ai.model);
        if self.told != Some(cx.ai.reports_off) {
            (self.told, self.reports_off) = (Some(cx.ai.reports_off), cx.ai.reports_off);
        }
        let changed = match ev {
            AppEvent::Click(WidgetId(id)) => self.click(id, cx),
            AppEvent::Wheel { dy, .. } => self.scroll.wheel(dy),
            AppEvent::Resized { .. } => true,
            _ => false,
        };
        fresh || changed
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((720.0, 520.0))
    }

    fn icon(&self) -> AppIcon {
        kit::SETTINGS
    }

    fn compact(&self) -> bool {
        true
    }
}

impl Settings {
    /// A click on widget `id`: a page, a theme, a model, the switch or the feedback line.
    fn click(&mut self, id: u32, cx: &mut Cx<'_>) -> bool {
        let page = id.wrapping_sub(NAV) as usize;
        if let Some(th) = THEMES.get(id.wrapping_sub(THEME) as usize) {
            cx.set_theme(th.name);
        } else if let Some(model) = MODELS.get(id.wrapping_sub(MODEL) as usize) {
            cx.pref(AI_MODEL, model.2);
            self.model.clone_from(&cx.ai.model);
        } else if id == SWITCH {
            self.reports_off = !self.reports_off;
            cx.pref(REPORTS, if self.reports_off { "off" } else { "on" });
        } else if id == FEEDBACK {
            cx.open("feedback");
            return false;
        } else if page < PAGES.len() {
            let new = page != self.page;
            if new {
                (self.page, self.scroll) = (page, Scroll::default());
            }
            return new;
        } else {
            return false;
        }
        true
    }

    /// The AI page from the cursor; returns its bottom.
    fn ai_page(&self, ui: &mut Ui<'_>) -> f32 {
        let t = ui.theme();
        ui.heading("AI");
        ui.wrapped(NOTE, t.body().with_color(t.text_dim));
        ui.subheading("Model");
        // A model not on offer (or none yet) is the default, as the page stores it.
        let on = MODELS.iter().position(|m| m.2 == self.model).unwrap_or(0);
        let (r, (x, y)) = (ui.rect(), ui.cursor());
        let w = (r.x + r.w - PAD - x).min(ROW_MAX);
        let mut bottom = y;
        for (i, model) in MODELS.iter().enumerate() {
            let at = RectF::new(x, y + i as f32 * (ROW_H + SPACING), w, ROW_H);
            let row = ui.snapped(at);
            model_row(ui, i, row, (model.0, model.1), i == on);
            bottom = row.y + row.h;
        }
        ui.advance_to(bottom);
        bottom
    }

    /// The Privacy page from the cursor: the reports switch, what a report holds, and a line
    /// that opens Feedback; returns its bottom.
    fn privacy(&self, ui: &mut Ui<'_>) -> f32 {
        let t = ui.theme();
        ui.heading("Privacy");
        let (r, (x, y)) = (ui.rect(), ui.cursor());
        let w = (r.x + r.w - PAD - x).min(ROW_MAX);
        let row = ui.snapped(RectF::new(x, y, w, ROW_H));
        let id = WidgetId(SWITCH);
        let (hover, down) = kit::pointer(ui, id);
        let (fill, edge) = kit::card_colors(t, hover, down);
        kit::raised(ui, row, RADIUS_LG, fill, edge);
        let label = t.body();
        let base = kit::cap_base(ui, row.y, row.h, label);
        let room = row.w - 2.0 * CARD_PAD - 34.0 - 13.0;
        let shown = ui.text_system().ellipsize(REPORT, label, room);
        ui.text(row.x + CARD_PAD, base, &shown, label);
        kit::switch(ui, row.inset(CARD_PAD), !self.reports_off);
        ui.hit(id, row, Sense::Click);
        ui.advance_to(row.y + row.h);
        ui.space(SPACING);
        ui.wrapped(HOLDS, t.small());
        ui.wrapped(TYPED, t.small());
        let (x, y) = ui.cursor();
        let link = ui.snapped(RectF::new(x, y + SPACING, w, 44.0));
        let (id, style) = (WidgetId(FEEDBACK), t.body().with_color(t.accent));
        let (hover, down) = kit::pointer(ui, id);
        if hover {
            ui.fill(link, RADIUS_SM, t.wash(down));
        }
        let base = kit::cap_base(ui, link.y, link.h, style);
        let end = ui.text(link.x + 8.0, base, "Send feedback", style);
        let at = ui.snapped(RectF::new(link.x + 8.0 + end + 5.0, link.y + 16.0, 13.0, 13.0));
        ui.glyph(at, Glyph::Chevron, t.accent);
        ui.hit(id, link, Sense::Click);
        ui.advance_to(link.y + link.h);
        link.y + link.h
    }

    /// The nav column and the line right of it; returns the page's area.
    fn nav(&self, ui: &mut Ui<'_>) -> RectF {
        let (r, t, line) = (ui.rect(), ui.theme(), ui.px(1.0));
        let sep = ui.text_system().snap(r.x + NAV_W);
        ui.fill(RectF::new(sep, r.y, line, r.h), 0.0, t.border);
        for (i, page) in PAGES.iter().enumerate() {
            let y = r.y + 12.0 + i as f32 * (ITEM_H + 2.0);
            let at = ui.snapped(RectF::new(r.x + 12.0, y, NAV_W - 24.0, ITEM_H));
            item(ui, NAV + i as u32, page, at, [self.page == i, false]);
        }
        RectF::new(sep + line, r.y, r.x + r.w - sep - line, r.h)
    }

    /// The pages as a segmented control on top; returns the page's area.
    fn tabs(&self, ui: &mut Ui<'_>) -> RectF {
        let r = ui.rect();
        let bar = ui.snapped(RectF::new(r.x + PAD, r.y + 14.0, r.w - 2.0 * PAD, 36.0));
        segmented(ui, bar, &PAGES, NAV, self.page);
        let y = bar.y + bar.h + 2.0;
        RectF::new(r.x, y, r.w, r.y + r.h - y)
    }
}

/// A segmented control in `bar`: `labels` as items `id`, `id + 1`, ..., item `on` lit.
fn segmented(ui: &mut Ui<'_>, bar: RectF, labels: &[&str], id: u32, on: usize) {
    let (t, line) = (ui.theme(), ui.px(1.0));
    ui.fill(bar, RADIUS_SM, t.surface_lo);
    ui.border(bar, RADIUS_SM, line, t.border);
    let seg = (bar.w - 8.0) / labels.len() as f32;
    for (i, label) in labels.iter().enumerate() {
        let at = RectF::new(bar.x + 4.0 + i as f32 * seg, bar.y + 4.0, seg, bar.h - 8.0);
        let rect = ui.snapped(at);
        item(ui, id + i as u32, label, rect, [i == on, true]);
    }
}

/// Nav item or tab `id`, `label`ed: lit if `on`, else dim until hovered.
fn item(ui: &mut Ui<'_>, id: u32, label: &str, rect: RectF, [on, tab]: [bool; 2]) {
    let (t, id) = (ui.theme(), WidgetId(id));
    let (hover, down) = kit::pointer(ui, id);
    match (on, tab) {
        (true, true) => kit::raised(ui, rect, RADIUS_SM - 2.0, t.surface_hi, t.border),
        (true, false) => ui.fill(rect, RADIUS_SM, t.accent.with_alpha(52)),
        (false, false) if hover => ui.fill(rect, RADIUS_SM, t.wash(down)),
        (false, _) => {}
    }
    let style = t.body().with_color(if on || hover { t.text } else { t.text_dim });
    let w = ui.text_system().measure(label, style);
    let x = if tab { rect.x + (rect.w - w) / 2.0 } else { rect.x + 12.0 };
    let base = kit::cap_base(ui, rect.y, rect.h, style);
    let x = ui.text_system().snap(x);
    ui.text(x, base, label, style);
    ui.hit(id, rect, Sense::Click);
}

/// Model `i` in `row`: its name over what it is best at, ringed and checked in the accent if `on`.
fn model_row(ui: &mut Ui<'_>, i: usize, row: RectF, (name, best): (&str, &str), on: bool) {
    let t = ui.theme();
    let id = WidgetId(MODEL + i as u32);
    let (hover, down) = kit::pointer(ui, id);
    let (fill, edge) = kit::card_colors(t, hover, down);
    if on {
        let (gap, wide) = (ui.px(3.0), ui.px(2.0));
        ui.border(row.inset(-gap), RADIUS_LG + gap, wide, t.accent);
    }
    kit::raised(ui, row, RADIUS_LG, fill, edge);
    let title = t.body().with_color(if on || hover { t.text } else { t.text_dim });
    let (sub, x) = (t.small(), row.x + CARD_PAD);
    let ts = ui.text_system();
    let top = row.y + (row.h - ts.line_height(title) - ts.line_height(sub)) / 2.0;
    let under = top + ts.line_height(title);
    for (text, style, top) in [(name, title, top), (best, sub, under)] {
        let ts = ui.text_system();
        let (lh, a, d) = (ts.line_height(style), ts.ascent(style), ts.descent(style));
        let base = ts.snap(top + (lh - a - d) / 2.0 + a);
        ui.text(x, base, text, style);
    }
    if on {
        let check = t.body().with_color(t.accent);
        let w = ui.text_system().measure(CHECK, check);
        let at = ui.text_system().snap(row.x + row.w - CARD_PAD - w);
        let base = kit::cap_base(ui, row.y, row.h, check);
        ui.text(at, base, CHECK, check);
    }
    ui.hit(id, row, Sense::Click);
}

/// The Appearance page from the cursor; returns its bottom.
fn appearance(ui: &mut Ui<'_>) -> f32 {
    ui.heading("Appearance");
    ui.small("Pick a theme. The whole desktop follows at once.");
    let (r, (x, y)) = (ui.rect(), ui.cursor());
    let y = y + SPACING;
    let avail = r.x + r.w - PAD - x;
    // `as` saturates (NaN is 0), and clamping an integer cannot panic.
    let cols = (((avail + CARD_GAP) / (CARD_MIN + CARD_GAP)) as usize).clamp(1, THEMES.len());
    let n = cols as f32;
    let cw = ((avail - (n - 1.0) * CARD_GAP) / n).min(CARD_MAX).floor().max(2.0 * INSET);
    let ph = ((cw - 2.0 * INSET) * 0.625).round();
    let ch = INSET + ph + NAME_H;
    // The default first (Mono), then the rest as THEMES has them.
    let first = THEMES.iter().position(|th| th.name == ui::theme("").name).unwrap_or(0);
    let order = (0..THEMES.len()).map(|k| if k == 0 { first } else { k - usize::from(k <= first) });
    for (n, i) in order.enumerate() {
        let (col, row) = ((n % cols) as f32, (n / cols) as f32);
        let at = RectF::new(x + col * (cw + CARD_GAP), y + row * (ch + CARD_GAP), cw, ch);
        let card = ui.snapped(at);
        theme_card(ui, i, card, &THEMES[i], ph);
    }
    let rows = THEMES.len().div_ceil(cols) as f32;
    let bottom = y + rows * ch + (rows - 1.0) * CARD_GAP;
    ui.advance_to(bottom);
    bottom
}

/// Theme card `i`: `th`'s miniature desktop, `ph` tall, over its name.
fn theme_card(ui: &mut Ui<'_>, i: usize, card: RectF, th: &Theme, ph: f32) {
    let t = ui.theme();
    let id = WidgetId(THEME + i as u32);
    let (hover, down) = kit::pointer(ui, id);
    let current = t.name == th.name;
    let (fill, edge) = kit::card_colors(t, hover, down);
    if current {
        let (gap, wide) = (ui.px(3.0), ui.px(2.0));
        ui.border(card.inset(-gap), RADIUS_LG + gap, wide, t.accent);
    }
    kit::raised(ui, card, RADIUS_LG, fill, edge);
    let preview = ui.snapped(RectF::new(card.x + INSET, card.y + INSET, card.w - 2.0 * INSET, ph));
    miniature(ui, preview, th, fill);
    let band = preview.y + preview.h;
    let band_h = card.y + card.h - band;
    let style = t.body().with_color(if current || hover { t.text } else { t.text_dim });
    let base = kit::cap_base(ui, band, band_h, style);
    ui.text(card.x + INSET + 6.0, base, th.name, style);
    if current {
        let check = style.with_color(t.accent);
        let w = ui.text_system().measure(CHECK, check);
        let x = ui.text_system().snap(card.x + card.w - INSET - 6.0 - w);
        ui.text(x, base, CHECK, check);
    }
    ui.hit(id, card, Sense::Click);
}

/// `th`'s desktop in small inside `p` (light, a window, a dock), its corners
/// concentric with the card's, the light trimmed by a ring in `under`.
fn miniature(ui: &mut Ui<'_>, p: RectF, th: &Theme, under: gfx::Rgba) {
    let (t, line) = (ui.theme(), ui.px(1.0));
    let radius = RADIUS_LG - INSET;
    ui.fill(p, radius, th.base);
    ui.push_clip(p);
    for g in th.glows.iter().filter(|g| g.color.3 > 0) {
        let (rx, ry) = (g.rx * p.w, g.ry * p.h);
        let (cx, cy) = (p.x + g.cx * p.w, p.y + g.cy * p.h);
        ui.list().glow(RectF::new(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry), g.color);
    }
    ui.list().grain(p, th.grain, 1.0);
    let u = p.w / 200.0;
    let at = RectF::new(p.x + 0.14 * p.w, p.y + 0.13 * p.h, 0.6 * p.w, 0.56 * p.h);
    let win = ui.snapped(at);
    ui.list().shadow_offset(win, 4.0, 10.0 * u, 3.0 * u, th.shadow);
    ui.fill(win, 4.0, th.surface);
    let bar = (win.h * 0.2).round();
    let divider = ui.snapped(RectF::new(win.x, win.y + bar, win.w, line));
    ui.fill(divider, 0.0, th.border);
    let (pad, stroke) = ((8.0 * u).round(), (3.0 * u).round().max(2.0));
    let bars = [
        (pad, (bar - stroke) / 2.0, 0.3, th.text_faint),
        (pad, bar + pad, 0.56, th.text_dim),
        (pad, bar + pad + 2.0 * stroke, 0.4, th.text_faint),
    ];
    for (dx, dy, frac, color) in bars {
        let r = ui.snapped(RectF::new(win.x + dx, win.y + dy, win.w * frac, stroke));
        ui.fill(r, stroke / 2.0, color);
    }
    let (bw, bh) = ((win.w * 0.26).round(), (2.4 * stroke).round());
    let at = RectF::new(win.x + win.w - pad - bw, win.y + win.h - pad - bh, bw, bh);
    let button = ui.snapped(at);
    ui.fill(button, bh / 2.0, th.accent);
    ui.border(win, 4.0, line, th.border);
    // The dock: a glass pill of app colors.
    let dots = [kit::TERMINAL, kit::STUDIO, kit::SETTINGS, kit::WELCOME];
    let (dot, gap) = ((7.0 * u).round().max(4.0), (4.0 * u).round().max(2.0));
    let (dw, dh) = (4.0 * dot + 3.0 * gap + 2.0 * gap + 2.0, dot + 2.0 * gap);
    let at = RectF::new(p.x + (p.w - dw) / 2.0, p.y + p.h - dh - (0.06 * p.h).round(), dw, dh);
    let dock = ui.snapped(at);
    ui.fill(dock, dh / 2.0, th.glass);
    ui.border(dock, dh / 2.0, line, th.border);
    for (k, icon) in dots.iter().enumerate() {
        let x = dock.x + gap + 1.0 + k as f32 * (dot + gap);
        let r = ui.snapped(RectF::new(x, dock.y + gap, dot, dot));
        ui.fill(r, (dot * 0.3).round(), icon.hue);
    }
    ui.pop_clip();
    let k = ui.px((radius * 0.5).ceil());
    ui.border(p.inset(-k), radius + k, k, under);
    ui.border(p, radius, line, t.border);
}
