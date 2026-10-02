//! What a reply holds ([`read`]): a whole program, or SEARCH/REPLACE edit blocks to apply to the
//! program it was shown ([`apply`]), atomically:
//!
//! ```text
//! <<<<<<< SEARCH
//! lines copied from the program
//! =======
//! the lines to put there
//! >>>>>>> REPLACE
//! ```
//!
//! The program is the first closed `app` block without markers that is [`honest`]: a make stops
//! reading there, so a reply reads the same however the network cut it into chunks, and an
//! example before the program is not the program. It wins over edits. Else edit blocks, every
//! one in the reply, whatever fences are around them; else the longest `app` block without
//! markers. A SEARCH matches where its lines equal the program's, line numbers copied with them
//! (`43| `) and trailing spaces aside; failing that, leading spaces aside too (applang ignores
//! indentation, so the replacement keeps its own). It must match exactly one place.

use crate::ai::{blocks, num, numbered};
use applang::Class;

/// One edit block: the lines to find and the lines to put there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub search: Vec<String>,
    pub replace: Vec<String>,
}

/// What a reply holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// A whole program.
    Program(String),
    /// Edit blocks, in order.
    Edits(Vec<Edit>),
    /// Cut off by the token limit inside its program or an edit block: part of one.
    Cut,
    /// Edit block `n` (from 1) never reached its REPLACE line.
    Unclosed(usize),
    /// Neither.
    Nothing,
}

/// Whether `text` has an edit block's first line.
pub fn marked(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("<<<<<<<"))
}

/// Whether `src` begins as the prompts ask a program to: with a comment saying what the app is
/// ([`crate::ai::HONEST`]). Not whether it compiles: one that does not is the program still (a
/// fix comes next), and waiting for one that compiles reads on while the model drafts past its
/// block (live, a Tetris whose block closed at 53 s was read on to its 6,144 tokens at 109 s).
pub fn honest(src: &str) -> bool {
    applang::highlight(src).first().is_some_and(|(_, c)| *c == Class::Comment)
}

/// The program in `reply` and whether its block was closed: its first closed `app` block without
/// edit markers that is [`honest`]; else its longest `app` block without markers (the first of
/// equals), to the reply's end if it is open.
pub fn program(reply: &str) -> Option<(&str, bool)> {
    let mut long: Option<(&str, bool)> = None;
    for b in blocks(reply).into_iter().filter(|b| !marked(b.0)) {
        if b.1 && honest(b.0) {
            return Some(b);
        }
        if long.is_none_or(|l| b.0.len() > l.0.len()) {
            long = Some(b);
        }
    }
    long
}

/// How many lines of `src` are not blank.
pub fn lines(src: &str) -> usize {
    src.lines().filter(|l| !l.trim().is_empty()).count()
}

/// What `reply` holds; `cut` when it ran out of room (`finish_reason` length): a reply cut off
/// inside an open block holds part of a program, never a program.
pub fn read(reply: &str, cut: bool) -> Reply {
    let reply: String = reply.chars().filter(|&c| c != '\r').collect();
    match program(&reply) {
        Some((src, true)) if honest(src) => return Reply::Program(src.into()),
        Some((_, false)) if cut && !marked(&reply) => return Reply::Cut,
        Some((src, _)) if !marked(&reply) => return Reply::Program(src.into()),
        _ => {}
    }
    let (mut edits, mut open): (Vec<Edit>, Option<Edit>) = (Vec::new(), None);
    let mut replacing = false;
    for line in reply.lines() {
        let t = line.trim();
        if t.starts_with("<<<<<<<") {
            if open.is_some() {
                return Reply::Unclosed(edits.len() + 1);
            }
            (open, replacing) = (Some(Edit { search: Vec::new(), replace: Vec::new() }), false);
        } else if let Some(e) = &mut open {
            if !replacing && t.len() >= 5 && t.bytes().all(|b| b == b'=') {
                replacing = true;
            } else if replacing && t.starts_with(">>>>>>>") {
                edits.extend(open.take());
            } else if replacing {
                e.replace.push(line.into());
            } else {
                e.search.push(line.into());
            }
        }
    }
    match open {
        Some(_) if cut => Reply::Cut,
        Some(_) => Reply::Unclosed(edits.len() + 1),
        None if !edits.is_empty() => Reply::Edits(edits),
        None => Reply::Nothing,
    }
}

/// `line` without a line number copied from a numbered program (`  43| `).
fn bare(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let rest = t[digits..].strip_prefix('|').filter(|r| digits > 0 && !r.starts_with('|'))?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// `lines` without their blank lines at either end, and without line numbers if every other
/// line of `numbers` has one (they were copied from the numbered program).
fn inner(lines: &[String], numbers: &[String]) -> Vec<String> {
    let copied = numbers.iter().all(|l| l.trim().is_empty() || bare(l).is_some());
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if !out.is_empty() || !l.trim().is_empty() {
            out.push(bare(l).filter(|_| copied).unwrap_or(l.as_str()).into());
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out
}

/// `src` with `edits` applied in order, or what to tell the model of the first that did not
/// apply (`src` is then unchanged).
pub fn apply(src: &str, edits: &[Edit]) -> Result<String, String> {
    let mut text = src.to_string();
    for (i, e) in edits.iter().enumerate() {
        let block = ["SEARCH block ", &num(i as u64 + 1)].concat();
        let search = inner(&e.search, &e.search);
        let replace = inner(&e.replace, &e.search);
        if search.is_empty() {
            return Err([&block, " is empty."].concat());
        }
        let lines: Vec<&str> = text.lines().collect();
        // Where it matches as copied (trailing spaces aside), and leading spaces aside too.
        let mut found = [Vec::new(), Vec::new()];
        for at in 0..(lines.len() + 1).saturating_sub(search.len()) {
            let pairs = || lines[at..].iter().zip(&search);
            if pairs().all(|(l, s)| l.trim_end() == s.trim_end()) {
                found[0].push(at);
            }
            if pairs().all(|(l, s)| l.trim() == s.trim()) {
                found[1].push(at);
            }
        }
        let at = match found[usize::from(found[0].is_empty())][..] {
            [at] => at,
            [a, b, ..] => {
                let (a, b) = (num(a as u64 + 1), num(b as u64 + 1));
                let again = "; quote a line around it so it matches one place.";
                return Err([&block, " matched lines ", &a, " and ", &b, again].concat());
            }
            [] => return Err([&block, " matched no lines.", &nearest(&lines, &search)].concat()),
        };
        let after = &lines[at + search.len()..];
        let all = lines[..at].iter().copied().chain(replace.iter().map(String::as_str));
        text = all.chain(after.iter().copied()).flat_map(|l| [l, "\n"]).collect();
    }
    Ok(text)
}

/// " The nearest lines are:" and, numbered, at most 8 lines of the program from the first that
/// equals one of `search`'s, spaces aside; "" if none does.
fn nearest(lines: &[&str], search: &[String]) -> String {
    let same = |l: &&str| search.iter().any(|s| !s.trim().is_empty() && l.trim() == s.trim());
    let Some(at) = lines.iter().position(same) else { return String::new() };
    let shown: String = lines[at..].iter().take(8).flat_map(|l| [*l, "\n"]).collect();
    [" The nearest lines are:\n", &numbered(&shown, at + 1)].concat()
}
