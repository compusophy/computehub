//! The text filters: `rev [file...]` and `wc [-lwmc] [file...]`. Each reads the files it names
//! in order (`-` is the input), or its input when it names none, as POSIX's do. A file that
//! cannot be read is said (`wc: nope: no such file or directory`) and the rest still are; the
//! status is then 1, and 2 for an option it does not know.

use std::io::{self, ErrorKind, Read, Write};

/// Reads the file at a path: `std::fs::read` (on compusophyOS WASI's view of the desktop's
/// files; relative paths from the job's directory, made sure of in `main`'s `here`), a table
/// in tests.
pub type Files<'a> = &'a mut dyn FnMut(&str) -> io::Result<Vec<u8>>;

/// `wc`'s counts in the order it prints them, and their flags.
const COUNTS: &str = "lwmc";

/// Runs `rev` or `wc` (`name`) on `words`, its arguments: flags first (`--` ends them), then
/// the files. `wc` prints a line for each: its lines (line breaks), words, bytes and name, or
/// only the counts its flags pick (characters with -m), in that order; a `total` line after
/// several; no name for its input when it names no file. `rev` prints each line reversed.
pub fn filter(
    name: &str,
    words: &[String],
    inp: &mut dyn Read,
    files: Files<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let mut pick = 0;
    let mut names: Vec<&str> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if w == "--" || w.len() < 2 || !w.starts_with('-') {
            let from = i + usize::from(w == "--");
            names.extend(words.get(from..).unwrap_or_default().iter().map(String::as_str));
            break;
        }
        for c in w.chars().skip(1) {
            match COUNTS.chars().position(|f| f == c) {
                Some(bit) if name == "wc" => pick |= 1 << bit,
                _ => {
                    // A long option (`--lines`) is said whole. A failed write to stderr leaves
                    // nothing to report it on.
                    let bad = if w.starts_with("--") { w.clone() } else { format!("-{c}") };
                    let _ = writeln!(err, "{name}: unknown option {bad}");
                    return 2;
                }
            }
        }
    }
    // Lines, words and bytes unless the flags pick.
    let pick = if pick == 0 { 0b1011 } else { pick };
    let (several, mut total, mut status) = (names.len() > 1, [0; 4], 0);
    // The input, unnamed, when no file is named.
    let paths = if names.is_empty() { vec![None] } else { names.into_iter().map(Some).collect() };
    for path in paths {
        let data = match path {
            None | Some("-") => {
                let mut data = Vec::new();
                inp.read_to_end(&mut data).map(|_| data)
            }
            Some(path) => files(path),
        };
        let data = match (data, path) {
            (Ok(data), _) => data,
            (Err(e), None) => {
                let _ = writeln!(err, "{name}: {}", why(&e));
                return 1;
            }
            (Err(e), Some(path)) => {
                let _ = writeln!(err, "{name}: {path}: {}", why(&e));
                status = 1;
                continue;
            }
        };
        let said = if name == "rev" {
            let text = String::from_utf8_lossy(&data);
            text.lines().map(|l| l.chars().rev().collect::<String>() + "\n").collect()
        } else {
            let counts = count(&data);
            total.iter_mut().zip(counts).for_each(|(t, n)| *t += n);
            line(&counts, pick, path)
        };
        if let Err(e) = out.write_all(said.as_bytes()) {
            let _ = writeln!(err, "{name}: {e}");
            return 1;
        }
    }
    let said =
        if several && name == "wc" { line(&total, pick, Some("total")) } else { String::new() };
    match out.write_all(said.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => status,
        Err(e) => {
            let _ = writeln!(err, "{name}: {e}");
            1
        }
    }
}

/// `data`'s line breaks, words (runs of anything but blanks), characters (UTF-8's lead bytes)
/// and bytes.
fn count(data: &[u8]) -> [u64; 4] {
    let blank = |b: &u8| matches!(b, b' ' | b'\t'..=b'\r');
    let words = data.split(blank).filter(|w| !w.is_empty()).count();
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    let chars = data.iter().filter(|&&b| b & 0xC0 != 0x80).count();
    [lines, words, chars, data.len()].map(|n| n as u64)
}

/// The counts `pick` picks (bit `i` for [`COUNTS`]'s `i`th), separated by spaces, then `path`
/// (if any) and a line break.
fn line(counts: &[u64; 4], pick: u32, path: Option<&str>) -> String {
    let mut parts: Vec<String> =
        (0..4).filter(|i| pick & (1 << i) != 0).map(|i| counts[i].to_string()).collect();
    parts.extend(path.map(String::from));
    parts.join(" ") + "\n"
}

/// Why a file could not be read, as the shell words it.
fn why(e: &io::Error) -> String {
    match e.kind() {
        ErrorKind::NotFound => "no such file or directory".into(),
        ErrorKind::IsADirectory => "is a directory".into(),
        ErrorKind::NotADirectory => "not a directory".into(),
        ErrorKind::PermissionDenied => "permission denied".into(),
        _ => e.to_string(),
    }
}
