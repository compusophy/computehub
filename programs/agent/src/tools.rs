//! The tools the model calls ([`TOOLS`], in OpenAI's function form) and what each does: read,
//! write and edit files, list a folder, search under one, run a command line in the OS's shell,
//! check an applang app, and hand over applang's guide. Each shows the person a line of what it
//! does (`● name what`) and one of how it went (`⎿ ...`); writes and edits show what changes
//! first. Writes, edits and commands wait for the person's yes; but for a line that only reads
//! ([`reads`]), which runs unasked, and a write over a file or a line that may remove, move or
//! overwrite ([`risky`]), which ask each time. No file tool reads or writes a device (/dev).

use coder::ai::{put_clip, put_num, shown};
use coder::json::Json;

use crate::{ACCENT, Agent, BOLD, Call, DIM, GREEN, MAX_RESULT, PLAIN, RED, World, quoted, safe};
use crate::{RISK, RUN, WRITE};

/// The tools, in OpenAI's function calling form.
pub const TOOLS: &str = r#"[{"type":"function","function":{"name":"read_file","description":"Read a text file, its lines numbered (as 12| text). A long file comes in parts: pass offset (the first line, from 1) and limit (lines, at most 1000) for more.","parameters":{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":1000}},"required":["path"],"additionalProperties":false}}},{"type":"function","function":{"name":"write_file","description":"Write a whole file, making it and its folders if missing; with append, add to its end instead. To change part of a file, use edit_file.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"},"append":{"type":"boolean"}},"required":["path","content"],"additionalProperties":false}}},{"type":"function","function":{"name":"edit_file","description":"Replace old_text with new_text in a file. old_text must match the file exactly (spaces and line breaks too; copy it from read_file without the line numbers) and only once, unless replace_all.","parameters":{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"replace_all":{"type":"boolean"}},"required":["path","old_text","new_text"],"additionalProperties":false}}},{"type":"function","function":{"name":"list_dir","description":"List a folder: folders end in /, files show their size in bytes.","parameters":{"type":"object","properties":{"path":{"type":"string","description":"Default: the working directory."}},"additionalProperties":false}}},{"type":"function","function":{"name":"search","description":"Find text in the files under a folder (dot folders only when the path is in one): each match as path:line: text, 100 at most.","parameters":{"type":"object","properties":{"text":{"type":"string"},"path":{"type":"string","description":"A folder or file; default: the working directory."},"ignore_case":{"type":"boolean"}},"required":["text"],"additionalProperties":false}}},{"type":"function","function":{"name":"shell","description":"Run one command line in the OS's shell, sh (its commands are in the system prompt): what it wrote. The working directory it leaves stays for the next call and for paths.","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"],"additionalProperties":false}}},{"type":"function","function":{"name":"check_app","description":"Test an applang app (.app): compile it, then run it on 3 seeds of the smoke test (drawn, clicked, ticked, keyed, tapped, typed into) and from its saved states. Says ok, or the first problem with its line.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}},{"type":"function","function":{"name":"applang_guide","description":"The applang language card and example apps: read it before writing or changing an app.","parameters":{"type":"object","properties":{},"additionalProperties":false}}}]"#;

/// The most lines a read gives, by default and at all; the most chars of a line it gives.
const READ_LINES: u64 = 400;
const MAX_LINES: u64 = 1000;
const LINE: usize = 400;
/// The most entries a listing gives, matches a search gives, files it reads (each at most
/// [`SEARCHED`] bytes), and lines of each side of a change shown.
const ENTRIES: usize = 300;
const MATCHES: usize = 100;
const FILES: usize = 2000;
const SEARCHED: usize = 256 << 10;
const SHOWN: usize = 8;
/// The commands that only read: one of them alone runs without asking ([`reads`]).
pub const READS: [&str; 10] =
    ["ls", "cat", "pwd", "cd", "echo", "apps", "history", "whoami", "uname", "help"];
/// The shell's other commands that remove, move and overwrite nothing (but by a `>`): with
/// [`READS`], what the person's always to commands covers of its own ([`risky`]).
pub const MAKES: [&str; 8] = ["mkdir", "touch", "open", "run", "edit", "theme", "clear", "exit"];
/// The programs always covers: /bin's applets that only read files and write their output, run
/// by their name alone from /bin, which only the desktop writes. Any other program (by a path,
/// from elsewhere, `sh`, another `agent`; a `#!wasm` marker may name any) asks each time.
pub const APPLETS: [&str; 3] = ["wc", "rev", "hello"];

