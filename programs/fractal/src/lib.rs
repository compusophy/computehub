//! Fractal, the mesh's demo: a Mandelbrot picture of [`SIDE`] x [`SIDE`] tiles, each a chunk of
//! one job on the pool ([`Request::Job`]): every core of every linked tab renders tiles, and each
//! tile shows in the color of the device that rendered it ([`pool::TINTS`]). A tap zooms in four
//! times there; Zoom out goes back. Under the picture each device's share of the tiles, and how
//! long the picture took.
//!
//! A tile is heavy on purpose, about 100 ms of a core: [`TILE`] x [`TILE`] pixels of [`SS`] x
//! [`SS`] samples, each up to [`MAX_ITER`] steps. [`work_tile`] answers a tile's line with its fuel
//! (the steps taken), the SHA-256 of its pixels and the pixels, one digit each: 0 inside the set
//! (drawn in the device's color), 1 a band (dim), 2 a band left dark. f64 math in wasm is exact
//! IEEE, so every device answers a tile with the same bytes, and a replay checks it.

#![forbid(unsafe_code)]

use std::fmt::Write;

use uiwire::pool::{self, Snap};
use uiwire::{Draw, Event, Frame, Node, Request, Shape, Style, Variant};

/// Tiles a side, pixels a tile's side, samples a pixel's side, steps a sample at most.
pub const SIDE: u32 = 12;
pub const TILE: u32 = 8;
pub const SS: u32 = 16;
pub const MAX_ITER: u32 = 8000;
/// The picture's side in pixels.
pub const PX: u32 = SIDE * TILE;
/// Node ids: the picture, Render, Zoom out.
pub const PICTURE: u32 = 1;
pub const RENDER: u32 = 2;
pub const OUT: u32 = 3;

/// The part of the plane in view: its center and width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: f64,
    pub y: f64,
    pub span: f64,
}

/// The whole set.
pub const HOME: View = View { x: -0.6, y: 0.0, span: 3.0 };

/// Tile `(tx, ty)` of `v` as a chunk's line.
pub fn chunk(v: &View, tx: u32, ty: u32) -> String {
    let mut s = String::new();
    _ = write!(s, "{tx} {ty} {:?} {:?} {:?}", v.x, v.y, v.span);
    s
}

/// A tile's answer: `<fuel> <sha256> <pixels>`; `None` for a line that is no tile.
pub fn work_tile(line: &str) -> Option<String> {
    let mut w = line.split(' ');
    let (tx, ty): (u32, u32) = (w.next()?.parse().ok()?, w.next()?.parse().ok()?);
    let (x, y, span): (f64, f64, f64) =
        (w.next()?.parse().ok()?, w.next()?.parse().ok()?, w.next()?.parse().ok()?);
    if w.next().is_some() || tx >= SIDE || ty >= SIDE || !span.is_finite() || span <= 0.0 {
        return None;
    }
    let (px, mut fuel, mut pixels) = (span / f64::from(PX), 0u64, String::new());
    for row in 0..TILE {
        for col in 0..TILE {
            let (mut inside, mut steps) = (0u32, 0u64);
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = f64::from(tx * TILE + col) + (f64::from(sx) + 0.5) / f64::from(SS);
                    let fy = f64::from(ty * TILE + row) + (f64::from(sy) + 0.5) / f64::from(SS);
                    let (cr, ci) = (x - span / 2.0 + fx * px, y + span / 2.0 - fy * px);
                    let (mut zr, mut zi, mut n) = (0.0f64, 0.0f64, 0);
                    while n < MAX_ITER && zr * zr + zi * zi <= 4.0 {
                        (zr, zi) = (zr * zr - zi * zi + cr, 2.0 * zr * zi + ci);
                        n += 1;
                    }
                    fuel += u64::from(n);
                    if n == MAX_ITER {
                        inside += 1;
                    } else {
                        steps += u64::from(n);
                    }
                }
            }
            let escaped = u64::from(SS * SS - inside);
            pixels.push(match escaped {
                0 => '0',
                _ if inside > 0 => '1',
                _ => {
                    let avg = (steps / escaped).max(1) as u32;
                    if (32 - avg.leading_zeros()) % 2 == 0 { '1' } else { '2' }
                }
            });
        }
    }
    let mut out = String::new();
    _ = write!(out, "{fuel} ");
    sha256(pixels.as_bytes()).iter().for_each(|b| _ = write!(out, "{b:02x}"));
    out.push(' ');
    out.push_str(&pixels);
    Some(out)
}

/// SHA-256 (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// The window: the view, each tile's pixels and device once answered, the pool as last heard,
/// whether a render was asked, and the requests since the last frame.
#[derive(Debug)]
pub struct Fractal {
    pub view: View,
    pub tiles: Vec<Option<(u8, String)>>,
    pub snap: Snap,
    started: bool,
    requests: Vec<Request>,
}

impl Default for Fractal {
    fn default() -> Fractal {
        let tiles = vec![None; (SIDE * SIDE) as usize];
        Fractal { view: HOME, tiles, snap: Snap::default(), started: false, requests: Vec::new() }
    }
}

impl Fractal {
    /// Renders the view: a job of every tile, the picture cleared.
    pub fn render(&mut self) {
        self.tiles.iter_mut().for_each(|t| *t = None);
        let chunks = (0..SIDE * SIDE).map(|i| chunk(&self.view, i % SIDE, i / SIDE)).collect();
        self.requests.push(Request::Job { name: "fractal".into(), chunks });
    }

