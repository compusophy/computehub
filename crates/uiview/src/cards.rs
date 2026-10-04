//! The OS's own widgets, as Settings shows them: a window's pages ([`Node::Pages`]: a column at
//! its left, or narrow a segmented control on top), raised cards lighter under the pointer and
//! sunk when held (a [`Node::Choice`], ringed and checked when chosen; a [`Node::Switch`]; the
//! desktop's themes, [`Node::Themes`], each a miniature of its desktop), a profile's faces
//! ([`Node::Faces`]) and a link ([`uiwire::Variant::Link`]).
//!
//! [`Node::Pages`]: uiwire::Node::Pages
//! [`Node::Choice`]: uiwire::Node::Choice
//! [`Node::Switch`]: uiwire::Node::Switch
//! [`Node::Themes`]: uiwire::Node::Themes
//! [`Node::Faces`]: uiwire::Node::Faces

use gfx::RectF;
use ui::icon::{FACES, Glyph};
use ui::{CARD_PAD, PAD, RADIUS_LG, RADIUS_SM, Rgba, Sense, THEMES, Theme, Ui, WidgetId, sem};

use crate::cap_base;

/// The pages' column: its width, its items' height, and the margin right of its line.
const NAV_W: f32 = 172.0;
const ITEM_H: f32 = 32.0;
const MARGIN: f32 = 24.0;
/// Theme cards: the gap between them, the miniature's inset in its card, the name's band.
const GAP: f32 = 16.0;
const INSET: f32 = 6.0;
const NAME_H: f32 = 34.0;
const CHECK: &str = "\u{2713}";
/// The gap between faces.
const FACE_GAP: f32 = 13.0;

/// Whether `id` is under the pointer, and whether it is also held.
fn pointer(ui: &Ui<'_>, id: WidgetId) -> (bool, bool) {
    let s = ui.state();
    let hover = s.hover == Some(id);
    (hover, hover && s.pressed == Some(id))
}

/// A raised card in `r` lighter under the pointer and sunk when held, as `id`'s: its fill.
fn card(ui: &mut Ui<'_>, id: WidgetId, r: RectF, radius: f32) -> (Rgba, bool) {
    let t = ui.theme();
    let (hover, down) = pointer(ui, id);
    let fill = match (hover, down) {
        (_, true) => t.pressed(t.surface_hi),
        (true, false) => t.hover(t.surface_hi),
        _ => t.surface_hi,
    };
    let edge = if hover { t.text.with_alpha(t.border.3.saturating_mul(2)) } else { t.border };
    raised(ui, r, radius, fill, edge);
    (fill, hover)
}

/// A raised rounded rect: `fill`, a 1 px `edge`, and the theme's light top.
fn raised(ui: &mut Ui<'_>, r: RectF, radius: f32, fill: Rgba, edge: Rgba) {
    let (t, line) = (ui.theme(), ui.px(1.0));
    ui.fill(r, radius, fill);
    ui.border(r, radius, line, edge);
    ui.push_clip(RectF::new(r.x, r.y, r.w, line));
    ui.border(r, radius, line, t.highlight);
    ui.pop_clip();
}

/// The accent's 2 px ring 3 px outside the card `r`: it is chosen.
fn ring(ui: &mut Ui<'_>, r: RectF) {
    let (gap, wide) = (ui.px(3.0), ui.px(2.0));
    ui.border(r.inset(-gap), RADIUS_LG + gap, wide, ui.theme().accent);
}

/// A check in the accent ending at `right`, on `base`.
fn check(ui: &mut Ui<'_>, right: f32, base: f32) {
    let style = ui.theme().body().with_color(ui.theme().accent);
    let w = ui.text_system().measure(CHECK, style);
    let x = ui.text_system().snap(right - w);
    ui.text(x, base, CHECK, style);
}

