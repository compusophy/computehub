//! The desktop's half of [`uiwire`]: a GUI program's window as `ui` draws it. [`draw`] lays out a
//! frame's nodes top to bottom, [`PAD`] inside the content rect, and draws them in the frame's
//! theme, scrolled as the window's [`View`] says: a following view at the bottom stays there as
//! the content grows, and an [`Area`] being typed in keeps its caret in view. While the user edits
//! an Input, a Code or an Area its text is the host's ([`Texts`]). A Glyph or an Entry's tile
//! whose glyph the desktop does not know is empty space. In a window narrower than [`NARROW`] (a
//! phone's) chips and quiet buttons are touch targets, [`TOUCH`] tall.

#![forbid(unsafe_code)]

mod area;
#[cfg(test)]
mod tests;

use std::mem;

pub use area::Area;
use gfx::RectF;
use ui::icon::Glyph;
use ui::{AppIcon, BUTTON_H, CARD_PAD, Code, FIELD_H, PAD, RADIUS_SM, Rgba, SPACING, Sense};
use ui::{TextStyle, TextSystem, Theme, Ui, WidgetId};
use uiwire::{Node, Style, Variant};

/// An Item's height, a chip's, a touch target's (a chip's or a quiet button's on a narrow
/// window), a chip's padding either side of its label, an Entry's (a touch target, Fibonacci as
/// its tile's side is), a Toggle's.
pub const ITEM_H: f32 = 36.0;
pub const CHIP_H: f32 = 28.0;
pub const TOUCH: f32 = 44.0;
const CHIP_PAD: f32 = 12.0;
pub const ENTRY_H: f32 = 55.0;
const TILE: f32 = 34.0;
pub const TOGGLE_H: f32 = 44.0;
/// Inter's cap height in ems: one-line controls center it.
const CAP: f32 = 0.727;
/// The widest window that is narrow.
pub const NARROW: f32 = 560.0;

/// The text the host owns: each Input's id, text and version, each Code and Area by id, and the
/// focused one (0 for none).
#[derive(Debug, Default)]
pub struct Texts {
    pub inputs: Vec<(u32, String, u32)>,
    pub codes: Vec<(u32, Code)>,
    pub areas: Vec<(u32, Area)>,
    pub focus: u32,
}

impl Texts {
    /// Whether the frame holds an Input, Code or Area `id`.
    pub fn has(&self, id: u32) -> bool {
        let (i, c) = (self.inputs.iter().any(|i| i.0 == id), self.codes.iter().any(|c| c.0 == id));
        i || c || self.areas.iter().any(|a| a.0 == id)
    }
}

/// How a window is scrolled: pixels down, and the content's and the view's height as last drawn;
/// whether a view at the bottom stays there as the content grows (the Assistant's transcript).
#[derive(Debug, Default)]
pub struct View {
    pub scroll: f32,
    pub heights: (f32, f32),
    pub follow: bool,
}

/// Lays out `nodes` in `ui`'s rect and draws them, scrolled as `view` says (see the crate docs).
pub fn draw(ui: &mut Ui<'_>, nodes: &[Node], texts: &mut Texts, view: &mut View) {
    let r = ui.rect();
    let (w, inner) = ((r.w - 2.0 * PAD).max(0.0), r.h - 2.0 * PAD);
    let (t, sizes, slack) = (ui.theme(), Vec::new(), Vec::new());
    let touch = r.w < NARROW;
    #[rustfmt::skip]
    let mut lay = Lay { t, texts, sizes, extra: 0.0, fills: 0, slack, again: false, i: 0, y: 0.0,
        touch };
    let ts = ui.text_system();
    let mut h = lay.stack(ts, nodes, w, SPACING, None);
    // Again with the room left shared by the Fills, each also taking what it is short of a
    // taller sibling in a Row.
    let fills = mem::take(&mut lay.fills);
    if fills > 0 && (h < inner || lay.slack.iter().any(|s| *s > 0.0)) {
        (lay.extra, lay.sizes, lay.again) = ((inner - h).max(0.0) / fills as f32, Vec::new(), true);
        h = lay.stack(ts, nodes, w, SPACING, None);
    }
    let end = view.follow && view.scroll >= view.heights.0 - view.heights.1;
    view.heights = (h + 2.0 * PAD, r.h);
    let mut y = if end { f32::MAX } else { view.scroll };
    // A caret just moved or typed at, in view.
    if let Some((_, a)) = lay.texts.areas.iter_mut().find(|a| a.1.follow) {
        let (top, bottom) = a.caret_band();
        (y, a.follow) = (y.max(PAD + bottom - r.h).min(PAD + top), false);
    }
    view.scroll = y.min(view.heights.0 - r.h).max(0.0);
    lay.draw_stack(ui, nodes, (r.x + PAD, r.y + PAD - view.scroll), SPACING);
}

