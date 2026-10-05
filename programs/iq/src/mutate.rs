//! Mutants: programs one slip away from a reference ([`mutants`]: a line dropped, a number made
//! one more), and whether a person poking at a mutant could tell it from the reference at all
//! ([`differs`]). A mutant no exploration tells apart (a starting value the app overwrites, a
//! loop bound that changes nothing) is no slip, so it counts neither for nor against a check.

use applang::Node;
use makes::probe::{Board, Probe, walk};

/// The most mutants of a reference graded, spread evenly over all those made.
pub const MAX_MUTANTS: usize = 40;
/// The rounds of an exploration: each clicks every button, presses every key, taps, types and
/// lets time pass.
const ROUNDS: usize = 3;
/// The keys an exploration presses besides the named ones.
const CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// A program one slip away from a reference, and what the slip was (`line 7 dropped: ...`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mutant {
    pub src: String,
    pub what: String,
}

/// Programs one slip away from `src`: each line of code whose brackets close on it dropped, and
/// each number made one more, in source order, distinct and unlike `src`; at most
/// [`MAX_MUTANTS`], spread evenly over all of them.
pub fn mutants(src: &str) -> Vec<Mutant> {
    let mut all: Vec<Mutant> = Vec::new();
    let mut add = |m: String, what: String| {
        if m != src && !all.iter().any(|a| a.src == m) {
            all.push(Mutant { src: m, what });
        }
    };
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    for (i, line) in lines.iter().enumerate().filter(|l| dropped(l.1)) {
        let m = lines.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, l)| *l).collect();
        add(m, format!("line {} dropped: {}", i + 1, line.trim()));
    }
    for (span, class) in applang::highlight(src) {
        let word = src.get(span.start..span.end).unwrap_or("");
        let n = word.parse::<i64>().ok().filter(|_| class == applang::Class::Number);
        if let Some(n) = n {
            let to = n.saturating_add(1).to_string();
            let m = [&src[..span.start], &to, &src[span.end..]].concat();
            let (no, col) = coder::ai::line_col(src, span.start);
            let line = lines.get(no - 1).map_or("", |l| l.trim());
            add(m, format!("line {no}:{col}, {word} made {to}: {line}"));
        }
    }
    if all.len() <= MAX_MUTANTS {
        return all;
    }
    (0..MAX_MUTANTS).map(|i| all[i * all.len() / MAX_MUTANTS].clone()).collect()
}

/// Whether `line` may be dropped: it is code (not blank, not only a comment) and its brackets
/// close on it (a widget, a statement, a state, a one-line function).
fn dropped(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.starts_with("//") || t.starts_with("/*") {
        return false;
    }
    let mut depth = 0i32;
    for b in line.split("//").next().unwrap_or("").bytes() {
        depth += match b {
            b'(' | b'[' | b'{' => 1,
            b')' | b']' | b'}' => -1,
            _ => 0,
        };
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// One event of an exploration.
enum Poke {
    Click(String),
    Key(&'static str),
    Tap(i64, i64),
    Square(Board, i64, i64),
    Type(usize),
    Wait(u64),
}

/// The events of a round, from what `p` shows (never from a check: what tells a mutant apart is
/// the same whatever the script): each button clicked, every key pressed, the first canvas
/// tapped at nine points, each grid tapped at its corners and middle, each input typed into, and
/// time let pass.
fn pokes(p: &Probe) -> Vec<Poke> {
    let mut out = Vec::new();
    let (mut grids, mut inputs) = (Vec::new(), 0);
    walk(p.nodes(), &mut |n| match n {
        Node::Button { text, .. } => out.push(Poke::Click(text.clone())),
        Node::Grid { id, cols, cells, .. } => {
            grids.push((id.is_some(), i64::from(*cols), cells.len() as i64))
        }
        Node::Input { .. } => inputs += 1,
        _ => {}
    });
    out.extend(applang_syntax::KEYS.iter().map(|k| Poke::Key(k)));
    out.extend((0..CHARS.len()).filter_map(|i| CHARS.get(i..=i)).map(Poke::Key));
    if let Some(c) = p.canvas().filter(|c| c.id.is_some()) {
        for (i, j) in (1..=3).flat_map(|i| (1..=3).map(move |j| (i, j))) {
            out.push(Poke::Tap(c.w * i / 4, c.h * j / 4));
        }
    }
    for (nth, &(_, cols, n)) in grids.iter().enumerate().filter(|g| g.1.0 && g.1.1 > 0) {
        let (b, rows) = (Board::Grid { nth, cols }, (n / cols).max(1));
        for (c, r) in [(0, 0), (cols / 2, rows / 2), (cols - 1, rows - 1)] {
            out.push(Poke::Square(b, c, r));
        }
    }
    out.extend((0..inputs).map(Poke::Type));
    out.extend([Poke::Wait(100), Poke::Wait(1000)]);
    out
}

/// Does `e` to `p`: whether it could be done.
fn poke(p: &mut Probe, e: &Poke, round: usize) -> bool {
    match e {
        Poke::Click(label) => p.click(label),
        Poke::Key(k) => p.key(k),
        Poke::Tap(x, y) => p.tap(*x, *y),
        Poke::Square(b, c, r) => p.tap_cell(*b, *c, *r),
        Poke::Type(n) => p.type_in(*n, ["12", "hello", ""][round % 3]),
        Poke::Wait(ms) => p.wait(*ms),
    }
    .is_ok()
}

/// Whether `a` and `b` can be told apart by exploring them in step, on seeds 1 to 3: whether
/// either one starts while the other does not, an event can be done to one and not the other,
/// or they ever show differently (texts, buttons, inputs, grids, canvases).
pub fn differs(a: &str, b: &str) -> bool {
    for seed in crate::script::SEEDS {
        let (mut p, mut q) = match (Probe::start(a, seed), Probe::start(b, seed)) {
            (Ok(p), Ok(q)) => (p, q),
            (Err(_), Err(_)) => continue,
            _ => return true,
        };
        if crate::script::view(&p) != crate::script::view(&q) {
            return true;
        }
        for round in 0..ROUNDS {
            for e in pokes(&p) {
                if poke(&mut p, &e, round) != poke(&mut q, &e, round)
                    || crate::script::view(&p) != crate::script::view(&q)
                {
                    return true;
                }
            }
        }
    }
    false
}
