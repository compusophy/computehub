//! The checkers of Suite 1: each drives an app made for its task ([`Probe`]) and says why it
//! fails, if it does.

use crate::probe::{Board, Probe, holds};
use applang::Shape;
use coder::ai::clip;

/// Returns `Err` with the reason formatted from the rest unless `ok`.
macro_rules! need {
    ($ok:expr, $($why:tt)+) => {
        if !$ok {
            return Err(format!($($why)+));
        }
    };
}

/// How long a checker lets an animation run (a die tumbling, a disc falling, tiles sliding)
/// before it reads the result: time passes only while the app runs a timer.
const SETTLE: u64 = 3000;

/// What `p` shows, quoted and clipped, for a reason.
fn shown(p: &Probe) -> String {
    clip(&format!("{:?}", p.texts()), 200)
}

/// The texts `p` shows that it did not show `before` (each as often as it is new).
fn fresh(before: &[String], p: &Probe) -> Vec<String> {
    let mut old = before.to_vec();
    let mut out = Vec::new();
    for t in p.texts() {
        match old.iter().position(|o| *o == t) {
            Some(i) => _ = old.remove(i),
            None => out.push(t),
        }
    }
    out
}

/// The board of `cols` x `rows` that `p` shows.
fn board(p: &Probe, cols: i64, rows: i64) -> Result<Board, String> {
    p.board(cols, rows).ok_or_else(|| format!("no {cols} x {rows} board: no grid nor canvas"))
}

/// Every square of `b`, row by row.
fn squares(cols: i64, rows: i64) -> impl Iterator<Item = (i64, i64)> {
    (0..rows).flat_map(move |r| (0..cols).map(move |c| (c, r)))
}

/// The squares of `b` that do not look like `empty`.
fn marked(p: &Probe, b: Board, cols: i64, rows: i64, empty: &str) -> Vec<(i64, i64)> {
    squares(cols, rows).filter(|&(c, r)| p.look(b, c, r) != empty).collect()
}

pub(crate) fn counter(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    need!(p.numbers().contains(&0), "it does not show 0 at first: {}", shown(&p));
    for _ in 0..3 {
        p.click("+")?;
    }
    need!(p.numbers().contains(&3), "+ three times shows {}, not 3", shown(&p));
    p.click("-|\u{2212}|\u{2013}")?;
    let n = p.numbers();
    need!(n.contains(&2) && !n.contains(&3), "then - shows {}, not 2", shown(&p));
    Ok(())
}

pub(crate) fn greeter(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    p.type_in(0, "Ada")?;
    let hello = |p: &Probe, name: &str| {
        p.texts().iter().any(|t| {
            holds(std::slice::from_ref(t), "hello") && holds(std::slice::from_ref(t), name)
        })
    };
    need!(hello(&p, "ada"), "typing Ada shows {}, not Hello, Ada", shown(&p));
    p.type_in(0, "Grace")?;
    need!(hello(&p, "grace") && !p.says("ada"), "typing Grace shows {}", shown(&p));
    Ok(())
}

pub(crate) fn temperature(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    for (c, f) in [("100", 212), ("-40", -40), ("37", 98), ("0", 32)] {
        p.type_in(0, c)?;
        need!(p.numbers().contains(&f), "{c} C shows {}, not {f}", shown(&p));
    }
    Ok(())
}

pub(crate) fn tip(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    for (bill, rate, tip, total) in
        [("200", "15%", 30, 230), ("200", "20%", 40, 240), ("99", "10%", 9, 108)]
    {
        p.type_in(0, bill)?;
        p.click(rate)?;
        let n = p.numbers();
        need!(
            n.contains(&tip) && n.contains(&total),
            "{bill} at {rate} shows {}, not {tip} and {total}",
            shown(&p)
        );
    }
    Ok(())
}

pub(crate) fn dice(src: &str) -> Result<(), String> {
    let mut seen = Vec::new();
    for seed in 1..=3 {
        let mut p = Probe::start(src, seed)?;
        for _ in 0..12 {
            p.click("Roll")?;
            // A die may tumble a while before it settles.
            p.wait(SETTLE)?;
            let draws = p.canvas().map_or(&[][..], |c| c.draws);
            let pips = draws.iter().filter(|d| d.shape == Shape::Circle).count() as i64;
            let said = p.numbers().contains(&pips);
            need!(
                (1..=6).contains(&pips) && said,
                "a roll draws {pips} circles and shows {}",
                shown(&p)
            );
            seen.push(pips);
        }
    }
    seen.sort_unstable();
    seen.dedup();
    need!(seen.len() >= 4, "36 rolls showed only {seen:?}");
    Ok(())
}