/// A switch 34 x 21 centered on `r`'s height, `inset` in from its right edge: an accent track
/// with its knob right when `on`, else a sunken one with its knob left; its track.
pub(crate) fn switch(ui: &mut Ui<'_>, r: RectF, inset: f32, on: bool) -> RectF {
    let (t, line) = (ui.theme(), ui.px(1.0));
    let at = RectF::new(r.x + r.w - inset - 34.0, r.y + (r.h - 21.0) / 2.0, 34.0, 21.0);
    let track = ui.snapped(at);
    let (fill, knob) = if on { (t.accent, t.accent_text) } else { (t.surface_lo, t.text_dim) };
    ui.fill(track, 10.5, fill);
    if !on {
        ui.border(track, 10.5, line, t.border);
    }
    let x = if on { track.x + track.w - 18.0 } else { track.x + 3.0 };
    let dot = ui.snapped(RectF::new(x, track.y + 3.0, 15.0, 15.0));
    ui.fill(dot, dot.w / 2.0, knob);
    track
}

/// A [`Node::Choice`](uiwire::Node::Choice) in `r`: its first line over its second (small),
/// the first dim unless chosen or under the pointer; the ring and a check when `on`.
pub(crate) fn choice(ui: &mut Ui<'_>, id: WidgetId, r: RectF, text: &str, on: bool) {
    let t = ui.theme();
    if on {
        ring(ui, r);
    }
    let hover = card(ui, id, r, RADIUS_LG).1;
    let title = t.body().with_color(if on || hover { t.text } else { t.text_dim });
    let (name, more) = text.split_once('\n').unwrap_or((text, ""));
    let (sub, x, ts) = (t.small(), r.x + CARD_PAD, ui.text_system());
    let lines = ts.line_height(title) + if more.is_empty() { 0.0 } else { ts.line_height(sub) };
    let top = r.y + (r.h - lines) / 2.0;
    let under = top + ts.line_height(title);
    for (text, style, top) in [(name, title, top), (more, sub, under)] {
        let ts = ui.text_system();
        let (lh, a, d) = (ts.line_height(style), ts.ascent(style), ts.descent(style));
        let base = ts.snap(top + (lh - a - d) / 2.0 + a);
        ui.text(x, base, text, style);
    }
    if on {
        let base = cap_base(ui, r.y, r.h, t.body());
        check(ui, r.x + r.w - CARD_PAD, base);
    }
    ui.hit(id, r, Sense::Click);
    ui.mark(id, sem::OPTION, if on { sem::SELECTED } else { 0 }, "");
}

/// A [`Node::Switch`](uiwire::Node::Switch) in `r`: `label` (cut to fit) and the switch.
pub(crate) fn switch_card(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) {
    card(ui, id, r, RADIUS_LG);
    let style = ui.theme().body();
    let base = cap_base(ui, r.y, r.h, style);
    let room = r.w - 2.0 * CARD_PAD - 34.0 - 13.0;
    let shown = ui.text_system().ellipsize(label, style, room);
    ui.text(r.x + CARD_PAD, base, &shown, style);
    switch(ui, r, CARD_PAD, on);
    ui.hit(id, r, Sense::Click);
    ui.mark(id, sem::SWITCH, if on { sem::CHECKED } else { 0 }, "");
}

/// A [`Variant::Link`](uiwire::Variant::Link) in `r`: `label` in the accent and a chevron after
/// it, washed under the pointer.
pub(crate) fn link(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str) -> RectF {
    let t = ui.theme();
    let (hover, down) = pointer(ui, id);
    if hover {
        ui.fill(r, RADIUS_SM, t.wash(down));
    }
    let style = t.body().with_color(t.accent);
    let base = cap_base(ui, r.y, r.h, style);
    let end = ui.text(r.x + 8.0, base, label, style);
    let at = ui.snapped(RectF::new(r.x + 8.0 + end + 5.0, r.y + 16.0, 13.0, 13.0));
    ui.glyph(at, Glyph::Chevron, t.accent);
    ui.hit(id, r, Sense::Click);
    ui.mark(id, sem::LINK, 0, "");
    r
}

