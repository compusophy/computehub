//! A [`uiwire::Node::Canvas`] as the desktop draws it: a well of square units scaled to fit, its
//! draws in the frame's theme over it, and for the AI a mark that lists them.
//!
//! - A Rect's and a Sprite's squares cover whole units, each edge on a device pixel from its own
//!   unit, so neighbors tile with no seam; a Sprite's row joins the squares of a color in a run.
//! - A Circle, a Ring, a Line's ends and a Text center on their unit's middle. A Line along an
//!   axis is a snapped fill with round ends; any other a [`gfx::Kind::Line`]. Strokes are a
//!   device pixel at least.
//! - A Text is set in the boot's Inter on a ladder of sizes ([`LADDER`]), so a size that moves
//!   frame by frame never fills the glyph atlas.

use gfx::RectF;
use ui::{FontId, Rgba, Sense, TextStyle, Theme, Ui, WidgetId};
use uiwire::{Draw, Shape};

use crate::{Board, cap_base, color};

/// The px sizes a Text is set in: its size in px rounded down to one of these (the least at
/// least).
pub const LADDER: [f32; 11] = [8.0, 10.0, 12.0, 14.0, 16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 64.0];
/// The most draws a canvas's mark lists one by one.
const LISTED: usize = 64;

/// The color `c` of a canvas in theme `t`: 0 the canvas (a Grid's empty well), 1 to 8 a Grid's
/// squares', 9 ink, 10 dim ink, 11 the accent.
pub fn ink(t: &Theme, c: u8) -> Rgba {
    match c {
        9 => t.text,
        10 => t.text_dim,
        11 => t.accent,
        c => color(t, c).0,
    }
}

/// The px size a Text `px` tall is set in.
fn rung(px: f32) -> f32 {
    LADDER.iter().copied().rfind(|&s| s <= px).unwrap_or(LADDER[0])
}

