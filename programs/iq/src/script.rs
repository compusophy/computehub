//! The checker language: a task's check is a script, data rather than code, that drives a made
//! app headlessly through [`makes::probe::Probe`] as a person would (clicks, keys, typing, taps,
//! time) and says what must show. [`parse`] reads it into a [`Script`] (a coded [`Coded`] if it
//! does not read, naming the line); [`check`] runs it on seeds 1, 2 and 3, each from a fresh
//! start, and every expectation must hold on all three. [`CARD`] is what a teacher writing checks
//! is prompted with; a test holds it to [`parse`], so the card and the language cannot drift.

use applang::Node;
use makes::probe::{Board, Probe, walk};

use crate::{Coded, codes};

/// The card a model writing checks is prompted with: every statement, each with an example. The
/// lines that begin with two spaces are a script that parses (a test runs them).
pub const CARD: &str = "\
iq checks: a script that drives an app as a person would and says what must show. One
statement a line; # starts a comment; strings are in double quotes (\\\" and \\\\ inside). The
script runs from the app's start on seeds 1, 2 and 3: random differs between seeds, and
between any two programs, so never expect what chance decides. Every expect must hold on all
three. Labels and phrases match case aside; \"A|B\" is either; a button is found by its whole
label, else by a whole word of it (Start finds Start game).
ACTIONS (one fails the check if what it needs does not show, or if the app faults):
  start                         # clicks Start, New game or Play if one shows, else does nothing
  click \"+\"                     # clicks the button labelled +
  key \"left\"                    # left right up down space enter escape, a to z, 0 to 9
  press \"Drop\" \"space\"          # clicks Drop if it shows, else presses the key space
  type 1 \"200\"                  # the 1st text input now holds 200, as if typed
  wait 2500                     # 2500 ms pass by the app's own timer (none runs: none passes)
  tap 80 60                     # taps the first canvas at 80, 60, in its units
  tapcell 2 0 3x3               # taps the square at column 2, row 0 (from 0) of the 3 x 3 board
  repeat 3: click \"+\"           # one action, 3 times (500 at most)
CHECKS (what shows: its labels and the texts drawn on its canvases and grids, never a
button's label nor what an input holds):
  expect has \"Start\"            # a button labelled Start shows
  expect not has \"Start\"        # none does
  expect says \"You win\"         # some text holds the phrase
  expect not says \"Game over\"   # none does
  expect after \"Score\" = 1      # the first number after Score, in the first text holding it
  expect number = 230           # some number shown is 230 (whole: 2.5 reads as 2 and 5)
  expect not number > 4         # no number shown is over 4
  expect color 5 5 32x32 = 4    # the square at column 5, row 5 of the 32 x 32 board is blue
  expect squares 2 10x10 >= 3   # 3 or more squares of the 10 x 10 board are green
  mark                          # remembers what shows: texts, buttons, inputs, canvases, grids
  expect changed                # what shows differs from what the last mark saw
  expect same                   # what shows is what the last mark saw
OP is one of = != < > <= >=. A board is found the first time a script names its size, and
kept: a grid widget of that shape, else on the first canvas pixels of that shape, a lattice of
equal rects or circles, or the whole canvas cut into that many squares. A square's color is a
grid's square (0 empty, 1 red 2 green 3 yellow 4 blue 5 purple 6 cyan 7 silver 8 gray), else
the color drawn at its middle (0 if nothing is; 9 ink 10 dim ink 11 accent).
Check what the ask promises, so that every app that does what it asks passes, whatever its
design, and an app that does not fails: a check no app can fail, or that a small slip in its
program still passes, is refused. Where chance decides, let the ask fix a start a check can
play (the first game dealt in order, tiles set in given squares). At most 200 statements,
5,000 actions (a repeat counts each time) and 3,600,000 ms of waits in all.";

/// The seeds a check runs on, each from a fresh start.
pub const SEEDS: [u64; 3] = [1, 2, 3];
/// The most statements a script holds.
pub const MAX_LINES: usize = 200;
/// The most bytes a script is.
pub const MAX_BYTES: usize = 16 * 1024;
/// The most times a repeat repeats.
pub const MAX_REPEAT: i64 = 500;
/// The most actions a script takes on one seed, each time a repeat repeats counted.
pub const MAX_ACTIONS: i64 = 5_000;
/// The most ms a script lets pass on one seed, its waits together.
pub const MAX_WAIT: i64 = 3_600_000;
/// The most squares a board has across and down.
pub const MAX_SIDE: i64 = 64;
/// The most text inputs a script types into.
const MAX_INPUT: i64 = 16;
/// What `start` clicks, if one shows (Suite 1's convention).
const START: &str = "Start|New game|Play";

/// A comparison: `=`, `!=`, `<`, `>`, `<=`, `>=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl Op {
    #[rustfmt::skip]
    const ALL: [(&'static str, Op); 6] = [
        ("=", Op::Eq), ("!=", Op::Ne), ("<", Op::Lt), (">", Op::Gt), ("<=", Op::Le), (">=", Op::Ge),
    ];

    /// Whether `a OP b`.
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Op::Eq => a == b,
            Op::Ne => a != b,
            Op::Lt => a < b,
            Op::Gt => a > b,
            Op::Le => a <= b,
            Op::Ge => a >= b,
        }
    }
}

