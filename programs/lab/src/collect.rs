//! Every applang source in the repo, found where programs live: whole `.app` files, ```app
//! blocks in text files (the coder's recorded replies), and Rust string literals (the tests'
//! programs, and ```app blocks inside them), but in the folders [`SKIP`] names. Whether each is
//! a program is the compiler's call ([`crate::corpus`]); here everything that might be is found.

use std::fs;
use std::path::Path;

/// Where a candidate was found (its path from the repo's root, with `/`, and its line) and its
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// The directories searched, from the repo's root.
pub const ROOTS: [&str; 3] = ["crates", "programs", "tools"];
/// Never searched: build output; this lab, tiny and teach, whose own tests hold programs; and the
/// evals (Suite 1's answer keys; the IQ suite's, kept in `evals/`, and its tests' programs): a
/// model is measured on them, never trained on them.
#[rustfmt::skip]
pub const SKIP: [&str; 8] = [
    "target", "dist", "programs/lab", "programs/tiny", "programs/makes", "programs/evals",
    "programs/iq", "programs/teach",
];

/// Every candidate under `root`'s [`ROOTS`], in path order.
pub fn find(root: &Path) -> Vec<Found> {
    let mut files = Vec::new();
    for r in ROOTS {
        walk(root, r, &mut files);
    }
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = fs::read_to_string(root.join(&path)) else { continue };
        let text = text.replace("\r\n", "\n");
        let mut put = |line: usize, text: String| {
            out.push(Found { path: path.clone(), line, text });
        };
        match path.rsplit('.').next() {
            Some("app") => put(1, text),
            Some("txt" | "md") => blocks(&text).into_iter().for_each(|(l, b)| put(l, b)),
            Some("rs") => {
                for (line, lit) in literals(&text) {
                    match lit.contains("```app") {
                        true => blocks(&lit).into_iter().for_each(|(l, b)| put(line + l - 1, b)),
                        false => put(line, lit),
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The files under `dir` (from `root`) that may hold programs, as paths from `root`.
fn walk(root: &Path, dir: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(root.join(dir)) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = [dir, "/", &name].concat();
        if name.starts_with('.') || SKIP.iter().any(|s| path == *s || name == *s) {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(root, &path, out),
            Ok(t) if t.is_file() => {
                if matches!(name.rsplit('.').next(), Some("app" | "rs" | "txt" | "md")) {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

/// The ```app blocks of `text` and the line each program starts on (from 1): from a line that
/// is ```app (spaces after it allowed) to the next line starting ```, or the end.
pub fn blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        match &mut open {
            None if line.trim_end() == "```app" => open = Some((i + 2, String::new())),
            None => {}
            Some(_) if line.starts_with("```") => out.extend(open.take()),
            Some((_, body)) => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    out.extend(open);
    out
}

/// Each string literal in Rust source `src` (plain, raw, byte and raw byte), unescaped, and the
/// line it starts on. Comments, char literals and lifetimes are passed over.
pub fn literals(src: &str) -> Vec<(usize, String)> {
    let b = src.as_bytes();
    let (mut out, mut i, mut line) = (Vec::new(), 0, 1);
    while i < b.len() {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if next == Some(b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if next == Some(b'*') => {
                let mut depth = 0;
                while i < b.len() {
                    match (b[i], b.get(i + 1)) {
                        (b'/', Some(b'*')) => (depth, i) = (depth + 1, i + 2),
                        (b'*', Some(b'/')) => (depth, i) = (depth - 1, i + 2),
                        (c, _) => (line, i) = (line + usize::from(c == b'\n'), i + 1),
                    }
                    if depth == 0 {
                        break;
                    }
                }
            }
            b'\'' => i = past_char(b, i),
            b'"' => {
                let (text, end) = plain(b, i + 1);
                out.push((line, text));
                line += b[i..end].iter().filter(|&&c| c == b'\n').count();
                i = end;
            }
            c if c == b'_' || c.is_ascii_alphanumeric() => {
                let start = i;
                while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                // r"..", r#".."#, br"..": a raw string (b".." is read as a plain one).
                let raw = matches!(&b[start..i], b"r" | b"br");
                if let Some((text, end)) = raw.then(|| raw_str(b, i)).flatten() {
                    out.push((line, text));
                    line += b[i..end].iter().filter(|&&c| c == b'\n').count();
                    i = end;
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Past a char literal (`'a'`, `'\n'`, `'\u{e9}'`, `'é'`) or a lifetime's quote at `i`.
fn past_char(b: &[u8], i: usize) -> usize {
    if b.get(i + 1) == Some(&b'\\') {
        let mut j = i + 3;
        while j < b.len() && b[j] != b'\'' && b[j] != b'\n' {
            j += 1;
        }
        return j + 1;
    }
    let len = match b.get(i + 1) {
        Some(&c) if c >= 0xf0 => 4,
        Some(&c) if c >= 0xe0 => 3,
        Some(&c) if c >= 0xc0 => 2,
        _ => 1,
    };
    if b.get(i + 1 + len) == Some(&b'\'') { i + 2 + len } else { i + 1 }
}

/// A plain string's text from `start` (just past its quote), unescaped, and where it ends (past
/// its closing quote).
fn plain(b: &[u8], start: usize) -> (String, usize) {
    let (mut out, mut i) = (Vec::new(), start);
    while i < b.len() && b[i] != b'"' {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let e = b.get(i + 1).copied().unwrap_or(b'\\');
        i += 2;
        match e {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'0' => out.push(0),
            b'x' => {
                let hex = std::str::from_utf8(b.get(i..i + 2).unwrap_or(b"")).unwrap_or("");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 2;
            }
            b'u' => {
                let end = b[i..].iter().position(|&c| c == b'}').map_or(b.len(), |p| i + p);
                let hex = std::str::from_utf8(b.get(i + 1..end).unwrap_or(b"")).unwrap_or("");
                let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).unwrap_or('?');
                out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                i = end + 1;
            }
            // A line continuation: the newline and the next line's leading whitespace go.
            b'\n' => {
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            e => out.push(e),
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), (i + 1).min(b.len()))
}

/// A raw string's text from `at` (its hashes, then its quote), and where it ends.
fn raw_str(b: &[u8], at: usize) -> Option<(String, usize)> {
    let hashes = b[at..].iter().take_while(|&&c| c == b'#').count();
    if b.get(at + hashes) != Some(&b'"') {
        return None;
    }
    let start = at + hashes + 1;
    let mut close = vec![b'"'];
    close.extend(std::iter::repeat_n(b'#', hashes));
    let len = b[start..].windows(close.len()).position(|w| w == close.as_slice())?;
    let text = String::from_utf8_lossy(&b[start..start + len]).into_owned();
    Some((text, start + len + close.len()))
}
