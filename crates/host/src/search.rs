//! The launcher's search: ranking labels against a query, and what a query
//! keeps, with a selection that moves through a grid of apps, then down a
//! list of files.

use ui::{AppIcon, Key};

/// Where `query` first matches `label` as a subsequence, ignoring the case
/// of ASCII and Latin-1 letters ([`lower`]): the char index of its first
/// char, 0 for an empty query, `None` for no match.
pub fn rank(query: &str, label: &str) -> Option<usize> {
    let mut hay = label.chars().enumerate();
    let mut first = None;
    for q in query.chars().map(lower) {
        let (i, _) = hay.find(|&(_, h)| lower(h) == q)?;
        first.get_or_insert(i);
    }
    Some(first.unwrap_or(0))
}

/// `c` in lower case if it is an ASCII or Latin-1 capital (`É` is `é`),
/// else `c`. Not `char::to_lowercase`: core's Unicode case tables cost the
/// boot download kilobytes.
pub fn lower(c: char) -> char {
    match c {
        'A'..='Z' | 'À'..='Ö' | 'Ø'..='Þ' => char::from(c as u8 + 32),
        _ => c,
    }
}

/// `c` in upper case if it is an ASCII or Latin-1 small letter with a
/// Latin-1 capital (`é` is `É`; `ß` and `ÿ` stay), else `c`. See [`lower`].
pub fn upper(c: char) -> char {
    match c {
        'a'..='z' | 'à'..='ö' | 'ø'..='þ' => char::from(c as u8 - 32),
        _ => c,
    }
}

/// The indices of `labels` that `query` matches ([`rank`]), best first, ties
/// in order.
pub fn filter<'a>(query: &str, labels: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut ranked: Vec<(usize, usize)> = Vec::new();
    for (i, label) in labels.into_iter().enumerate() {
        if let Some(r) = rank(query, label) {
            let at = ranked.iter().take_while(|e| e.0 <= r).count();
            ranked.insert(at, (r, i));
        }
    }
    ranked.into_iter().map(|e| e.1).collect()
}

/// Something the launcher opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// A registry name or a `.app` path.
    pub name: String,
    /// What it is called, and searched by.
    pub label: String,
    pub icon: AppIcon,
    /// Where a file lives, shown beside it; `None` for an app (a tile).
    pub place: Option<String>,
}

/// A query and its results: the apps that match as `tiles`, the files as
/// `rows`, both best first. The selection `sel` runs over the tiles, then
/// the rows; `first` is the first row on screen and `fit` how many fit.
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
    /// A search over `items` with an empty query.
    pub fn new(items: Vec<Entry>) -> Search {
        let mut s = Search { items, fit: 1, ..Search::default() };
        s.refilter();
        s
    }

    /// How many results there are.
    pub fn count(&self) -> usize {
        self.tiles.len() + self.rows.len()
    }

    /// Result `k`, counting the tiles, then the rows.
    pub fn get(&self, k: usize) -> Option<&Entry> {
        let i = self.tiles.get(k).or_else(|| self.rows.get(k.checked_sub(self.tiles.len())?))?;
        self.items.get(*i)
    }

    /// Matches the query again, selecting the best, scrolled to the top.
    fn refilter(&mut self) {
        let (items, q) = (&self.items, self.query.as_str());
        let of = |file: bool| {
            let mine: Vec<usize> =
                (0..items.len()).filter(|&i| items[i].place.is_some() == file).collect();
            let found = filter(q, mine.iter().map(|&i| items[i].label.as_str()));
            found.into_iter().map(|k| mine[k]).collect::<Vec<usize>>()
        };
        (self.tiles, self.rows, self.sel, self.first) = (of(false), of(true), 0, 0);
    }

    /// Typed text joins the query (control characters do not).
    pub fn type_text(&mut self, s: &str) {
        let len = self.query.len();
        self.query.extend(s.chars().filter(|c| !c.is_control()));
        if self.query.len() != len {
            self.refilter();
        }
    }

    /// Backspace edits the query; the arrows move the selection through a
    /// grid of `cols` tiles (Down from its last line, and Right from its
    /// last tile, go on to the list), then along the list, keeping the
    /// selected row on screen. Returns whether the key is one of these.
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

    /// The wheel scrolls the list by `dy` px, rows `row_h` px tall.
    pub fn scroll(&mut self, dy: f32, row_h: f32) {
        let rows = (dy / row_h).round();
        let rows = if rows.is_finite() { rows.clamp(-64.0, 64.0) as isize } else { 0 };
        let last = self.rows.len().saturating_sub(self.fit.max(1));
        self.first = self.first.saturating_add_signed(rows).min(last);
    }
}
