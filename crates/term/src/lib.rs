//! The screen model of an xterm-compatible terminal: a grid of [`Cell`]s, a
//! cursor, scroll regions, the alternate screen, scrollback, the replies a
//! program asks for, and the bytes a key press or a paste sends back. It is
//! pure: bytes go in through [`Term::feed`] and state comes out through
//! accessors, with no I/O and no clocks. The `vt` crate parses the byte
//! stream; this crate gives it meaning. A new crate, not a fork.
//!
//! The target is what Claude Code's Ink UI, vim, htop and less expect from
//! `TERM=xterm-256color`.
//!
//! # Behavior
//!
//! - **Pending wrap.** A character printed in the last column leaves the
//!   cursor there with a pending-wrap flag (DECAWM is on by default); the
//!   next printable character wraps first. Any cursor movement clears the
//!   flag, so `CR LF` after a full line does not leave a blank line.
//! - **Erase.** Erased, inserted and scrolled-in cells take the current
//!   background color (xterm's bce).
//! - **Wide characters.** East Asian wide characters and emoji take two cells:
//!   a head (`width` 2) and a tail (`width` 0). Overwriting either half blanks
//!   the other. A wide character that does not fit in the last column wraps
//!   first, or becomes a space when autowrap is off. Combining marks, ZWJ,
//!   variation selectors and skin-tone modifiers are dropped (see
//!   [`char_width`]).
//! - **Scrollback.** Only lines scrolled off the top of the main screen, with
//!   the scroll region starting at the top, are kept: at most [`SCROLLBACK`]
//!   of them. `ED 3` clears them, as in xterm.
//! - **Resize** does not reflow. When rows are taken away, the main screen's
//!   top rows go into the scrollback as far as needed to keep the cursor's
//!   row on screen, as in xterm; otherwise content stays top-aligned. The
//!   cursor is clamped and the scroll region resets.
//! - **Security.** OSC 52 (clipboard) is ignored, titles lose control
//!   characters, [`paste`] strips ESC so pasted text cannot end a bracketed
//!   paste early, and REP repeats only to the end of the line (as in VTE).
//! - **Recorded, not acted on:** mouse and focus reporting modes and
//!   synchronized output (DEC 2026), for the host to read.
//! - **Not yet:** combining characters, left and right margins, reflow, the
//!   kitty keyboard protocol, color queries.
//!
//! # Example
//!
//! ```
//! use term::{Attrs, Color, Term};
//!
//! let mut t = Term::new(20, 3);
//! t.feed(b"\x1b[1;31mhi\x1b[m there\r\n");
//! let h = t.row(0)[0];
//! assert_eq!((h.ch, h.fg), ('h', Color::Indexed(1)));
//! assert!(h.attrs.contains(Attrs::BOLD));
//! assert_eq!(t.cursor(), (1, 0));
//! t.feed(b"\x1b[6n"); // where is the cursor?
//! assert_eq!(t.take_replies(), b"\x1b[2;1R");
//! ```

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
/// Unread reply bytes kept; past this, replies are dropped until the host
/// takes them.
const MAX_REPLIES: usize = 1 << 16;

/// A cell's foreground or background color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    /// The theme's default foreground or background.
    #[default]
    Default,
    /// An xterm palette color: 0-7 normal, 8-15 bright, 16-231 the 6x6x6
    /// cube, 232-255 grays.
    Indexed(u8),
    /// A 24-bit color.
    Rgb(u8, u8, u8),
}

/// Text attributes, as a set of bit flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Attrs(u16);

impl Attrs {
    /// Bold (SGR 1).
    pub const BOLD: Attrs = Attrs(1);
    /// Dim, or faint (SGR 2).
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

