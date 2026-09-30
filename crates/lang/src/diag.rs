//! Diagnostics kernel: byte-offset [`Span`]s, coded [`Diag`]s, and caret-snippet
//! rendering. Zero dependencies, native + wasm32.
//!
//! Lineage: hoisted from `localharness::rustlite` (where `soliditylite` already
//! consumed it verbatim — the existence proof that this kernel is language-neutral).

/// A byte-offset range in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start byte offset (inclusive).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
}

impl Span {
    /// Construct a span; `end < start` is normalized to empty at `start`.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end: end.max(start),
        }
    }
}

/// A diagnostic: message + optional source span + optional stable numeric code.
///
/// `Display` prefixes `E{code:04}:` when a code is present. A language wanting
/// its own label space (e.g. `LH0204`) formats the code itself instead of using
/// the default `Display`.
#[derive(Debug, Clone)]
pub struct Diag {
    /// Human-readable description.
    pub message: String,
    /// Source location, if available.
    pub span: Option<Span>,
    /// Stable registry code. Keep codes per-stage-banded (lex 0xx, parse 1xx, …)
    /// so agents and tests can assert on them.
    pub code: Option<u16>,
}

impl Diag {
    /// Error with no span and no code.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
            code: None,
        }
    }
    /// Error pinned to a span (no code).
    pub fn at(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
            code: None,
        }
    }
    /// Coded error pinned to a span — the canonical constructor.
    pub fn at_code(code: u16, message: impl Into<String>, span: Span) -> Self {
        Self::at(message, span).with_code(code)
    }
    /// Coded error with no span.
    pub fn new_code(code: u16, message: impl Into<String>) -> Self {
        Self::new(message).with_code(code)
    }
    /// Attach (or replace) the stable code.
    pub fn with_code(mut self, code: u16) -> Self {
        self.code = Some(code);
        self
    }

    /// `"line N, col M"` of this diag's span in `source`, when it has one.
    pub fn location(&self, source: &str) -> Option<String> {
        let span = self.span?;
        let (line, col) = line_col(source, span.start.min(source.len()));
        Some(format!("line {line}, col {col}"))
    }

    /// Full rendering: `Display` form plus, when a span is present, the
    /// offending line with a caret marker. Prefer this over `to_string()` on
    /// every surface that has the source at hand — a byte offset alone makes
    /// the reader (human or agent) hunt.
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
        if let Some(span) = self.span {
            write!(f, "{} [{}..{}]", self.message, span.start, span.end)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for Diag {}

impl From<String> for Diag {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// 1-based `(line, column)` of a byte offset in `source`.
///
/// The column counts CHARACTERS from the start of the line (so a caret row of
/// single-width spaces lines up). Offsets past the end clamp; an offset inside
/// a multi-byte char floors to that char's start.
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Largest char boundary `<= i`, clamped to `s.len()`. A span offset can land
/// INSIDE a multi-byte char (e.g. an em-dash in a string literal) and slicing
/// there panics; `str::floor_char_boundary` is still unstable, so roll the
/// two-liner. (Without this, one non-ASCII source byte turned a clean Diag
/// into a compiler PANIC — caught by the rustlite cartridge corpus.)
fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Render `line N, col M` + offending line + caret row for `span`:
///
/// ```text
/// line 2, col 11
///   let x = true + 1;
///           ^^^^^^^^
/// ```
///
/// The caret row underlines the span where it intersects its FIRST line
/// (multi-line spans clamp to that line; a zero-width or line-end span still
/// gets one `^`). Tabs widen to a single space so the caret row stays aligned.
/// Returns `None` only when `source` is empty.
pub fn render_snippet(source: &str, span: Span) -> Option<String> {
    if source.is_empty() {
        return None;
    }
    let start = floor_char_boundary(source, span.start);
    let (line, col) = line_col(source, start);
    let line_start = source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = source[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(source.len());
    let line_text: String = source[line_start..line_end]
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .collect();
    let span_end = floor_char_boundary(source, span.end.clamp(start, line_end.max(start)));
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
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 3), (2, 1));
        assert_eq!(line_col(src, 5), (2, 3));
        assert_eq!(line_col(src, 7), (3, 1));
        assert_eq!(line_col(src, 999), (3, 2)); // clamps to end
    }

    #[test]
    fn snippet_carets_cover_the_span() {
        let src = "let a = 1;\nlet x = true + 1;";
        let s = render_snippet(src, Span::new(19, 27)).unwrap();
        assert_eq!(s, "line 2, col 9\n  let x = true + 1;\n          ^^^^^^^^");
    }

    #[test]
    fn snippet_zero_width_and_line_end_get_one_caret() {
        let src = "abc";
        let s = render_snippet(src, Span::new(3, 3)).unwrap();
        assert!(s.ends_with('^'), "{s}");
        assert!(render_snippet("", Span::new(0, 0)).is_none());
    }

    #[test]
    fn snippet_survives_mid_char_offsets() {
        // Span start landing inside the em-dash's UTF-8 bytes must not panic.
        let src = "a — b";
        let s = render_snippet(src, Span::new(3, 4)).unwrap();
        assert!(s.contains("a — b"), "{s}");
    }

    #[test]
    fn snippet_widens_tabs_to_keep_carets_aligned() {
        let src = "\tlet x = 1;";
        let s = render_snippet(src, Span::new(1, 4)).unwrap();
        assert_eq!(s, "line 1, col 2\n   let x = 1;\n   ^^^");
    }

    #[test]
    fn display_prefixes_code_and_span() {
        let d = Diag::at_code(204, "type mismatch", Span::new(12, 18));
        assert_eq!(d.to_string(), "E0204: type mismatch [12..18]");
        assert_eq!(Diag::new("boom").to_string(), "boom");
        assert_eq!(
            Diag::at("x", Span::new(19, 20))
                .location("let a = 1;\nlet x = 1;")
                .unwrap(),
            "line 2, col 9"
        );
    }

    #[test]
    fn render_combines_display_and_snippet() {
        let d = Diag::at_code(1, "bad", Span::new(0, 3));
        let r = d.render("abc");
        assert!(r.starts_with("E0001: bad [0..3]\nline 1, col 1"), "{r}");
        assert_eq!(Diag::new_code(7, "no span").render("abc"), "E0007: no span");
    }
}
