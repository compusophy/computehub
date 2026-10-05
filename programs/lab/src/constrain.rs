//! Constrained decoding: a writer draws only tokens that keep its program possible. Three rules,
//! each measured ([`Rule`]):
//!
//! - free: any token (the first one healing the prompt, as always);
//! - lexer: the program stays lexically valid by applang's lexer, followed a byte at a time
//!   ([`Lex`], the rules of `applang-syntax`'s `lex.rs`: strings closed on their line with only
//!   `\"`, `\\` and `\n` escapes, no character the language lacks, no letter run into a number,
//!   `.`, `&` and `|` only doubled) and its brackets matched; it ends only where the lexer ends
//!   clean, every bracket closed;
//! - parser: the lexer's rule, and applang's own parser finds no error before the end of what
//!   is written so far (the last token, while it may still grow, judged by what it can become);
//!   it ends only where the program parses. A token the parser refuses is drawn again without
//!   it, [`TRIES`] times at most; then the last one drawn stands.
//!
//! Neither rule looks at names or types (the checker's work) or runs anything.

use applang::{Span, codes};
use tiny::{EOS, Tokenizer};

/// How a writer's tokens are constrained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Free,
    Lexer,
    Parser,
}

impl Rule {
    pub const ALL: [Rule; 3] = [Rule::Free, Rule::Lexer, Rule::Parser];

    /// What the results call it, after a writer's name.
    pub fn suffix(self) -> &'static str {
        match self {
            Rule::Free => "",
            Rule::Lexer => ", lexer-constrained",
            Rule::Parser => ", parser-constrained",
        }
    }
}

/// Draws the parser rule makes before the last one drawn stands.
pub const TRIES: usize = 32;

/// Where the lexer is within a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Between tokens, or just past one that cannot grow.
    Code,
    /// Past a `/`: an operator, or a comment's start.
    Slash,
    Line,
    /// In a block comment `depth` deep; `prev` is a `*` or `/` that may begin `*/` or `/*`.
    Block {
        depth: u32,
        prev: u8,
    },
    Str,
    /// Past a string's `\`.
    Esc,
    /// In a number: its digits so far.
    Num(u8),
    Ident,
    /// Past a `.`, `&` or `|`: only its double may follow.
    Half(u8),
    /// Past an operator that a second character may lengthen (`=`, `!`, `<`, `>`, `+`, `-`).
    Op(u8),
}

/// The most digits a number may have: every 18-digit number fits applang's 2^63.
const DIGITS: u8 = 18;

/// The lexer's state over a program written so far: its mode, the brackets open, and where the
/// last token began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lex {
    mode: Mode,
    open: Vec<u8>,
    pos: usize,
    start: usize,
}

impl Default for Lex {
    fn default() -> Lex {
        Lex { mode: Mode::Code, open: Vec::new(), pos: 0, start: 0 }
    }
}

