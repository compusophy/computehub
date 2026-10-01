//! The screen model of an xterm-compatible terminal (what vim, htop, less
//! and Ink UIs expect of `TERM=xterm-256color`): a grid of [`Cell`]s, cursor,
//! scroll region, alternate screen, scrollback, the replies a program asks
//! for, and the bytes a key or a paste sends. Pure: bytes in through
//! [`Term::feed`] (parsed by `vt`), state out through accessors; no I/O, no
//! clocks. A new crate.
//!
//! Invariants: the cursor is always on screen, with xterm's pending wrap
//! (any movement clears it). Erased cells take the current background (bce).
//! A wide character is a head (`width` 2) and a tail (`width` 0), never cut
//! in half. Only lines leaving the top of the main screen reach the
//! scrollback, at most [`SCROLLBACK`]. Resize does not reflow; it keeps the
//! cursor's row on screen. OSC 52 is ignored, titles lose controls, [`paste`]
//! strips ESC, REP stops at the end of the line, and unread replies are
//! capped. Mouse, focus and synchronized-output modes are only recorded.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::VecDeque;
use std::mem;

mod key;
mod seq;
mod width;

pub use key::{Key, KeyMods, encode_key, paste};
pub use width::char_width;

/// The most scrollback rows kept; older ones are dropped.
pub const SCROLLBACK: usize = 5000;
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;
const MAX_TITLE: usize = 256;
/// Unread reply bytes kept; later replies are dropped until they are taken.
const MAX_REPLIES: usize = 1 << 16;

/// A cell's foreground or background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// The theme's default foreground or background.
    Default,
    /// An xterm palette index: 0-15 the ANSI colors, then the cube and grays.
    Indexed(u8),
    /// A 24-bit color.
    Rgb(u8, u8, u8),
}

/// Text attributes, as bit flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Attrs(u16);

impl Attrs {
    /// Bold (SGR 1).
    pub const BOLD: Attrs = Attrs(1);
    /// Dim (SGR 2).
    pub const DIM: Attrs = Attrs(1 << 1);
    /// Italic (SGR 3).
    pub const ITALIC: Attrs = Attrs(1 << 2);
    /// Underlined (SGR 4, 4:1 to 4:5, 21).
    pub const UNDERLINE: Attrs = Attrs(1 << 3);
    /// Blinking (SGR 5 or 6).
    pub const BLINK: Attrs = Attrs(1 << 4);
    /// Foreground and background swapped (SGR 7).
    pub const INVERSE: Attrs = Attrs(1 << 5);
    /// Invisible (SGR 8).
    pub const HIDDEN: Attrs = Attrs(1 << 6);
    /// Struck through (SGR 9).
    pub const STRIKE: Attrs = Attrs(1 << 7);

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    fn remove(&mut self, other: Attrs) {
        self.0 &= !other.0;
    }
}

impl core::ops::BitOr for Attrs {
    type Output = Attrs;
    fn bitor(self, other: Attrs) -> Attrs {
        Attrs(self.0 | other.0)
    }
}

/// One character cell of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The character: a space in an empty cell and in a wide tail.
    pub ch: char,
    /// 1, or 2 for a wide character's head and 0 for its tail (drawn by it).
    pub width: u8,
    /// The foreground color.
    pub fg: Color,
    /// The background color.
    pub bg: Color,
    /// Bold, underline and the rest.
    pub attrs: Attrs,
}

impl Cell {
    /// An empty cell: a space, width 1, default colors, no attributes.
    pub const BLANK: Cell =
        Cell { ch: ' ', width: 1, fg: Color::Default, bg: Color::Default, attrs: Attrs(0) };
}

impl Default for Cell {
    fn default() -> Cell {
        Cell::BLANK
    }
}

/// The cursor and the state DECSC saves with it.
#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    y: usize,
    x: usize,
    /// Pending wrap: the next printable character starts a new line first.
    wrap: bool,
    /// The colors and attributes (SGR) new characters get; `ch` is unused.
    pen: Cell,
    /// G0 and G1: true when designated DEC special graphics.
    charset: [bool; 2],
    /// The charset in use: 0 (G0, SI) or 1 (G1, SO).
    gl: usize,
    /// Origin mode (DECOM): rows count from the top margin.
    origin: bool,
}