pub(crate) fn traffic(src: &str) -> Result<(), String> {
    // Which light is lit, 0 the top: the three largest circles, top to bottom, one in its color
    // and the others gray.
    let lit = |p: &Probe| -> Result<usize, String> {
        let mut c: Vec<_> = p
            .canvas()
            .map_or(&[][..], |c| c.draws)
            .iter()
            .filter(|d| d.shape == Shape::Circle)
            .collect();
        c.sort_by_key(|d| -i64::from(d.at[2]));
        c.truncate(3);
        c.sort_by_key(|d| d.at[1]);
        let colors: Vec<u8> = c.iter().map(|d| d.color).collect();
        let on: Vec<usize> = (0..colors.len()).filter(|&i| colors[i] != 8).collect();
        match (colors.len(), &on[..]) {
            (3, &[i]) if colors[i] == [1, 3, 2][i] => Ok(i),
            _ => Err(format!("the lights' colors, top to bottom, are {colors:?}")),
        }
    };
    let mut p = Probe::begin(src, 1)?;
    let mut t = 0;
    for (at, want) in
        [(0, 2), (500, 2), (1500, 2), (2500, 2), (3500, 1), (4500, 0), (6500, 0), (7500, 2)]
    {
        p.wait(at - t)?;
        t = at;
        let now = lit(&p)?;
        need!(now == want, "at {at} ms light {now} is lit (0 the top), not {want}");
    }
    p.click("Pause")?;
    p.wait(4000)?;
    need!(lit(&p)? == 2, "Pause does not stop it");
    p.click("Pause|Resume|Play|Start|Continue")?;
    p.wait(3200)?;
    need!(lit(&p)? == 1, "pressed again, it does not go on to yellow");
    Ok(())
}

pub(crate) fn stopwatch(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let near = |p: &Probe, t: i64| p.tenths().iter().any(|&s| (s - t).abs() <= 1);
    need!(p.tenths().contains(&0), "it does not show 0.0 at first: {}", shown(&p));
    p.click("Start")?;
    p.wait(2500)?;
    need!(near(&p, 25), "2.5 s after Start it shows {}", shown(&p));
    p.click("Stop")?;
    p.wait(1000)?;
    need!(near(&p, 25), "1 s after Stop it shows {}, not 2.5", shown(&p));
    p.click("Start|Resume")?;
    p.wait(1000)?;
    need!(near(&p, 35), "1 s after starting again it shows {}, not 3.5", shown(&p));
    p.click("Reset")?;
    need!(p.tenths().contains(&0), "Reset shows {}, not 0.0", shown(&p));
    Ok(())
}

/// Rock, paper, scissors: a pick is its place here.
const PICKS: [&str; 3] = ["rock", "paper", "scissors"];

/// The computer's pick: after "computer" in the text that says it; else, on a canvas, the pick
/// written nearest that text (its own column or row); else the first pick in a later text.
fn their_pick(p: &Probe) -> Option<usize> {
    let first = |t: &str| {
        let at = |(i, k): (usize, &&str)| Some((t.find(*k)?, i));
        PICKS.iter().enumerate().filter_map(at).min().map(|x| x.1)
    };
    let texts: Vec<String> = p.texts().iter().map(|t| crate::probe::fold(t)).collect();
    let at = texts.iter().position(|t| t.contains("computer"))?;
    if let Some(i) = first(&texts[at][texts[at].find("computer")? + 8..]) {
        return Some(i);
    }
    let draws = p.canvas().map_or(&[][..], |c| c.draws).iter().filter(|d| d.shape == Shape::Text);
    let on: Vec<(i64, i64, String)> =
        draws.map(|d| (d.at[0].into(), d.at[1].into(), crate::probe::fold(&d.text))).collect();
    if let Some((x, y, _)) = on.iter().find(|t| t.2.contains("computer")) {
        let near =
            on.iter().filter_map(|t| Some(((t.0 - x).pow(2) + (t.1 - y).pow(2), first(&t.2)?)));
        if let Some((_, i)) = near.min() {
            return Some(i);
        }
    }
    texts[at + 1..].iter().find_map(|t| first(t))
}

pub(crate) fn rps(src: &str) -> Result<(), String> {
    let mut theirs = Vec::new();
    for seed in 1..=3 {
        let mut p = Probe::start(src, seed)?;
        for i in 0..10 {
            let mine = PICKS[i % 3];
            p.click(mine)?;
            let Some(pick) = their_pick(&p).map(|i| PICKS[i]) else {
                return Err(format!("{mine} shows no Computer: pick, but {}", shown(&p)));
            };
            let words: Vec<String> = p.texts().iter().map(|t| crate::probe::fold(t)).collect();
            let draw = words.iter().any(|t| {
                t.split(|c: char| !c.is_alphanumeric()).any(|w| w == "draw" || w == "tie")
            });
            let said = match (
                holds(&words, "you win|you won"),
                holds(&words, "you lose|you lost"),
                draw,
            ) {
                (true, false, false) => 1,
                (false, true, false) => 2,
                (false, false, true) => 0,
                _ => return Err(format!("{mine} against {pick} shows {}", shown(&p))),
            };
            let (a, b) = (i % 3, PICKS.iter().position(|k| *k == pick).unwrap_or(0));
            let right = (a + 3 - b) % 3;
            need!(said == right, "{mine} against {pick} shows {}", shown(&p));
            theirs.push(b);
        }
    }
    theirs.sort_unstable();
    theirs.dedup();
    need!(theirs.len() >= 2, "the computer always picks {}", PICKS[theirs[0]]);
    Ok(())
}