/// One byte `b` at `pos` read in `mode` with `open` brackets; false if it makes a lexer error
/// or closes a bracket not open.
fn step(mode: &mut Mode, open: &mut Vec<u8>, start: &mut usize, (b, pos): (u8, usize)) -> bool {
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    match *mode {
        Mode::Code => {}
        Mode::Slash => match b {
            b'/' => return set(mode, Mode::Line),
            b'*' => return set(mode, Mode::Block { depth: 1, prev: 0 }),
            _ => *mode = Mode::Code,
        },
        Mode::Line => return b != b'\n' || set(mode, Mode::Code),
        Mode::Block { depth, prev } => {
            *mode = match (prev, b) {
                (b'*', b'/') if depth == 1 => Mode::Code,
                (b'*', b'/') => Mode::Block { depth: depth - 1, prev: 0 },
                (b'/', b'*') => Mode::Block { depth: depth + 1, prev: 0 },
                (_, b'*' | b'/') => Mode::Block { depth, prev: b },
                _ => Mode::Block { depth, prev: 0 },
            };
            return true;
        }
        Mode::Str => {
            return match b {
                b'"' => set(mode, Mode::Code),
                b'\\' => set(mode, Mode::Esc),
                b'\n' => false,
                _ => true,
            };
        }
        Mode::Esc => return matches!(b, b'"' | b'\\' | b'n') && set(mode, Mode::Str),
        Mode::Num(n) => match b {
            b'0'..=b'9' if n < DIGITS => return set(mode, Mode::Num(n + 1)),
            b'_' => return true,
            _ if ident(b) => return false,
            _ => *mode = Mode::Code,
        },
        Mode::Ident if ident(b) => return true,
        // `==`, `!=`, `<=`, `>=`, `+=`, `-=`, `->`: one token.
        Mode::Op(_) if b == b'=' => return set(mode, Mode::Code),
        Mode::Op(b'-') if b == b'>' => return set(mode, Mode::Code),
        Mode::Ident | Mode::Op(_) => *mode = Mode::Code,
        Mode::Half(h) => return b == h && set(mode, Mode::Code),
    }
    // A new token, or space.
    if !matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
        *start = pos;
    }
    *mode = match b {
        b' ' | b'\t' | b'\n' | b'\r' => Mode::Code,
        b'/' => Mode::Slash,
        b'"' => Mode::Str,
        b'0'..=b'9' => Mode::Num(1),
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => Mode::Ident,
        b'.' | b'&' | b'|' => Mode::Half(b),
        b'=' | b'!' | b'<' | b'>' | b'+' | b'-' => Mode::Op(b),
        b'*' | b'%' | b';' | b',' | b':' => Mode::Code,
        b'(' | b'[' | b'{' => {
            open.push(b);
            Mode::Code
        }
        b')' | b']' | b'}' => {
            let want = match b {
                b')' => b'(',
                b']' => b'[',
                _ => b'{',
            };
            if open.pop() != Some(want) {
                return false;
            }
            Mode::Code
        }
        _ => return false,
    };
    true
}

fn set(mode: &mut Mode, to: Mode) -> bool {
    *mode = to;
    true
}

impl Lex {
    /// Reads `bytes`; false if one makes a lexer error or closes a bracket not open (the state
    /// is then past use).
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        let (mode, open, start) = (&mut self.mode, &mut self.open, &mut self.start);
        bytes.iter().all(|&b| {
            self.pos += 1;
            step(mode, open, start, (b, self.pos - 1))
        })
    }

    /// Whether `bytes` may follow, the state unchanged (`scratch` is reused for the brackets).
    pub fn allows(&self, bytes: &[u8], scratch: &mut Vec<u8>) -> bool {
        let (mut mode, mut start) = (self.mode, self.start);
        scratch.clear();
        scratch.extend_from_slice(&self.open);
        let at = self.pos;
        bytes.iter().enumerate().all(|(i, &b)| step(&mut mode, scratch, &mut start, (b, at + i)))
    }

    /// Whether the program may end here: the lexer ends clean and every bracket is closed.
    pub fn can_end(&self) -> bool {
        self.open.is_empty()
            && matches!(
                self.mode,
                Mode::Code | Mode::Line | Mode::Num(_) | Mode::Ident | Mode::Op(_) | Mode::Slash
            )
    }
}

/// The words a name may still grow into that the parser reads apart from names.
pub const WORDS: [&str; 25] = [
    "state", "label", "button", "input", "row", "col", "let", "if", "else", "repeat", "true",
    "false", "fn", "for", "in", "return", "saved", "every", "on", "key", "grid", "canvas", "int",
    "bool", "string",
];