/// The window's pages, `labels` one a line (page `i` the button `id + i`, page `on` lit): a
/// column at the left and a line right of it on a window [`uiwire::WIDE`] or wider, else a
/// segmented control across the top. The rect left for the rest, and its left margin.
pub(crate) fn pages(ui: &mut Ui<'_>, id: u32, on: u8, labels: &str) -> (RectF, f32) {
    let (r, t, line) = (ui.rect(), ui.theme(), ui.px(1.0));
    let tab = r.w < f32::from(uiwire::WIDE);
    let bar = ui.snapped(RectF::new(r.x + PAD, r.y + 14.0, r.w - 2.0 * PAD, 36.0));
    let rest = if tab {
        ui.fill(bar, RADIUS_SM, t.surface_lo);
        ui.border(bar, RADIUS_SM, line, t.border);
        let y = bar.y + bar.h + 2.0;
        (RectF::new(r.x, y, r.w, r.y + r.h - y), PAD)
    } else {
        let sep = ui.text_system().snap(r.x + NAV_W);
        ui.fill(RectF::new(sep, r.y, line, r.h), 0.0, t.border);
        (RectF::new(sep + line, r.y, r.x + r.w - sep - line, r.h), MARGIN)
    };
    // A tab is its label's width and an even share of the rest, so a long label still fits.
    let (body, n) = (t.body(), labels.split('\n').count() as f32);
    let widths = labels.split('\n').map(|l| ui.text_system().measure(l, body)).sum::<f32>();
    let (spare, mut x) = ((bar.w - 8.0 - widths) / n, bar.x + 4.0);
    for (i, label) in labels.split('\n').enumerate() {
        let (k, seg) = (i as f32, ui.text_system().measure(label, body) + spare);
        let at = match tab {
            true => RectF::new(x, bar.y + 4.0, seg, bar.h - 8.0),
            false => RectF::new(r.x + 12.0, r.y + 12.0 + k * (ITEM_H + 2.0), NAV_W - 24.0, ITEM_H),
        };
        x += seg;
        let at = ui.snapped(at);
        item(ui, WidgetId(id.wrapping_add(i as u32)), label, at, [usize::from(on) == i, tab]);
    }
    rest
}

/// A page's item or tab `id` in `r`, `label`ed: lit if `on`, else dim until hovered.
fn item(ui: &mut Ui<'_>, id: WidgetId, label: &str, r: RectF, [on, tab]: [bool; 2]) {
    let t = ui.theme();
    let (hover, down) = pointer(ui, id);
    match (on, tab) {
        (true, true) => raised(ui, r, RADIUS_SM - 2.0, t.surface_hi, t.border),
        (true, false) => ui.fill(r, RADIUS_SM, t.accent.with_alpha(52)),
        (false, false) if hover => ui.fill(r, RADIUS_SM, t.wash(down)),
        (false, _) => {}
    }
    let style = t.body().with_color(if on || hover { t.text } else { t.text_dim });
    let w = ui.text_system().measure(label, style);
    let x = if tab { r.x + (r.w - w) / 2.0 } else { r.x + 12.0 };
    let base = cap_base(ui, r.y, r.h, style);
    let x = ui.text_system().snap(x);
    ui.text(x, base, label, style);
    ui.hit(id, r, Sense::Click);
    ui.mark(id, sem::TAB, if on { sem::SELECTED } else { 0 }, "");
}

/// The faces `w` wide: how many across (as many as fit, the rows even), and their height.
pub(crate) fn faces_size(w: f32) -> (usize, f32) {
    let (n, side) = (usize::from(FACES), f32::from(uiwire::FACE));
    // `as` saturates (NaN is 0).
    let rows = n.div_ceil((((w + FACE_GAP) / (side + FACE_GAP)) as usize).clamp(1, n));
    (n.div_ceil(rows), rows as f32 * (side + FACE_GAP) - FACE_GAP)
}

/// The faces from `(x, y)`, `w` wide, face `i` the button `id + i`: a ring holding `i` dots,
/// face `on`'s ring in the accent and its dots in the text color, as under the pointer (washed
/// there); the rest faint.
pub(crate) fn faces(ui: &mut Ui<'_>, id: u32, on: u8, (x, y, w): (f32, f32, f32)) {
    let (cols, side, t) = (faces_size(w).0, f32::from(uiwire::FACE), ui.theme());
    for i in 0..FACES {
        let (k, wid) = (usize::from(i), WidgetId(id.wrapping_add(u32::from(i))));
        let step = |n: usize| n as f32 * (side + FACE_GAP);
        let r = ui.snapped(RectF::new(x + step(k % cols), y + step(k / cols), side, side));
        let (hover, down) = pointer(ui, wid);
        if hover {
            ui.fill(r, side / 2.0, t.wash(down));
        }
        let lit = i == on || hover;
        let ring = if i == on {
            t.accent
        } else if hover {
            t.text_dim
        } else {
            t.text_faint
        };
        ui.face(r, i, [ring, if lit { t.text } else { t.text_dim }]);
        ui.hit(wid, r, Sense::Click);
        let mut dots = String::new();
        ui::push_num(&mut dots, k);
        ui.mark(wid, sem::OPTION, if i == on { sem::SELECTED } else { 0 }, &dots);
    }
}