pub(crate) fn guess(src: &str) -> Result<(), String> {
    let mut secrets = Vec::new();
    for seed in 1..=3 {
        let mut p = Probe::start(src, seed)?;
        let (mut lo, mut hi, mut found) = (1, 100, None);
        for _ in 0..8 {
            let g = (lo + hi) / 2;
            let before = p.texts();
            p.type_in(0, &g.to_string())?;
            p.click("Guess")?;
            // What it says now: in the texts that are new, else in all.
            let new = fresh(&before, &p);
            let texts = if new.iter().any(|t| holds(std::slice::from_ref(t), "too|correct")) {
                new
            } else {
                p.texts()
            };
            match (holds(&texts, "too high"), holds(&texts, "too low"), holds(&texts, "correct")) {
                (false, false, true) => {
                    found = Some(g);
                    break;
                }
                (true, false, false) => hi = g - 1,
                (false, true, false) => lo = g + 1,
                _ => return Err(format!("guessing {g} shows {}", shown(&p))),
            }
            need!(
                lo <= hi,
                "its answers leave no number from 1 to 100 (seed {seed}): {}",
                shown(&p)
            );
        }
        need!(found.is_some(), "8 halving guesses never hear Correct (seed {seed})");
        secrets.push(found);
    }
    secrets.dedup();
    need!(secrets.len() >= 2, "three games had the same secret");
    Ok(())
}

pub(crate) fn poll(src: &str) -> Result<(), String> {
    // The bars: rects (not across most of the canvas) on the bottom most of them share, left to
    // right, as heights.
    let bars = |p: &Probe| -> Vec<i64> {
        let Some(c) = p.canvas() else { return Vec::new() };
        let rects: Vec<_> = c
            .draws
            .iter()
            .filter(|d| {
                d.shape == Shape::Rect
                    && d.at[2] > 0
                    && d.at[3] > 0
                    && i64::from(d.at[2]) * 4 < c.w * 3
            })
            .collect();
        let bottom = |d: &&applang::Draw| i64::from(d.at[1]) + i64::from(d.at[3]);
        let base = rects
            .iter()
            .map(bottom)
            .max_by_key(|b| (rects.iter().filter(|d| bottom(d) == *b).count(), *b));
        let mut on: Vec<_> = rects
            .iter()
            .filter(|d| Some(bottom(d)) == base)
            .map(|d| (d.at[0], i64::from(d.at[3])))
            .collect();
        on.sort_unstable();
        on.into_iter().map(|b| b.1).collect()
    };
    let mut p = Probe::start(src, 1)?;
    for _ in 0..3 {
        p.click("Dogs")?;
    }
    p.click("Cats")?;
    let n = p.numbers();
    need!(n.contains(&3) && n.contains(&1), "3 dogs and 1 cat show {}", shown(&p));
    let b = bars(&p);
    let close =
        |big: i64, small: i64, k: i64| small > 0 && (big - small * k).abs() <= (big / 8).max(2);
    need!(b.len() >= 2 && close(b[1], b[0], 3), "3 dogs and 1 cat draw bars {b:?}");
    for _ in 0..5 {
        p.click("Birds")?;
    }
    let b = bars(&p);
    need!(b.len() == 3 && close(b[2] * 3, b[1], 5) && b[2] > b[1], "then 5 birds draw bars {b:?}");
    Ok(())
}

pub(crate) fn tictactoe(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 3, 3)?;
    let empty: Vec<String> = squares(3, 3).map(|(c, r)| p.look(b, c, r)).collect();
    // Plays the squares in turn, X first; the texts new after the last.
    let play = |p: &mut Probe, moves: &[i64]| -> Result<Vec<String>, String> {
        let mut before = p.texts();
        for (k, &i) in moves.iter().enumerate() {
            if k + 1 == moves.len() {
                before = p.texts();
            }
            p.tap_cell(b, i % 3, i / 3)?;
        }
        Ok(fresh(&before, p))
    };
    let won = play(&mut p, &[0, 1, 4, 2, 8])?;
    need!(
        holds(&won, "x wins|x won|winner: x|x is the winner"),
        "X on 0, 4, 8 shows {}",
        shown(&p)
    );
    p.click("New game")?;
    let now: Vec<String> = squares(3, 3).map(|(c, r)| p.look(b, c, r)).collect();
    need!(now == empty, "New game does not empty the board");
    let drawn = play(&mut p, &[0, 1, 2, 5, 3, 6, 4, 8, 7])?;
    let tie = drawn.iter().any(|t| {
        crate::probe::fold(t)
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == "draw" || w == "tie")
    });
    need!(tie, "a full board with no line shows {}", shown(&p));
    Ok(())
}