/// Whether `text`, lexed to `lex`, may still become a program by applang's parser: no lexer or
/// parser error before the end of what is whole. A last token that may grow is judged by what
/// it can become: a string as any string, a name as a name unless it begins one of [`WORDS`]
/// (then not yet), a number as a number, an operator as each it can still be, a comment not
/// yet. With `end`, the text must parse whole.
pub fn parses(text: &str, lex: &Lex, end: bool) -> bool {
    let start = lex.start.min(text.len());
    let whole = |more: &str| ([text, more, " "].concat(), text.len() + more.len());
    let probes = match (end, lex.mode) {
        (true, _) | (_, Mode::Code | Mode::Line) => vec![(text.to_string(), text.len())],
        (_, Mode::Str | Mode::Esc) => vec![([&text[..start], "\"\""].concat(), start + 2)],
        (_, Mode::Ident) if WORDS.iter().any(|w| w.starts_with(&text[start..])) => {
            vec![(text[..start].to_string(), start)]
        }
        (_, Mode::Ident | Mode::Num(_)) => vec![whole("")],
        (_, Mode::Half(c)) => vec![whole(&char::from(c).to_string())],
        (_, Mode::Op(c)) => ["", "=", if c == b'-' { ">" } else { "=" }].map(whole).to_vec(),
        (_, Mode::Slash | Mode::Block { .. }) => vec![(text[..start].to_string(), start)],
    };
    probes.iter().any(|(probe, cut)| viable(probe, *cut, end))
}

/// Whether applang's parser finds no error in `probe` before `cut` (with `end`, none at all).
fn viable(probe: &str, cut: usize, end: bool) -> bool {
    match applang::compile(probe) {
        Ok(_) => true,
        // The checker's errors (names, types) are not the parser's; nor is a fault at run.
        Err(d) if d.code.is_some_and(|c| c >= codes::DUP_STATE) => true,
        // The one error the parser pins on a whole expression: a canvas's scene that is not a
        // call. A name with nothing after it may still be called.
        Err(d) => {
            let called = |s: Span| {
                d.message.starts_with("expected a call") && probe[s.end..].trim().is_empty()
            };
            !end && d.span.is_some_and(|s| s.start >= cut || called(s))
        }
    }
}

/// A program being written under a [`Rule`]: its text and lexer, and the tokens' bytes.
#[derive(Debug, Clone)]
pub struct Guide<'t> {
    pub rule: Rule,
    tok: &'t Tokenizer,
    text: Vec<u8>,
    lex: Lex,
    /// Whether the lexer has failed: from then on (the free rule only) nothing is checked.
    broken: bool,
    scratch: Vec<u8>,
}

impl<'t> Guide<'t> {
    /// A program that begins with `text` (a prompt), written under `rule`.
    pub fn new(tok: &'t Tokenizer, rule: Rule, text: &str) -> Guide<'t> {
        let mut lex = Lex::default();
        let broken = !lex.feed(text.as_bytes());
        let text = text.as_bytes().to_vec();
        Guide { rule, tok, text, lex, broken, scratch: Vec::new() }
    }

    /// Which tokens the lexer's rule lets come next (all, under the free rule): a token whose
    /// bytes keep it valid, the end token where it may end.
    pub fn mask(&mut self) -> Vec<bool> {
        let n = self.tok.vocab() as u32;
        if self.rule == Rule::Free || self.broken {
            return vec![true; n as usize];
        }
        (0..n)
            .map(|t| match t {
                EOS => self.lex.can_end(),
                _ => self.lex.allows(self.tok.piece(t), &mut self.scratch),
            })
            .collect()
    }

    /// Whether the parser's rule lets `t` come next (any token, under the other rules); `t`
    /// is one [`Guide::mask`] lets through.
    pub fn parses(&self, t: u32) -> bool {
        if self.rule != Rule::Parser || self.broken {
            return true;
        }
        let mut lex = self.lex.clone();
        let piece = self.tok.piece(t);
        let mut bytes = self.text.clone();
        bytes.extend_from_slice(piece);
        let Ok(text) = String::from_utf8(bytes) else {
            // A character cut between tokens: only inside a string or comment (the lexer
            // allows no other), so nothing the parser reads changed.
            return true;
        };
        lex.feed(piece);
        parses(&text, &lex, t == EOS)
    }

    /// Writes `t`.
    pub fn push(&mut self, t: u32) {
        let piece = self.tok.piece(t);
        self.text.extend_from_slice(piece);
        if !self.broken && !self.lex.feed(piece) {
            self.broken = true;
        }
    }
}