/// How a Text of `style` is set.
fn style_of(style: Style, t: &Theme) -> TextStyle {
    match style {
        Style::Body => t.body(),
        Style::Title => t.title(),
        Style::Heading => t.heading(),
        Style::Subheading => t.subheading(),
        Style::Small => t.small(),
        Style::Mono => t.mono(),
        Style::Dim => t.body().with_color(t.text_dim),
        Style::Error => t.body().with_color(t.danger),
        Style::Success => t.body().with_color(t.ansi[2]),
        Style::Accent => t.body().with_color(t.accent),
    }
}

/// One frame's layout in theme `t` over the host's text: each node's size in pre-order, the
/// height each Fill adds, the Fills so far, each Fill's shortfall beside a taller sibling,
/// whether this is the second pass, the next size to draw, the top of the node being measured
/// in the content, whether chips and quiet buttons are touch targets.
struct Lay<'t> {
    t: &'t Theme,
    texts: &'t mut Texts,
    sizes: Vec<(f32, f32)>,
    extra: f32,
    fills: usize,
    slack: Vec<f32>,
    again: bool,
    i: usize,
    y: f32,
    touch: bool,
}

impl Lay<'_> {
    /// Measures `ns` stacked `gap` apart in width `w`; their height. Each Code
    /// takes `g` more height (in a Fill).
    fn stack(&mut self, ts: &mut TextSystem, ns: &[Node], w: f32, gap: f32, g: Option<f32>) -> f32 {
        let (top, mut h) = (self.y, 0.0);
        for n in ns {
            self.y = top + h;
            h += self.measure(ts, n, w, g).1 + gap;
        }
        self.y = top;
        h - if ns.is_empty() { 0.0 } else { gap }
    }

    /// Measures `n` given width `w` (Buttons, Spacers and Glyphs take their own).
    fn measure(&mut self, ts: &mut TextSystem, n: &Node, w: f32, grow: Option<f32>) -> (f32, f32) {
        let (at, t) = (self.sizes.len(), self.t);
        self.sizes.push((0.0, 0.0));
        let size = match n {
            Node::Col { gap, children: c, .. } | Node::Center { gap, children: c, .. } => {
                (w, self.stack(ts, c, w, f32::from(*gap), None))
            }
            Node::Card { children, .. } => {
                self.y += CARD_PAD;
                let h = self.stack(ts, children, w - 2.0 * CARD_PAD, SPACING, None);
                self.y -= CARD_PAD;
                (w, h + 2.0 * CARD_PAD)
            }
            Node::Fill { children, .. } => {
                if self.slack.len() <= self.fills {
                    self.slack.push(0.0);
                }
                let extra = self.extra + self.slack[self.fills];
                self.fills += 1;
                let codes = children.iter().filter(|c| matches!(c, Node::Code { .. })).count();
                let grow = Some(extra / codes.max(1) as f32);
                (w, self.stack(ts, children, w, SPACING, grow) + [extra, 0.0][codes.min(1)])
            }
            Node::Pane { w: pw, children, .. } => {
                let w = w.min(f32::from(*pw));
                (w, self.stack(ts, children, w, SPACING, None))
            }
            // Buttons, Spacers and Glyphs take their width, each Text its own up to an even
            // share of the rest, the others share what is left.
            Node::Row { gap, children, .. } => {
                let (gap, mut x, mut h) = (f32::from(*gap), 0.0, 0.0f32);
                let mut own: Vec<_> = children.iter().map(|c| own_width(ts, t, c, w)).collect();
                let gaps = gap * children.len().saturating_sub(1) as f32;
                let mut room = (w - own.iter().flatten().sum::<f32>() - gaps).max(0.0);
                let mut flex = own.iter().filter(|o| o.is_none()).count();
                let share = room / flex.max(1) as f32;
                for (c, o) in children.iter().zip(&mut own) {
                    if let Node::Text { style, text, .. } = c {
                        let style = style_of(*style, t);
                        let lines = ts.wrap(text, style, f32::MAX);
                        let one = lines.iter().map(|l| ts.measure(l, style)).fold(0.0, f32::max);
                        // A pixel more: snapping must not wrap it.
                        let one = (one.ceil() + 1.0).min(share);
                        (*o, room, flex) = (Some(one), room - one, flex - 1);
                    }
                }
                let share = room / flex.max(1) as f32;
                let mut fills = Vec::new();
                for (c, o) in children.iter().zip(own) {
                    let first = self.fills;
                    let s = self.measure(ts, c, o.unwrap_or(share), None);
                    (x, h) = (x + s.0 + gap, h.max(s.1));
                    fills.extend((self.fills > first).then_some((first, s.1)));
                }
                // A child's first Fill grows by what the child is short of the Row's height.
                for (fill, ch) in fills.into_iter().filter(|_| !self.again) {
                    self.slack[fill] += h - ch;
                }
                (x - if children.is_empty() { 0.0 } else { gap }, h)
            }
            // 3 to 12 rows as the text has lines; 3 and what is left in a Fill.
            Node::Code { id, text, .. } => {
                let n = self.texts.codes.iter().find(|c| c.0 == *id).map(|c| c.1.ed.line_count());
                let rows = grow.map_or(n.unwrap_or(text.lines().count()).clamp(3, 12), |_| 3);
                (w, rows as f32 * ts.line_height(t.mono()) + 12.0 + grow.unwrap_or(0.0))
            }
            Node::Text { style, text, .. } => {
                let style = style_of(*style, t);
                (w, ts.wrap(text, style, w).len() as f32 * ts.line_height(style))
            }
            Node::Button { variant: Variant::Chip | Variant::On, label, .. } => {
                (chip_width(ts, t, label), if self.touch { TOUCH } else { CHIP_H })
            }
            Node::Button { variant: Variant::Quiet, label, .. } => {
                (quiet_width(ts, t, label), if self.touch { TOUCH } else { BUTTON_H })
            }
            Node::Button { label, .. } => (ui::button_width(ts, t, label), BUTTON_H),
            Node::Spacer { px: side } | Node::Glyph { size: side, .. } => {
                (f32::from(*side), f32::from(*side))
            }
            Node::Input { .. } => (w, FIELD_H),
            Node::Item { .. } => (w, ITEM_H),
            Node::Entry { .. } => (w, ENTRY_H),
            Node::Toggle { .. } => (w, TOGGLE_H),
            Node::Area { id, .. } => {
                let (y, a) = (self.y, self.texts.areas.iter_mut().find(|a| a.0 == *id));
                (w, a.map_or(area::MIN_H, |a| a.1.layout(ts, t.body(), w, y)))
            }
            Node::Separator => (w, 1.0),
        };
        self.sizes[at] = size;
        size
    }

    /// The next size to draw; `take` moves past it.
    fn next(&mut self, take: bool) -> (f32, f32) {
        self.i += usize::from(take);
        self.sizes.get(self.i - usize::from(take)).copied().unwrap_or_default()
    }

    /// Draws `nodes` down from `(x, y)`, `gap` apart.
    fn draw_stack(&mut self, ui: &mut Ui<'_>, nodes: &[Node], (x, mut y): (f32, f32), gap: f32) {
        for n in nodes {
            let h = self.next(false).1;
            self.draw(ui, n, x, y);
            y += h + gap;
        }
    }

    /// Draws `n` at `(x, y)` in its measured size.
    fn draw(&mut self, ui: &mut Ui<'_>, n: &Node, x: f32, y: f32) {
        let ((w, h), t) = (self.next(true), self.t);
        let r = ui.snapped(RectF::new(x, y, w, h));
        let at = |ui: &mut Ui<'_>, f: &mut dyn FnMut(&mut Ui<'_>)| ui.within(r, |ui| f(ui));
        match n {
            Node::Col { gap, children: c, .. } => self.draw_stack(ui, c, (x, y), f32::from(*gap)),
            Node::Center { gap, children, .. } => {
                let mut y = y;
                for c in children {
                    let (cw, ch) = self.next(false);
                    match c {
                        Node::Text { style, text, .. } => {
                            self.next(true);
                            lines(ui, text, style_of(*style, t), (x, w), y);
                        }
                        c => self.draw(ui, c, x + (w - cw) / 2.0, y),
                    }
                    y += ch + f32::from(*gap);
                }
            }
            Node::Fill { children, .. } | Node::Pane { children, .. } => {
                self.draw_stack(ui, children, (x, y), SPACING)
            }
            Node::Card { children, .. } => {
                ui.raised(r);
                self.draw_stack(ui, children, (x + CARD_PAD, y + CARD_PAD), SPACING);
            }
            // Leaves center on the row's height; the editors keep to its top.
            Node::Row { gap, children, .. } => {
                let mut cx = x;
                for c in children {
                    let (cw, ch) = self.next(false);
                    let edits = matches!(c, Node::Code { .. } | Node::Area { .. });
                    let cy = if c.children().is_empty() && !edits { y + (h - ch) / 2.0 } else { y };
                    self.draw(ui, c, cx, cy);
                    cx += cw + f32::from(*gap);
                }
            }
            Node::Text { style, text, .. } => {
                at(ui, &mut |ui| _ = ui.wrapped(text, style_of(*style, t)))
            }
            Node::Button { id, variant, label } => at(ui, &mut |ui| {
                let id = WidgetId(*id);
                _ = match variant {
                    Variant::Normal => ui.button(id, label),
                    Variant::Primary => ui.button_primary(id, label),
                    Variant::Danger => ui.button_danger(id, label),
                    Variant::Chip | Variant::On => chip(ui, id, r, label, *variant == Variant::On),
                    Variant::Quiet => quiet(ui, id, r, label),
                }
            }),
            Node::Input { id, placeholder, .. } => {
                let value = self.texts.inputs.iter().find(|i| i.0 == *id).map_or("", |i| &i.1);
                let focus = self.texts.focus == *id && *id != 0;
                at(ui, &mut |ui| _ = ui.text_field(WidgetId(*id), value, focus, placeholder));
            }
            Node::Code { id, line_numbers, text, .. } => {
                let focus = self.texts.focus == *id;
                match self.texts.codes.iter_mut().find(|c| c.0 == *id) {
                    Some(c) => c.1.draw(ui, WidgetId(*id), r, *line_numbers, focus),
                    None => at(ui, &mut |ui| _ = ui.wrapped(text, t.mono())),
                }
            }
            Node::Area { id, value, placeholder } => {
                let focus = self.texts.focus == *id && *id != 0;
                match self.texts.areas.iter_mut().find(|a| a.0 == *id) {
                    Some((_, a)) => a.draw(ui, WidgetId(*id), r, focus, placeholder),
                    None => at(ui, &mut |ui| _ = ui.wrapped(value, t.body())),
                }
            }
            Node::Item { id, text, detail, selected } => {
                item(ui, WidgetId(*id), r, [text, detail], *selected)
            }
            Node::Glyph { glyph, .. } => {
                if let Some(&g) = Glyph::ALL.get(usize::from(*glyph)) {
                    ui.glyph(r, g, t.text);
                }
            }
            Node::Entry { id, glyph, hue, text, detail, more } => {
                entry(ui, WidgetId(*id), r, (*glyph, *hue), [text, detail], *more)
            }
            Node::Toggle { id, on, label } => toggle(ui, WidgetId(*id), r, label, *on),
            Node::Separator => ui.fill(RectF { h: ui.px(1.0), ..r }, 0.0, t.border),
            Node::Spacer { .. } => {}
        }
    }
}

