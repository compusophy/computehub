//! What every program that asks the free AI shares (the Assistant and Studio): the request body
//! and coded failures; and what making apps takes: the program in a reply and what is wrong
//! with it (as the person reads it and as the model fixing it does), file names, the numbered
//! program and the corpus line.

use applang::Class;

use crate::json::put;

/// The guest's home, where made apps and the corpus go.
pub const HOME: &str = concat!("/home/", "guest");
/// The fine-tuning corpus: a JSON line per app that compiled.
pub const CORPUS: &str = concat!("/home/", "guest", "/.ai/corpus.jsonl");
/// The model until the desktop names one.
pub const DEFAULT_MODEL: &str = "zai/glm-5.3";
/// The most bytes of a reply kept.
pub const MAX_REPLY: usize = 64 * 1024;
/// The most bytes a request's body takes: the free AI (`api/ai.mjs`) takes 96 KiB at most, so
/// this leaves room.
pub const MAX_BODY: usize = 80 * 1024;
/// What a prompt that makes apps asks of the program, before its examples
/// ([`applang::SHOTS`]): to say what it is, and what of the ask applang could not do.
pub const HONEST: &str = "Begin the program with a // comment of one or two short lines: what the \
                          app does and how to use it. If applang cannot do part of what was asked, \
                          make the closest app it can, and end that comment with \"Without:\" and \
                          what it leaves out. For example, for ";
/// What a make asks for after a reply that ran out of room before its program ended.
pub const SHORTER: &str = "Your reply ran out of room before the program ended. Write the same \
                           app, shorter: under 100 lines, extras left out (named after \
                           \"Without:\" in its first comment). Reply with the complete program in \
                           one app block.";
/// How a reply ends that ran out of room before it ended.
pub const ROOM: &str = "E0907 the AI ran out of room before its reply ended; ask for less";
/// What a fix is told of a program that faults only from the states it keeps.
const KEPT: &str = "Saved states come back as they were kept, also after the program changes: a \
                    list keeps its length, and a saved state the program adds starts as \
                    declared. The program must run from them too.";
/// Past this many steps on its first seed, a smoke test is not run on more seeds.
const SEED_STEPS: u64 = 5_000_000;

/// A streamed chat-completions request for `model` with the JSON members `options` (each with
/// a leading comma), the `system` prompt and then the `user` message.
pub fn chat(model: &str, options: &str, system: &str, user: &str) -> String {
    let mut out = String::from("{\"model\":");
    put(&mut out, model);
    out += ",\"stream\":true,\"stream_options\":{\"include_usage\":true}";
    out += options;
    out += ",\"messages\":[{\"role\":\"system\",\"content\":";
    put(&mut out, system);
    out += "},{\"role\":\"user\",\"content\":";
    put(&mut out, user);
    out += "}]}";
    out
}

/// What went wrong with a request that ended with HTTP `status` (0: none made), the host's
/// `error` and the AI service's own message `said`: its code (901 to 905; 0 when it was
/// cancelled) and what to say.
pub fn failure(status: u16, error: &str, said: &str) -> Option<(u16, String)> {
    let (code, what) = match status {
        _ if error == "cancelled" => return Some((0, "Stopped.".into())),
        200..=299 if error.is_empty() && said.is_empty() => return None,
        200..=299 if error.is_empty() => (5, "the AI stopped with an error"),
        401 | 403 => (1, "the AI service refused the request"),
        402 => (2, "the free AI is out of credit for now, try again later"),
        429 => (3, "the free AI is busy, try again in a minute"),
        400 | 404 => (4, "the model was not found or refused the request"),
        503 => (5, "the AI is not available right now"),
        _ if !error.is_empty() => (5, error),
        _ => (5, "couldn't reach the AI service"),
    };
    let said = if said.is_empty() { String::new() } else { [": ", &clip(said, 300)].concat() };
    let code = 900 + code;
    Some((code, [&ecode(code), " ", what, &said].concat()))
}

/// Each fenced block in `reply` whose info string is `app`, in order: its text and whether it
/// was closed. Only the last may be open; it runs to the reply's end.
pub fn blocks(reply: &str) -> Vec<(&str, bool)> {
    let (mut at, mut start, mut out) = (0, None, Vec::new());
    for line in reply.split_inclusive('\n') {
        let fence = line.trim().strip_prefix("```");
        match start {
            None if fence.map(str::trim) == Some("app") => start = Some(at + line.len()),
            Some(s) if fence.is_some() => {
                out.push((&reply[s..at], true));
                start = None;
            }
            _ => {}
        }
        at += line.len();
    }
    out.extend(start.map(|s| (&reply[s..], false)));
    out
}

