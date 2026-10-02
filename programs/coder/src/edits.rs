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
//! A closed `app` block with no markers in it is a whole program and wins over edits; a fence
//! around edit blocks is ignored. A SEARCH matches where its lines equal the program's, line
//! numbers copied with them (`43| `) and trailing spaces aside; failing that, leading spaces
//! aside too (applang ignores indentation, so the replacement keeps its own). It must match
//! exactly one place.

use crate::ai::{fenced, num, numbered};

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

/// What `reply` holds; `cut` when it ran out of room (`finish_reason` length).
pub fn read(reply: &str, cut: bool) -> Reply {
    let reply: String = reply.chars().filter(|&c| c != '\r').collect();
    match fenced(&reply) {
        Some((src, closed)) if !marked(src) => {
            return if cut && !closed { Reply::Cut } else { Reply::Program(src.into()) };
        }
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