/// The width a Button, Spacer, Glyph or Pane takes in a Row `w` wide; `None` for the others.
fn own_width(ts: &mut TextSystem, t: &Theme, n: &Node, w: f32) -> Option<f32> {
    match n {
        Node::Button { variant: Variant::Chip | Variant::On, label, .. } => {
            Some(chip_width(ts, t, label))
        }
        Node::Button { variant: Variant::Quiet, label, .. } => Some(quiet_width(ts, t, label)),
        Node::Button { label, .. } => Some(ui::button_width(ts, t, label)),
        Node::Spacer { px: side } | Node::Glyph { size: side, .. } => Some(f32::from(*side)),
        Node::Pane { w: pw, .. } => Some(f32::from(*pw).min(w)),
        _ => None,
    }
}

/// `w` logical px on whole device pixels, rounded up.
fn device(ts: &TextSystem, w: f32) -> f32 {
    (w * ts.dpr()).ceil() / ts.dpr()
}

/// The width of a chip labeled `label`, and of a quiet button.
fn chip_width(ts: &mut TextSystem, t: &Theme, label: &str) -> f32 {
    let w = ts.measure(label, t.small()) + 2.0 * CHIP_PAD;
    device(ts, w)
}

fn quiet_width(ts: &mut TextSystem, t: &Theme, label: &str) -> f32 {
    let w = (ts.measure(label, t.body()) + 16.0).max(BUTTON_H);
    device(ts, w)
}