pub(crate) fn connect4(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 7, 6)?;
    let (top, low) = (p.look(b, 3, 0), p.look(b, 3, 5));
    p.tap_cell(b, 3, 0)?;
    p.wait(SETTLE)?;
    need!(
        p.look(b, 3, 5) != low && p.look(b, 3, 0) == top,
        "a tap atop column 3 drops no disc to its bottom"
    );
    for (moves, who) in [([0, 1, 0, 1, 0, 1, 0, 0], "red"), ([0, 1, 0, 1, 0, 1, 6, 1], "yellow")] {
        p.click("New game")?;
        need!(p.look(b, 3, 5) == low, "New game does not empty the board");
        let n = if who == "red" { 7 } else { 8 };
        let mut before = Vec::new();
        for &c in &moves[..n] {
            before = p.texts();
            p.tap_cell(b, c, 0)?;
            p.wait(SETTLE)?;
        }
        let said = fresh(&before, &p);
        need!(
            holds(&said, &format!("{who} wins|{who} won")),
            "four {who} discs in a column show {}",
            shown(&p)
        );
    }
    Ok(())
}

pub(crate) fn pomodoro(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let at = |p: &Probe, m: i64, s: i64| p.times().iter().any(|t| t[..] == [m, s]);
    need!(at(&p, 25, 0), "it does not show 25:00 at first: {}", shown(&p));
    p.click("Start")?;
    p.wait(60_000)?;
    need!(at(&p, 24, 0), "a minute after Start it shows {}", shown(&p));
    p.click("Pause")?;
    p.wait(5000)?;
    need!(at(&p, 24, 0), "5 s after Pause it shows {}", shown(&p));
    p.click("Start|Resume")?;
    p.wait(24 * 60_000)?;
    need!(p.says("break") && at(&p, 5, 0), "after 25 minutes it shows {}", shown(&p));
    p.wait(60_000)?;
    need!(at(&p, 4, 0), "a minute into the break it shows {}", shown(&p));
    p.click("Reset")?;
    need!(at(&p, 25, 0), "Reset shows {}", shown(&p));
    Ok(())
}

pub(crate) fn clock(src: &str) -> Result<(), String> {
    let mut p = Probe::begin(src, 1)?;
    // The hands: lines from the face's center (the largest ring or circle), as where they point.
    let hands = |p: &Probe| -> Vec<(i64, i64)> {
        let Some(c) = p.canvas() else { return Vec::new() };
        let face = c
            .draws
            .iter()
            .filter(|d| matches!(d.shape, Shape::Ring | Shape::Circle))
            .max_by_key(|d| d.at[2]);
        let (cx, cy) = face.map_or((c.w / 2, c.h / 2), |d| (d.at[0].into(), d.at[1].into()));
        let near =
            |x: i16, y: i16| (i64::from(x) - cx).abs() <= 2 && (i64::from(y) - cy).abs() <= 2;
        let lines = c.draws.iter().filter(|d| d.shape == Shape::Line);
        lines
            .filter_map(|d| match d.at {
                [x, y, a, b, _] if near(x, y) => Some((i64::from(a) - cx, i64::from(b) - cy)),
                [x, y, a, b, _] if near(a, b) => Some((i64::from(x) - cx, i64::from(y) - cy)),
                _ => None,
            })
            .collect()
    };
    let up = |&(x, y): &(i64, i64)| y < 0 && x.abs() * 8 <= -y;
    let right = |&(x, y): &(i64, i64)| x > 0 && y.abs() * 8 <= x;
    let at = |p: &Probe, t: &[&[i64]]| p.times().iter().any(|s| t.contains(&&s[..]));
    need!(at(&p, &[&[12, 0, 0]]), "it does not show 12:00:00 at first: {}", shown(&p));
    let h = hands(&p);
    need!(h.len() >= 3 && h.iter().all(up), "at 12:00:00 its hands from the center are {h:?}");
    p.wait(15_000)?;
    need!(at(&p, &[&[12, 0, 15]]), "15 s on it shows {}", shown(&p));
    need!(hands(&p).iter().any(right), "at 12:00:15 no hand points right: {:?}", hands(&p));
    p.wait(45_000)?;
    need!(at(&p, &[&[12, 1, 0]]), "a minute on it shows {}", shown(&p));
    p.wait(3 * 3_600_000 - 60_000)?;
    need!(at(&p, &[&[3, 0, 0], &[15, 0, 0]]), "3 hours on it shows {}", shown(&p));
    need!(hands(&p).iter().any(right), "at 3:00:00 no hand points right: {:?}", hands(&p));
    Ok(())
}