/// An action: what a person or a clock does to the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Clicks Start, New game or Play if one shows.
    Start,
    Click(String),
    Key(String),
    /// A button if one shows, else the key.
    Press(String, String),
    /// The `n`th text input (from 1) and its text.
    Type(i64, String),
    Wait(i64),
    Tap(i64, i64),
    TapCell {
        c: i64,
        r: i64,
        cols: i64,
        rows: i64,
    },
    Repeat(i64, Box<Act>),
}

/// An expectation: what must show now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    Has { not: bool, label: String },
    Says { not: bool, phrase: String },
    After { word: String, op: Op, n: i64 },
    Number { not: bool, op: Op, n: i64 },
    Color { c: i64, r: i64, cols: i64, rows: i64, op: Op, k: i64 },
    Squares { k: i64, cols: i64, rows: i64, op: Op, n: i64 },
    Changed,
    Same,
}

/// One statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Act(Act),
    Expect(Expect),
    Mark,
}

impl Stmt {
    /// Its kind, as [`KINDS`] names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Stmt::Mark => "mark",
            Stmt::Act(a) => match a {
                Act::Start => "start",
                Act::Click(_) => "click",
                Act::Key(_) => "key",
                Act::Press(..) => "press",
                Act::Type(..) => "type",
                Act::Wait(_) => "wait",
                Act::Tap(..) => "tap",
                Act::TapCell { .. } => "tapcell",
                Act::Repeat(..) => "repeat",
            },
            Stmt::Expect(e) => match e {
                Expect::Has { not: false, .. } => "expect has",
                Expect::Has { not: true, .. } => "expect not has",
                Expect::Says { not: false, .. } => "expect says",
                Expect::Says { not: true, .. } => "expect not says",
                Expect::After { .. } => "expect after",
                Expect::Number { not: false, .. } => "expect number",
                Expect::Number { not: true, .. } => "expect not number",
                Expect::Color { .. } => "expect color",
                Expect::Squares { .. } => "expect squares",
                Expect::Changed => "expect changed",
                Expect::Same => "expect same",
            },
        }
    }
}

/// Every kind of statement.
#[rustfmt::skip]
pub const KINDS: [&str; 21] = [
    "start", "click", "key", "press", "type", "wait", "tap", "tapcell", "repeat", "mark",
    "expect has", "expect not has", "expect says", "expect not says", "expect after",
    "expect number", "expect not number", "expect color", "expect squares", "expect changed",
    "expect same",
];

/// A statement and the line it is on (from 1), as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub no: usize,
    pub text: String,
    pub stmt: Stmt,
}

/// A script that reads: its statements, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub lines: Vec<Line>,
}

