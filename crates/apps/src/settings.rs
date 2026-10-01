//! Settings: the themes, and what compusophyOS is made of.

use gfx::RectF;
use ui::{App, AppEvent, AppIcon, Cx, PAD, RADIUS_LG, RADIUS_SM, Sense, THEMES, Theme, Ui};
use ui::{SPACING, WidgetId};

use crate::kit::{self, Scroll};

const PAGES: [&str; 2] = ["Appearance", "About"];
/// Widget ids: page `i`'s nav item is `NAV + i`, theme card `i` `THEME + i`.
const NAV: u32 = 1;
const THEME: u32 = 10;
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
const CHECK: &str = "✓";

const WHAT: &str = "A whole computer in one browser tab: Rust compiled to WebAssembly, every \
pixel drawn by the GPU. Nothing to install, and your files never leave the tab.";
const FONTS: &str = "Fonts: Inter, JetBrains Mono, and Noto Sans Symbols 1 and 2, under the SIL \
Open Font License 1.1. Their license texts are at /licenses/ on the site.";
const FORKS: &str = "fuel, lang, applang-syntax and applang are forks of litelite 0.2.0 \
(commit 4f5e056, 2026-07-20), under Apache-2.0.";
#[rustfmt::skip]
const STACK: [(&str, &str); 19] = [
    ("os", "The wasm entry point"), ("platform", "The browser boundary: canvas, WebGL2, input"),
    ("shell", "The desktop: chrome, dock and keys"), ("host", "Windows and the apps in them"),
    ("wm", "Window manager; deterministic"), ("apps", "Terminal, Welcome and Settings"),
    ("studio", "Write, check and run apps"), ("guest", "The shell the Terminal runs"),
    ("term", "Terminal screen model"), ("vt", "Terminal escape-sequence parser"),
    ("ui", "Widgets, themes and the App trait"), ("text", "Fonts and glyphs on the atlas"),
    ("font", "TrueType reader and rasterizer"), ("gfx", "Draw lists and the WebGL2 shaders"),
    ("vfs", "In-memory filesystem; deterministic"), ("applang", "The tier 0 app language"),
    ("applang-syntax", "applang's lexer and parser"), ("lang", "Diagnostics, lexer and parser kit"),
    ("fuel", "Fuel and byte budgets"),
];

/// Settings: Appearance (each of [`THEMES`] as a miniature desktop; a click applies it) and
/// About (version, stack, credits), by nav column or, narrow, tabs; tall pages scroll.
#[derive(Debug, Default)]
pub struct Settings {
    /// The page shown, an index into [`PAGES`].
    pub(crate) page: usize,
    scroll: Scroll,
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
            _ => about(ui),
        };
        ui.pop_clip();
        self.scroll.measure(bottom - top + 2.0 * PAD, view.h);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Click(WidgetId(id)) if (NAV..NAV + PAGES.len() as u32).contains(&id) => {
                let page = (id - NAV) as usize;
                if page == self.page {
                    return false;
                }
                (self.page, self.scroll) = (page, Scroll::default());
            }
            AppEvent::Click(WidgetId(id)) if (THEME..THEME + THEMES.len() as u32).contains(&id) => {
                cx.set_theme(THEMES[(id - THEME) as usize].name);
            }
            AppEvent::Wheel { dy, .. } => return self.scroll.wheel(dy),
            AppEvent::Resized { .. } => {}
            _ => return false,
        }
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((720.0, 520.0))
    }

    fn icon(&self) -> AppIcon {
        kit::SETTINGS
    }
}

impl Settings {
    /// The nav column and the line right of it; returns the page's area.
    fn nav(&self, ui: &mut Ui<'_>) -> RectF {
        let (r, t, line) = (ui.rect(), ui.theme(), ui.px(1.0));
        let sep = ui.text_system().snap(r.x + NAV_W);
        ui.fill(RectF::new(sep, r.y, line, r.h), 0.0, t.border);
        for i in 0..PAGES.len() {
            let y = r.y + 12.0 + i as f32 * (ITEM_H + 2.0);
            let item = ui.snapped(RectF::new(r.x + 12.0, y, NAV_W - 24.0, ITEM_H));
            self.item(ui, i, item, false);
        }
        RectF::new(sep + line, r.y, r.x + r.w - sep - line, r.h)
    }

