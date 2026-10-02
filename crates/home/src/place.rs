//! Where the home screen's icons sit, as a phone's home screen keeps them: each app in a cell of
//! the grid, staying where the person put it, empty cells staying empty. A cell is a column and
//! a row. A wide screen's grid (columns down from the top left, as many rows as fit) and a
//! phone's (rows of four) differ in shape, so each keeps its own arrangement: one the person
//! never arranged shows the apps packed in their order (the apps' own, or the order kept before
//! cells were; new ones last). From its first change on (a drop, or a new app there), every
//! icon there has its cell.
//!
//! Kept as the `home.order` preference ([`format()`]): `@2`, then each app as
//! `name:wide:narrow`, a cell as `col.row` or empty for none. Before `@2` it held the names
//! alone, in order, which [`parse`] reads as places with no cells: each layout packed in that
//! order, so nothing moves.

/// A cell: its column and row.
pub type Cell = (u16, u16);

/// The mark that starts a stored arrangement (the version); before it, the order alone.
const V2: &str = "@2";

/// An app's place: its name, and its cell on a wide screen and on a phone, if it has one there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub cells: [Option<Cell>; 2],
}

impl Place {
    pub fn new(name: &str) -> Place {
        Place { name: name.to_string(), cells: [None; 2] }
    }
}

/// The grid's shape on a screen: its columns and rows shown (at least one of each), and whether
/// it is a phone's, its cells in reading order across the rows (a wide screen's: down the
/// columns). Past the cells shown, positions go on: more rows on a phone, more columns wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dims {
    cols: usize,
    rows: usize,
    narrow: bool,
}

impl Dims {
    pub fn new(cols: usize, rows: usize, narrow: bool) -> Dims {
        Dims { cols: cols.max(1), rows: rows.max(1), narrow }
    }

    /// Which of a place's cells is this layout's: 0 wide, 1 narrow.
    pub fn layout(&self) -> usize {
        usize::from(self.narrow)
    }

    /// How many cells show.
    pub fn shown(&self) -> usize {
        self.cols.saturating_mul(self.rows)
    }

    /// The position in reading order of the cell at column `c`, row `r`.
    fn index(&self, c: usize, r: usize) -> usize {
        match self.narrow {
            true => r.saturating_mul(self.cols).saturating_add(c),
            false => c.saturating_mul(self.rows).saturating_add(r),
        }
    }

    /// Cell `c`'s position in reading order, if it shows.
    pub fn pos(&self, (c, r): Cell) -> Option<usize> {
        let (c, r) = (usize::from(c), usize::from(r));
        (c < self.cols && r < self.rows).then(|| self.index(c, r))
    }

    /// The position of the cell shown nearest column `c`, row `r`.
    pub fn nearest(&self, c: usize, r: usize) -> usize {
        self.index(c.min(self.cols - 1), r.min(self.rows - 1))
    }

    /// The cell at position `p`.
    pub fn cell(&self, p: usize) -> Cell {
        let (c, r) = match self.narrow {
            true => (p % self.cols, p / self.cols),
            false => (p / self.rows, p % self.rows),
        };
        let fit = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
        (fit(c), fit(r))
    }
}

/// The places a stored preference names, each once (see the module's docs).
pub fn parse(stored: &str) -> Vec<Place> {
    let (v2, rest) = match stored.strip_prefix(V2) {
        Some(rest) if rest.is_empty() || rest.starts_with(',') => (true, rest),
        _ => (false, stored),
    };
    let mut out: Vec<Place> = Vec::new();
    for item in rest.split(',').filter(|i| !i.is_empty()) {
        let place = if v2 { entry(item) } else { Place::new(item) };
        if !place.name.is_empty() && !out.iter().any(|p| p.name == place.name) {
            out.push(place);
        }
    }
    out
}