/// A terminal: the screen, the cursor, the modes the program set and the
/// replies it is owed.
#[derive(Clone, Debug)]
pub struct Term {
    parser: vt::Parser,
    cols: usize,
    rows: usize,
    /// The visible screen, and the hidden one (the alternate, or the main).
    screen: Vec<Vec<Cell>>,
    other: Vec<Vec<Cell>>,
    alt: bool,
    scrollback: VecDeque<Vec<Cell>>,
    cur: Cursor,
    /// The DECSC slots of the main and the alternate screen.
    saved: [Cursor; 2],
    /// The scroll region, inclusive.
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    autowrap: bool,
    insert: bool,
    app_cursor: bool,
    cursor_visible: bool,
    bracketed: bool,
    mouse: u16,
    mouse_sgr: bool,
    focus: bool,
    sync: bool,
    /// The last printed character, for REP; any control clears it.
    last: Option<char>,
    title: String,
    replies: Vec<u8>,
    generation: u64,
    /// Something visible changed during the current `feed`.
    dirty: bool,
}

/// Blanks the wide character the boundary before column `x` would cut.
fn split_at(line: &mut [Cell], x: usize, blank: Cell) {
    if x > 0 && x < line.len() && line[x].width == 0 {
        line[x - 1] = blank;
        line[x] = blank;
    }
}

/// Appends `line`, trailing blanks trimmed, reusing the oldest row when full.
fn save_line(scrollback: &mut VecDeque<Vec<Cell>>, line: &[Cell]) {
    let used = line.iter().rposition(|c| *c != Cell::BLANK);
    let mut kept = Vec::new();
    if scrollback.len() >= SCROLLBACK {
        kept = scrollback.pop_front().unwrap_or_default();
        kept.clear();
    }
    kept.extend_from_slice(&line[..used.map_or(0, |i| i + 1)]);
    scrollback.push_back(kept);
}

impl Term {
    /// A blank terminal, clamped to 1..=1000 columns and 1..=500 rows.
    pub fn new(cols: u16, rows: u16) -> Term {
        let cols = usize::from(cols.clamp(1, MAX_COLS));
        let rows = usize::from(rows.clamp(1, MAX_ROWS));
        let screen = vec![vec![Cell::BLANK; cols]; rows];
        Term {
            parser: vt::Parser::default(),
            cols,
            rows,
            other: screen.clone(),
            screen,
            alt: false,
            scrollback: VecDeque::new(),
            cur: Cursor::default(),
            saved: [Cursor::default(); 2],
            top: 0,
            bottom: rows - 1,
            tabs: (0..cols).map(|x| x % 8 == 0).collect(),
            autowrap: true,
            insert: false,
            app_cursor: false,
            cursor_visible: true,
            bracketed: false,
            mouse: 0,
            mouse_sgr: false,
            focus: false,
            sync: false,
            last: None,
            title: String::new(),
            replies: Vec::new(),
            generation: 0,
            dirty: false,
        }
    }

    /// Processes the program's output; sequences may span calls.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = mem::take(&mut self.parser);
        parser.advance(bytes, &mut seq::Sink(self));
        self.parser = parser;
        if mem::take(&mut self.dirty) {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Resizes (clamped like [`Term::new`]) without reflow. Rows taken away
    /// go into the scrollback from the main screen's top as far as needed to
    /// keep the cursor's row (on the alternate screen, the saved cursor's) on
    /// screen, the rest from the bottom; the scroll region resets.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = usize::from(cols.clamp(1, MAX_COLS));
        let rows = usize::from(rows.clamp(1, MAX_ROWS));
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        let (main, anchor) = if self.alt {
            (&mut self.other, self.saved[0].y)
        } else {
            (&mut self.screen, self.cur.y)
        };
        let n = (anchor.min(self.rows - 1) + 1).saturating_sub(rows);
        for line in main.drain(..n) {
            save_line(&mut self.scrollback, &line);
        }
        if !self.alt {
            self.cur.y -= n;
        }
        self.saved[0].y = self.saved[0].y.saturating_sub(n);
        for grid in [&mut self.screen, &mut self.other] {
            grid.resize_with(rows, Vec::new);
            for line in grid.iter_mut() {
                line.resize(cols, Cell::BLANK);
                if line[cols - 1].width == 2 {
                    line[cols - 1] = Cell::BLANK;
                }
            }
        }
        self.tabs = (0..cols).map(|x| self.tabs.get(x).copied().unwrap_or(x % 8 == 0)).collect();
        (self.cols, self.rows) = (cols, rows);
        (self.top, self.bottom) = (0, rows - 1);
        self.goto(self.cur.y, self.cur.x);
        self.generation = self.generation.wrapping_add(1);
    }

    /// The width in cells.
    pub fn cols(&self) -> u16 {
        self.cols as u16
    }