    /// Handles one event; whether the window changed. The first size renders the whole set.
    pub fn event(&mut self, ev: &Event) -> bool {
        match ev {
            Event::Resize { .. } if !self.started => {
                self.started = true;
                self.render();
            }
            Event::Done { index, node, out } => {
                let pixels = out.rsplit(' ').next().unwrap_or("");
                let tile = self.tiles.get_mut(*index as usize);
                if let Some(t) = tile.filter(|_| pixels.len() == (TILE * TILE) as usize) {
                    *t = Some((*node, pixels.into()));
                }
            }
            Event::Pool { data } => match Snap::decode(data) {
                Some(s) => self.snap = s,
                None => return false,
            },
            Event::Tap { id: PICTURE, cell } => {
                let (col, row) = (cell % PX, cell / PX);
                let px = self.view.span / f64::from(PX);
                self.view.x += (f64::from(col) + 0.5 - f64::from(PX) / 2.0) * px;
                self.view.y -= (f64::from(row) + 0.5 - f64::from(PX) / 2.0) * px;
                self.view.span /= 4.0;
                self.render();
            }
            Event::Click { id: RENDER } => self.render(),
            Event::Click { id: OUT } => {
                self.view.span = (self.view.span * 4.0).min(HOME.span);
                self.render();
            }
            Event::Resize { .. } => {}
            _ => return false,
        }
        true
    }

    /// The picture: each tile answered, in its device's color.
    fn picture(&self) -> Node {
        let mut draws = Vec::new();
        for (i, t) in self.tiles.iter().enumerate() {
            let Some((node, pixels)) = t else { continue };
            let tint = pool::TINTS[usize::from(*node) % pool::TINTS.len()];
            // Inside the set in the device's color, so every tile says who rendered it; the bands
            // outside in dim ink.
            let paint = |c: char| match c {
                '0' => char::from(uiwire::PAINT[usize::from(tint)]),
                '1' => 'a',
                _ => '.',
            };
            let (x, y) = ((i as u32 % SIDE * TILE) as i16, (i as u32 / SIDE * TILE) as i16);
            let text = pixels.chars().map(paint).collect();
            draws.push(Draw {
                shape: Shape::Pixels,
                color: 0,
                at: [x, y, TILE as i16, 1, 0],
                text,
            });
        }
        Node::Canvas { id: PICTURE, w: PX as u16, h: PX as u16, draws }
    }

    pub fn frame(&mut self) -> Frame {
        let done = self.tiles.iter().filter(|t| t.is_some()).count();
        let s = &self.snap;
        let n = s.devices.len().max(1);
        let mut line = String::new();
        _ = write!(line, "{done} of {} tiles", SIDE * SIDE);
        if let Some(j) = s.job.as_ref().filter(|j| j.mine) {
            _ = write!(line, " \u{b7} {}.{} s", j.ms / 1000, j.ms % 1000 / 100);
        }
        _ = write!(line, " \u{b7} {n} device{}", if n == 1 { "" } else { "s" });
        let mut nodes = vec![Node::Text { id: 0, style: Style::Small, text: line }, self.picture()];
        let per = s.job.as_ref().filter(|j| j.mine).map(|j| j.per.clone()).unwrap_or_default();
        for (k, d) in s.devices.iter().enumerate() {
            let tiles = per.get(k).copied().unwrap_or(0);
            let share = (tiles as usize * 1000).checked_div(done).unwrap_or(0).min(1000) as u16;
            let mut text = d.name.clone();
            _ = write!(text, " \u{b7} {} cores \u{b7} {tiles} tiles", d.cores);
            nodes.push(Node::Text { id: 0, style: Style::Small, text });
            let hue = pool::TINTS[k % pool::TINTS.len()];
            nodes.push(Node::Meter { id: 0, hue, value: share });
        }
        let buttons = vec![
            Node::Button { id: RENDER, variant: Variant::Primary, label: "Render".into() },
            Node::Button { id: OUT, variant: Variant::Normal, label: "Zoom out".into() },
        ];
        nodes.push(Node::Row { id: 0, gap: 8, children: buttons });
        let col = Node::Col { id: 0, gap: 10, children: nodes };
        let requests = std::mem::take(&mut self.requests);
        let nodes = vec![Node::Scroll { id: 0, children: vec![col] }];
        Frame { seq: 0, title: "Fractal".into(), nodes, requests }
    }
}

/// Runs the window until it closes: a frame after each event that changed it.
pub fn serve<R: std::io::Read, W: std::io::Write>(
    ui: &mut uiwire::client::Client<R, W>,
    f: &mut Fractal,
) -> std::io::Result<()> {
    use std::io::ErrorKind;
    let mut seq = 0u32;
    loop {
        let changed = match ui.next_event() {
            Ok(Event::Close) => return Ok(()),
            Ok(ev) => f.event(&ev),
            Err(e) if e.kind() == ErrorKind::InvalidData => false,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if changed {
            ui.show(&Frame { seq, ..f.frame() })?;
            seq = seq.wrapping_add(1);
        }
    }
}

/// The worker: echo off, `ready`, then each line `<index> <tile>` answered `<index> <answer>`
/// (a line that is no tile: fuel 0, no hash, no pixels), until the console ends.
pub fn work() -> std::io::Result<()> {
    use std::io::{BufRead, Write};
    let ctl = std::fs::OpenOptions::new().write(true).open("/dev/consctl");
    ctl.and_then(|mut c| c.write_all(b"echooff"))?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "ready")?;
    out.flush()?;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let (index, tile) = line.split_once(' ').unwrap_or((&line, ""));
        let answer = work_tile(tile).unwrap_or_else(|| "0 - ".into());
        writeln!(out, "{index} {answer}")?;
        out.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