/// Runs call `c`: its result for the model, and whether it failed (an `Error:` result).
pub fn run(a: &mut Agent, c: &Call, w: &mut impl World) -> (String, bool) {
    let v = Json::parse(&c.args).filter(|v| matches!(v, Json::Obj(_)));
    let done = match (c.name.as_str(), v) {
        (name, _) if c.cut => Err([
            name,
            ": the arguments were cut off (8 KB at most); do less at \
            once: a long file in parts (write_file, then write_file with append)",
        ]
        .concat()),
        // A tool of no arguments may get none at all.
        ("applang_guide", _) => {
            doing(w, "applang_guide", "");
            Ok(guide())
        }
        (name, None) => Err([name, ": the arguments are not a JSON object"].concat()),
        ("read_file", Some(v)) => read(a, &v, w),
        ("write_file", Some(v)) => write(a, &v, w),
        ("edit_file", Some(v)) => edit(a, &v, w),
        ("list_dir", Some(v)) => list(a, &v, w),
        ("search", Some(v)) => search(a, &v, w),
        ("shell", Some(v)) => shell(a, &v, w),
        ("check_app", Some(v)) => check(a, &v, w),
        (name, _) => Err(["there is no tool ", name].concat()),
    };
    match done {
        Ok(text) => (text, false),
        Err(why) => {
            let mut said = String::from(RED);
            put_clip(&mut said, &safe(why.lines().next().unwrap_or_default()), 200);
            how(w, &[&said, PLAIN].concat());
            (["Error: ", &why].concat(), true)
        }
    }
}

/// Shows `● name what`.
fn doing(w: &mut impl World, name: &str, what: &str) {
    w.say(&[ACCENT, "\u{25cf} ", PLAIN, BOLD, name, PLAIN, " ", &safe(what), "\n"].concat());
}

/// Shows `  ⎿ text`, dim.
fn how(w: &mut impl World, text: &str) {
    w.say(&[DIM, "  \u{23bf} ", PLAIN, DIM, text, PLAIN, "\n"].concat());
}