/// A token of a line: a word, a string or the colon after a repeat's count.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Str(String),
    Colon,
}

/// What a statement takes, said when it is given otherwise.
fn usage(word: &str) -> &'static str {
    match word {
        "start" => "start",
        "click" => "click \"Label\"",
        "key" => "key \"left\"",
        "press" => "press \"Label\" \"space\"",
        "type" => "type 1 \"text\"",
        "wait" => "wait 1000",
        "tap" => "tap 80 60",
        "tapcell" => "tapcell 2 0 3x3",
        "repeat" => "repeat 3: click \"+\"",
        "mark" => "mark",
        "has" => "expect has \"Label\" (or expect not has \"Label\")",
        "says" => "expect says \"phrase\" (or expect not says \"phrase\")",
        "after" => "expect after \"Score\" = 1",
        "number" => "expect number = 230 (or expect not number > 4)",
        "color" => "expect color 5 5 32x32 = 4",
        "squares" => "expect squares 2 10x10 >= 3",
        "changed" => "expect changed",
        "same" => "expect same",
        _ => "",
    }
}

/// Reads `src` into a [`Script`]: a statement a line, `#` comments, at least one `expect`, and
/// within the limits ([`MAX_LINES`], [`MAX_ACTIONS`], [`MAX_WAIT`]).
pub fn parse(src: &str) -> Result<Script, Coded> {
    if src.len() > MAX_BYTES {
        let why = format!("the script is {} bytes; {MAX_BYTES} at most", src.len());
        return Err(Coded::new(codes::TOO_BIG, why));
    }
    let mut lines = Vec::new();
    let (mut actions, mut waits, mut marked) = (0i64, 0i64, false);
    for (i, raw) in src.split('\n').enumerate() {
        let no = i + 1;
        let at = |(code, why): (u16, String)| Coded::new(code, format!("line {no}: {why}"));
        let toks = tokens(raw).map_err(at)?;
        if toks.is_empty() {
            continue;
        }
        let stmt = statement(&toks).map_err(at)?;
        match &stmt {
            Stmt::Act(a) => {
                let (n, ms) = cost(a);
                actions += n;
                waits += ms;
            }
            Stmt::Mark => marked = true,
            Stmt::Expect(Expect::Changed | Expect::Same) if !marked => {
                let why = "expect changed and expect same compare with a mark before them".into();
                return Err(at((codes::NO_MARK, why)));
            }
            Stmt::Expect(_) => {}
        }
        lines.push(Line { no, text: code_of(raw), stmt });
    }
    let too_big = |why: String| Err(Coded::new(codes::TOO_BIG, why));
    if lines.len() > MAX_LINES {
        return too_big(format!("{} statements; {MAX_LINES} at most", lines.len()));
    }
    if actions > MAX_ACTIONS {
        return too_big(format!("{actions} actions in all; {MAX_ACTIONS} at most"));
    }
    if waits > MAX_WAIT {
        return too_big(format!("{waits} ms of waits in all; {MAX_WAIT} at most"));
    }
    if !lines.iter().any(|l| matches!(l.stmt, Stmt::Expect(_))) {
        let why = "the script expects nothing: a check says what must show (expect ...)";
        return Err(Coded::new(codes::NO_EXPECT, why));
    }
    Ok(Script { lines })
}

/// The statement as written: `raw` up to its comment (a `#` outside a string), trimmed.
fn code_of(raw: &str) -> String {
    let (mut in_str, mut escaped) = (false, false);
    for (i, ch) in raw.char_indices() {
        match (in_str, ch) {
            (true, _) if escaped => escaped = false,
            (true, '\\') => escaped = true,
            (_, '"') => in_str = !in_str,
            (false, '#') => return raw[..i].trim().to_string(),
            _ => {}
        }
    }
    raw.trim().to_string()
}

