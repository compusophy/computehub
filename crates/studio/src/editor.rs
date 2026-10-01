//! The text buffer behind Studio's editor: lines and one caret, with the
//! edits and moves the keyboard maps to. Pure data; Studio draws it.

/// A multi-line text buffer with one caret. Columns count chars, not bytes,
/// so they match a monospace grid. Text coming in keeps `\n` as line
/// breaks, turns a tab into two spaces and drops other control chars (`\r`
/// too). There is always at least one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    lines: Vec<String>,
    line: usize,
    col: usize,
    /// The column up and down aim for, kept across short lines.
    goal: usize,
}

impl Default for Editor {
    fn default() -> Editor {
        Editor::new("")
    }
}

/// The byte offset of char `col` in `s` (its length past the end).
fn byte(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

/// The length of `s` in chars.
fn chars(s: &str) -> usize {
    s.chars().count()
}

/// `text` as the buffer keeps it.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' => out.push_str("  "),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl Editor {
    /// A buffer holding `text`, the caret at its start.
    pub fn new(text: &str) -> Editor {
        let mut e = Editor { lines: vec![String::new()], line: 0, col: 0, goal: 0 };
        e.insert(text);
        e.set_caret(0, 0);
        e
    }

    /// The whole text, lines joined with `\n`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
        }
        out
    }

    /// How many lines there are (at least one).
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Line `i`, or `""` past the end.
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", |s| s)
    }

    /// The caret as (line, column), both from 0.
    pub fn caret(&self) -> (usize, usize) {
        (self.line, self.col)
    }

    /// Puts the caret at (line, column), clamped into the text.
    pub fn set_caret(&mut self, line: usize, col: usize) {
        self.line = line.min(self.lines.len() - 1);
        self.col = col.min(chars(&self.lines[self.line]));
        self.goal = self.col;
    }

    /// Moves to `line` (clamped), at the goal column or the line's end.
    pub fn move_to_line(&mut self, line: usize) {
        self.line = line.min(self.lines.len() - 1);
        self.col = self.goal.min(chars(&self.lines[self.line]));
    }

    /// Inserts `text` at the caret and puts the caret after it.
    pub fn insert(&mut self, text: &str) {
        let text = clean(text);
        let cur = &mut self.lines[self.line];
        let tail = cur.split_off(byte(cur, self.col));
        let mut parts = text.split('\n');
        cur.push_str(parts.next().unwrap_or_default());
        if text.contains('\n') {
            // The new lines go between this one and the ones after it.
            let mut after = self.lines.split_off(self.line + 1);
            for part in parts {
                self.lines.push(part.to_string());
            }
            self.line = self.lines.len() - 1;
            self.lines.append(&mut after);
        }
        let last = &mut self.lines[self.line];
        self.col = chars(last);
        last.push_str(&tail);
        self.goal = self.col;
    }

    /// Enter: breaks the line at the caret and starts the new one with the
    /// current line's indentation (its leading spaces up to the caret).
    pub fn newline(&mut self) {
        let cur = &self.lines[self.line];
        let indent = cur.chars().take_while(|&c| c == ' ').count().min(self.col);
        let mut text = String::from("\n");
        for _ in 0..indent {
            text.push(' ');
        }
        self.insert(&text);
    }

    /// Deletes the char before the caret; at a line's start, joins the line
    /// to the one above.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let cur = &mut self.lines[self.line];
            cur.remove(byte(cur, self.col - 1));
            self.col -= 1;
        } else if self.line > 0 {
            let cur = self.lines.remove(self.line);
            self.line -= 1;
            let prev = &mut self.lines[self.line];
            self.col = chars(prev);
            prev.push_str(&cur);
        }
        self.goal = self.col;
    }

    /// Deletes the char after the caret; at a line's end, joins the next
    /// line to it.
    pub fn delete(&mut self) {
        let cur = &mut self.lines[self.line];
        if self.col < chars(cur) {
            cur.remove(byte(cur, self.col));
        } else if self.line + 1 < self.lines.len() {
            let next = self.lines.remove(self.line + 1);
            self.lines[self.line].push_str(&next);
        }
        self.goal = self.col;
    }

    /// One char left, onto the end of the line above at a line's start.
    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.col = chars(&self.lines[self.line]);
        }
        self.goal = self.col;
    }

    /// One char right, onto the start of the next line at a line's end.
    pub fn right(&mut self) {
        if self.col < chars(&self.lines[self.line]) {
            self.col += 1;
        } else if self.line + 1 < self.lines.len() {
            self.line += 1;
            self.col = 0;
        }
        self.goal = self.col;
    }

    /// One line up; on the first line, to its start.
    pub fn up(&mut self) {
        match self.line {
            0 => self.set_caret(0, 0),
            l => self.move_to_line(l - 1),
        }
    }

    /// One line down; on the last line, to its end.
    pub fn down(&mut self) {
        match self.line + 1 < self.lines.len() {
            true => self.move_to_line(self.line + 1),
            false => self.set_caret(self.line, usize::MAX),
        }
    }

    /// To the start of the line.
    pub fn home(&mut self) {
        self.set_caret(self.line, 0);
    }

    /// To the end of the line.
    pub fn end(&mut self) {
        self.set_caret(self.line, usize::MAX);
    }

    /// Puts the caret at the char boundary nearest `(x, y)`, measured from
    /// the top left of the first line's first cell on a grid of
    /// `cell_w` x `row_h` cells. Points above or left of the text land on
    /// its first line or column, points below it on its last line.
    pub fn click(&mut self, x: f32, y: f32, cell_w: f32, row_h: f32) {
        // `as` saturates, and NaN (0 / 0, a NaN point) becomes 0.
        let (row, col) = ((y / row_h).max(0.0), (x / cell_w).round().max(0.0));
        self.set_caret(row as usize, col as usize);
    }
}
