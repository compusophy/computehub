//! The launcher's search: what a query keeps, best first, and a selection
//! that moves through a grid of apps, then down a list of files.

use ui::{AppIcon, Key};

/// Where `query` first matches `label` as a subsequence (ASCII and Latin-1
/// case ignored): the char index of its first char, 0 for an empty query.
pub fn rank(query: &str, label: &str) -> Option<usize> {
    let mut hay = label.chars().enumerate();
    let mut first = None;
    for q in query.chars().map(lower) {
        let (i, _) = hay.find(|&(_, h)| lower(h) == q)?;
        first.get_or_insert(i);
    }
    Some(first.unwrap_or(0))
}

/// `c` in lower case if an ASCII or Latin-1 capital. Not `to_lowercase`:
/// core's Unicode tables cost the boot kilobytes.
pub fn lower(c: char) -> char {
    match c {
        'A'..='Z' | 'À'..='Ö' | 'Ø'..='Þ' => char::from(c as u8 + 32),
        _ => c,
    }
}

/// `c` in upper case if a small letter with a Latin-1 capital (not `ß`, `ÿ`).
pub fn upper(c: char) -> char {
    match c {
        'a'..='z' | 'à'..='ö' | 'ø'..='þ' => char::from(c as u8 - 32),
        _ => c,
    }
}

/// Something the launcher opens: a registry name or a `.app` path, what it
/// is called (and searched by), and where a file lives (`None` for an app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub label: String,
    pub icon: AppIcon,
    pub place: Option<String>,
}

/// A query and its results (indices into `items`, best first), the selection
/// over the tiles then the rows, and the first row on screen of `fit`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub items: Vec<Entry>,
    pub tiles: Vec<usize>,
    pub rows: Vec<usize>,
    pub sel: usize,
    pub first: usize,
    pub fit: usize,
}

impl Search {
    pub fn new(items: Vec<Entry>) -> Search {
        let mut s = Search { items, fit: 1, ..Search::default() };
        s.refilter();
        s
    }

    pub fn count(&self) -> usize {
        self.tiles.len() + self.rows.len()
    }

    /// Result `k`: the tiles, then the rows.
    pub fn get(&self, k: usize) -> Option<&Entry> {
        let i = self.tiles.get(k).or_else(|| self.rows.get(k.checked_sub(self.tiles.len())?))?;
        self.items.get(*i)
    }

    /// Matches the query again (ties keep their order), selecting the best.
    fn refilter(&mut self) {
        let mut ranked: Vec<(bool, usize, usize)> = Vec::new();
        for (i, e) in self.items.iter().enumerate() {
            if let Some(r) = rank(&self.query, &e.label) {
                let key = (e.place.is_some(), r);
                let at = ranked.iter().take_while(|e| (e.0, e.1) <= key).count();
                ranked.insert(at, (key.0, r, i));
            }
        }
        let of = |file: bool| ranked.iter().filter(|e| e.0 == file).map(|e| e.2).collect();
        (self.tiles, self.rows, self.sel, self.first) = (of(false), of(true), 0, 0);
    }

    pub fn type_text(&mut self, s: &str) {
        let len = self.query.len();
        self.query.extend(s.chars().filter(|c| !c.is_control()));
        if self.query.len() != len {
            self.refilter();
        }
    }

    /// Backspace edits the query; the arrows move through a grid of `cols`
    /// tiles, then the list. Whether the key is one of these.
    pub fn key(&mut self, key: Key, cols: usize) -> bool {
        let (n, tiles, s, cols) = (self.count(), self.tiles.len(), self.sel, cols.max(1));
        self.sel = match key {
            Key::Backspace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
                return true;
            }
            Key::Left => s.saturating_sub(1),
            Key::Right => (s + 1).min(n.saturating_sub(1)),
            Key::Up if s > tiles => s - 1,
            Key::Up if s == tiles => s.saturating_sub(1),
            Key::Up => s.checked_sub(cols).unwrap_or(s),
            Key::Down if s >= tiles => (s + 1).min(n.saturating_sub(1)),
            Key::Down if s / cols < (tiles - 1) / cols => (s + cols).min(tiles - 1),
            Key::Down if tiles < n => tiles,
            Key::Down => s,
            _ => return false,
        };
        if let Some(row) = self.sel.checked_sub(tiles) {
            self.first = self.first.min(row).max((row + 1).saturating_sub(self.fit.max(1)));
        }
        true
    }

    pub fn scroll(&mut self, dy: f32, row_h: f32) {
        let rows = (dy / row_h).round();
        let rows = if rows.is_finite() { rows.clamp(-64.0, 64.0) as isize } else { 0 };
        let last = self.rows.len().saturating_sub(self.fit.max(1));
        self.first = self.first.saturating_add_signed(rows).min(last);
    }
}
