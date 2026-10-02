//! The text buffer behind `ui`'s code editor: the text and one caret. Pure data.

/// A text with one caret. Columns count chars, so they match a monospace
/// grid. Inserted text keeps `\n`, turns a tab into two spaces and drops
/// other control chars (`\r` too).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Editor {
    text: String,
    /// The caret's byte offset.
    at: usize,
    /// The column up and down aim for, kept across short lines.
    goal: usize,
}

impl Editor {
    /// A buffer holding `text`, the caret at its start.
    pub fn new(text: &str) -> Editor {
        let mut e = Editor::default();
        e.insert(text);
        e.set_caret(0, 0);
        e
    }

    pub fn text(&self) -> String {
        self.text.clone()
    }

    /// The length of the text in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The lines, without their `\n`: always at least one.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }

    pub fn line_count(&self) -> usize {
        self.lines().count()
    }

    /// The caret as (line, column), both from 0.
    pub fn caret(&self) -> (usize, usize) {
        let mut before = self.halves().0.split('\n');
        let col = before.next_back().map_or(0, |l| l.chars().count());
        (before.count(), col)
    }

    /// Puts the caret at (line, column), clamped into the text.
    pub fn set_caret(&mut self, line: usize, col: usize) {
        self.goal = col;
        self.move_to_line(line);
        self.goal = self.caret().1;
    }

    /// Moves to `line` (clamped), at the goal column or the line's end.
    pub fn move_to_line(&mut self, line: usize) {
        let line = line.min(self.line_count() - 1);
        let start: usize = self.lines().take(line).map(|l| l.len() + 1).sum();
        let l = self.text.get(start..).unwrap_or_default().split('\n').next().unwrap_or_default();
        self.at = start + l.char_indices().nth(self.goal).map_or(l.len(), |(i, _)| i);
    }

    /// Inserts `text` at the caret and puts the caret after it.
    pub fn insert(&mut self, text: &str) {
        let (mut clean, mut buf) = (String::with_capacity(text.len()), [0; 4]);
        for c in text.chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()) {
            clean.push_str(if c == '\t' { "  " } else { c.encode_utf8(&mut buf) });
        }
        self.text.insert_str(self.at, &clean);
        self.at += clean.len();
        self.goal = self.caret().1;
    }

    /// Enter: breaks the line, the new one indented like the current line
    /// (its leading spaces up to the caret).
    pub fn newline(&mut self) {
        let line = self.halves().0.split('\n').next_back().unwrap_or_default();
        let indent = line.bytes().take_while(|&b| b == b' ').count();
        self.insert(&["\n", &" ".repeat(indent)].concat());
    }

    /// The text before the caret and after it.
    fn halves(&self) -> (&str, &str) {
        self.text.split_at_checked(self.at).unwrap_or_default()
    }

    /// The char before the caret (`back`) or after it.
    fn next(&self, back: bool) -> Option<char> {
        match back {
            true => self.halves().0.chars().next_back(),
            false => self.halves().1.chars().next(),
        }
    }

    /// Moves the caret one char back (Left) or forward (Right), across lines.
    pub fn step(&mut self, back: bool) {
        let n = self.next(back).map_or(0, char::len_utf8);
        self.at = if back { self.at - n } else { self.at + n };
        self.goal = self.caret().1;
    }

    /// Deletes the char before the caret (`back`, Backspace) or after it
    /// (Delete); deleting a `\n` joins two lines.
    pub fn delete(&mut self, back: bool) {
        if let Some(c) = self.next(back) {
            self.at -= if back { c.len_utf8() } else { 0 };
            self.text.drain(self.at..self.at + c.len_utf8());
        }
        self.goal = self.caret().1;
    }
}