    /// No attributes.
    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    /// The raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets the flags in `other`.
    pub fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    /// Clears the flags in `other`.
    pub fn remove(&mut self, other: Attrs) {
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
    /// 1 for a normal cell, 2 for a wide character's head, 0 for its tail
    /// (the cell right of the head, which draws nothing).
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
    /// Row `y` and column `x`, always on screen.
    y: usize,
    x: usize,
    /// Pending wrap: the last column was just written, so the next printable
    /// character starts a new line first.
    wrap: bool,
    /// The colors and attributes (SGR) new characters get; `ch` is unused.
    pen: Cell,
    /// G0 and G1: true when designated DEC special graphics.
    charset: [bool; 2],
    /// The charset in use: 0 for G0 (SI), 1 for G1 (SO).
    gl: usize,
    /// Origin mode (DECOM): rows count from the top margin.
    origin: bool,
}

/// A terminal: the screen, the cursor, the modes the program has set and the
/// replies it is owed. Feed it the program's output, draw from [`Term::row`]
/// and [`Term::cursor`], and send the program [`encode_key`] and [`paste`]
/// bytes.
#[derive(Clone, Debug)]
pub struct Term {
    parser: vt::Parser,
    cols: usize,
    rows: usize,
    /// The visible screen: `rows` lines of `cols` cells.
    screen: Vec<Vec<Cell>>,
    /// The hidden screen: the alternate one while `alt` is off, else the main.
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

fn to_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// Blanks a wide character that the boundary before column `x` would cut in
/// half, that is, when `x` holds a tail.
fn split_at(line: &mut [Cell], x: usize, blank: Cell) {
    if x > 0 && x < line.len() && line[x].width == 0 {
        line[x - 1] = blank;
        line[x] = blank;
    }
}

/// Appends `line` to the scrollback with its trailing blank cells trimmed,
/// reusing the oldest row's allocation once the scrollback is full.
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
    /// A blank terminal of `cols` x `rows` cells, clamped to 1..=1000 columns
    /// and 1..=500 rows.
    pub fn new(cols: u16, rows: u16) -> Term {
        let cols = usize::from(cols.clamp(1, MAX_COLS));
        let rows = usize::from(rows.clamp(1, MAX_ROWS));
        let grid = vec![vec![Cell::BLANK; cols]; rows];
        Term::with_grids(cols, rows, grid.clone(), grid)
    }

