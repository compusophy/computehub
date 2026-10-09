//! A helper's unified diff, read as the edit blocks it means. Small models asked for SEARCH/REPLACE
//! blocks often answer with a ```diff instead (milestone 1: 120 of the base 3B's 127 unreadable
//! turns, 131 of the fine-tune's 162), in three shapes: bare `-`/`+` lines, a git diff with its
//! headers and `@@` hunks, and a hunk whose lines carry the program's numbers (`35|-  x;`). Each
//! hunk becomes one block: its context and `-` lines are what to find, its context and `+` lines
//! what to put there. `coder` itself is untouched (its files are in the IQ verifier's hash): the
//! blocks are handed to `coder::edits::read` as text.

/// The edit blocks `reply`'s diff fences mean, as SEARCH/REPLACE text; `None` when it has no
/// ```diff or ```patch fence, or no hunk in one that changes a line and has a line to find.
pub fn blocks(reply: &str) -> Option<String> {
    let mut out = String::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let fence = line.trim();
        if fence != "```diff" && fence != "```patch" {
            continue;
        }
        let mut hunk: Vec<&str> = Vec::new();
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            if l.starts_with("@@") {
                put(&mut out, &hunk);
                hunk.clear();
            } else if !header(l) {
                hunk.push(l);
            }
        }
        put(&mut out, &hunk);
    }
    (!out.is_empty()).then_some(out)
}

/// A git diff's header line: `diff --git`, `index`, `---`/`+++` file names.
fn header(l: &str) -> bool {
    l.starts_with("diff --git")
        || l.starts_with("index ")
        || l.starts_with("--- ")
        || l.starts_with("+++ ")
}

/// `l` without a line number copied from a numbered program (`  35|`), and the rest.
fn unnumbered(l: &str) -> &str {
    let t = l.trim_start();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    match t.get(digits..).and_then(|r| r.strip_prefix('|')) {
        Some(rest) if digits > 0 => rest,
        _ => l,
    }
}

/// One hunk as a SEARCH/REPLACE block, if it changes a line and has a line to find.
fn put(out: &mut String, hunk: &[&str]) {
    let (mut search, mut replace, mut changed) = (Vec::new(), Vec::new(), false);
    for l in hunk {
        let l = unnumbered(l);
        if let Some(gone) = l.strip_prefix('-') {
            changed = true;
            search.push(gone);
        } else if let Some(new) = l.strip_prefix('+') {
            changed = true;
            replace.push(new);
        } else {
            let same = l.strip_prefix(' ').unwrap_or(l);
            search.push(same);
            replace.push(same);
        }
    }
    let trim = |v: &mut Vec<&str>| {
        while v.first().is_some_and(|l| l.trim().is_empty()) {
            v.remove(0);
        }
        while v.last().is_some_and(|l| l.trim().is_empty()) {
            v.pop();
        }
    };
    trim(&mut search);
    trim(&mut replace);
    if !changed || search.is_empty() {
        return;
    }
    // Bare `- x` / `+ x` lines (no context, a space after each mark): the space is the mark's.
    let bare = hunk.iter().all(|l| {
        let l = unnumbered(l);
        l.trim().is_empty() || l.starts_with("- ") || l.starts_with("+ ")
    });
    if bare {
        for l in search.iter_mut().chain(replace.iter_mut()) {
            *l = l.strip_prefix(' ').unwrap_or(l);
        }
    }
    out.push_str("<<<<<<< SEARCH\n");
    for l in &search {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("=======\n");
    for l in &replace {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(">>>>>>> REPLACE\n");
}