/// The actions `a` takes and the ms it lets pass, a repeat counting each time.
fn cost(a: &Act) -> (i64, i64) {
    match a {
        Act::Repeat(n, a) => {
            let (k, ms) = cost(a);
            (n * k, n * ms)
        }
        Act::Wait(ms) => (1, *ms),
        _ => (1, 0),
    }
}

/// The tokens of a line, its comment aside.
fn tokens(line: &str) -> Result<Vec<Tok>, (u16, String)> {
    let chars: Vec<char> = line.trim_end_matches('\r').chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            break;
        } else if c == ':' {
            out.push(Tok::Colon);
            i += 1;
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err((codes::STRING, "a string has no closing \"".into())),
                    Some('"') => break,
                    Some('\\') => match chars.get(i + 1) {
                        Some(&e @ ('"' | '\\')) => {
                            s.push(e);
                            i += 1;
                        }
                        _ => {
                            let why = "the only escapes in a string are \\\" and \\\\";
                            return Err((codes::STRING, why.into()));
                        }
                    },
                    Some(&ch) => s.push(ch),
                }
                i += 1;
            }
            out.push(Tok::Str(s));
            i += 1;
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && !"\"#:".contains(chars[i]) {
                i += 1;
            }
            out.push(Tok::Word(chars[start..i].iter().collect()));
        }
    }
    Ok(out)
}

/// The arguments of a statement, read in turn; `word` names its usage.
struct Args<'a> {
    toks: &'a [Tok],
    at: usize,
    word: &'a str,
}

impl Args<'_> {
    fn bad(&self, what: &str) -> (u16, String) {
        (codes::ARGS, format!("{what}; it is written {}", usage(self.word)))
    }

    fn next(&mut self) -> Option<&Tok> {
        self.at += 1;
        self.toks.get(self.at - 1)
    }

    /// A string none of whose alternatives (split by `|`) is blank.
    fn label(&mut self, what: &str) -> Result<String, (u16, String)> {
        match self.next() {
            Some(Tok::Str(s)) if s.split('|').all(|p| !p.trim().is_empty()) => Ok(s.clone()),
            Some(Tok::Str(_)) => Err(self.bad(&format!("{what} is blank, or an alternative is"))),
            _ => Err(self.bad(&format!("{what} is missing (in double quotes)"))),
        }
    }

    /// Any string.
    fn text(&mut self) -> Result<String, (u16, String)> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(s.clone()),
            _ => Err(self.bad("the text is missing (in double quotes)")),
        }
    }

    /// A whole number from `lo` to `hi`.
    fn num(&mut self, what: &str, lo: i64, hi: i64) -> Result<i64, (u16, String)> {
        let w = match self.next() {
            Some(Tok::Word(w)) => w.clone(),
            _ => return Err(self.bad(&format!("{what} is missing"))),
        };
        let n =
            whole(&w).ok_or_else(|| self.bad(&format!("{what} `{w}` is not a whole number")))?;
        if !(lo..=hi).contains(&n) {
            let why = format!("{what} is {n}; it is {lo} to {hi}");
            return Err((codes::RANGE, why));
        }
        Ok(n)
    }

    /// A board's size, `COLSxROWS`.
    fn dims(&mut self) -> Result<(i64, i64), (u16, String)> {
        let w = match self.next() {
            Some(Tok::Word(w)) => w.clone(),
            _ => return Err(self.bad("the board's size (like 3x3) is missing")),
        };
        let (c, r) = w.split_once('x').ok_or_else(|| self.bad(&format!("`{w}` is no size")))?;
        let (Some(c), Some(r)) = (whole(c), whole(r)) else {
            return Err(self.bad(&format!("`{w}` is no size: columns x rows, like 3x3")));
        };
        if !(1..=MAX_SIDE).contains(&c) || !(1..=MAX_SIDE).contains(&r) {
            let why = format!("a board is 1 to {MAX_SIDE} squares across and down, not {w}");
            return Err((codes::RANGE, why));
        }
        Ok((c, r))
    }

    fn op(&mut self) -> Result<Op, (u16, String)> {
        let w = match self.next() {
            Some(Tok::Word(w)) => w.clone(),
            _ => return Err(self.bad("the comparison (= != < > <= >=) is missing")),
        };
        let op = Op::ALL.iter().find(|(s, _)| *s == w).map(|p| p.1);
        op.ok_or_else(|| self.bad(&format!("`{w}` is no comparison: = != < > <= >=")))
    }

    /// A square of a board: its column, row and the board's size.
    fn square(&mut self) -> Result<(i64, i64, i64, i64), (u16, String)> {
        let (c, r) =
            (self.num("the column", 0, MAX_SIDE - 1)?, self.num("the row", 0, MAX_SIDE - 1)?);
        let (cols, rows) = self.dims()?;
        if c >= cols || r >= rows {
            let why = format!("square {c}, {r} is off a board of {cols}x{rows} (from 0)");
            return Err((codes::RANGE, why));
        }
        Ok((c, r, cols, rows))
    }

    fn end(&self) -> Result<(), (u16, String)> {
        match self.toks.get(self.at) {
            None => Ok(()),
            Some(_) => Err(self.bad("it has more after its last argument")),
        }
    }
}