    /// The height in cells.
    pub fn rows(&self) -> u16 {
        self.rows as u16
    }

    /// Row `r` of the visible screen; empty past the last row.
    pub fn row(&self, r: u16) -> &[Cell] {
        self.screen.get(usize::from(r)).map_or(&[][..], Vec::as_slice)
    }

    /// The cursor as (row, column), zero-based.
    pub fn cursor(&self) -> (u16, u16) {
        (self.cur.y as u16, self.cur.x as u16)
    }

    /// Whether the cursor should be drawn (DECTCEM).
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// The window title (OSC 0 or 2), without controls, at most 256 chars.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Takes the answers to status, attribute and mode queries owed to the
    /// program's input.
    pub fn take_replies(&mut self) -> Vec<u8> {
        mem::take(&mut self.replies)
    }

    /// The number of scrollback rows.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Scrollback row `i`, 0 the oldest, empty when out of range. Trailing
    /// blanks are trimmed and old rows keep their width: draw the rest blank.
    pub fn scrollback_row(&self, i: usize) -> &[Cell] {
        self.scrollback.get(i).map_or(&[][..], Vec::as_slice)
    }

    /// Whether the alternate screen is showing.
    pub fn alt_screen(&self) -> bool {
        self.alt
    }

    /// Bracketed paste (`CSI ? 2004 h`), for [`paste`].
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed
    }

    /// Application cursor keys (DECCKM), for [`encode_key`].
    pub fn app_cursor_keys(&self) -> bool {
        self.app_cursor
    }

    /// Changes whenever something visible does, at most once per
    /// [`Term::feed`] or [`Term::resize`]: redraw when it differs.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// An erased cell: blank, in the current background color.
    fn blank(&self) -> Cell {
        Cell { bg: self.cur.pen.bg, ..Cell::BLANK }
    }

    fn print(&mut self, mut c: char) {
        let mut w = usize::from(char_width(c));
        if w == 0 {
            return;
        }
        self.last = Some(c);
        self.dirty = true;
        if self.cur.charset[self.cur.gl] {
            c = width::dec_graphics(c);
        }
        if self.cur.wrap {
            self.cur.x = 0;
            self.index();
        }
        if w == 2 && self.cur.x + 2 > self.cols {
            if self.autowrap && self.cols > 1 {
                self.cur.x = 0;
                self.index();
            } else {
                (c, w) = (' ', 1);
            }
        }
        if self.insert {
            self.insert_cells(w);
        }
        let (x, blank, mut cell) = (self.cur.x, self.blank(), self.cur.pen);
        (cell.ch, cell.width) = (c, if w == 2 { 2 } else { 1 });
        let line = &mut self.screen[self.cur.y];
        split_at(line, x, blank);
        split_at(line, x + w, blank);
        line[x] = cell;
        if w == 2 {
            (cell.ch, cell.width) = (' ', 0);
            line[x + 1] = cell;
        }
        if x + w >= self.cols {
            self.cur.x = self.cols - 1;
            self.cur.wrap = self.autowrap;
        } else {
            self.cur.x = x + w;
        }
    }

    /// Moves to (`y`, `x`), clamped to the screen; clears pending wrap.
    fn goto(&mut self, y: usize, x: usize) {
        self.cur.y = y.min(self.rows - 1);
        self.cur.x = x.min(self.cols - 1);
        self.cur.wrap = false;
    }

    /// `goto`, but in origin mode rows count from, and stay in, the region.
    fn goto_origin(&mut self, y: usize, x: usize) {
        let (lo, hi) = if self.cur.origin { (self.top, self.bottom) } else { (0, self.rows - 1) };
        self.goto(lo.saturating_add(y).min(hi), x);
    }

    /// Cursor up, stopping at the top margin when starting inside the region.
    fn up(&mut self, n: usize) {
        let lo = if self.cur.y >= self.top { self.top } else { 0 };
        self.goto(self.cur.y.saturating_sub(n).max(lo), self.cur.x);
    }

    /// Cursor down, stopping at the bottom margin when starting inside it.
    fn down(&mut self, n: usize) {
        let hi = if self.cur.y <= self.bottom { self.bottom } else { self.rows - 1 };
        self.goto(self.cur.y.saturating_add(n).min(hi), self.cur.x);
    }

    /// Line feed: down a row, scrolling the region at its bottom margin.
    fn index(&mut self) {
        self.cur.wrap = false;
        if self.cur.y == self.bottom {
            let save = self.top == 0 && !self.alt;
            self.scroll_up(self.top, self.bottom, 1, save);
        } else if self.cur.y + 1 < self.rows {
            self.cur.y += 1;
        }
    }

    /// Reverse index: up a row, scrolling the region down at its top margin.
    fn reverse_index(&mut self) {
        self.cur.wrap = false;
        if self.cur.y == self.top {
            self.scroll_down(self.top, self.bottom, 1);
        } else if self.cur.y > 0 {
            self.cur.y -= 1;
        }
    }

    /// Scrolls rows `top..=bottom` up by `n`, into the scrollback if `save`.
    fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, save: bool) {
        let n = n.min(bottom + 1 - top);
        let blank = self.blank();
        for y in top..top + n {
            if save {
                save_line(&mut self.scrollback, &self.screen[y]);
            }
            self.screen[y].fill(blank);
        }
        self.screen[top..=bottom].rotate_left(n);
    }

    /// Scrolls rows `top..=bottom` down by `n`.
    fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        let blank = self.blank();
        self.screen[top..=bottom].rotate_right(n);
        for line in &mut self.screen[top..top + n] {
            line.fill(blank);
        }
    }

    /// Erases columns `a..b` of row `y`.
    fn erase(&mut self, y: usize, a: usize, b: usize) {
        let blank = self.blank();
        let line = &mut self.screen[y];
        split_at(line, a, blank);
        split_at(line, b, blank);
        line[a..b].fill(blank);
    }

    /// ICH: inserts `n` blanks at the cursor, pushing the rest right.
    fn insert_cells(&mut self, n: usize) {
        let (x, cols, blank) = (self.cur.x, self.cols, self.blank());
        let n = n.min(cols - x);
        let line = &mut self.screen[self.cur.y];
        split_at(line, x, blank);
        line[x..].rotate_right(n);
        line[x..x + n].fill(blank);
        if line[cols - 1].width == 2 {
            line[cols - 1] = blank;
        }
        self.cur.wrap = false;
    }

    /// DCH: deletes `n` cells at the cursor, pulling the rest left.
    fn delete_cells(&mut self, n: usize) {
        let (x, cols, blank) = (self.cur.x, self.cols, self.blank());
        let n = n.min(cols - x);
        let line = &mut self.screen[self.cur.y];
        split_at(line, x, blank);
        split_at(line, x + n, blank);
        line[x..].rotate_left(n);
        line[cols - n..].fill(blank);
        self.cur.wrap = false;
    }

    /// HT (CBT when `back`), `n` times: to the next (previous) stop or the edge.
    fn tab(&mut self, n: usize, back: bool) {
        for _ in 0..n.min(self.cols) {
            self.cur.x = if back {
                (0..self.cur.x).rev().find(|&x| self.tabs[x]).unwrap_or(0)
            } else {
                (self.cur.x + 1..self.cols).find(|&x| self.tabs[x]).unwrap_or(self.cols - 1)
            };
        }
        self.cur.wrap = false;
    }

    /// DECSC, into the current screen's slot.
    fn save_cursor(&mut self) {
        self.saved[usize::from(self.alt)] = self.cur;
    }

    /// DECRC; without a save it homes the cursor and resets attributes.
    fn restore_cursor(&mut self) {
        let saved = self.saved[usize::from(self.alt)];
        self.cur = saved;
        self.goto(saved.y, saved.x);
        self.cur.wrap = saved.wrap && self.autowrap && self.cur.x == self.cols - 1;
    }

    fn set_alt(&mut self, on: bool) {
        if on != self.alt {
            mem::swap(&mut self.screen, &mut self.other);
            self.alt = on;
            self.cur.wrap = false;
        }
    }

    /// RIS: the power-on state, keeping the size, scrollback, title, unread
    /// replies and generation.
    fn reset(&mut self) {
        let mut fresh = Term::new(self.cols as u16, self.rows as u16);
        fresh.scrollback = mem::take(&mut self.scrollback);
        fresh.title = mem::take(&mut self.title);
        fresh.replies = mem::take(&mut self.replies);
        fresh.generation = self.generation;
        *self = fresh;
    }

    /// DECSTR: resets modes, attributes, charsets, margins and the saved
    /// cursor; the screen and the cursor position stay.
    fn soft_reset(&mut self) {
        (self.cursor_visible, self.autowrap) = (true, true);
        (self.insert, self.app_cursor) = (false, false);
        let (y, x) = (self.cur.y, self.cur.x);
        self.cur = Cursor { y, x, ..Cursor::default() };
        (self.top, self.bottom) = (0, self.rows - 1);
        self.saved = [Cursor::default(); 2];
    }
}

#[cfg(test)]
mod tests;
