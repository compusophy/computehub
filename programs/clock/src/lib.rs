//! The clocks, the fractal shown: a [`Kind::Face`] (its hands, ticks and numerals on a
//! [`Node::Canvas`]), a [`Kind::Grandfather`] clock that holds a face ([`Node::Embed`]: the face
//! its own program, in its own process, drawn in the rect the case leaves it) above its swinging
//! pendulum, and a [`Kind::Shop`] that holds a wall clock and two grandfather clocks, which hold
//! theirs: three levels, one rule. A held face cannot tell it is held; it draws the window it is
//! given, whatever its size, in the theme's inks, so it stays sharp at any scale.
//!
//! One program, run as the name of its `/bin` marker (`clock`, `grandfather` or `shop`), with
//! `tz=<minutes>` (east of UTC) for local time: a holder passes its own on to what it holds. The
//! face ticks each second; the grandfather's pendulum swings at [`SWING_MS`] a frame.

#![forbid(unsafe_code)]

use std::f64::consts::TAU;
use std::fmt::Write;

use uiwire::{Draw, Event, Frame, Node, Request, Shape, Style};

/// What the program shows, by the name it runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Face,
    Grandfather,
    Shop,
}

impl Kind {
    /// The kind a program run as `name` (argv\[0\], a `/bin` name or path) shows; a face for any
    /// other.
    pub fn of(name: &str) -> Kind {
        match name.rsplit('/').next().unwrap_or(name) {
            "grandfather" => Kind::Grandfather,
            "shop" => Kind::Shop,
            _ => Kind::Face,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Kind::Face => "Clock",
            Kind::Grandfather => "Grandfather clock",
            Kind::Shop => "Clock shop",
        }
    }

    /// How often it wants a Tick, in ms (0: never: the shop's clocks tick on their own).
    fn every(self) -> u32 {
        match self {
            Kind::Face => 1000,
            Kind::Grandfather => SWING_MS,
            Kind::Shop => 0,
        }
    }
}

/// A face's side in canvas units, and a grandfather's pendulum window's.
pub const FACE: u16 = 200;
pub const PENDULUM: (u16, u16) = (120, 150);
/// A grandfather's frame each `SWING_MS`, its pendulum's period and its swing (radians each way).
pub const SWING_MS: u32 = 100;
pub const PERIOD_MS: u64 = 2000;
pub const SWING: f64 = 0.28;
/// Node ids: the face's canvas, a grandfather's face and pendulum, the shop's clocks.
pub const DIAL: u32 = 1;
pub const HELD_FACE: u32 = 2;
pub const BOB: u32 = 3;
pub const WALL: u32 = 4;
pub const LEFT: u32 = 5;
pub const RIGHT: u32 = 6;

/// Canvas inks: the dial's (the theme's ink), its minute ticks' (dim ink), the second hand's
/// (the accent).
const INK: u8 = 9;
const DIM: u8 = 10;
const ACCENT: u8 = 11;

/// A clock: what it shows, its time zone (minutes east of UTC) and the time it shows (ms since
/// the epoch, UTC), and whether it has asked for its ticks.
#[derive(Clone, Debug)]
pub struct Clock {
    pub kind: Kind,
    pub tz: i32,
    pub now_ms: u64,
    asked: bool,
}

impl Clock {
    /// The clock argv says: argv\[0\] its kind, `tz=<minutes>` among the rest its zone.
    pub fn new(argv: &[String]) -> Clock {
        let kind = Kind::of(argv.first().map_or("", String::as_str));
        let tz = argv.iter().skip(1).find_map(|a| a.strip_prefix("tz=")?.parse().ok());
        Clock { kind, tz: tz.unwrap_or(0).clamp(-14 * 60, 14 * 60), now_ms: 0, asked: false }
    }

    /// The args a holder passes on: its own zone.
    fn args(&self) -> String {
        let mut s = String::from("tz=");
        _ = write!(s, "{}", self.tz);
        s
    }

    /// Takes `ev` at wall time `now_ms`: whether the window changes (a new size, a tick).
    pub fn event(&mut self, ev: &Event, now_ms: u64) -> bool {
        match ev {
            Event::Resize { .. } | Event::Tick { .. } | Event::Focus { .. } => {
                self.now_ms = now_ms;
                true
            }
            _ => false,
        }
    }

    /// The local time, in ms into the day.
    fn day_ms(&self) -> u64 {
        let local = self.now_ms as i64 + i64::from(self.tz) * 60_000;
        local.rem_euclid(86_400_000) as u64
    }