/// `w` as a whole number: digits, a `-` before them allowed.
fn whole(w: &str) -> Option<i64> {
    let digits = w.strip_prefix('-').unwrap_or(w);
    let ok = !digits.is_empty() && digits.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| w.parse().ok()).flatten()
}

/// The statement `toks` (not empty) say.
fn statement(toks: &[Tok]) -> Result<Stmt, (u16, String)> {
    let Some(Tok::Word(w)) = toks.first() else {
        return Err((codes::UNKNOWN, "a statement begins with its word".into()));
    };
    match w.as_str() {
        "mark" => {
            let a = Args { toks, at: 1, word: "mark" };
            a.end().map(|()| Stmt::Mark)
        }
        "expect" => expect(toks).map(Stmt::Expect),
        _ => act(toks, true).map(Stmt::Act),
    }
}

/// The action `toks` say; `repeat` only if `outer`.
fn act(toks: &[Tok], outer: bool) -> Result<Act, (u16, String)> {
    let w = match toks.first() {
        Some(Tok::Word(w)) => w.as_str(),
        _ => return Err((codes::REPEAT, "a repeat's colon is followed by one action".into())),
    };
    let mut a = Args { toks, at: 1, word: w };
    let out = match w {
        "start" => Act::Start,
        "click" => Act::Click(a.label("the label")?),
        "key" => Act::Key(key(&a.text()?)?),
        "press" => Act::Press(a.label("the label")?, key(&a.text()?)?),
        "type" => Act::Type(a.num("the input", 1, MAX_INPUT)?, a.text()?),
        "wait" => Act::Wait(a.num("the time in ms", 0, MAX_WAIT)?),
        "tap" => Act::Tap(a.num("x", 0, 1023)?, a.num("y", 0, 1023)?),
        "tapcell" => {
            let (c, r, cols, rows) = a.square()?;
            Act::TapCell { c, r, cols, rows }
        }
        "repeat" if outer => {
            let n = a.num("the count", 1, MAX_REPEAT)?;
            if a.next() != Some(&Tok::Colon) {
                return Err((codes::REPEAT, format!("a repeat is written {}", usage("repeat"))));
            }
            return Ok(Act::Repeat(n, Box::new(act(&toks[a.at..], false)?)));
        }
        "repeat" => {
            return Err((codes::REPEAT, "a repeat repeats one action, not a repeat".into()));
        }
        "expect" | "mark" => {
            return Err((codes::REPEAT, "a repeat repeats an action, not a check".into()));
        }
        _ => {
            let why = format!(
                "`{w}` is no statement: start, click, key, press, type, wait, tap, tapcell, \
                 repeat, mark or expect"
            );
            return Err((codes::UNKNOWN, why));
        }
    };
    a.end()?;
    Ok(out)
}