/// Canvas `id` (0: none) of `units` (w, h) and `draws` in `r` (its measured size: its units fill
/// the height, centered across): its well and draws; with an id, a pad hit and, for the AI, a
/// mark. Its board, for taps.
pub fn draw(ui: &mut Ui<'_>, r: RectF, id: u32, units: (u16, u16), draws: &[Draw]) -> Board {
    let (t, (cw, ch)) = (ui.theme(), (f32::from(units.0), f32::from(units.1)));
    let k = r.h / ch.max(1.0);
    let x0 = ui.text_system().snap(r.x + (r.w - cw * k) / 2.0);
    let at = RectF::new(x0, r.y, cw * k, r.h);
    let well = ui.snapped(at);
    ui.fill(well, 0.0, t.surface_lo);
    let edge = ui.px(1.0);
    ui.border(well, 0.0, edge, t.border);
    ui.push_clip(well);
    let snap = |ui: &Ui<'_>, x: f32, y: f32, w: f32, h: f32| {
        let (u, v) = (|u: f32| x0 + u * k, |v: f32| r.y + v * k);
        ui.snapped(RectF::new(u(x), v(y), u(x + w) - u(x), v(y + h) - v(y)))
    };
    // A point at its unit's middle, in px.
    let mid = |x: i16, y: i16| (x0 + (f32::from(x) + 0.5) * k, r.y + (f32::from(y) + 0.5) * k);
    for d in draws {
        let ([x, y, a, b, c], color) = (d.at, ink(t, d.color));
        let (fx, fy, fa, fb) = (f32::from(x), f32::from(y), f32::from(a), f32::from(b));
        match d.shape {
            Shape::Rect => ui.fill(snap(ui, fx, fy, fa, fb), 0.0, color),
            Shape::Circle | Shape::Ring => {
                let ((cx, cy), radius) = (mid(x, y), fa * k);
                let disc = RectF::new(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius);
                match d.shape {
                    Shape::Circle => ui.fill(disc, radius, color),
                    _ if b > 0 => ui.border(disc, radius, (fb * k).max(edge), color),
                    _ => {}
                }
            }
            Shape::Line if c > 0 => {
                let (p, q, w) = (mid(x, y), mid(a, b), (f32::from(c) * k).max(edge));
                if x == a || y == b {
                    // Along an axis: a fill on device pixels, its ends round.
                    let (left, top) = (p.0.min(q.0) - w / 2.0, p.1.min(q.1) - w / 2.0);
                    let r = RectF::new(left, top, (p.0 - q.0).abs() + w, (p.1 - q.1).abs() + w);
                    let r = ui.snapped(r);
                    ui.fill(r, r.w.min(r.h) / 2.0, color);
                } else {
                    ui.list().line(p, q, w, color);
                }
            }
            Shape::Line => {}
            Shape::Text if a > 0 => {
                let style = TextStyle::new(FontId::Sans, rung(fa * k), color);
                let ((cx, cy), ts) = (mid(x, y), ui.text_system());
                let lw = ts.measure(&d.text, style);
                let left = ts.snap(cx - lw / 2.0);
                let base = cap_base(ui, cy - style.size / 2.0, style.size, style);
                ui.text(left, base, &d.text, style);
            }
            Shape::Text => {}
            Shape::Sprite if a > 0 => {
                for (j, row) in d.text.split('\n').enumerate() {
                    let top = fy + j as f32 * fa;
                    // Runs of one char; a digit's paint.
                    let mut run: Option<(usize, char)> = None;
                    for (i, ch) in row.chars().chain([' ']).enumerate() {
                        if let Some((from, c)) = run.filter(|r| r.1 != ch) {
                            let n = (i - from) as f32;
                            let sq = snap(ui, fx + from as f32 * fa, top, n * fa, fa);
                            ui.fill(sq, 0.0, ink(t, c as u8 - b'0'));
                            run = None;
                        }
                        if ch.is_ascii_digit() && run.is_none() {
                            run = Some((i, ch));
                        }
                    }
                }
            }
            Shape::Sprite => {}
        }
    }
    ui.pop_clip();
    let n = u32::from(units.0) * u32::from(units.1);
    if id != 0 {
        ui.hit(WidgetId(id), well, Sense::Pad);
        if ui.list().sem().is_some() {
            ui.mark(WidgetId(id), ui::sem::CANVAS, 0, &said(units, draws));
        }
    }
    Board { id, nth: 0, rect: at, cols: units.0, n, fine: true }
}

/// A canvas as the AI reads it: "W x H units", then a line a draw (its shape's name, then its
/// text in quotes, its slots, its color), [`LISTED`] at most and then how many more.
fn said((w, h): (u16, u16), draws: &[Draw]) -> String {
    let mut v = String::new();
    let num = |v: &mut String, n: i32| {
        if n < 0 {
            v.push('-');
        }
        ui::push_num(v, n.unsigned_abs() as usize);
    };
    num(&mut v, w.into());
    v += " x ";
    num(&mut v, h.into());
    v += " units";
    // The slots each shape names, and whether its color.
    const NAMES: [(&str, usize, bool); 6] = [
        ("rect", 4, true),
        ("circle", 3, true),
        ("ring", 4, true),
        ("line", 5, true),
        ("text", 3, true),
        ("sprite", 3, false),
    ];
    for d in draws.iter().take(LISTED) {
        let (name, slots, colored) = NAMES[d.shape as usize];
        v.push('\n');
        v += name;
        if d.shape == Shape::Text {
            v += " \"";
            v += &d.text;
            v.push('"');
        }
        for &s in d.at.iter().take(slots) {
            v.push(' ');
            num(&mut v, s.into());
        }
        if colored {
            v.push(' ');
            num(&mut v, d.color.into());
        }
    }
    if draws.len() > LISTED {
        v += "\nand ";
        num(&mut v, (draws.len() - LISTED) as i32);
        v += " more";
    }
    v
}