pub(crate) fn calculator(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let keys = |p: &mut Probe, s: &str| -> Result<(), String> {
        for k in s.chars() {
            match k {
                '*' => p.click("*|\u{d7}|x")?,
                '/' => p.click("/|\u{f7}")?,
                '-' => p.click("-|\u{2212}|\u{2013}")?,
                'C' => p.click("C|AC|Clear")?,
                k => p.click(&k.to_string())?,
            }
        }
        Ok(())
    };
    for (typed, want) in [("12+7=", 19), ("C9*8=", 72), ("C7-9=", -2), ("C7/2=", 3)] {
        keys(&mut p, typed)?;
        need!(p.numbers().contains(&want), "{typed} shows {}, not {want}", shown(&p));
    }
    keys(&mut p, "C")?;
    need!(!p.numbers().contains(&3), "C leaves {}", shown(&p));
    keys(&mut p, "8/0=")?;
    need!(p.says("error"), "8/0= shows {}, not Error", shown(&p));
    Ok(())
}

pub(crate) fn paint(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 32, 32)?;
    let pick = |p: &mut Probe, name: &str, color: u8| -> Result<(), String> {
        if p.has(name) {
            return p.click(name);
        }
        // A swatch: a rect or circle of the color, its middle off the board.
        let Board::Area { x, y, w, h, .. } = b else {
            return Err(format!("no button labelled {name}"));
        };
        let swatch = p.canvas().map_or(&[][..], |c| c.draws).iter().find_map(|d| {
            let (a, t, z, e) = crate::probe::extent(d);
            let (mx, my) = ((a + z - 1) / 2, (t + e - 1) / 2);
            let off = !(x..x + w).contains(&mx) || !(y..y + h).contains(&my);
            (matches!(d.shape, Shape::Rect | Shape::Circle) && d.color == color && off)
                .then_some((mx, my))
        });
        let (sx, sy) = swatch
            .ok_or_else(|| format!("no button labelled {name}, nor a swatch of color {color}"))?;
        p.tap(sx, sy)
    };
    let spots = [("Blue", 4, (5, 5)), ("Red", 1, (10, 12)), ("Yellow", 3, (20, 20))];
    for (name, color, (c, r)) in spots {
        pick(&mut p, name, color)?;
        p.tap_cell(b, c, r)?;
        let now = p.color(b, c, r);
        need!(
            now == i64::from(color),
            "{name} picked, a tap at square {c}, {r} paints color {now}, not {color}"
        );
    }
    need!(p.color(b, 5, 5) == 4, "painting elsewhere changes square 5, 5");
    p.click("Clear")?;
    for (name, color, (c, r)) in spots {
        need!(p.color(b, c, r) != i64::from(color), "Clear leaves the {name} square");
    }
    Ok(())
}

pub(crate) fn drawpad(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 64, 64)?;
    let ink = |p: &Probe, c: i64, r: i64| p.color(b, c, r) == 9;
    p.tap_cell(b, 10, 20)?;
    need!(ink(&p, 10, 20), "a tap at 10, 20 inks color {} there", p.color(b, 10, 20));
    need!(!ink(&p, 53, 20), "mirror mode is on before Mirror is pressed");
    p.click("Mirror")?;
    p.tap_cell(b, 5, 40)?;
    need!(
        ink(&p, 5, 40) && ink(&p, 58, 40),
        "with Mirror on, a tap at 5, 40 inks no square at 58, 40"
    );
    p.click("Mirror")?;
    p.tap_cell(b, 7, 50)?;
    need!(ink(&p, 7, 50) && !ink(&p, 56, 50), "Mirror pressed again does not turn it off");
    p.click("Clear")?;
    need!(!ink(&p, 10, 20) && !ink(&p, 58, 40), "Clear leaves ink");
    Ok(())
}

pub(crate) fn whack(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 3, 3)?;
    p.click("Start")?;
    let mut hits = 0;
    for _ in 0..12 {
        p.wait(800)?;
        // The one square unlike the rest: by how it looks, else (a score written in a square)
        // by the color at its middle.
        let one = |looks: Vec<String>| {
            let odd = (0..9).filter(|&i| looks.iter().filter(|l| **l == looks[i]).count() == 1);
            let odd: Vec<usize> = odd.collect();
            if let [k] = odd[..] { Some(k) } else { None }
        };
        let looks = squares(3, 3).map(|(c, r)| p.look(b, c, r)).collect();
        let colors = squares(3, 3).map(|(c, r)| p.color(b, c, r).to_string()).collect();
        let Some(k) = one(looks).or_else(|| one(colors)) else { continue };
        let before = p.after("score").unwrap_or(0);
        p.tap_cell(b, k as i64 % 3, k as i64 / 3)?;
        let now = p.after("score");
        need!(now == Some(before + 1), "tapping the mole moves Score: from {before} to {now:?}");
        hits += 1;
    }
    need!(hits >= 5, "the mole showed as one odd square only {hits} times in 12");
    p.wait(31_000)?;
    need!(p.says("game over"), "30 s on it shows {}, not Game over", shown(&p));
    Ok(())
}