/// The baseline that centers the capitals of `style` in a band `h` tall from `top`.
fn cap_base(ui: &mut Ui<'_>, top: f32, h: f32, style: TextStyle) -> f32 {
    ui.text_system().snap(top + (h + CAP * style.size) / 2.0)
}

/// `label` centered in `r`.
fn label_in(ui: &mut Ui<'_>, r: RectF, label: &str, style: TextStyle) {
    let base = cap_base(ui, r.y, r.h, style);
    let ts = ui.text_system();
    let lw = ts.measure(label, style);
    let x = ts.snap(r.x + (r.w - lw) / 2.0);
    ui.text(x, base, label, style);
}

/// `text` wrapped to `w`, each line centered on `x + w / 2`, from `top` down.
fn lines(ui: &mut Ui<'_>, text: &str, style: TextStyle, (x, w): (f32, f32), top: f32) {
    let ts = ui.text_system();
    let (lh, a, d) = (ts.line_height(style), ts.ascent(style), ts.descent(style));
    let base = ts.snap((lh - a - d) / 2.0 + a);
    for (i, line) in ts.wrap(text, style, w).into_iter().enumerate() {
        let ts = ui.text_system();
        let lw = ts.measure(line, style);
        let (lx, ly) = (ts.snap(x + (w - lw) / 2.0), ts.snap(top + i as f32 * lh));
        ui.text(lx, ly + base, line, style);
    }
}

