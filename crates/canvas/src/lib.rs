//! # canvas: a uiwire Canvas as the desktop draws it
//!
//! [`draw`] lays a [`uiwire::Node::Canvas`] out as a well of square units scaled to fit, draws
//! its shapes over it in the frame's theme, and with an id makes it a pad and, for the AI, a mark
//! that lists them. Its colors ([`ink`]) are a Grid's squares' ([`square`]) and the theme's ink.
//!
//! - A Rect's, a Sprite's and Pixels' squares cover whole units, each edge on a device pixel from
//!   its own unit, so neighbors tile with no seam; a row of a Sprite or of Pixels joins the
//!   squares of a color in a run, one fill.
//! - A Circle, a Ring, a Line's ends and a Text center on their unit's middle. A Line along an
//!   axis is a snapped fill with round ends; any other a [`gfx::Kind::Line`]. Strokes are a
//!   device pixel at least.
//! - A Text is set in the boot's Inter on a ladder of sizes ([`LADDER`]), so a size that moves
//!   frame by frame never fills the glyph atlas.
//! - The mark lists Pixels by their place and size, and while a canvas's come to [`SQUARES`] or
//!   fewer, their squares too, a row a line, as a Grid's.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

use gfx::RectF;
use ui::{FontId, Rgba, Sense, TextStyle, Theme, Ui, WidgetId};
use uiwire::{Draw, PAINT, Shape};

/// The px sizes a Text is set in: its size in px rounded down to one of these (the least at
/// least).
pub const LADDER: [f32; 11] = [8.0, 10.0, 12.0, 14.0, 16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 64.0];
/// The most draws a canvas's mark lists one by one, and the most squares of its Pixels.
const LISTED: usize = 64;
pub const SQUARES: usize = 1024;
/// Inter's cap height in ems: a Text centers its capitals on its point.
const CAP: f32 = 0.727;

/// The fill of a Grid's square of color `c` in theme `t`, and the ink of its text: 0 the
/// sunken well, 1 to 8 the palette (ANSI 1 to 8); but 7, silver, is the faint ink on a light
/// theme, whose ANSI white is dark and next to 8, gray.
pub fn square(t: &Theme, c: u8) -> (Rgba, Rgba) {
    match c {
        0 => (t.surface_lo, t.text),
        7 if !t.dark => (t.text_faint, t.text),
        c => (t.ansi[usize::from(c.min(8))], t.base),
    }
}

/// The color `c` of a canvas in theme `t`: 0 the canvas (a Grid's empty well), 1 to 8 a Grid's
/// squares', 9 ink, 10 dim ink, 11 the accent.
pub fn ink(t: &Theme, c: u8) -> Rgba {
    match c {
        9 => t.text,
        10 => t.text_dim,
        11 => t.accent,
        c => square(t, c).0,
    }
}

/// The px size a Text `px` tall is set in.
fn rung(px: f32) -> f32 {
    LADDER.iter().copied().rfind(|&s| s <= px).unwrap_or(LADDER[0])
}

/// The color a Pixels' cell `c` ([`PAINT`]'s char) paints.
fn paint(c: u8) -> u8 {
    PAINT.iter().position(|&p| p == c).unwrap_or(0) as u8
}

/// Canvas `id` (0: none) of `units` (w, h) and `draws` in `r` (its measured size: its units fill
/// the height, centered across): its well and draws; with an id, a pad hit and, for the AI, a
/// mark. Where its units are, for taps.
pub fn draw(ui: &mut Ui<'_>, r: RectF, id: u32, units: (u16, u16), draws: &[Draw]) -> RectF {
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
                let (left, base) = (ts.snap(cx - lw / 2.0), ts.snap(cy + CAP * style.size / 2.0));
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
            // Rows of `a` cells, each `b` units a side: a run of one paint is a fill.
            Shape::Pixels if a > 0 && b > 0 => {
                for (j, row) in d.text.as_bytes().chunks(a as usize).enumerate() {
                    let top = fy + j as f32 * fb;
                    let mut run: Option<(usize, u8)> = None;
                    for (i, c) in row.iter().copied().chain([b'.']).enumerate() {
                        if let Some((from, p)) = run.filter(|r| r.1 != c) {
                            let n = (i - from) as f32;
                            let sq = snap(ui, fx + from as f32 * fb, top, n * fb, fb);
                            ui.fill(sq, 0.0, ink(t, paint(p)));
                            run = None;
                        }
                        if c != b'.' && run.is_none() {
                            run = Some((i, c));
                        }
                    }
                }
            }
            Shape::Pixels => {}
        }
    }
    ui.pop_clip();
    if id != 0 {
        ui.hit(WidgetId(id), well, Sense::Pad);
        if ui.list().sem().is_some() {
            ui.mark(WidgetId(id), ui::sem::CANVAS, 0, &said(units, draws));
        }
    }
    at
}

/// A canvas as the AI reads it: "W x H units", then a line a draw (its shape's name, then its
/// text in quotes, its slots, its color; Pixels as "pixels X Y, W x H squares of S units", then
/// its rows, a char a square as they came, while [`SQUARES`] in all are not passed), [`LISTED`]
/// at most and then how many more.
fn said((w, h): (u16, u16), draws: &[Draw]) -> String {
    let (mut v, mut room) = (String::new(), SQUARES);
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
    const NAMES: [(&str, usize, bool); 7] = [
        ("rect", 4, true),
        ("circle", 3, true),
        ("ring", 4, true),
        ("line", 5, true),
        ("text", 3, true),
        ("sprite", 3, false),
        ("pixels", 0, false),
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
        if d.shape == Shape::Pixels {
            // Where, its squares across and down and their side; then, on a small board, each.
            let [x, y, across, side, _] = d.at.map(i32::from);
            let down = d.text.len() as i32 / across.max(1);
            v.push(' ');
            let parts = [(x, " "), (y, ", "), (across, " x "), (down, " squares of "), (side, "")];
            for (n, then) in parts {
                num(&mut v, n);
                v += then;
            }
            v += " units";
            if d.text.len() <= room {
                room -= d.text.len();
                for (i, c) in d.text.chars().enumerate() {
                    if i % across.max(1) as usize == 0 {
                        v.push('\n');
                    }
                    v.push(c);
                }
            }
        }
    }
    if draws.len() > LISTED {
        v += "\nand ";
        num(&mut v, (draws.len() - LISTED) as i32);
        v += " more";
    }
    v
}