    /// The window now.
    pub fn frame(&mut self) -> Frame {
        let mut requests = Vec::new();
        if !std::mem::replace(&mut self.asked, true) && self.kind.every() > 0 {
            requests.push(Request::Timer { ms: self.kind.every() });
        }
        let embed = |id, (w, h), program: &str, args: String| Node::Embed {
            id,
            w,
            h,
            program: program.into(),
            args,
        };
        let nodes = match self.kind {
            Kind::Face => {
                vec![Node::Canvas { id: DIAL, w: FACE, h: FACE, draws: face(self.day_ms()) }]
            }
            Kind::Grandfather => vec![Node::Center {
                id: 0,
                gap: 0,
                children: vec![
                    embed(HELD_FACE, (FACE, FACE), "clock", self.args()),
                    pendulum(self.now_ms),
                ],
            }],
            Kind::Shop => vec![Node::Col {
                id: 0,
                gap: 12,
                children: vec![
                    Node::Text { id: 0, style: Style::Title, text: "The clock shop".into() },
                    embed(WALL, (FACE, FACE), "clock", self.args()),
                    Node::Row {
                        id: 0,
                        gap: 16,
                        children: vec![
                            embed(LEFT, (240, 460), "grandfather", self.args()),
                            embed(RIGHT, (240, 460), "grandfather", self.args()),
                        ],
                    },
                ],
            }],
        };
        Frame { seq: 0, title: self.kind.title().into(), nodes, requests }
    }
}

/// A point `r` units from the face's middle at `turn` of a full turn clockwise from 12.
fn at(turn: f64, r: f64) -> (i16, i16) {
    let c = f64::from(FACE) / 2.0;
    let (s, k) = (turn * TAU).sin_cos();
    ((c + r * s).round() as i16, (c - r * k).round() as i16)
}

fn line(color: u8, (x1, y1): (i16, i16), (x2, y2): (i16, i16), width: i16) -> Draw {
    Draw { shape: Shape::Line, color, at: [x1, y1, x2, y2, width], text: String::new() }
}

/// A face at `day_ms` into the day: its rim, a tick each minute (each hour's longer), the
/// numerals 12, 3, 6 and 9, the hour, minute and second hands, and the middle's cap.
pub fn face(day_ms: u64) -> Vec<Draw> {
    let c = (FACE / 2) as i16;
    let mut d =
        vec![Draw { shape: Shape::Ring, color: INK, at: [c, c, 96, 4, 0], text: String::new() }];
    for m in 0..60 {
        let turn = f64::from(m) / 60.0;
        let (inner, color, width) = if m % 5 == 0 { (78.0, INK, 3) } else { (86.0, DIM, 1) };
        d.push(line(color, at(turn, inner), at(turn, 89.0), width));
    }
    for (n, label) in [(0, "12"), (3, "3"), (6, "6"), (9, "9")] {
        let (x, y) = at(f64::from(n) / 12.0, 62.0);
        d.push(Draw { shape: Shape::Text, color: INK, at: [x, y, 18, 0, 0], text: label.into() });
    }
    let secs = day_ms as f64 / 1000.0;
    let (h, m, s) =
        ((secs / 3600.0) % 12.0 / 12.0, (secs / 60.0) % 60.0 / 60.0, (secs.floor() % 60.0) / 60.0);
    let mid = (c, c);
    d.push(line(INK, mid, at(h, 48.0), 7));
    d.push(line(INK, mid, at(m, 72.0), 4));
    d.push(line(ACCENT, at(s + 0.5, 16.0), at(s, 80.0), 2));
    d.push(Draw { shape: Shape::Circle, color: ACCENT, at: [c, c, 6, 0, 0], text: String::new() });
    d
}

/// A grandfather's pendulum window at wall time `now_ms`: the case's sides, the rod swinging
/// from the top's middle, the bob.
pub fn pendulum(now_ms: u64) -> Node {
    let (w, h) = PENDULUM;
    let phase = (now_ms % PERIOD_MS) as f64 / PERIOD_MS as f64;
    let angle = SWING * (phase * TAU).sin();
    let (top, len) = ((i16::try_from(w / 2).unwrap_or(0), 4), 112.0);
    let bob =
        (top.0 + (len * angle.sin()).round() as i16, top.1 + (len * angle.cos()).round() as i16);
    let (wi, hi) = (w as i16, h as i16);
    let draws = vec![
        line(DIM, (3, 0), (3, hi - 1), 3),
        line(DIM, (wi - 4, 0), (wi - 4, hi - 1), 3),
        line(INK, top, bob, 3),
        Draw {
            shape: Shape::Circle,
            color: ACCENT,
            at: [bob.0, bob.1, 14, 0, 0],
            text: String::new(),
        },
        Draw { shape: Shape::Circle, color: INK, at: [top.0, top.1, 4, 0, 0], text: String::new() },
    ];
    Node::Canvas { id: BOB, w, h, draws }
}

/// The wall clock's time: ms since the epoch, UTC.
pub fn wall_ms() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    now.map_or(0, |d| d.as_millis() as u64)
}

/// Runs the window until it closes: a frame after each event that changed it.
pub fn serve<R: std::io::Read, W: std::io::Write>(
    ui: &mut uiwire::client::Client<R, W>,
    c: &mut Clock,
) -> std::io::Result<()> {
    use std::io::ErrorKind;
    let mut seq = 0u32;
    loop {
        let changed = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => c.event(&ev, wall_ms()),
            Err(e) if e.kind() == ErrorKind::InvalidData => false,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if changed {
            ui.show(&Frame { seq, ..c.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests;
