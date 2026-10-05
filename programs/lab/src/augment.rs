//! Augmentation that keeps a program's meaning: the names it declares renamed consistently (a
//! one-to-one map, so no two meet), and its state declarations reordered. Comments and strings
//! stay. The corpus compiles every result again and keeps only those that do.

use std::collections::{BTreeMap, BTreeSet};

use applang::{Class, highlight};
use applang_syntax::ast::BUILTINS;
use tiny::Rng;

/// Names never renamed nor taken: a canvas handler's `x` and `y`, a grid handler's `cell`, the
/// types, and the words that are keywords only in their places.
const KEPT: [&str; 13] = [
    "x", "y", "cell", "int", "bool", "string", "saved", "every", "on", "key", "grid", "canvas",
    "col",
];

/// Names for what a function does.
#[rustfmt::skip]
const VERBS: &[&str] = &[
    "reset", "restart", "start", "begin", "spawn", "step", "advance", "move", "shift", "slide",
    "turn", "rotate", "drop", "fall", "land", "place", "put", "add", "take", "pick", "fill",
    "paint", "mark", "toggle", "flip", "swap", "score", "check", "test", "fits", "free",
    "blocked", "hit", "wins", "full", "count", "find", "at", "idx", "cellAt", "spot", "pos",
    "show", "shown", "lit", "name", "draw", "scene",
];

/// Names for what a state, a parameter, a let or a loop holds.
#[rustfmt::skip]
const NOUNS: &[&str] = &[
    "a", "b", "c", "d", "i", "j", "k", "n", "m", "p", "q", "r", "s", "t", "u", "v", "w", "z",
    "dx", "dy", "px", "py", "vx", "vy", "bx", "by", "x0", "y0", "x1", "y1", "cx", "cy",
    "age", "amount", "angle", "ball", "balls", "bar", "base", "best", "block", "board", "body",
    "bonus", "brick", "card", "cards", "cells", "coin", "coins", "color", "counter", "cursor",
    "dir", "done", "dot", "dots", "edge", "enemy", "field", "flag", "food", "frame", "gap",
    "goal", "hand", "head", "hits", "item", "items", "kind", "last", "level", "lives", "mode",
    "moves", "next", "note", "notes", "over", "paddle", "piece", "player", "points", "prev",
    "rows", "seen", "shape", "speed", "stack", "tally", "tasks", "tick", "total", "turn",
];

/// `src` with each name it declares (states, functions, parameters, lets, loops) mapped to a
/// fresh one drawn by `rng`: verbs for functions, nouns for the rest, never a name `src`
/// already uses, a built-in or a kept word. None if nothing changed.
pub fn rename(src: &str, rng: &mut Rng) -> Option<String> {
    let toks = highlight(src);
    let text = |i: usize| &src[toks[i].0.start..toks[i].0.end];
    let code: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].1 != Class::Comment).collect();
    let used: BTreeSet<&str> =
        code.iter().filter(|&&i| toks[i].1 == Class::Name).map(|&i| text(i)).collect();
    let mut declared: BTreeMap<&str, bool> = BTreeMap::new();
    for w in code.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (ta, tb) = (text(a), text(b));
        match (toks[a].1, toks[b].1) {
            (Class::Keyword, Class::Name) if matches!(ta, "state" | "let" | "for") => {
                declared.entry(tb).or_insert(false);
            }
            (Class::Keyword, Class::Name) if ta == "fn" => _ = declared.insert(tb, true),
            (Class::Name, Class::Punct) if tb == ":" => _ = declared.entry(ta).or_insert(false),
            _ => {}
        }
    }
    let taken = |n: &str| used.contains(n) || BUILTINS.contains(&n) || KEPT.contains(&n);
    let mut map: BTreeMap<&str, &str> = BTreeMap::new();
    let mut fresh: BTreeSet<&str> = BTreeSet::new();
    for (&name, &is_fn) in &declared {
        if KEPT.contains(&name) {
            continue;
        }
        let pool = if is_fn { VERBS } else { NOUNS };
        for _ in 0..64 {
            let pick = pool[rng.below(pool.len())];
            if !taken(pick) && !fresh.contains(pick) {
                fresh.insert(pick);
                map.insert(name, pick);
                break;
            }
        }
    }
    if map.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for &(span, class) in &toks {
        out.push_str(&src[at..span.start]);
        let word = &src[span.start..span.end];
        let to = map.get(word).filter(|_| class == Class::Name);
        out.push_str(to.copied().unwrap_or(word));
        at = span.end;
    }
    out.push_str(&src[at..]);
    Some(out)
}

/// `src` with its run of one-line state declarations (the first lines that are not comments
/// or blank, each `state` or `saved state`, one `;`) in an order drawn by `rng`. None if there
/// are fewer than two, or the order came out the same.
pub fn shuffle_states(src: &str, rng: &mut Rng) -> Option<String> {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let first = lines.iter().position(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with("//")
    })?;
    let one = |l: &str| {
        let t = l.trim_start();
        (t.starts_with("state ") || t.starts_with("saved state "))
            && l.split("//").next().is_some_and(|code| code.matches(';').count() == 1)
            && !l.contains("/*")
    };
    let run = lines[first..].iter().take_while(|l| one(l)).count();
    if run < 2 {
        return None;
    }
    let mut order: Vec<usize> = (0..run).collect();
    for i in (1..run).rev() {
        order.swap(i, rng.below(i + 1));
    }
    if order.iter().enumerate().all(|(i, &o)| i == o) {
        return None;
    }
    let mut out: String = lines[..first].concat();
    for &o in &order {
        let l = lines[first + o];
        out.push_str(l);
        if !l.ends_with('\n') {
            out.push('\n');
        }
    }
    out.extend(lines[first + run..].iter().copied());
    Some(out)
}