/// One stored place, `name:wide:narrow` (the name may hold colons); without both cells, a name.
fn entry(item: &str) -> Place {
    let mut parts = item.rsplitn(3, ':');
    let (narrow, wide) = (parts.next().unwrap_or(""), parts.next());
    match (parts.next(), wide) {
        (Some(name), Some(wide)) => {
            Place { name: name.to_string(), cells: [cell(wide), cell(narrow)] }
        }
        _ => Place::new(item),
    }
}

/// A stored cell, `col.row`.
fn cell(s: &str) -> Option<Cell> {
    let (c, r) = s.split_once('.')?;
    Some((num(c)?, num(r)?))
}

/// A stored number: decimal digits, at least one (not `parse`: a few hundred bytes of boot).
fn num(s: &str) -> Option<u16> {
    let digit = |n: u16, b: u8| n.checked_mul(10)?.checked_add(u16::from(b.checked_sub(b'0')?));
    s.bytes().try_fold(0, |n, b| digit(n, b).filter(|_| b <= b'9')).filter(|_| !s.is_empty())
}

/// `places` as stored: `@2`, then each (but one without a name or with a comma in it).
pub fn format(places: &[Place]) -> String {
    let mut out = String::from(V2);
    for p in places.iter().filter(|p| !p.name.is_empty() && !p.name.contains(',')) {
        out.push(',');
        out.push_str(&p.name);
        for c in p.cells {
            out.push(':');
            if let Some((c, r)) = c {
                ui::push_num(&mut out, c.into());
                out.push('.');
                ui::push_num(&mut out, r.into());
            }
        }
    }
    out
}

/// `items` (the apps in their first order, each called `name(item)`) as kept: those of `kept`
/// first, in its order and with their places, then the rest with none; and whether there were
/// any.
pub fn arrange<T>(
    kept: &[Place],
    mut items: Vec<T>,
    name: impl Fn(&T) -> &str,
) -> (Vec<T>, Vec<Place>, bool) {
    let (mut out, mut places) = (Vec::new(), Vec::new());
    for p in kept {
        if let Some(i) = items.iter().position(|t| name(t) == p.name) {
            out.push(items.remove(i));
            places.push(p.clone());
        }
    }
    let new = !items.is_empty();
    places.extend(items.iter().map(|t| Place::new(name(t))));
    out.append(&mut items);
    (out, places, new)
}

/// Where each of `places` shows on a grid of `dims`, as a position in reading order: at its own
/// cell in this layout if that shows and no earlier one took it; the rest (none there yet, or
/// one off this screen, kept for a larger one) in order, in the first free cells, past the
/// shown ones once they are full.
pub fn resolve(places: &[Place], dims: Dims) -> Vec<usize> {
    let l = dims.layout();
    let mut taken: Vec<usize> = Vec::new();
    let mut out: Vec<Option<usize>> = Vec::new();
    for p in places {
        let pos = p.cells[l].and_then(|c| dims.pos(c)).filter(|p| !taken.contains(p));
        taken.extend(pos);
        out.push(pos);
    }
    let (mut spots, mut free) = (Vec::new(), 0);
    for pos in out {
        if pos.is_none() {
            while taken.contains(&free) {
                free += 1;
            }
            taken.push(free);
        }
        spots.push(pos.unwrap_or(free));
    }
    spots
}

/// Keeps where `places` show (`spots`, on a grid of `dims`) as their cells in this layout: every
/// one's if `all`, else but those whose own cell is off this screen (kept for a larger one, as
/// when a phone's keyboard shortens it).
pub fn keep(places: &mut [Place], spots: &[usize], dims: Dims, all: bool) {
    let l = dims.layout();
    for (p, &s) in places.iter_mut().zip(spots) {
        let off = p.cells[l].is_some_and(|c| dims.pos(c).is_none());
        if all || !off {
            p.cells[l] = Some(dims.cell(s));
        }
    }
}