/// `name` if it is a key an app can handle.
fn key(name: &str) -> Result<String, (u16, String)> {
    let one = name.len() == 1 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if one || applang_syntax::KEYS.contains(&name) {
        Ok(name.to_string())
    } else {
        let why =
            format!("\"{name}\" is no key: left right up down space enter escape, a to z, 0 to 9");
        Err((codes::KEY, why))
    }
}

/// The expectation `toks` (after `expect`) say.
fn expect(toks: &[Tok]) -> Result<Expect, (u16, String)> {
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => w.as_str(),
        _ => "",
    };
    let not = word(1) == "not";
    let at = 2 + usize::from(not);
    let w = word(at - 1);
    let mut a = Args { toks, at, word: w };
    let out = match (w, not) {
        ("has", _) => Expect::Has { not, label: a.label("the label")? },
        ("says", _) => Expect::Says { not, phrase: a.label("the phrase")? },
        ("number", _) => {
            Expect::Number { not, op: a.op()?, n: a.num("the number", i64::MIN + 1, i64::MAX)? }
        }
        ("after", false) => {
            let word = a.label("the word")?;
            if word.contains('|') || word.trim() != word {
                return Err(a.bad("after takes one word, no spaces around it and no |"));
            }
            Expect::After { word, op: a.op()?, n: a.num("the number", i64::MIN + 1, i64::MAX)? }
        }
        ("color", false) => {
            let (c, r, cols, rows) = a.square()?;
            Expect::Color { c, r, cols, rows, op: a.op()?, k: a.num("the color", -1, 11)? }
        }
        ("squares", false) => {
            let k = a.num("the color", -1, 11)?;
            let (cols, rows) = a.dims()?;
            Expect::Squares {
                k,
                cols,
                rows,
                op: a.op()?,
                n: a.num("the count", 0, MAX_SIDE * MAX_SIDE)?,
            }
        }
        ("changed", false) => Expect::Changed,
        ("same", false) => Expect::Same,
        _ => {
            let why = "an expect is: has, not has, says, not says, after, number, not number, \
                       color, squares, changed or same";
            return Err((codes::UNKNOWN, why.into()));
        }
    };
    a.end()?;
    Ok(out)
}

/// A script running on one seed: the app, the boards found and the last mark.
struct Run {
    p: Probe,
    boards: Vec<((i64, i64), Board)>,
    mark: Option<u64>,
}

/// Runs `script` on `src` (which compiles) on each of [`SEEDS`], from a fresh start each: the
/// first failure, coded and naming its seed and line.
pub fn check(script: &Script, src: &str) -> Result<(), Coded> {
    SEEDS.iter().try_for_each(|&seed| run(script, src, seed))
}

/// Runs `script` on `src` with `random` seeded `seed`.
pub fn run(script: &Script, src: &str, seed: u64) -> Result<(), Coded> {
    let p = Probe::start(src, seed)
        .map_err(|e| Coded::new(codes::STARTS, format!("seed {seed}: it {e}")))?;
    let mut r = Run { p, boards: Vec::new(), mark: None };
    for l in &script.lines {
        let at = |code: u16, why: String| {
            Coded::new(code, format!("seed {seed}, line {} `{}`: {why}", l.no, l.text))
        };
        match &l.stmt {
            Stmt::Mark => r.mark = Some(view(&r.p)),
            Stmt::Act(a) => r.act(a).map_err(|e| at(codes::ACTION, e))?,
            Stmt::Expect(e) => r.expect(e).map_err(|e| at(codes::EXPECTED, e))?,
        }
    }
    Ok(())
}

impl Run {
    /// The board of `cols` x `rows`: as found the first time, else found now.
    fn board(&mut self, cols: i64, rows: i64) -> Result<Board, String> {
        if let Some(b) = self.boards.iter().find(|b| b.0 == (cols, rows)) {
            return Ok(b.1);
        }
        let b = self.p.board(cols, rows).ok_or_else(|| {
            format!("no {cols}x{rows} board: it shows no grid of that shape and no canvas")
        })?;
        self.boards.push(((cols, rows), b));
        Ok(b)
    }

