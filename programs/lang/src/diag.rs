//! Byte-offset [`Span`]s, coded [`Diag`]s and caret-snippet rendering.

/// A byte-offset range in the source text, `start` inclusive, `end` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// A span; `end < start` is normalized to empty at `start`.
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end: end.max(start) }
    }
}

/// A diagnostic: a message, an optional span and an optional stable code.
/// `Display` writes `E{code:04}: message [start..end]`.
#[derive(Debug, Clone)]
pub struct Diag {
    pub message: String,
    pub span: Option<Span>,
    /// Stable registry code, banded per stage so agents and tests can assert on it.
    pub code: Option<u16>,
}

impl Diag {
    /// A coded diagnostic pinned to `span`.
    pub fn at_code(code: u16, message: impl Into<String>, span: Span) -> Self {
        Self { message: message.into(), span: Some(span), code: Some(code) }
    }

    /// A coded diagnostic with no span.
    pub fn new_code(code: u16, message: impl Into<String>) -> Self {
        Self { message: message.into(), span: None, code: Some(code) }
    }

    /// `Display`, then the offending line with a caret row when there is a span.
    pub fn render(&self, source: &str) -> String {
        match self.span.and_then(|s| render_snippet(source, s)) {
            Some(snippet) => format!("{self}\n{snippet}"),
            None => self.to_string(),
        }
    }
}

impl std::fmt::Display for Diag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = self.code {
            write!(f, "E{code:04}: ")?;
        }
        match self.span {
            Some(span) => write!(f, "{} [{}..{}]", self.message, span.start, span.end),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for Diag {}

/// 1-based `(line, column)` of a byte offset, the column in chars. Offsets
/// past the end clamp; one inside a multi-byte char floors to its start.
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let (mut line, mut col) = (1, 1);
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            (line, col) = (line + 1, 1);
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The largest char boundary `<= i`, clamped to `s.len()`.
pub(crate) fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `line N, col M`, the span's first line and a caret row under the span
/// (at least one `^`, clamped to that line; tabs shown as one space).
/// `None` only for an empty `source`.
pub fn render_snippet(source: &str, span: Span) -> Option<String> {
    if source.is_empty() {
        return None;
    }
    let start = floor_boundary(source, span.start);
    let (line, col) = line_col(source, start);
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[line_start..].find('\n').map_or(source.len(), |i| line_start + i);
    let line_text: String =
        source[line_start..line_end].chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
    let span_end = floor_boundary(source, span.end.clamp(start, line_end.max(start)));
    let width = source[start..span_end].chars().count().max(1);
    let line_chars = line_text.chars().count();
    let pad = (col - 1).min(line_chars);
    let carets = width.min((line_chars + 1).saturating_sub(pad)).max(1);
    Some(format!(
        "line {line}, col {col}\n  {line_text}\n  {}{}",
        " ".repeat(pad),
        "^".repeat(carets)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based_and_clamped() {
        let src = "ab\ncde\nf";
        let got: Vec<_> = [0, 3, 5, 7, 999].iter().map(|&o| line_col(src, o)).collect();
        assert_eq!(got, [(1, 1), (2, 1), (2, 3), (3, 1), (3, 2)]);
    }

    #[test]
    fn snippets_put_carets_under_the_span() {
        let snip = |src, s, e| render_snippet(src, Span::new(s, e));
        let src = "let a = 1;\nlet x = true + 1;";
        let want = "line 2, col 9\n  let x = true + 1;\n          ^^^^^^^^";
        assert_eq!(snip(src, 19, 27).unwrap(), want);
        // Zero-width at the end still gets one caret; an empty source none.
        assert!(snip("abc", 3, 3).unwrap().ends_with('^'));
        assert!(snip("", 0, 0).is_none());
        // A span starting inside the em-dash must not panic.
        assert!(snip("a — b", 3, 4).unwrap().contains("a — b"));
        // Tabs widen to one space so the carets stay aligned.
        assert_eq!(snip("\tlet x = 1;", 1, 4).unwrap(), "line 1, col 2\n   let x = 1;\n   ^^^");
    }

    #[test]
    fn display_and_render() {
        let d = Diag::at_code(204, "type mismatch", Span::new(12, 18));
        assert_eq!(d.to_string(), "E0204: type mismatch [12..18]");
        let plain = Diag { message: "boom".into(), span: None, code: None };
        assert_eq!(plain.to_string(), "boom");
        let r = Diag::at_code(1, "bad", Span::new(0, 3)).render("abc");
        assert!(r.starts_with("E0001: bad [0..3]\nline 1, col 1"), "{r}");
        assert_eq!(Diag::new_code(7, "no span").render("abc"), "E0007: no span");
    }
}
