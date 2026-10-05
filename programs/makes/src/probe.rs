//! A made app driven headlessly, as a person and a clock would drive it, and read as a person
//! reads it: by what it shows. A [`Probe`] runs an applang program ([`applang::App`]); finds a
//! button by its label, an input by its place and a board by its shape (a grid widget, pixels, a
//! lattice of equal rects or circles, else the whole canvas, [`Probe::board`]); taps squares and
//! canvas units, presses keys and lets time pass by the app's own timer; and reads the scene: its
//! texts and the numbers and times in them, the color drawn at a point, how a square looks. A
//! fault is an `Err` saying what was being done, the checker's reason.

use applang::{App, Diag, Draw, Event, Limits, Node, Shape};

/// The most ticks one [`Probe::wait`] lets pass.
pub const MAX_TICKS: u32 = 250_000;

/// A running app and what it showed last.
pub struct Probe {
    app: App,
    nodes: Vec<Node>,
}

/// Where a board of squares is: the `nth` grid widget, `cols` a row; or a rectangle of the
/// first canvas, in its units, `cols` x `rows` squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Grid { nth: usize, cols: i64 },
    Area { x: i64, y: i64, w: i64, h: i64, cols: i64, rows: i64 },
}

impl Board {
    /// Square (c, r)'s rectangle on the canvas: left, top, right and bottom (exclusive).
    fn rect(&self, c: i64, r: i64) -> (i64, i64, i64, i64) {
        match *self {
            Board::Area { x, y, w, h, cols, rows } => {
                (x + c * w / cols, y + r * h / rows, x + (c + 1) * w / cols, y + (r + 1) * h / rows)
            }
            Board::Grid { .. } => (0, 0, 0, 0),
        }
    }

    /// Square (c, r)'s middle.
    pub fn mid(&self, c: i64, r: i64) -> (i64, i64) {
        let (x0, y0, x1, y1) = self.rect(c, r);
        ((x0 + x1 - 1) / 2, (y0 + y1 - 1) / 2)
    }
}

/// The first canvas that takes taps, else the first: its handler's id, size and shapes.
#[derive(Clone, Copy, Debug)]
pub struct Canvas<'a> {
    pub id: Option<u32>,
    pub w: i64,
    pub h: i64,
    pub draws: &'a [Draw],
}

impl Probe {
    /// `src` compiled and started with `random` seeded `seed`, as it first shows.
    pub fn start(src: &str, seed: u64) -> Result<Probe, String> {
        let program = applang::compile(src).map_err(|d| said("compiling", &d))?;
        let mut p = Probe { app: App::new(program, Limits::default(), seed), nodes: Vec::new() };
        p.render("its first render")?;
        Ok(p)
    }

    /// [`Probe::start`], then Start (or New game, Play) clicked if it shows one.
    pub fn begin(src: &str, seed: u64) -> Result<Probe, String> {
        let mut p = Probe::start(src, seed)?;
        if p.has("Start|New game|Play") {
            p.click("Start|New game|Play")?;
        }
        Ok(p)
    }