    /// The pages as a segmented control on top; returns the page's area.
    fn tabs(&self, ui: &mut Ui<'_>) -> RectF {
        let (r, t, line) = (ui.rect(), ui.theme(), ui.px(1.0));
        let bar = ui.snapped(RectF::new(r.x + PAD, r.y + 14.0, r.w - 2.0 * PAD, 36.0));
        ui.fill(bar, RADIUS_SM, t.surface_lo);
        ui.border(bar, RADIUS_SM, line, t.border);
        let seg = (bar.w - 8.0) / PAGES.len() as f32;
        for i in 0..PAGES.len() {
            let at = RectF::new(bar.x + 4.0 + i as f32 * seg, bar.y + 4.0, seg, bar.h - 8.0);
            let item = ui.snapped(at);
            self.item(ui, i, item, true);
        }
        let y = bar.y + bar.h + 2.0;
        RectF::new(r.x, y, r.w, r.y + r.h - y)
    }

    /// Nav item or tab `i`: the current one lit, the others dim until hovered.
    fn item(&self, ui: &mut Ui<'_>, i: usize, rect: RectF, tab: bool) {
        let t = ui.theme();
        let id = WidgetId(NAV + i as u32);
        let (hover, down) = kit::pointer(ui, id);
        let on = self.page == i;
        match (on, tab) {
            (true, true) => kit::raised(ui, rect, RADIUS_SM - 2.0, t.surface_hi, t.border),
            (true, false) => ui.fill(rect, RADIUS_SM, t.accent.with_alpha(52)),
            (false, false) if hover => ui.fill(rect, RADIUS_SM, t.wash(down)),
            (false, _) => {}
        }
        let style = t.body().with_color(if on || hover { t.text } else { t.text_dim });
        let w = ui.text_system().measure(PAGES[i], style);
        let x = if tab { rect.x + (rect.w - w) / 2.0 } else { rect.x + 12.0 };
        let base = kit::cap_base(ui, rect.y, rect.h, style);
        let x = ui.text_system().snap(x);
        ui.text(x, base, PAGES[i], style);
        ui.hit(id, rect, Sense::Click);
    }
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
    for (i, th) in THEMES.iter().enumerate() {
        let (col, row) = ((i % cols) as f32, (i / cols) as f32);
        let at = RectF::new(x + col * (cw + CARD_GAP), y + row * (ch + CARD_GAP), cw, ch);
        let card = ui.snapped(at);
        theme_card(ui, i, card, th, ph);
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

/// The About page from the cursor; returns its bottom.
fn about(ui: &mut Ui<'_>) -> f32 {
    let t = ui.theme();
    ui.heading("compusophyOS 0.2");
    ui.wrapped(WHAT, t.body().with_color(t.text_dim));
    ui.subheading("The stack");
    ui.card(stack);
    ui.subheading("Credits");
    ui.label(FONTS);
    ui.label(FORKS);
    let r = ui.label("Author: compusophy");
    r.y + r.h
}

/// The stack as a table: crate names in dim mono, roles beside or under them.
fn stack(ui: &mut Ui<'_>) {
    let t = ui.theme();
    let (key, val) = (t.mono().with_color(t.text_dim), t.body());
    let ((x, mut y), w) = (ui.cursor(), ui.width());
    let ts = ui.text_system();
    let widest = STACK.iter().map(|(k, _)| ts.measure(k, key)).fold(0.0, f32::max);
    let (lh, a, d) = (ts.line_height(val), ts.ascent(val), ts.descent(val));
    let kw = (widest + 20.0).ceil();
    let beside = w >= kw + 240.0;
    for (name, role) in STACK {
        let base = ui.text_system().snap(y + (lh - a - d) / 2.0 + a);
        ui.text(x, base, name, key);
        let (vx, vy) = if beside { (x + kw, y) } else { (x, y + lh) };
        let lines = ui.text_system().wrap(role, val, x + w - vx);
        y = vy + kit::lines(ui, &lines, val, (vx, w), vy, false) + SPACING;
    }
    ui.advance_to(y - SPACING);
}
