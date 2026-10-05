//! What a resource monitor draws: a meter's history ([`Node::Chart`]), how full something is
//! ([`Node::Meter`]), and a table's head ([`Node::Columns`]) over Entries whose detail is cells
//! ([`cells`]), its columns [`uiwire::COLUMN`] px wide from the right, a chevron's room kept.
//!
//! [`Node::Chart`]: uiwire::Node::Chart
//! [`Node::Meter`]: uiwire::Node::Meter
//! [`Node::Columns`]: uiwire::Node::Columns

use gfx::RectF;
use ui::{RADIUS_SM, Sense, Ui, WidgetId};

use crate::{TILE, cap_base};
use canvas::ink;

/// A Meter's height, and a table head's.
pub(crate) const METER_H: f32 = 6.0;
pub(crate) const HEAD_H: f32 = 28.0;
const COLUMN: f32 = uiwire::COLUMN as f32;

/// A Chart in `r`: a well with rules at its quarters, the values as a soft area under a line in
/// `hue` (none between a value and an unknown one), the newest a dot at the right.
pub(crate) fn chart(ui: &mut Ui<'_>, r: RectF, hue: u8, values: &[u16]) {
    let (t, line) = (ui.theme(), ui.px(1.0));
    let (color, well) = (ink(t, hue), ui.snapped(r));
    ui.fill(well, RADIUS_SM, t.surface_lo);
    ui.push_clip(well);
    for q in 1..4 {
        let y = ui.text_system().snap(well.y + well.h * q as f32 / 4.0);
        ui.fill(RectF::new(well.x, y, well.w, line), 0.0, t.border);
    }
    // The newest point sits a dot's room in from the right edge, its values off the rims.
    let (n, dot, rim) = (values.len(), 3.0, 2.0 * line);
    let (wide, span, floor) = (well.w - 2.0 * dot, well.h - 2.0 * rim - dot, well.y + well.h - rim);
    let at = |i: usize| {
        let x = if n > 1 { well.x + wide * i as f32 / (n - 1) as f32 } else { well.x + wide };
        (x, floor - span * f32::from(values[i].min(1000)) / 1000.0)
    };
    let area = color.with_alpha(color.3 / 5);
    let known = |i: usize| values[i] != uiwire::UNKNOWN;
    let segments = || (1..n).filter(|&i| known(i - 1) && known(i));
    for i in segments() {
        let (p, q) = (at(i - 1), at(i));
        let top = (p.1 + q.1) / 2.0;
        let r = ui.snapped(RectF::new(p.0, top, q.0 - p.0, well.y + well.h - top));
        ui.fill(r, 0.0, area);
    }
    for i in segments() {
        ui.list().line(at(i - 1), at(i), 1.5 * line, color);
    }
    if let Some(last) = n.checked_sub(1).filter(|&i| known(i)).map(at) {
        let r = RectF::new(last.0 - dot, last.1 - dot, 2.0 * dot, 2.0 * dot);
        ui.fill(r, dot, color);
    }
    ui.pop_clip();
    ui.border(well, RADIUS_SM, line, t.border);
}

/// A Meter in `r`: a sunken track, `value` per mille of it filled in `hue`.
pub(crate) fn meter(ui: &mut Ui<'_>, r: RectF, hue: u8, value: u16) {
    let t = ui.theme();
    let track = ui.snapped(RectF::new(r.x, r.y, r.w, METER_H));
    ui.fill(track, METER_H / 2.0, t.surface_lo);
    if value > 0 {
        let w = (track.w * f32::from(value.min(1000)) / 1000.0).max(METER_H);
        ui.fill(ui.snapped(RectF { w, ..track }), METER_H / 2.0, ink(t, hue));
    }
}

/// The right edge of an Entry's last column in `r` (left of its chevron, kept for all).
fn right_edge(ui: &mut Ui<'_>, r: RectF) -> f32 {
    ui.text_system().snap(r.x + r.w - 21.0) - 8.0
}

/// An Entry's cells in `r` from the right (each right aligned in its column), small; the
/// left edge of the first column.
pub(crate) fn cells(ui: &mut Ui<'_>, r: RectF, detail: &str) -> f32 {
    let (small, mut right) = (ui.theme().small(), right_edge(ui, r));
    let base = cap_base(ui, r.y, r.h, small);
    for cell in detail.rsplit('\t') {
        let x = ui.text_system().measure(cell, small);
        let x = ui.text_system().snap(right - x);
        ui.text(x, base, cell, small);
        right -= COLUMN;
    }
    right
}

/// A table's head in `r`: label 0 over the rows' names, each other right aligned over its
/// column; label `on - 1` lit and underlined; with an id, label `i` the button `id + i`.
pub(crate) fn columns(ui: &mut Ui<'_>, id: u32, r: RectF, on: u8, labels: &str) {
    let (t, line, mut right) = (ui.theme(), ui.px(1.0), right_edge(ui, r));
    let n = labels.split('\t').count();
    for (i, label) in labels.rsplit('\t').enumerate() {
        let (k, lit) = (n - 1 - i, usize::from(on) == n - i);
        let style = t.small().with_color(if lit { t.text } else { t.text_dim });
        let w = ui.text_system().measure(label, style);
        let (left, cell) = match k {
            0 => (r.x + 8.0 + TILE + 13.0, RectF::new(r.x, r.y, right - r.x, r.h)),
            _ => (right - w, RectF::new(right - COLUMN, r.y, COLUMN, r.h)),
        };
        let left = ui.text_system().snap(left);
        let base = cap_base(ui, r.y, r.h, style);
        ui.text(left, base, label, style);
        if lit {
            let under = ui.snapped(RectF::new(left, base + 4.0, w, line));
            ui.fill(under, 0.0, t.text);
        }
        if id != 0 {
            ui.hit(WidgetId(id.wrapping_add(k as u32)), cell, Sense::Click);
        }
        right -= COLUMN;
    }
    let rule = ui.snapped(RectF::new(r.x, r.y + r.h - line, r.w, line));
    ui.fill(rule, 0.0, t.border);
}