pub(crate) fn bounce(src: &str) -> Result<(), String> {
    let mut p = Probe::begin(src, 1)?;
    let ball = |p: &Probe| -> Option<(i64, i64, i64, i64, i64)> {
        let c = p.canvas()?;
        let d = c.draws.iter().find(|d| d.shape == Shape::Circle)?;
        Some((d.at[0].into(), d.at[1].into(), d.at[2].into(), c.w, c.h))
    };
    let mut seen = vec![ball(&p).ok_or("no circle on a canvas")?];
    for _ in 0..300 {
        p.wait(33)?;
        let (x, y, r, w, h) = ball(&p).ok_or("the ball is gone")?;
        need!(
            x + r >= 0 && x - r <= w && y + r >= 0 && y - r <= h,
            "the ball leaves the canvas, at {x}, {y}"
        );
        seen.push((x, y, r, w, h));
    }
    let turns = |k: fn(&(i64, i64, i64, i64, i64)) -> i64| {
        let steps: Vec<i64> =
            seen.windows(2).map(|s| k(&s[1]) - k(&s[0])).filter(|d| *d != 0).collect();
        steps.windows(2).filter(|s| (s[0] > 0) != (s[1] > 0)).count()
    };
    let (tx, ty) = (turns(|b| b.0), turns(|b| b.1));
    need!(tx >= 1 && ty >= 1, "in 10 s it turns back {tx} times across and {ty} times down");
    let moved = |p: &mut Probe| -> Result<i64, String> {
        let mut far = 0;
        for _ in 0..10 {
            let (x0, y0, ..) = ball(p).unwrap_or_default();
            p.wait(33)?;
            let (x1, y1, ..) = ball(p).unwrap_or_default();
            far += (x1 - x0).abs() + (y1 - y0).abs();
        }
        Ok(far)
    };
    let slow = moved(&mut p)?;
    p.click("Faster")?;
    let fast = moved(&mut p)?;
    need!(fast > slow, "Faster moves it {fast} units in 330 ms, before {slow}");
    Ok(())
}

pub(crate) fn life(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    let b = board(&p, 20, 20)?;
    p.click("Clear")?;
    let dead = p.look(b, 0, 0);
    let alive = |p: &Probe| marked(p, b, 20, 20, &dead);
    let set = |p: &mut Probe, cells: &[(i64, i64)]| -> Result<(), String> {
        cells.iter().try_for_each(|&(c, r)| p.tap_cell(b, c, r))
    };
    let flat = [(9, 10), (10, 10), (11, 10)];
    set(&mut p, &flat)?;
    need!(alive(&p) == flat, "tapping 9..11, 10 makes {:?} alive", alive(&p));
    p.click("Step")?;
    need!(alive(&p) == [(10, 9), (10, 10), (10, 11)], "a step of a blinker gives {:?}", alive(&p));
    p.click("Step")?;
    need!(alive(&p) == flat, "two steps of a blinker give {:?}", alive(&p));
    p.tap_cell(b, 10, 10)?;
    need!(alive(&p) == [(9, 10), (11, 10)], "a tap does not kill 10, 10");
    p.click("Step")?;
    need!(alive(&p).is_empty(), "two lone squares live on as {:?}", alive(&p));
    let glider = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
    set(&mut p, &glider)?;
    for _ in 0..4 {
        p.click("Step")?;
    }
    let mut moved: Vec<(i64, i64)> = glider.iter().map(|&(c, r)| (c + 1, r + 1)).collect();
    moved.sort_by_key(|&(c, r)| (r, c));
    need!(alive(&p) == moved, "a glider 4 steps on is {:?}, not {moved:?}", alive(&p));
    p.click("Clear")?;
    set(&mut p, &flat)?;
    p.click("Play|Start|Run")?;
    let mut t = 0;
    while alive(&p) == flat && t < 2000 {
        p.wait(50)?;
        t += 50;
    }
    need!(alive(&p) != flat, "2 s after Play the blinker has not moved");
    p.click("Pause|Stop")?;
    let now = alive(&p);
    p.wait(2000)?;
    need!(alive(&p) == now, "Pause does not stop it");
    Ok(())
}