    /// What it shows.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    fn render(&mut self, what: &str) -> Result<(), String> {
        self.nodes =
            self.app.render().map_err(|d| said(&["the render after ", what].concat(), &d))?;
        Ok(())
    }

    fn send(&mut self, ev: Event, what: &str) -> Result<(), String> {
        self.app.handle(&ev).map_err(|d| said(what, &d))?;
        self.render(what)
    }

    /// The id of the first button labelled `label` (alternatives split by `|`, case aside):
    /// exactly, else as a word of its label (`Start` in `Start game`).
    fn button(&self, label: &str) -> Option<u32> {
        let mut all = Vec::new();
        walk(&self.nodes, &mut |n| {
            if let Node::Button { text, id } = n {
                all.push((fold(text), *id));
            }
        });
        let alts: Vec<String> = label.split('|').map(fold).collect();
        let exact = all.iter().find(|(t, _)| alts.contains(t));
        let loose = || all.iter().find(|(t, _)| alts.iter().any(|a| word(t, a)));
        exact.or_else(loose).map(|b| b.1)
    }

    /// Whether a button labelled `label` shows.
    pub fn has(&self, label: &str) -> bool {
        self.button(label).is_some()
    }

    /// Clicks the button labelled `label` ([`Probe::has`]).
    pub fn click(&mut self, label: &str) -> Result<(), String> {
        let id = self.button(label).ok_or_else(|| ["no button labelled ", label].concat())?;
        self.send(Event::Click { id }, &["clicking ", label].concat())
    }

    /// Presses the key `name` (`left`, `space`, `a`).
    pub fn key(&mut self, name: &str) -> Result<(), String> {
        self.send(Event::Key { name: name.into() }, &["the key ", name].concat())
    }

    /// Clicks `label` if a button says it, else presses the key `key`.
    pub fn press(&mut self, label: &str, key: &str) -> Result<(), String> {
        if self.has(label) { self.click(label) } else { self.key(key) }
    }

    /// Types `text` into the `n`th text input (from 0), as a whole.
    pub fn type_in(&mut self, n: usize, text: &str) -> Result<(), String> {
        let mut inputs = Vec::new();
        walk(&self.nodes, &mut |node| {
            if let Node::Input { state, .. } = node {
                inputs.push(state.clone());
            }
        });
        let state = inputs.get(n).cloned().ok_or("it shows no text input to type in")?;
        self.send(Event::Input { state, text: text.into() }, &["typing ", text].concat())
    }

    /// Lets `ms` pass in ticks of its own timer (its shortest `every` now), as the desktop sends
    /// them; time stops passing while no timer runs. A timer so fast that [`MAX_TICKS`] ticks
    /// do not let `ms` pass is an `Err`, never a reading taken early.
    pub fn wait(&mut self, ms: u64) -> Result<(), String> {
        let mut left = ms;
        for _ in 0..MAX_TICKS {
            let every = u64::from(self.app.timer());
            if left == 0 || every == 0 {
                return self.render("waiting");
            }
            let dt = every.min(left);
            self.app.handle(&Event::Tick { ms: dt as u32 }).map_err(|d| said("a tick", &d))?;
            left -= dt;
        }
        let every = self.app.timer();
        if left > 0 && every > 0 {
            return Err(format!(
                "its timer (every {every}) needs more than {MAX_TICKS} ticks to let {ms} ms pass, \
                 the most a checker lets pass"
            ));
        }
        self.render("waiting")
    }

    /// The first canvas that takes taps, else the first.
    pub fn canvas(&self) -> Option<Canvas<'_>> {
        let mut all = Vec::new();
        walk(&self.nodes, &mut |n| {
            if let Node::Canvas { id, w, h, draws } = n {
                all.push(Canvas { id: *id, w: (*w).into(), h: (*h).into(), draws });
            }
        });
        all.iter().find(|c| c.id.is_some()).or(all.first()).copied()
    }

    /// Taps the canvas at unit (x, y), kept on it.
    pub fn tap(&mut self, x: i64, y: i64) -> Result<(), String> {
        let c = self.canvas().filter(|c| c.id.is_some()).ok_or("no canvas takes taps")?;
        let (x, y) = (x.clamp(0, c.w - 1), y.clamp(0, c.h - 1));
        let ev = Event::Tap { id: c.id.unwrap_or(0), cell: (y * c.w + x) as u32 };
        self.send(ev, &format!("tapping the canvas at {x}, {y}"))
    }

    /// Where a board of `cols` x `rows` squares is: a grid widget of that shape; else on the
    /// first canvas, pixels of that shape, a lattice of equal rects or circles (the gaps between
    /// them going to the squares), or the whole canvas.
    pub fn board(&self, cols: i64, rows: i64) -> Option<Board> {
        let mut grids = Vec::new();
        walk(&self.nodes, &mut |n| {
            if let Node::Grid { cols: c, cells, .. } = n {
                grids.push(i64::from(*c) == cols && cells.len() as i64 == cols * rows);
            }
        });
        if let Some(nth) = grids.iter().position(|&fits| fits) {
            return Some(Board::Grid { nth, cols });
        }
        let c = self.canvas()?;
        let area = |(x, y, sx, sy): (i64, i64, i64, i64)| Board::Area {
            x,
            y,
            w: sx * cols,
            h: sy * rows,
            cols,
            rows,
        };
        let pixels = c.draws.iter().find(|d| {
            d.shape == Shape::Pixels
                && i64::from(d.at[2]) == cols
                && d.text.len() as i64 == cols * rows
        });
        if let Some(d) = pixels {
            let side = i64::from(d.at[3]);
            return Some(area((d.at[0].into(), d.at[1].into(), side, side)));
        }
        let whole = Board::Area { x: 0, y: 0, w: c.w, h: c.h, cols, rows };
        Some(lattice(c.draws, cols, rows).map_or(whole, area))
    }

    /// Taps square (c, r) of `b`.
    pub fn tap_cell(&mut self, b: Board, c: i64, r: i64) -> Result<(), String> {
        match b {
            Board::Grid { nth, cols } => {
                let id = self.grid(nth).and_then(|g| g.0).ok_or("the board takes no taps")?;
                let ev = Event::Tap { id, cell: (r * cols + c) as u32 };
                self.send(ev, &format!("tapping square {c}, {r}"))
            }
            Board::Area { .. } => {
                let (x, y) = b.mid(c, r);
                self.tap(x, y)
            }
        }
    }

    /// The `nth` grid widget: its handler's id, squares and texts.
    fn grid(&self, nth: usize) -> Option<(Option<u32>, Vec<u8>, Vec<String>)> {
        let mut all = Vec::new();
        walk(&self.nodes, &mut |n| {
            if let Node::Grid { id, cells, texts, .. } = n {
                all.push((*id, cells.clone(), texts.clone()));
            }
        });
        all.into_iter().nth(nth)
    }

    /// The color of square (c, r) of `b`: a grid's square, else the color at its middle.
    pub fn color(&self, b: Board, c: i64, r: i64) -> i64 {
        match b {
            Board::Grid { nth, cols } => self
                .grid(nth)
                .map_or(-1, |g| g.1.get((r * cols + c) as usize).map_or(-1, |&v| i64::from(v))),
            Board::Area { .. } => {
                let (x, y) = b.mid(c, r);
                self.color_at(x, y)
            }
        }
    }

    /// How square (c, r) of `b` looks: a grid's square and text; on a canvas, the color at its
    /// middle and each shape whose middle is in it (not pixels, nor one half again a square's
    /// size), by kind, color and text, sorted, wherever in the square it is.
    pub fn look(&self, b: Board, c: i64, r: i64) -> String {
        if let Board::Grid { nth, cols } = b {
            let g = self.grid(nth).unwrap_or_default();
            let i = (r * cols + c) as usize;
            return format!(
                "{}:{}",
                g.1.get(i).copied().unwrap_or(0),
                g.2.get(i).map_or("", |t| t)
            );
        }
        let (x0, y0, x1, y1) = b.rect(c, r);
        let mut parts = vec![format!("@{}", self.color(b, c, r))];
        for d in self.canvas().map_or(&[][..], |c| c.draws) {
            let (a, t, z, e) = extent(d);
            let small = (z - a) * 2 <= (x1 - x0) * 3 && (e - t) * 2 <= (y1 - y0) * 3;
            let (mx, my) = ((a + z - 1) / 2, (t + e - 1) / 2);
            if d.shape != Shape::Pixels && small && (x0..x1).contains(&mx) && (y0..y1).contains(&my)
            {
                parts.push(format!("{:?}{}{}", d.shape, d.color, d.text));
            }
        }
        parts[1..].sort();
        parts.join(" ")
    }

    /// The color drawn at unit (x, y) of the canvas: the last shape over it (texts aside), else 0.
    pub fn color_at(&self, x: i64, y: i64) -> i64 {
        let draws = self.canvas().map_or(&[][..], |c| c.draws);
        draws.iter().rev().find_map(|d| over(d, x, y)).unwrap_or(0)
    }

    /// The texts it shows as its output: labels and the texts of canvases and grids (never a
    /// button's label nor what an input holds).
    pub fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        walk(&self.nodes, &mut |n| match n {
            Node::Label { text } => out.push(text.clone()),
            Node::Canvas { draws, .. } => {
                out.extend(draws.iter().filter(|d| d.shape == Shape::Text).map(|d| d.text.clone()))
            }
            Node::Grid { texts, .. } => out.extend(texts.iter().filter(|t| !t.is_empty()).cloned()),
            _ => {}
        });
        out
    }

    /// Whether a text it shows holds `phrase` (alternatives split by `|`), case aside.
    pub fn says(&self, phrase: &str) -> bool {
        holds(&self.texts(), phrase)
    }

    /// The whole numbers in its texts, in order (a `-` right before one makes it negative).
    pub fn numbers(&self) -> Vec<i64> {
        self.texts().iter().flat_map(|t| ints(t)).collect()
    }

    /// The first number after `word` (case aside) in a text that holds it: `Score: 3` is 3.
    pub fn after(&self, word: &str) -> Option<i64> {
        self.texts().iter().find_map(|t| {
            let t = fold(t);
            let at = t.find(&fold(word))? + word.len();
            ints(&t[at..]).first().copied()
        })
    }

    /// The times in its texts: each run of numbers joined by `:` (`12:00:05` is [12, 0, 5]).
    pub fn times(&self) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        for t in self.texts() {
            let (mut cur, mut num): (Vec<i64>, Option<i64>) = (Vec::new(), None);
            for ch in t.chars().chain([' ']) {
                if let Some(d) = ch.to_digit(10) {
                    num = Some(num.unwrap_or(0).saturating_mul(10).saturating_add(d.into()));
                } else if ch == ':' && num.is_some() {
                    cur.extend(num.take());
                } else {
                    if let (Some(n), false) = (num.take(), cur.is_empty()) {
                        cur.push(n);
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
        }
        out
    }

    /// The numbers with a decimal point in its texts, in tenths (`2.5` is 25, `12.34` 123).
    pub fn tenths(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for t in self.texts() {
            let b = t.as_bytes();
            for i in 1..b.len().saturating_sub(1) {
                if b[i] == b'.' && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit() {
                    let start =
                        (0..i).rev().take_while(|&j| b[j].is_ascii_digit()).last().unwrap_or(i);
                    let whole: i64 = t[start..i].parse().unwrap_or(0);
                    out.push(whole * 10 + i64::from(b[i + 1] - b'0'));
                }
            }
        }
        out
    }
}

/// A fault while doing `what`, as a checker's reason.
fn said(what: &str, d: &Diag) -> String {
    format!("faults {what}: E{:04} {}", d.code.unwrap_or(0), coder::ai::clip(&d.message, 160))
}

/// `s` lowercased (ASCII) and trimmed.
pub fn fold(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// Whether `hay` holds `needle` as a word: no letter or digit just before or after it.
fn word(hay: &str, needle: &str) -> bool {
    let alnum = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    hay.match_indices(needle).any(|(i, _)| {
        !alnum(hay[..i].chars().next_back()) && !alnum(hay[i + needle.len()..].chars().next())
    })
}

/// Whether one of `texts` holds `phrase` (alternatives split by `|`), case aside.
pub fn holds(texts: &[String], phrase: &str) -> bool {
    texts.iter().any(|t| phrase.split('|').any(|p| fold(t).contains(&fold(p))))
}

/// The whole numbers in `s`; a `-` (or `−`) right before one, not after a letter or digit,
/// makes it negative.
pub fn ints(s: &str) -> Vec<i64> {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < c.len() {
        if !c[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut n: i64 = 0;
        while i < c.len() && c[i].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(i64::from(c[i] as u8 - b'0'));
            i += 1;
        }
        let sign = start > 0 && matches!(c[start - 1], '-' | '\u{2212}');
        let neg = sign && (start < 2 || !c[start - 2].is_alphanumeric());
        out.push(if neg { -n } else { n });
    }
    out
}

/// Every node of `nodes`, depth first.
pub fn walk<'a>(nodes: &'a [Node], f: &mut dyn FnMut(&'a Node)) {
    for n in nodes {
        f(n);
        if let Node::Row { children } | Node::Col { children } = n {
            walk(children, f);
        }
    }
}

/// What `d` covers, as near as its shape is known: left, top, right, bottom (exclusive).
pub fn extent(d: &Draw) -> (i64, i64, i64, i64) {
    let [x, y, a, b, c] = d.at.map(i64::from);
    let rows = d.text.lines().count() as i64;
    let cols = d.text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as i64;
    match d.shape {
        Shape::Rect => (x, y, x + a, y + b),
        Shape::Circle | Shape::Ring => (x - a, y - a, x + a + 1, y + a + 1),
        Shape::Line => {
            (x.min(a) - c / 2, y.min(b) - c / 2, x.max(a) + c / 2 + 1, y.max(b) + c / 2 + 1)
        }
        Shape::Text => {
            let n = d.text.chars().count() as i64;
            (x - n * a / 4, y - a / 2, x + n * a / 4 + 1, y + a / 2 + 1)
        }
        Shape::Sprite => (x, y, x + cols * a, y + rows * a),
        Shape::Pixels => (x, y, x + a * b, y + d.text.len() as i64 / a.max(1) * b),
    }
}

/// The color `d` draws at unit (px, py), if it covers it (a text never does).
fn over(d: &Draw, px: i64, py: i64) -> Option<i64> {
    let [x, y, a, b, c] = d.at.map(i64::from);
    let color = i64::from(d.color);
    // Circles, rings and lines center on their units' middles: twice the units, plus one.
    let (qx, qy) = (2 * (px - x), 2 * (py - y));
    // Which square of side `side` (a sprite's or pixels') the unit is in.
    let at = |side: i64| {
        let (col, row) = ((px - x).div_euclid(side.max(1)), (py - y).div_euclid(side.max(1)));
        Some((usize::try_from(col).ok()?, usize::try_from(row).ok()?))
    };
    match d.shape {
        Shape::Rect => ((x..x + a).contains(&px) && (y..y + b).contains(&py)).then_some(color),
        Shape::Circle => (qx * qx + qy * qy <= 4 * a * a).then_some(color),
        Shape::Ring => {
            let d2 = qx * qx + qy * qy;
            (d2 <= 4 * a * a && d2 >= 4 * (a - b).max(0).pow(2)).then_some(color)
        }
        // Within half its width of the segment, in whole numbers: past an end, from that end;
        // else |q|^2 - (q.e)^2 / |e|^2, both sides times |e|^2.
        Shape::Line => {
            let (ex, ey) = (i128::from(2 * (a - x)), i128::from(2 * (b - y)));
            let (qx, qy) = (i128::from(qx), i128::from(qy));
            let (len, dot, w2) =
                (ex * ex + ey * ey, qx * ex + qy * ey, i128::from(c.max(1).pow(2)));
            let near = match () {
                _ if dot <= 0 || len == 0 => qx * qx + qy * qy <= w2,
                _ if dot >= len => (qx - ex).pow(2) + (qy - ey).pow(2) <= w2,
                _ => (qx * qx + qy * qy) * len - dot * dot <= w2 * len,
            };
            near.then_some(color)
        }
        Shape::Text => None,
        // A sprite paints its digits; pixels, `0` to `9`, `a` and `b`.
        Shape::Sprite => {
            let (col, row) = at(a)?;
            let ch = *d.text.split('\n').nth(row)?.as_bytes().get(col)?;
            char::from(ch).to_digit(10).map(i64::from)
        }
        Shape::Pixels => {
            let (col, row) = at(b).filter(|&(col, _)| (col as i64) < a)?;
            let ch = *d.text.as_bytes().get(row * a as usize + col)?;
            char::from(ch).to_digit(12).map(i64::from)
        }
    }
}

/// A lattice of `cols` x `rows` equal rects (or circles), evenly spaced: where it starts and the
/// step between squares, across and down.
fn lattice(draws: &[Draw], cols: i64, rows: i64) -> Option<(i64, i64, i64, i64)> {
    for shape in [Shape::Rect, Shape::Circle] {
        let mut sizes: Vec<(i64, i64)> = draws
            .iter()
            .filter(|d| d.shape == shape)
            .map(|d| {
                (d.at[2].into(), if shape == Shape::Rect { d.at[3].into() } else { d.at[2].into() })
            })
            .filter(|s| s.0 > 0 && s.1 > 0)
            .collect();
        sizes.sort_unstable();
        sizes.dedup();
        for (w, h) in sizes {
            let same = draws.iter().filter(|d| d.shape == shape && i64::from(d.at[2]) == w);
            let same: Vec<&Draw> =
                same.filter(|d| shape == Shape::Circle || i64::from(d.at[3]) == h).collect();
            let axis = |k: usize| {
                let mut v: Vec<i64> = same.iter().map(|d| i64::from(d.at[k])).collect();
                v.sort_unstable();
                v.dedup();
                v
            };
            let (xs, ys) = (axis(0), axis(1));
            let even = |v: &[i64], n: i64, size: i64| {
                let step = if v.len() > 1 { v[1] - v[0] } else { size };
                let even = v.windows(2).all(|p| p[1] - p[0] == step);
                (v.len() as i64 == n && step > 0 && even).then_some(step)
            };
            let (Some(sx), Some(sy)) = (even(&xs, cols, w), even(&ys, rows, h)) else { continue };
            // A square's middle is a rect's middle, or a circle's center.
            let (ox, oy) = match shape {
                Shape::Rect => (xs[0] - (sx - w) / 2, ys[0] - (sy - h) / 2),
                _ => (xs[0] - sx / 2, ys[0] - sy / 2),
            };
            return Some((ox, oy, sx, sy));
        }
    }
    None
}