/// A chip in `r`: `label` small on a pill, brightening under the pointer, or on the accent when
/// `on`; a [`Sense::Click`] hit.
fn chip(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) -> RectF {
    let (t, s) = (ui.theme(), ui.state());
    let hover = s.hover == Some(id);
    let ink = if on {
        ui.fill(r, r.h / 2.0, t.accent);
        t.accent_text
    } else {
        ui.fill(r, r.h / 2.0, t.surface_lo);
        if hover {
            ui.fill(r, r.h / 2.0, t.wash(s.pressed == Some(id)));
        }
        let edge = ui.px(1.0);
        ui.border(r, r.h / 2.0, edge, t.border);
        if hover { t.text } else { t.text_dim }
    };
    label_in(ui, r, label, t.small().with_color(ink));
    ui.hit(id, r, Sense::Click);
    r
}

/// A quiet button in `r`: `label` dim, bright on a wash under the pointer; a click hit.
fn quiet(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str) -> RectF {
    let (t, s) = (ui.theme(), ui.state());
    let hover = s.hover == Some(id);
    if hover {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    label_in(ui, r, label, t.body().with_color(if hover { t.text } else { t.text_dim }));
    ui.hit(id, r, Sense::Click);
    r
}

/// A list row in `r`: `text`, then `detail` dim at the right; washed under
/// the pointer, tinted when `selected`; a [`Sense::Click`] hit.
fn item(ui: &mut Ui<'_>, id: WidgetId, r: RectF, [text, detail]: [&String; 2], selected: bool) {
    let (t, s) = (ui.theme(), ui.state());
    if selected || s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, if selected { t.selection } else { t.wash(s.pressed == Some(id)) });
    }
    let (body, small) = (t.body(), t.small());
    let base = cap_base(ui, r.y, r.h, body);
    let ts = ui.text_system();
    let dw = ts.measure(detail, small);
    let right = ts.snap(r.x + r.w - 12.0 - dw);
    ui.push_clip(RectF { w: (right - r.x - 12.0).max(0.0), ..r });
    ui.text(r.x + 12.0, base, text, body);
    ui.pop_clip();
    ui.push_clip(r);
    ui.text(right, base, detail, small);
    ui.pop_clip();
    ui.hit(id, r, Sense::Click);
}