pub(crate) fn minesweeper(src: &str) -> Result<(), String> {
    for seed in 3..=40 {
        let mut p = Probe::begin(src, seed)?;
        let b = board(&p, 9, 9)?;
        let (c, r) = [(4, 4), (0, 0), (8, 3)][seed as usize % 3];
        p.tap_cell(b, c, r)?;
        need!(!p.says("game over"), "a first tap on {c}, {r} is a mine (seed {seed})");
    }
    for seed in 1..=2 {
        // A new game whose first tap is the middle square.
        let first = || -> Result<(Probe, Board), String> {
            let mut p = Probe::begin(src, seed)?;
            let b = board(&p, 9, 9)?;
            let hidden = p.look(b, 4, 4);
            p.tap_cell(b, 4, 4)?;
            need!(!p.says("game over"), "the first tap is a mine");
            need!(p.look(b, 4, 4) != hidden, "the first tap reveals nothing");
            Ok((p, b))
        };
        let mut mines = Vec::new();
        for (c, r) in squares(9, 9).filter(|&s| s != (4, 4)) {
            let (mut p, b) = first()?;
            p.tap_cell(b, c, r)?;
            if p.says("game over") {
                mines.push((c, r));
            }
        }
        need!(mines.len() == 10, "{} squares end the game when tapped, not 10", mines.len());
        let (mut p, b) = first()?;
        let hidden = Probe::begin(src, seed)?.look(b, 0, 0);
        let safe =
            squares(9, 9).find(|&(c, r)| !mines.contains(&(c, r)) && p.look(b, c, r) == hidden);
        p.click("Flag")?;
        p.tap_cell(b, mines[0].0, mines[0].1)?;
        need!(!p.says("game over"), "a mine tapped after Flag ends the game");
        if let Some((c, r)) = safe {
            p.tap_cell(b, c, r)?;
            let flagged = p.look(b, c, r);
            let (mut q, _) = first()?;
            q.tap_cell(b, c, r)?;
            need!(
                flagged != hidden && flagged != q.look(b, c, r),
                "a tap after Flag reveals {c}, {r} instead of flagging it"
            );
        }
        let (mut p, b) = first()?;
        let mut before = p.texts();
        for (c, r) in squares(9, 9).filter(|s| !mines.contains(s)) {
            before = p.texts();
            p.tap_cell(b, c, r)?;
        }
        let won = fresh(&before, &p);
        need!(
            holds(&won, "you win|you won|win!"),
            "every safe square revealed shows {}",
            shown(&p)
        );
    }
    Ok(())
}

pub(crate) fn memory(src: &str) -> Result<(), String> {
    for seed in 1..=2 {
        let p = Probe::begin(src, seed)?;
        let b = board(&p, 4, 4)?;
        let down: Vec<String> = squares(4, 4).map(|(c, r)| p.look(b, c, r)).collect();
        let at = |i: usize| (i as i64 % 4, i as i64 / 4);
        // Each card's face, turned up alone in a new game.
        let mut faces = Vec::new();
        for (i, was) in down.iter().enumerate() {
            let mut p = Probe::begin(src, seed)?;
            p.tap_cell(b, at(i).0, at(i).1)?;
            let face = p.look(b, at(i).0, at(i).1);
            need!(face != *was, "a tap does not turn card {i} up");
            faces.push(face);
        }
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for i in 0..16 {
            let same: Vec<usize> = (0..16).filter(|&j| faces[j] == faces[i]).collect();
            need!(same.len() == 2, "card {i}'s face shows on {} cards, not 2", same.len());
            if same[0] == i {
                pairs.push((same[0], same[1]));
            }
        }
        let mut p = Probe::begin(src, seed)?;
        let (a, c, d) = (pairs[0].0, pairs[1].0, pairs[2].0);
        for i in [a, c] {
            p.tap_cell(b, at(i).0, at(i).1)?;
        }
        p.wait(1500)?;
        p.tap_cell(b, at(d).0, at(d).1)?;
        let back = p.look(b, at(a).0, at(a).1) == down[a] && p.look(b, at(c).0, at(c).1) == down[c];
        need!(back, "two cards that differ stay face up");
        let mut p = Probe::begin(src, seed)?;
        let mut before = p.texts();
        for &(i, j) in &pairs {
            before = p.texts();
            p.tap_cell(b, at(i).0, at(i).1)?;
            p.tap_cell(b, at(j).0, at(j).1)?;
            p.wait(1500)?;
            let up =
                p.look(b, at(i).0, at(i).1) == faces[i] && p.look(b, at(j).0, at(j).1) == faces[j];
            need!(up, "a matching pair does not stay face up");
        }
        need!(
            holds(&fresh(&before, &p), "you win|you won"),
            "every pair found shows {}",
            shown(&p)
        );
        need!(p.after("moves") == Some(8), "8 pairs turned show {}, not Moves: 8", shown(&p));
    }
    Ok(())
}