/// `src` with each line numbered as a fix sees it (`  7| label n;`), the first `first`.
pub fn numbered(src: &str, first: usize) -> String {
    let mut out = String::new();
    put_numbered(&mut out, src, first);
    out
}

/// Appends [`numbered`]`(src, first)` to `out`.
pub fn put_numbered(out: &mut String, src: &str, first: usize) {
    for (n, line) in (first..).zip(src.lines()) {
        for width in [10, 100] {
            if n < width {
                out.push(' ');
            }
        }
        put_num(out, n as u64);
        *out += "| ";
        *out += line;
        out.push('\n');
    }
}

/// A slug for `src`'s file: its first label's text, lowercase ASCII words
/// joined by `-`, at most 32 bytes; "app" if that leaves nothing.
pub fn slug(src: &str) -> String {
    let toks = applang::highlight(src);
    let text = |i: usize| &src[toks[i].0.start..toks[i].0.end];
    let label = (1..toks.len()).find(|&i| {
        (toks[i - 1].1, text(i - 1), toks[i].1) == (Class::Keyword, "label", Class::Str)
    });
    let mut out = String::new();
    for b in label.map_or("", text).bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(char::from(b.to_ascii_lowercase()));
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(32);
    let out = out.trim_end_matches('-');
    if out.is_empty() { "app".into() } else { out.into() }
}

/// What `src` says it is: its leading comments, markers and spaces trimmed, joined, at most 400
/// bytes (the "Without:" a long one ends with may be cut); "" if none.
pub fn about(src: &str) -> String {
    let lead = applang::highlight(src).into_iter().take_while(|(_, c)| *c == Class::Comment);
    let trim: &[char] = &['/', '*', ' ', '\t', '\r', '\n'];
    let text: Vec<&str> = lead.map(|(s, _)| src[s.start..s.end].trim_matches(trim)).collect();
    clip(&text.join(" "), 400)
}

/// Where a made app named `slug` goes: the first of `~/apps/<slug>.app`, `<slug>-2.app`, ... (to
/// `-999`) that nothing is at, by `exists`, so a made app never replaces a file nor starts from
/// the saved states another app of that name left ([`state_path`]).
pub fn free_path(slug: &str, exists: &mut dyn FnMut(&str) -> bool) -> Option<String> {
    let name = |n: u32| match n {
        1 => [HOME, "/apps/", slug, ".app"].concat(),
        n => [HOME, "/apps/", slug, "-", &num(n.into()), ".app"].concat(),
    };
    (1..1000).map(name).find(|p| !exists(p) && !exists(&state_path(p)))
}

/// The file the saved states of the app at `path` go to: `~/.appdata/<name>.state`, by its
/// file's name ("" for none).
pub fn state_path(path: &str) -> String {
    let name = &path[path.rfind('/').map_or(0, |i| i + 1)..];
    let stem = name.strip_suffix(".app").unwrap_or(name);
    if stem.is_empty() { String::new() } else { [HOME, "/.appdata/", stem, ".state"].concat() }
}

/// What is wrong with a program: its diagnostic, the problem as a person reads it ([`problem`]),
/// whether it compiles at all, and the account a model fixing it gets.
#[derive(Clone, Debug)]
pub struct Fault {
    pub diag: applang::Diag,
    pub said: String,
    pub compiles: bool,
    pub account: String,
}

/// What is wrong with `src`, if anything: that it does not compile, faults when it runs
/// ([`applang::smoke`] with `random` seeded 1 to `seeds`: rendered, clicked, ticked, keyed,
/// tapped and typed into; a seed that spends over 5,000,000 steps is the last), or faults as it
/// will really start, from `saved` (what its saved states' file holds; seed 1). The account says
/// the problem with its line and a caret under it, the rule its code says was broken, what was
/// being done when it came and what came before (and that it started from the states it keeps),
/// and that every occurrence wants fixing.
pub fn fault(src: &str, saved: &str, seeds: u8) -> Option<Fault> {
    let (what, d, when) = match applang::compile(src) {
        Err(d) => ("did not compile", d, String::new()),
        Ok(_) => {
            let (f, kept) = smoked(src, saved, seeds)?;
            let (what, from, rule) = match kept {
                true => (
                    "faults when it runs from its saved states",
                    ", started from the states it keeps between runs",
                    KEPT,
                ),
                false => ("faults when it runs", "", ""),
            };
            let mut when = String::from("\nIt came while ");
            when += &f.during;
            when += from;
            for (i, b) in f.before.iter().enumerate() {
                when += if i == 0 { ", after " } else { ", " };
                when += b;
            }
            when += ".\n";
            when += rule;
            (what, f.diag, when)
        }
    };
    // "line 3, col 5", then the line and its carets.
    let snip = d.span.and_then(|s| lang::diag::render_snippet(src, s)).unwrap_or_default();
    let (at, line) = snip.split_once('\n').unwrap_or(("", ""));
    let code = d.code.unwrap_or_default();
    let mut account = String::from("Your program ");
    account += what;
    account += ": ";
    put_code(&mut account, code);
    if !at.is_empty() {
        account += " at ";
        account += at;
    }
    account += ": ";
    put_clip(&mut account, &d.message, 1024);
    account.push('\n');
    put_clip(&mut account, line, 4096);
    account += "\nRule: ";
    account += applang::rule(code);
    put_clip(&mut account, &when, 1024);
    account += "\nThe same mistake may be on other lines too: fix every occurrence.";
    // Only a program that did not compile has no account of when.
    let compiles = !when.is_empty();
    Some(Fault { said: problem(&d, src), diag: d, compiles, account })
}