/// Argument `k`'s text, if it is a string.
fn text(v: &Json, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Argument `k`, which must be a string.
fn need(v: &Json, k: &str) -> Result<String, String> {
    text(v, k).ok_or_else(|| ["the argument ", k, " is missing, or not a string"].concat())
}

/// Argument `k` as a whole number, if it is one.
fn int(v: &Json, k: &str) -> Option<u64> {
    v.get(k)?.text()?.parse().ok()
}

/// Whether argument `k` is true.
fn flag(v: &Json, k: &str) -> bool {
    v.get(k) == Some(&Json::Bool(true))
}

/// `n` and what it counts, `one` or `many` of it: `3 lines`.
fn count(n: usize, one: &str, many: &str) -> String {
    let mut out = String::new();
    put_num(&mut out, n as u64);
    out.push(' ');
    out += if n == 1 { one } else { many };
    out
}

/// Whether `path` (absolute) is in /dev, where a read may wait for ever (the console, the
/// agent's own events) and a write may start a job or draw: no file tool goes there, nor the
/// agent's shell (but to /dev/null, which takes all and holds nothing).
pub fn device(path: &str) -> bool {
    (path == "/dev" || path.starts_with("/dev/")) && path != "/dev/null"
}

/// Err for a path in /dev.
fn no_device(path: &str) -> Result<(), String> {
    match device(path) {
        true => Err([path, ": a device, which no file tool reads or writes"].concat()),
        false => Ok(()),
    }
}

/// The text of the file at `path` (absolute); Err for one that is missing, binary or a device.
fn file(w: &mut impl World, path: &str) -> Result<String, String> {
    no_device(path)?;
    let data = w.read(path).map_err(|why| [&shown(path), ": ", why].concat())?;
    if data.contains(&0) {
        return Err([&shown(path), ": a binary file"].concat());
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

/// Appends lines `from` (1-based) on of `lines`, numbered, each cut to [`LINE`] chars, while
/// under [`MAX_RESULT`] bytes and `limit` lines: the last line given.
fn numbered(out: &mut String, lines: &[&str], from: usize, limit: usize) -> usize {
    let mut last = from - 1;
    for (i, line) in lines.iter().enumerate().skip(from - 1).take(limit) {
        if out.len() > MAX_RESULT {
            break;
        }
        let n = i + 1;
        for width in [10, 100, 1000] {
            if n < width {
                out.push(' ');
            }
        }
        put_num(out, n as u64);
        *out += "| ";
        put_clip(out, line, LINE);
        out.push('\n');
        last = n;
    }
    last
}

fn read(a: &Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let path = a.path(&need(v, "path")?)?;
    doing(w, "read_file", &shown(&path));
    let text = file(w, &path)?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        how(w, "empty");
        return Ok("(an empty file)".into());
    }
    let from = int(v, "offset").unwrap_or(1).max(1) as usize;
    if from > lines.len() {
        return Err(["the file has ", &count(lines.len(), "line", "lines")].concat());
    }
    let limit = int(v, "limit").unwrap_or(READ_LINES).clamp(1, MAX_LINES) as usize;
    let mut out = String::new();
    let last = numbered(&mut out, &lines, from, limit);
    let mut span = String::from("lines ");
    put_num(&mut span, from as u64);
    span.push('-');
    put_num(&mut span, last as u64);
    span += " of ";
    put_num(&mut span, lines.len() as u64);
    how(w, &span);
    if from > 1 || last < lines.len() {
        out += "(";
        out += &span;
        if last < lines.len() {
            out += "; read on with offset ";
            put_num(&mut out, last as u64 + 1);
        }
        out += ")";
    }
    Ok(out)
}

/// The folder `path` is in.
fn parent(path: &str) -> &str {
    path.rfind('/').map_or("/", |i| if i == 0 { "/" } else { &path[..i] })
}

/// Shows up to [`SHOWN`] of `lines`, each after `mark` in `color`, then how many more.
fn show(w: &mut impl World, lines: &str, mark: &str, color: &str) {
    let mut out = String::new();
    for line in lines.lines().take(SHOWN) {
        out += "    ";
        out += color;
        out += mark;
        put_clip(&mut out, &safe(line), 120);
        out += PLAIN;
        out.push('\n');
    }
    let more = lines.lines().count().saturating_sub(SHOWN);
    if more > 0 {
        out += DIM;
        out += "    \u{2026} ";
        out += &count(more, "more line", "more lines");
        out += PLAIN;
        out.push('\n');
    }
    w.say(&out);
}

/// What the person declined, as the model hears it.
fn declined(what: &str) -> Result<String, String> {
    Ok(["The user declined: ", what, ". Do not do it another way; ask what they want instead."]
        .concat())
}

fn write(a: &mut Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let (path, content, append) =
        (a.path(&need(v, "path")?)?, need(v, "content")?, flag(v, "append"));
    let (lines, name) = (count(content.lines().count(), "line", "lines"), shown(&path));
    doing(w, "write_file", &[&name, " (", &lines, ")"].concat());
    no_device(&path)?;
    show(w, &content, "+ ", GREEN);
    // Replacing a file is overwriting it, as `>` does: asked each time, always or not.
    let replaces = !append && w.kind(&path) == Some(false);
    let verb = match (append, replaces) {
        (true, _) => "append to ",
        (_, true) => "replace ",
        _ => "write ",
    };
    let what = [verb, &name, " (", &lines, ")"].concat();
    if !a.allowed(if replaces { RISK } else { WRITE }, &what, w) {
        how(w, "declined");
        return declined(&what);
    }
    let _ = w.mkdir(parent(&path), true);
    w.write(&path, content.as_bytes(), append)
        .map_err(|why| [&shown(&path), ": ", why].concat())?;
    how(w, &[if append { "appended " } else { "wrote " }, &lines].concat());
    let mut out = String::from(if append { "Appended " } else { "Wrote " });
    out += &lines;
    out += " to ";
    out += &name;
    out.push('.');
    Ok(out)
}

/// `text` with the `  12| ` a numbered read puts before each line taken off, if every line has
/// one: a model may copy them.
fn unnumbered(text: &str) -> Option<String> {
    fn strip(l: &str) -> Option<&str> {
        let t = l.trim_start();
        let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
        if rest.len() == t.len() {
            return None;
        }
        rest.strip_prefix("| ").or_else(|| rest.strip_prefix('|'))
    }
    let lines: Option<Vec<&str>> = text.lines().map(strip).collect();
    lines.map(|l| l.join("\n") + if text.ends_with('\n') { "\n" } else { "" })
}

fn edit(a: &mut Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let path = a.path(&need(v, "path")?)?;
    let (mut old, new, all) = (need(v, "old_text")?, need(v, "new_text")?, flag(v, "replace_all"));
    doing(w, "edit_file", &shown(&path));
    if old.is_empty() {
        return Err("old_text is empty; to write a whole file, use write_file".into());
    }
    let text = file(w, &path)?;
    if !text.contains(&old) {
        match unnumbered(&old).filter(|o| !o.is_empty() && text.contains(o.as_str())) {
            Some(o) => old = o,
            None => {
                return Err([
                    "old_text is not in ",
                    &shown(&path),
                    "; read it again and copy the text exactly, spaces and line breaks too",
                ]
                .concat());
            }
        }
    }
    let n = text.matches(&old).count();
    if n > 1 && !all {
        return Err([
            "old_text is in the file ",
            &count(n, "time", "times"),
            "; add lines around it to make it unique, or set replace_all",
        ]
        .concat());
    }
    show(w, &old, "- ", RED);
    show(w, &new, "+ ", GREEN);
    if !a.allowed(WRITE, &["edit ", &shown(&path)].concat(), w) {
        how(w, "declined");
        return declined(&["edit ", &shown(&path)].concat());
    }
    let at = text.find(&old).unwrap_or(0);
    let changed = if all { text.replace(&old, &new) } else { text.replacen(&old, &new, 1) };
    w.write(&path, changed.as_bytes(), false).map_err(|why| [&shown(&path), ": ", why].concat())?;
    how(w, &["replaced ", &count(if all { n } else { 1 }, "place", "places")].concat());
    // The first change and two lines around it, as the file now reads.
    let first = changed[..at].matches('\n').count() + 1;
    let lines: Vec<&str> = changed.lines().collect();
    let from = first.saturating_sub(2).max(1);
    let shown_lines = new.lines().count().max(1) + 4;
    let mut out = ["Edited ", &shown(&path), "; it now reads:\n"].concat();
    numbered(&mut out, &lines, from.min(lines.len().max(1)), shown_lines.min(40));
    Ok(out)
}

fn list(a: &Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let path = a.path(&text(v, "path").unwrap_or_default())?;
    doing(w, "list_dir", &shown(&path));
    let entries = w.list(&path).map_err(|why| [&shown(&path), ": ", why].concat())?;
    let mut out = String::new();
    for e in entries.iter().take(ENTRIES) {
        out += &e.name;
        if e.is_dir {
            out.push('/');
        } else {
            out += "  ";
            put_num(&mut out, e.size);
        }
        out.push('\n');
    }
    if entries.len() > ENTRIES {
        out += "(";
        out += &count(entries.len() - ENTRIES, "more entry", "more entries");
        out += ")\n";
    }
    how(w, &count(entries.len(), "entry", "entries"));
    if entries.is_empty() {
        out += "(empty)";
    }
    Ok(out)
}

fn search(a: &Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let needle = need(v, "text")?;
    let root = a.path(&text(v, "path").unwrap_or_default())?;
    let fold = flag(v, "ignore_case");
    doing(w, "search", &[&quoted(&needle), " in ", &shown(&root)].concat());
    if needle.is_empty() {
        return Err("the text to find is empty".into());
    }
    let low = |s: &str| if fold { s.to_ascii_lowercase() } else { s.to_string() };
    let want = low(&needle);
    let (mut out, mut hits, mut files, mut stack) = (String::new(), 0, 0, vec![root.clone()]);
    while let Some(path) = stack.pop() {
        if device(&path) || hits >= MATCHES || files >= FILES {
            continue;
        }
        match w.kind(&path) {
            Some(true) => {
                let entries = w.list(&path).unwrap_or_default();
                for e in entries.iter().rev().filter(|e| !e.name.starts_with('.')) {
                    stack.push([path.trim_end_matches('/'), "/", &e.name].concat());
                }
            }
            Some(false) => {
                files += 1;
                let Ok(data) = w.read(&path) else { continue };
                if data.len() > SEARCHED || data.contains(&0) {
                    continue;
                }
                let text = String::from_utf8_lossy(&data);
                for (i, line) in text.lines().enumerate().filter(|l| low(l.1).contains(&want)) {
                    if hits >= MATCHES {
                        break;
                    }
                    hits += 1;
                    out += &shown(&path);
                    out.push(':');
                    put_num(&mut out, i as u64 + 1);
                    out += ": ";
                    put_clip(&mut out, line.trim(), 200);
                    out.push('\n');
                }
            }
            None if path == root => return Err([&shown(&root), ": not found"].concat()),
            None => {}
        }
    }
    how(w, &count(hits, "match", "matches"));
    if hits >= MATCHES {
        out += "(the first 100; search a narrower folder for more)";
    }
    Ok(if out.is_empty() { "No matches.".into() } else { out })
}

/// Whether `line` only reads, so it runs unasked: one of [`READS`] alone, as the shell reads
/// the line (joined to no other by `;`, `&&`, `||` or `|`, writing no file), naming no device.
pub fn reads(line: &str) -> bool {
    let alone = |c: &sh::Command| match (&c.0[..], &c.1) {
        ([words], None) => words.first().is_some_and(|w| READS.contains(&w.as_str())),
        _ => false,
    };
    matches!(sh::commands(line).as_deref(), Some([c]) if alone(c)) && !line.contains("/dev")
}

/// Whether `line`, as the shell reads it, may remove, move or overwrite, or run what could
/// unseen: a `>` that does not append, or a stage that runs anything but one of the shell's
/// own commands of [`READS`] and [`MAKES`] or an applet of [`APPLETS`] that `sys` has in /bin
/// (so `rm`, `mv`, `/bin/sh/`, `./x`, a pattern, which could be any). Such a line asks each time.
pub fn risky(line: &str, sys: &mut dyn sh::Sys) -> bool {
    // A command's name is one only as a command's first word; elsewhere the shell refuses it.
    let mut kept = |w: &String| {
        READS.contains(&w.as_str())
            || MAKES.contains(&w.as_str())
            || APPLETS.contains(&w.as_str()) && sys.kind(&["/bin/", w].concat()) == Some(false)
    };
    let mut risk = |(stages, out): &sh::Command| {
        out.as_ref().is_some_and(|o| !o.1)
            || !stages.iter().filter_map(|s| s.first()).all(&mut kept)
    };
    sh::commands(line).unwrap_or_default().iter().any(&mut risk)
}

fn shell(a: &mut Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let line = need(v, "command")?;
    doing(w, "shell", &line);
    if line.chars().any(|c| c.is_control() && c != '\t') {
        return Err("one command line at a time, with no control characters".into());
    }
    let (reads, kind) = (reads(&line), if risky(&line, w) { RISK } else { RUN });
    if !reads && !a.allowed(kind, &["run ", &line].concat(), w) {
        how(w, "declined");
        return declined(&["run ", &line].concat());
    }
    let before = a.cwd.clone();
    let mut out = a.sh(&line, !reads, w);
    let lines = out.lines().count();
    let first_line = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output");
    let mut said = String::new();
    put_clip(&mut said, &safe(first_line), 100);
    if lines > 1 {
        said += " \u{2026} (";
        said += &count(lines, "line", "lines");
        said += ")";
    }
    how(w, &said);
    if out.len() > MAX_RESULT {
        let mut cut = String::new();
        put_clip(&mut cut, &out, MAX_RESULT);
        out = cut + "\n(the rest is cut; write it to a file and read that in parts)";
    }
    if out.trim().is_empty() {
        out = "(no output)".into();
    }
    if a.cwd != before {
        out += "\n(The working directory is now ";
        out += &shown(&a.cwd);
        out.push(')');
    }
    Ok(out)
}

fn check(a: &Agent, v: &Json, w: &mut impl World) -> Result<String, String> {
    let path = a.path(&need(v, "path")?)?;
    doing(w, "check_app", &shown(&path));
    if !path.ends_with(".app") {
        return Err([&shown(&path), " is not an .app file"].concat());
    }
    let src = file(w, &path)?;
    let saved = w.read(&coder::ai::state_path(&path)).unwrap_or_default();
    match coder::ai::fault(&src, &String::from_utf8_lossy(&saved), 3) {
        None => {
            how(w, &[GREEN, "ok", PLAIN].concat());
            Ok("ok: it compiles, and runs clean on 3 seeds of the smoke test and from its saved \
                states."
                .into())
        }
        Some(f) => Err(f.account),
    }
}

/// applang's card, then how an app begins (what it does, its icon) and the example apps.
pub fn guide() -> String {
    let mut out = String::from(applang::REFERENCE);
    out += "\n";
    out += coder::ai::HONEST;
    out += coder::ai::ICON;
    for (i, (ask, src)) in applang::SHOTS.iter().enumerate() {
        out += if i == 0 { "\"" } else { "\nAnd for \"" };
        out += ask;
        out += "\":\n```app\n";
        out += src;
        out += "```\n";
    }
    out
}