pub(crate) fn game2048(src: &str) -> Result<(), String> {
    let mut p = Probe::begin(src, 1)?;
    // The tiles: the canvas's texts that are whole numbers, where and what.
    let tiles = |p: &Probe| -> Vec<(i16, i16, i64)> {
        let draws = p.canvas().map_or(&[][..], |c| c.draws);
        let mut t: Vec<_> = draws
            .iter()
            .filter(|d| d.shape == Shape::Text)
            .filter_map(|d| Some((d.at[0], d.at[1], d.text.trim().parse::<i64>().ok()?)))
            .collect();
        t.sort_unstable();
        t
    };
    let sum = |t: &[(i16, i16, i64)]| t.iter().map(|t| t.2).sum::<i64>();
    let start = tiles(&p);
    need!(
        start.len() == 2 && start.iter().all(|t| t.2 == 2 || t.2 == 4),
        "it starts with tiles {start:?}"
    );
    let mut most = 0;
    'game: for _ in 0..300 {
        for (label, key) in [("Down", "down"), ("Left", "left"), ("Right", "right"), ("Up", "up")] {
            let before = tiles(&p);
            p.press(label, key)?;
            p.wait(SETTLE)?;
            let after = tiles(&p);
            if after == before {
                continue;
            }
            let grew = sum(&after) - sum(&before);
            need!(grew == 2 || grew == 4, "{label} turns tiles {before:?} into {after:?}");
            most = after.iter().map(|t| t.2).max().unwrap_or(0).max(most);
            if p.says("game over") {
                break 'game;
            }
            continue 'game;
        }
        need!(p.says("game over"), "no move changes the board, but it does not say Game over");
        break;
    }
    need!(most >= 64 || p.says("game over"), "300 moves reach only {most}");
    need!(most >= 16, "no tiles merged past {most}");
    Ok(())
}

pub(crate) fn tetris(src: &str) -> Result<(), String> {
    let mut p = Probe::start(src, 1)?;
    p.click("Start")?;
    let b = board(&p, 10, 20)?;
    // The filled squares: those unlike the most common look.
    let filled = |p: &Probe| -> Vec<(i64, i64)> {
        let looks: Vec<String> = squares(10, 20).map(|(c, r)| p.look(b, c, r)).collect();
        let empty = looks
            .iter()
            .max_by_key(|l| looks.iter().filter(|m| m == l).count())
            .cloned()
            .unwrap_or_default();
        squares(10, 20).zip(&looks).filter(|(_, l)| **l != empty).map(|(s, _)| s).collect()
    };
    let mut t = 0;
    while filled(&p).is_empty() && t < 3000 {
        p.wait(100)?;
        t += 100;
    }
    let piece = filled(&p);
    need!(!piece.is_empty() && piece.len() <= 4, "3 s after Start the board holds {piece:?}");
    let top = |s: &[(i64, i64)]| s.iter().map(|s| s.1).min().unwrap_or(0);
    p.wait(1500)?;
    need!(top(&filled(&p)) > top(&piece), "the piece does not fall in 1.5 s");
    let shape = |s: &[(i64, i64)]| {
        let (c0, r0) = (s.iter().map(|s| s.0).min().unwrap_or(0), top(s));
        let mut v: Vec<(i64, i64)> = s.iter().map(|&(c, r)| (c - c0, r - r0)).collect();
        v.sort_unstable();
        v
    };
    let left = |s: &[(i64, i64)]| s.iter().map(|s| s.0).min().unwrap_or(0);
    let before = filled(&p);
    for _ in 0..4 {
        p.press("Left", "left")?;
    }
    need!(left(&filled(&p)) < left(&before).max(1), "Left does not move the piece left");
    for _ in 0..12 {
        p.press("Right", "right")?;
    }
    need!(filled(&p).iter().any(|s| s.0 == 9), "Right does not take the piece to the right edge");
    // A rotation changes some piece's shape (an O's never does).
    let mut turned = false;
    for _ in 0..4 {
        let high: Vec<(i64, i64)> = filled(&p).into_iter().filter(|s| s.1 < 8).collect();
        p.press("Rotate", "up")?;
        let now: Vec<(i64, i64)> = filled(&p).into_iter().filter(|s| s.1 < 8).collect();
        if shape(&now) != shape(&high) {
            turned = true;
            break;
        }
        p.press("Drop", "space")?;
        p.wait(1000)?;
    }
    need!(turned, "Rotate does not turn any of four pieces");
    p.press("Drop", "space")?;
    p.wait(1000)?;
    need!(filled(&p).iter().any(|s| s.1 == 19), "a dropped piece does not reach the bottom row");
    for _ in 0..80 {
        if p.says("game over") {
            return Ok(());
        }
        p.press("Drop", "space")?;
        p.wait(600)?;
    }
    Err(format!("80 drops in one column never show Game over: {}", shown(&p)))
}