/// An Entry in `r`: its tile, `text` (cut to fit), `detail` small at the right and a chevron
/// when `more`; washed under the pointer, a hairline under it from the text on; a click hit.
fn entry(
    ui: &mut Ui<'_>,
    id: WidgetId,
    r: RectF,
    tile: (u8, u32),
    [text, detail]: [&String; 2],
    more: bool,
) {
    let (t, s) = (ui.theme(), ui.state());
    if s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    let at = ui.snapped(RectF::new(r.x + 8.0, r.y + (r.h - TILE) / 2.0, TILE, TILE));
    if let Some(&glyph) = Glyph::ALL.get(usize::from(tile.0)) {
        ui.app_icon(at, AppIcon { glyph, hue: Rgba::hex(tile.1) });
    }
    let (x, mut right) = (at.x + TILE + 13.0, r.x + r.w - 8.0);
    if more {
        let at = ui.snapped(RectF::new(right - 13.0, r.y + (r.h - 13.0) / 2.0, 13.0, 13.0));
        ui.glyph(at, Glyph::Chevron, t.text_dim);
        right = at.x - 8.0;
    }
    let (body, small) = (t.body(), t.small());
    if !detail.is_empty() {
        let base = cap_base(ui, r.y, r.h, small);
        let ts = ui.text_system();
        let dw = ts.measure(detail, small);
        let dx = ts.snap(right - dw);
        ui.text(dx, base, detail, small);
        right = dx - 13.0;
    }
    let shown = ui.text_system().ellipsize(text, body, (right - x).max(0.0));
    let base = cap_base(ui, r.y, r.h, body);
    ui.text(x, base, &shown, body);
    let line = ui.px(1.0);
    let rule = ui.snapped(RectF::new(x, r.y + r.h - line, r.x + r.w - x, line));
    ui.fill(RectF { h: line, ..rule }, 0.0, t.border);
    ui.hit(id, r, Sense::Click);
}

/// A Toggle in `r`: `label` (cut to fit) and a switch 34 x 21 at the right, its track the accent
/// and its knob right when `on`, else sunken with its knob left; washed under the pointer; a
/// click hit.
fn toggle(ui: &mut Ui<'_>, id: WidgetId, r: RectF, label: &str, on: bool) {
    let (t, s, line) = (ui.theme(), ui.state(), ui.px(1.0));
    if s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, t.wash(s.pressed == Some(id)));
    }
    let track = RectF::new(r.x + r.w - 8.0 - 34.0, r.y + (r.h - 21.0) / 2.0, 34.0, 21.0);
    let track = ui.snapped(track);
    let (fill, knob) = if on { (t.accent, t.accent_text) } else { (t.surface_lo, t.text_dim) };
    ui.fill(track, 10.5, fill);
    if !on {
        ui.border(track, 10.5, line, t.border);
    }
    let x = if on { track.x + track.w - 18.0 } else { track.x + 3.0 };
    let dot = ui.snapped(RectF::new(x, track.y + 3.0, 15.0, 15.0));
    ui.fill(dot, dot.w / 2.0, knob);
    let body = t.body();
    let shown = ui.text_system().ellipsize(label, body, (track.x - r.x - 21.0).max(0.0));
    let base = cap_base(ui, r.y, r.h, body);
    ui.text(r.x + 8.0, base, &shown, body);
    ui.hit(id, r, Sense::Click);
}