/// Where every icon shows (`spots` now, on a grid of `dims`) once those `carried` (indices, the
/// one pressed, `lead`, among them) are put down with the lead at position `to`: the others
/// carried in the same places around it (`group`). An icon in a cell they take moves a cell
/// on in reading order, the one there in turn, as far as the first empty cell: the others make
/// room as on a phone, and the rest stay (the cells they left stay empty). On a grid full from
/// there to its end, they move back instead; full both ways, on past its end.
pub fn plan(spots: &[usize], carried: &[usize], lead: usize, to: usize, dims: Dims) -> Vec<usize> {
    let n = spots.len();
    if !carried.contains(&lead) || carried.iter().any(|&k| k >= n) {
        return spots.to_vec();
    }
    let targets = group(spots, carried, lead, to, dims);
    let mut rest = spots.to_vec();
    for (&k, &t) in carried.iter().zip(&targets) {
        rest[k] = t;
    }
    let moved = |up, end| shove(&rest, carried, &targets, up, end);
    moved(true, dims.shown())
        .or_else(|| moved(false, 0))
        .or_else(|| moved(true, usize::MAX))
        .unwrap_or(rest)
}

/// Where `carried` land with the `lead` at `to`: each the same columns and rows from it as now,
/// the group moved whole and kept inside the cells shown; one larger than the grid, the lead
/// at `to` and the rest after it, in order.
fn group(spots: &[usize], carried: &[usize], lead: usize, to: usize, dims: Dims) -> Vec<usize> {
    let at = |p: usize| {
        let (c, r) = dims.cell(p);
        (i32::from(c), i32::from(r))
    };
    let ((lc, lr), (tc, tr)) = (at(spots[lead]), at(to));
    let offs: Vec<(i32, i32)> =
        carried.iter().map(|&k| at(spots[k])).map(|(c, r)| (c - lc, r - lr)).collect();
    // The group's extent around the lead (which is at 0, 0).
    let (mut lo, mut hi) = ((0, 0), (0, 0));
    for &(c, r) in &offs {
        (lo, hi) = ((lo.0.min(c), lo.1.min(r)), (hi.0.max(c), hi.1.max(r)));
    }
    let fit = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
    let (cols, rows) = (fit(dims.cols), fit(dims.rows));
    if hi.0 - lo.0 >= cols || hi.1 - lo.1 >= rows {
        let rank = |k: usize| carried.iter().filter(|&&j| j != lead && j <= k && k != lead).count();
        return carried.iter().map(|&k| to.saturating_add(rank(k))).collect();
    }
    let (c, r) = (tc.max(-lo.0).min(cols - 1 - hi.0), tr.max(-lo.1).min(rows - 1 - hi.1));
    offs.iter().map(|&(dc, dr)| dims.index((c + dc) as usize, (r + dr) as usize)).collect()
}

/// The icons not `carried` (at `spots`) moved off the `claimed` cells, walking from the first
/// claimed (`up`: on in reading order, below `end`; else from the last, back to the first
/// cell): each to the next cell along, the one there moving on in turn, until an empty one.
/// `None` if the walk ends first.
fn shove(
    spots: &[usize],
    carried: &[usize],
    claimed: &[usize],
    up: bool,
    end: usize,
) -> Option<Vec<usize>> {
    let mut out = spots.to_vec();
    let along = |p: usize, c: usize| if up { c > p } else { c < p };
    let first = if up { claimed.iter().min() } else { claimed.iter().max() };
    // Those waiting for a cell, in order: the queue from `head` on.
    let (mut p, mut queue, mut head) = (*first?, Vec::new(), 0);
    loop {
        if up && p >= end {
            return None;
        }
        let here = (0..out.len()).find(|&i| out[i] == p && !carried.contains(&i));
        if claimed.contains(&p) {
            queue.extend(here);
        } else if let Some(&i) = queue.get(head) {
            (out[i], head) = (p, head + 1);
            queue.extend(here);
        }
        let next = claimed.iter().copied().filter(|&c| along(p, c));
        let next = if up { next.min() } else { next.max() };
        p = match (head == queue.len(), next) {
            (true, None) => return Some(out),
            (true, Some(c)) => c,
            (false, _) if up => p.saturating_add(1),
            (false, _) => p.checked_sub(1)?,
        };
    }
}