    /// The power-on state over two blank `cols` x `rows` grids.
    fn with_grids(cols: usize, rows: usize, screen: Vec<Vec<Cell>>, other: Vec<Vec<Cell>>) -> Term {
        Term {
            parser: vt::Parser::new(),
            cols,
            rows,
            screen,
            other,
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

    /// Processes the program's output. Sequences and UTF-8 characters may be
    /// split across calls.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = mem::take(&mut self.parser);
        parser.advance(bytes, &mut seq::Sink(self));
        self.parser = parser;
        if mem::take(&mut self.dirty) {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Changes the size (clamped like [`Term::new`]) without reflowing.
    /// Columns are cut or added at the right, rows added at the bottom. When
    /// rows are taken away, the main screen's top rows go into the scrollback
    /// as far as needed to keep the cursor's row on screen (on the alternate
    /// screen, the row of the main screen's saved cursor), as xterm does, and
    /// the rest are cut at the bottom. The cursor is clamped and the scroll
    /// region reset. Scrollback rows keep their width.
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
        to_u16(self.cols)
    }

    /// The height in cells.
    pub fn rows(&self) -> u16 {
        to_u16(self.rows)
    }

    /// Row `r` of the visible screen (main or alternate), `cols` cells long;
    /// empty past the last row.
    pub fn row(&self, r: u16) -> &[Cell] {
        self.screen.get(usize::from(r)).map_or(&[][..], Vec::as_slice)
    }

    /// The cursor as (row, column), zero-based and always on screen.
    pub fn cursor(&self) -> (u16, u16) {
        (to_u16(self.cur.y), to_u16(self.cur.x))
    }

    /// Whether the cursor should be drawn (DECTCEM, `CSI ? 25 h`).
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// The window title (OSC 0 or 2), control characters removed, at most
    /// 256 characters.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Takes the bytes the terminal owes the program: answers to status,
    /// device-attribute and mode queries. Write them to the program's input.
    pub fn take_replies(&mut self) -> Vec<u8> {
        mem::take(&mut self.replies)
    }

    /// The number of scrollback rows.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Scrollback row `i`, 0 being the oldest; empty when out of range.
    /// Trailing blank cells are trimmed, and rows from before a resize keep
    /// their old width, so draw missing cells as [`Cell::BLANK`].
    pub fn scrollback_row(&self, i: usize) -> &[Cell] {
        self.scrollback.get(i).map_or(&[][..], Vec::as_slice)
    }

    /// Whether the alternate screen is showing.
    pub fn alt_screen(&self) -> bool {
        self.alt
    }

    /// Whether the program asked for bracketed paste (`CSI ? 2004 h`); pass
    /// it to [`paste`].
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed
    }

    /// Whether the program asked for application cursor keys (DECCKM); pass
    /// it to [`encode_key`].
    pub fn app_cursor_keys(&self) -> bool {
        self.app_cursor
    }

    /// The mouse tracking mode the program asked for: 0 (off), 1000 (clicks),
    /// 1002 (drags too) or 1003 (all motion). Recorded only.
    pub fn mouse_mode(&self) -> u16 {
        self.mouse
    }

    /// Whether the program asked for SGR mouse reports (`CSI ? 1006 h`).
    pub fn mouse_sgr(&self) -> bool {
        self.mouse_sgr
    }

    /// Whether the program asked for focus in and out reports (`CSI ? 1004 h`).
    pub fn focus_reporting(&self) -> bool {
        self.focus
    }

    /// Whether the program is inside a synchronized update (`CSI ? 2026 h`);
    /// a renderer may hold the previous frame until it ends.
    pub fn synchronized(&self) -> bool {
        self.sync
    }

    /// A counter that changes whenever something visible changes (screen,
    /// cursor, modes, title), at most once per [`Term::feed`] or
    /// [`Term::resize`]. Redraw when it differs from the last one drawn.
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

    /// Like `goto`, but in origin mode the row counts from the top margin
    /// and stays inside the scroll region.
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

    /// Line feed: down one row, scrolling the region at its bottom margin.
    fn index(&mut self) {
        self.cur.wrap = false;
        if self.cur.y == self.bottom {
            let save = self.top == 0 && !self.alt;
            self.scroll_up(self.top, self.bottom, 1, save);
        } else if self.cur.y + 1 < self.rows {
            self.cur.y += 1;
        }
    }

    /// Reverse index: up one row, scrolling the region down at its top margin.
    fn reverse_index(&mut self) {
        self.cur.wrap = false;
        if self.cur.y == self.top {
            self.scroll_down(self.top, self.bottom, 1);
        } else if self.cur.y > 0 {
            self.cur.y -= 1;
        }
    }

    /// Scrolls rows `top..=bottom` up by `n`, keeping the rows that leave in
    /// the scrollback when `save`.
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

    /// HT, `n` times: to the next tab stop, or the last column.
    fn tab(&mut self, n: usize) {
        for _ in 0..n.min(self.cols) {
            let next = (self.cur.x + 1..self.cols).find(|&x| self.tabs[x]);
            self.cur.x = next.unwrap_or(self.cols - 1);
        }
        self.cur.wrap = false;
    }

    /// CBT, `n` times: to the previous tab stop, or the first column.
    fn back_tab(&mut self, n: usize) {
        for _ in 0..n.min(self.cols) {
            self.cur.x = (0..self.cur.x).rev().find(|&x| self.tabs[x]).unwrap_or(0);
        }
        self.cur.wrap = false;
    }

    /// DECSC, into the current screen's slot.
    fn save_cursor(&mut self) {
        self.saved[usize::from(self.alt)] = self.cur;
    }

    /// DECRC. Without a save it homes the cursor and resets attributes.
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

    /// RIS: back to the power-on state, keeping the size, the scrollback, the
    /// title, unread replies and the generation. The grids are cleared in
    /// place, not reallocated.
    fn reset(&mut self) {
        let (mut screen, mut other) = (mem::take(&mut self.screen), mem::take(&mut self.other));
        for line in screen.iter_mut().chain(&mut other) {
            line.fill(Cell::BLANK);
        }
        let mut fresh = Term::with_grids(self.cols, self.rows, screen, other);
        fresh.scrollback = mem::take(&mut self.scrollback);
        fresh.title = mem::take(&mut self.title);
        fresh.replies = mem::take(&mut self.replies);
        fresh.generation = self.generation;
        *self = fresh;
    }

    /// DECSTR: resets modes, attributes, charsets, margins and the saved
    /// cursor; leaves the screen and the cursor position alone.
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