/// The theme cards `w` wide: how many across, a card's width, its miniature's height, and
/// their height.
pub(crate) fn themes_size(w: f32) -> (usize, f32, f32, f32) {
    let n = THEMES.len();
    let (least, most) = (f32::from(uiwire::THEME_MIN), f32::from(uiwire::THEME_MAX));
    // `as` saturates (NaN is 0).
    let cols = (((w + GAP) / (least + GAP)) as usize).max(1).min(n);
    let k = cols as f32;
    let cw = ((w - (k - 1.0) * GAP) / k).min(most).floor().max(2.0 * INSET);
    let ph = ((cw - 2.0 * INSET) * 0.625).round();
    let rows = n.div_ceil(cols) as f32;
    (cols, cw, ph, rows * (INSET + ph + NAME_H) + (rows - 1.0) * GAP)
}

/// The theme cards from `(x, y)`, `w` wide: the default first (Mono), then the rest as
/// [`THEMES`] has them, theme `i` the button `id + i`.
pub(crate) fn themes(ui: &mut Ui<'_>, id: u32, (x, y, w): (f32, f32, f32)) {
    let (cols, cw, ph, _) = themes_size(w);
    let ch = INSET + ph + NAME_H;
    let first = THEMES.iter().position(|th| th.name == ui::theme("").name).unwrap_or(0);
    let order = (0..THEMES.len()).map(|k| if k == 0 { first } else { k - usize::from(k <= first) });
    for (n, i) in order.enumerate() {
        let (col, row) = ((n % cols) as f32, (n / cols) as f32);
        let at = RectF::new(x + col * (cw + GAP), y + row * (ch + GAP), cw, ch);
        let card = ui.snapped(at);
        theme_card(ui, WidgetId(id.wrapping_add(i as u32)), card, &THEMES[i], ph);
    }
}

/// Theme card `id`: `th`'s miniature desktop, `ph` tall, over its name.
fn theme_card(ui: &mut Ui<'_>, id: WidgetId, r: RectF, th: &Theme, ph: f32) {
    let t = ui.theme();
    let current = t.name == th.name;
    if current {
        ring(ui, r);
    }
    let (fill, hover) = card(ui, id, r, RADIUS_LG);
    let preview = ui.snapped(RectF::new(r.x + INSET, r.y + INSET, r.w - 2.0 * INSET, ph));
    miniature(ui, preview, th, fill);
    let band = preview.y + preview.h;
    let style = t.body().with_color(if current || hover { t.text } else { t.text_dim });
    let base = cap_base(ui, band, r.y + r.h - band, style);
    ui.text(r.x + INSET + 6.0, base, th.name, style);
    if current {
        check(ui, r.x + r.w - INSET - 6.0, base);
    }
    ui.hit(id, r, Sense::Click);
    ui.mark(id, sem::OPTION, if current { sem::SELECTED } else { 0 }, "");
}

/// `th`'s desktop in small inside `p` (light, a window, the Assistant), its corners
/// concentric with the card's, the light trimmed by a ring in `under`.
fn miniature(ui: &mut Ui<'_>, p: RectF, th: &Theme, under: Rgba) {
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
    // The Assistant's tile in the bottom-right corner, on the accent's hue: its sparkle in ink.
    let (side, edge) = ((12.0 * u).round().max(6.0), (0.05 * p.h).round());
    let at = RectF::new(p.x + p.w - side - edge, p.y + p.h - side - edge, side, side);
    let (tile, [plate, _, ink]) = (ui.snapped(at), th.icon_colors(th.accent));
    ui.fill(tile, (side / 4.0).round(), plate);
    ui.glyph(tile.inset(side * 0.2), Glyph::Assistant, ink);
    ui.pop_clip();
    let k = ui.px((radius * 0.5).ceil());
    ui.border(p.inset(-k), radius + k, k, under);
    ui.border(p, radius, line, t.border);
}