/// The first fault a smoke test finds in `src` (which compiles) on seeds 1 to `seeds`, else
/// started from `saved` (and `true`).
fn smoked(src: &str, saved: &str, seeds: u8) -> Option<(applang::Fault, bool)> {
    for seed in 1..=u64::from(seeds.max(1)) {
        let s = applang::smoke(applang::compile(src).ok()?, seed);
        if let Some(f) = s.fault {
            return Some((f, false));
        }
        if s.spent > SEED_STEPS {
            break;
        }
    }
    let s = applang::smoke_from(applang::compile(src).ok()?, 1, saved).fault;
    s.filter(|_| !saved.trim().is_empty()).map(|f| (f, true))
}

/// The 1-based line and column (in chars) of byte `at` of `src`.
pub fn line_col(src: &str, at: usize) -> (usize, usize) {
    let before = src.get(..at).unwrap_or(src);
    let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    (before.matches('\n').count() + 1, line.chars().count() + 1)
}

/// A diagnostic as `E0101 3:5 message`: line and column (in chars) from 1.
pub fn problem(d: &applang::Diag, src: &str) -> String {
    let mut out = String::new();
    match d.code {
        Some(c) => put_code(&mut out, c),
        None => out += "error",
    }
    if let Some(s) = d.span {
        let (line, col) = line_col(src, s.start);
        out.push(' ');
        put_num(&mut out, line as u64);
        out.push(':');
        put_num(&mut out, col as u64);
    }
    out.push(' ');
    put_clip(&mut out, &d.message, 1024);
    out
}

/// `s` cut to at most `max` bytes on a char boundary, ending in `…` if cut.
pub fn clip(s: &str, max: usize) -> String {
    let mut out = String::new();
    put_clip(&mut out, s, max);
    out
}

/// Appends [`clip`]`(s, max)` to `out`.
pub fn put_clip(out: &mut String, s: &str, max: usize) {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    *out += &s[..end];
    if end < s.len() {
        out.push('\u{2026}');
    }
}

/// The corpus line for `program`, made for `prompt` by `model` in `attempts` requests, changing
/// `base` (none: "").
pub fn corpus_line(prompt: &str, program: &str, base: &str, attempts: u32, model: &str) -> String {
    let mut out = String::from("{\"prompt\":");
    put(&mut out, prompt);
    out += ",\"program\":";
    put(&mut out, program);
    out += ",\"attempts\":";
    put_num(&mut out, attempts.into());
    out += ",\"model\":";
    put(&mut out, model);
    if !base.is_empty() {
        out += ",\"base\":";
        put(&mut out, base);
    }
    out += "}\n";
    out
}

/// `n` in decimal, without the formatting machinery.
pub fn num(n: u64) -> String {
    let mut out = String::new();
    put_num(&mut out, n);
    out
}

/// Appends `n` to `out` in decimal.
pub fn put_num(out: &mut String, n: u64) {
    if n >= 10 {
        put_num(out, n / 10);
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

/// A diagnostic's code as it is written: `E0215`.
pub fn ecode(code: u16) -> String {
    let mut out = String::new();
    put_code(&mut out, code);
    out
}

/// Appends [`ecode`]`(code)` to `out`.
pub fn put_code(out: &mut String, code: u16) {
    out.push('E');
    for d in [1000, 100, 10, 1] {
        out.push(char::from(b'0' + (code / d % 10) as u8));
    }
}

/// `path` as a person reads it: the home as `~`.
pub fn shown(path: &str) -> String {
    path.strip_prefix(HOME).map_or_else(|| path.into(), |rest| ["~", rest].concat())
}