    fn act(&mut self, a: &Act) -> Result<(), String> {
        match a {
            Act::Start if self.p.has(START) => self.p.click(START),
            Act::Start => Ok(()),
            Act::Click(label) => self.p.click(label),
            Act::Key(k) => self.p.key(k),
            Act::Press(label, k) => self.p.press(label, k),
            Act::Type(n, text) => self.p.type_in(usize::try_from(n - 1).unwrap_or(0), text),
            Act::Wait(ms) => self.p.wait(u64::try_from(*ms).unwrap_or(0)),
            Act::Tap(x, y) => self.p.tap(*x, *y),
            Act::TapCell { c, r, cols, rows } => {
                let b = self.board(*cols, *rows)?;
                self.p.tap_cell(b, *c, *r)
            }
            Act::Repeat(n, a) => {
                for i in 0..*n {
                    self.act(a).map_err(|e| format!("its time {} of {n}: {e}", i + 1))?;
                }
                Ok(())
            }
        }
    }

    /// `Ok` if `e` holds; else what shows instead.
    fn expect(&mut self, e: &Expect) -> Result<(), String> {
        let shown = |p: &Probe| coder::ai::clip(&format!("{:?}", p.texts()), 240);
        match e {
            Expect::Has { not, label } => match self.p.has(label) != *not {
                true => Ok(()),
                false => Err(format!("the buttons are {}", buttons(&self.p))),
            },
            Expect::Says { not, phrase } => match self.p.says(phrase) != *not {
                true => Ok(()),
                false => Err(format!("it shows {}", shown(&self.p))),
            },
            Expect::After { word, op, n } => match self.p.after(word) {
                Some(v) if op.holds(v, *n) => Ok(()),
                Some(v) => Err(format!("after {word} is {v}: it shows {}", shown(&self.p))),
                None => Err(format!("no number after {word}: it shows {}", shown(&self.p))),
            },
            Expect::Number { not, op, n } => {
                let nums = self.p.numbers();
                match nums.iter().any(|&v| op.holds(v, *n)) != *not {
                    true => Ok(()),
                    false => Err(format!("its numbers are {nums:?}: it shows {}", shown(&self.p))),
                }
            }
            Expect::Color { c, r, cols, rows, op, k } => {
                let b = self.board(*cols, *rows)?;
                let now = self.p.color(b, *c, *r);
                match op.holds(now, *k) {
                    true => Ok(()),
                    false => Err(format!("square {c}, {r} is color {now}")),
                }
            }
            Expect::Squares { k, cols, rows, op, n } => {
                let b = self.board(*cols, *rows)?;
                let all = (0..*rows).flat_map(|r| (0..*cols).map(move |c| (c, r)));
                let count = all.filter(|&(c, r)| self.p.color(b, c, r) == *k).count() as i64;
                match op.holds(count, *n) {
                    true => Ok(()),
                    false => Err(format!("{count} squares are color {k}")),
                }
            }
            Expect::Changed | Expect::Same => {
                let same = self.mark == Some(view(&self.p));
                match (same, e == &Expect::Same) {
                    (false, false) | (true, true) => Ok(()),
                    (true, false) => Err(format!("nothing changed: it shows {}", shown(&self.p))),
                    (false, true) => Err(format!("it changed: it shows {}", shown(&self.p))),
                }
            }
        }
    }
}

/// The labels of the buttons `p` shows, for a reason.
fn buttons(p: &Probe) -> String {
    let mut out = Vec::new();
    walk(p.nodes(), &mut |n| {
        if let Node::Button { text, .. } = n {
            out.push(text.as_str());
        }
    });
    coder::ai::clip(&format!("{out:?}"), 240)
}

/// A digest of all that `p` shows: its widgets as rendered (texts, buttons, inputs, grids'
/// squares and texts, canvases' shapes).
pub(crate) fn view(p: &Probe) -> u64 {
    crate::fnv(format!("{:?}", p.nodes()).as_bytes())
}
