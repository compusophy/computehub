//! The text buffer behind Studio's editor: lines and one caret. Pure data.

/// A multi-line buffer with one caret. Columns count chars, so they match a
/// monospace grid. Inserted text keeps `\n`, turns a tab into two spaces and
/// drops other control chars (`\r` too). There is always at least one line.
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

fn chars(s: &str) -> usize {
    s.chars().count()
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
        self.lines.join("\n")
    }

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
        let mut clean = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '\t' => clean.push_str("  "),
                c if c == '\n' || !c.is_control() => clean.push(c),
                _ => {}
            }
        }
        let cur = &mut self.lines[self.line];
        let tail = cur.split_off(byte(cur, self.col));
        let mut parts = clean.split('\n');
        cur.push_str(parts.next().unwrap_or_default());
        if clean.contains('\n') {
            // The new lines go between this one and the ones after it.
            let after = self.lines.split_off(self.line + 1);
            self.lines.extend(parts.map(String::from));
            self.line = self.lines.len() - 1;
            self.lines.extend(after);
        }
        let last = &mut self.lines[self.line];
        self.col = chars(last);
        last.push_str(&tail);
        self.goal = self.col;
    }

    /// Enter: breaks the line, the new one indented like the current line
    /// (its leading spaces up to the caret).
    pub fn newline(&mut self) {
        let indent = self.lines[self.line].chars().take_while(|&c| c == ' ').count();
        self.insert(&["\n", &" ".repeat(indent.min(self.col))].concat());
    }

    /// Deletes the char before the caret, or joins the line to the one above.
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

    /// Deletes the char after the caret, or joins the next line to this one.
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

    /// One char left, wrapping to the end of the line above.
    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.col = chars(&self.lines[self.line]);
        }
        self.goal = self.col;
    }

    /// One char right, wrapping to the start of the next line.
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
            false => self.end(),
        }
    }

    pub fn home(&mut self) {
        self.set_caret(self.line, 0);
    }

    pub fn end(&mut self) {
        self.set_caret(self.line, usize::MAX);
    }

    /// Puts the caret at the char boundary nearest `(x, y)`, measured from
    /// the first cell's top left on a `cell_w` x `row_h` grid; points
    /// outside the text land on its nearest line and column.
    pub fn click(&mut self, x: f32, y: f32, cell_w: f32, row_h: f32) {
        // `as` saturates, and NaN (0 / 0, a NaN point) becomes 0.
        let (row, col) = ((y / row_h).max(0.0), (x / cell_w).round().max(0.0));
        self.set_caret(row as usize, col as usize);
    }
}
