//! The launcher's search: what a query keeps, best first; a selection over the Ask row (while
//! there is a query), the apps, then the files.

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

/// A query and its results (indices into `items`, best first), the selection over the results
/// (the Ask row while there is a query, the tiles, then the rows), and the first row on screen
/// of `fit`.
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

    /// Whether result 0 is the Ask row: there is a query to ask the Assistant.
    pub fn ask(&self) -> bool {
        !self.query.is_empty()
    }

    fn skip(&self) -> usize {
        usize::from(self.ask())
    }

    pub fn count(&self) -> usize {
        self.skip() + self.tiles.len() + self.rows.len()
    }

    /// Result `k` if it is an app or a file (not the Ask row).
    pub fn get(&self, k: usize) -> Option<&Entry> {
        let k = k.checked_sub(self.skip())?;
        let i = self.tiles.get(k).or_else(|| self.rows.get(k.checked_sub(self.tiles.len())?))?;
        self.items.get(*i)
    }

    /// Matches the query again (ties keep their order), selecting the first app whose name
    /// starts with it, else the Ask row (with no query, the first app).
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
        (self.tiles, self.rows, self.first) = (of(false), of(true), 0);
        let starts = |i: &usize| {
            let mut label = self.items[*i].label.chars().map(lower);
            self.query.chars().all(|q| label.next() == Some(lower(q)))
        };
        self.sel = self.tiles.iter().position(starts).map_or(0, |t| t + self.skip());
    }

    pub fn type_text(&mut self, s: &str) {
        let len = self.query.len();
        self.query.extend(s.chars().filter(|c| !c.is_control()));
        if self.query.len() != len {
            self.refilter();
        }
    }

    /// Backspace edits the query; the arrows move from the Ask row through a grid of `cols`
    /// tiles, then the list. Whether the key is one of these.
    pub fn key(&mut self, key: Key, cols: usize) -> bool {
        let (skip, tiles, cols) = (self.skip(), self.tiles.len(), cols.max(1));
        let (s, last) = (self.sel, self.count().saturating_sub(1));
        // The selection among the tiles and rows; `None` on the Ask row.
        let t = s.checked_sub(skip);
        self.sel = match (key, t) {
            (Key::Backspace, _) => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
                return true;
            }
            (Key::Left, _) => s.saturating_sub(1),
            (Key::Right, _) | (Key::Down, None) => (s + 1).min(last),
            (Key::Up, None) => s,
            (Key::Up, Some(t)) if t >= tiles => s.saturating_sub(1),
            (Key::Up, Some(t)) if t >= cols => s - cols,
            (Key::Up, Some(_)) => {
                if skip > 0 {
                    0
                } else {
                    s
                }
            }
            (Key::Down, Some(t)) if t >= tiles => (s + 1).min(last),
            (Key::Down, Some(t)) if t / cols < (tiles - 1) / cols => {
                (t + cols).min(tiles - 1) + skip
            }
            (Key::Down, Some(_)) if skip + tiles <= last => skip + tiles,
            (Key::Down, Some(_)) => s,
            _ => return false,
        };
        if let Some(row) = self.sel.checked_sub(skip + tiles) {
            self.first = self.first.min(row).max((row + 1).saturating_sub(self.fit.max(1)));
        }
        true
    }

    /// Scrolls the list by `rows` (down if positive), keeping it full.
    pub fn scroll(&mut self, rows: isize) {
        let last = self.rows.len().saturating_sub(self.fit.max(1));
        self.first = self.first.saturating_add_signed(rows).min(last);
    }
}
