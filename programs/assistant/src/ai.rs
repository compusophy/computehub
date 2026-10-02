//! What every program that makes apps with the AI shares (the Assistant and Studio): the system
//! prompt and request body, coded failures, the program in a reply and what is wrong with it (as
//! the person reads it and as the model fixing it does), file names and the corpus line.

use applang::{App, Class, Event, Limits, Node, Value};
use uiwire::Style;

use crate::json::quote;

/// The guest's home, where made apps and the corpus go.
pub const HOME: &str = concat!("/home/", "guest");
/// The fine-tuning corpus: a JSON line per app that compiled.
pub const CORPUS: &str = concat!("/home/", "guest", "/.ai/corpus.jsonl");
/// The model until the desktop names one.
pub const DEFAULT_MODEL: &str = "zai/glm-5.3";
/// How many times a make goes back to the model: with a program that does not compile or faults
/// when it first renders, after a reply that ran out of room, or one without a program.
pub const RETRIES: u32 = 3;
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
pub const SHORTER: &str = "Your reply ran out of room before the program ended. Write the same app, \
                           shorter: under 100 lines, extras left out (named after \"Without:\" in \
                           its first comment). Reply with the complete program in one app block.";
/// How a make ends that kept running out of room with its program.
pub const ROOM: &str = "E0907 the AI ran out of room before its reply ended; ask for less";
/// How one ends whose room went to thinking, before the program (a provider may think on past
/// the free AI's budget): asking for less would not help.
pub const THOUGHT: &str =
    "E0907 the AI ran out of room thinking, before it wrote the program; try again";
/// What a fix is told of a program that faults only from the states it keeps.
const KEPT: &str = "Saved states come back as they were kept, also after the program changes: a \
                    list keeps its length, and a saved state the program adds starts as \
                    declared. The program must run from them too.";

/// A system prompt: `intro`, applang's card, `rules`, then [`HONEST`] and the example replies
/// ([`applang::SHOTS`], each tested to compile and pass the smoke test).
pub fn system(intro: &str, rules: &str) -> String {
    let mut out = [intro, applang::REFERENCE, rules, HONEST].concat();
    for (i, (ask, src)) in applang::SHOTS.iter().enumerate() {
        let and = if i == 0 { "" } else { "\nAnd for " };
        out += &[and, "\"", ask, "\":\n```app\n", src, "```\n"].concat();
    }
    out
}

/// A streamed chat-completions request for `model` with the JSON members `options` (each with
/// a leading comma), the `system` prompt and then `messages`, (role, content) each.
pub fn chat(model: &str, options: &str, system: &str, messages: &[(&str, String)]) -> String {
    let message = |(role, text): &(&str, String)| {
        format!(",{{\"role\":\"{role}\",\"content\":{}}}", quote(text))
    };
    let messages: String = messages.iter().map(message).collect();
    format!(
        "{{\"model\":{},\"stream\":true,\"stream_options\":{{\"include_usage\":true}}{options},\
         \"messages\":[{{\"role\":\"system\",\"content\":{}}}{messages}]}}",
        quote(model),
        quote(system)
    )
}

/// What went wrong with a request that ended with HTTP `status` (0: none
/// made), the host's `error` and the AI service's own message `said`.
pub fn failure(status: u16, error: &str, said: &str) -> Option<(Style, String)> {
    let (code, what) = match status {
        _ if error == "cancelled" => return Some((Style::Dim, "Stopped.".into())),
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
    Some((Style::Error, format!("E090{code} {what}{said}")))
}

/// The program in `reply`'s first fenced block whose info string is `app`
/// (to the end of the reply if the block is not closed).
pub fn app_block(reply: &str) -> Option<&str> {
    fenced(reply).map(|(src, _)| src)
}

/// [`app_block`], and whether its block was closed: a reply cut off by the token limit
/// (`finish_reason` length) inside an open block holds part of a program, never a program.
pub fn fenced(reply: &str) -> Option<(&str, bool)> {
    let (mut at, mut start) = (0, None);
    for line in reply.split_inclusive('\n') {
        let fence = line.trim().strip_prefix("```");
        match start {
            None if fence.map(str::trim) == Some("app") => start = Some(at + line.len()),
            Some(s) if fence.is_some() => return Some((&reply[s..at], true)),
            _ => {}
        }
        at += line.len();
    }
    start.map(|s| (&reply[s..], false))
}

/// A slug for `src`'s file: its first label's text, lowercase ASCII words
/// joined by `-`, at most 32 bytes; "app" if that leaves nothing.
pub fn slug(src: &str) -> String {
    let toks = applang::highlight(src);
    let tok = |i: usize| toks.get(i).map(|(s, c)| (&src[s.start..s.end], *c));
    let label = (0..toks.len()).find_map(|i| match (tok(i)?, tok(i + 1)?) {
        (("label", Class::Keyword), (text, Class::Str)) => Some(text),
        _ => None,
    });
    let alnum = |b: u8| if b.is_ascii_alphanumeric() { b.to_ascii_lowercase() } else { b' ' };
    let words: String = label.unwrap_or("").bytes().map(|b| char::from(alnum(b))).collect();
    let slug = words.split_whitespace().collect::<Vec<_>>().join("-");
    let slug = slug[..slug.len().min(32)].trim_end_matches('-');
    if slug.is_empty() { "app".into() } else { slug.into() }
}

/// Where a made app named `slug` goes: the first of `~/apps/<slug>.app`, `<slug>-2.app`, ... (to
/// `-999`) that nothing is at, by `exists`, so a made app never replaces a file nor starts from
/// the saved states another app of that name left ([`state_path`]).
pub fn free_path(slug: &str, mut exists: impl FnMut(&str) -> bool) -> Option<String> {
    let name = |n: u32| match n {
        1 => format!("{HOME}/apps/{slug}.app"),
        n => format!("{HOME}/apps/{slug}-{n}.app"),
    };
    (1..1000).map(name).find(|p| !exists(p) && !exists(&state_path(p)))
}

/// The file the saved states of the app at `path` go to: `~/.appdata/<name>.state`, by its
/// file's name ("" for none).
pub fn state_path(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.strip_suffix(".app").unwrap_or(name);
    if stem.is_empty() { String::new() } else { [HOME, "/.appdata/", stem, ".state"].concat() }
}

/// What is wrong with `src`, if anything: that it does not compile, faults when it runs
/// ([`applang::smoke`]: rendered, clicked, ticked, keyed, tapped and typed into), or faults as it
/// will really start, from `saved` (what its saved states' file holds: rendered, then each button
/// clicked once); as [what it did, what it still does after fixes], the problem as a person reads
/// it ([`problem`]) and the account a model fixing it gets: the problem with its line and a caret
/// under it, the rule its code says was broken, what was being done when it came and what came
/// before (or the states it started from), and that every occurrence wants fixing.
pub fn fault(src: &str, saved: &str) -> Option<([&'static str; 2], String, String)> {
    let (what, d, when) = match applang::compile(src) {
        Err(d) => (["did not compile", "not compiling"], d, String::new()),
        Ok(p) => match applang::smoke(p, 1).fault {
            Some(f) => {
                let before = match f.before.is_empty() {
                    true => String::new(),
                    false => [", after ", &f.before.join(", ")].concat(),
                };
                let when = ["\nIt came while ", &f.during, &before, "."].concat();
                (["faults when it runs", "faulting"], f.diag, when)
            }
            None => {
                let (d, during, kept) = resumed(src, saved)?;
                let from = ", started from the states it keeps between runs: ";
                let when = ["\nIt came while ", &during, from, &kept, ".\n", KEPT].concat();
                let still = "faulting from its saved states";
                (["faults when it runs from its saved states", still], d, when)
            }
        },
    };
    // "line 3, col 5", then the line and its carets.
    let snip = d.span.and_then(|s| lang::diag::render_snippet(src, s)).unwrap_or_default();
    let (at, line) = snip
        .split_once('\n')
        .map_or((String::new(), ""), |(at, line)| ([" at ", at].concat(), line));
    let code = d.code.unwrap_or_default();
    let account = format!(
        "Your program {}: E{code:04}{at}: {}\n{}\nRule: {}{}\nThe same mistake may be on other \
         lines too: fix every occurrence.",
        what[0],
        clip(&d.message, 1024),
        clip(line, 4096),
        applang::rule(code),
        clip(&when, 1024)
    );
    Some((what, problem(&d, src), account))
}

/// The first fault of `src` (which compiles) started as it really will be, from `saved` (what its
/// saved states' file holds; none if that is "" or it saves none): its first render, then each
/// button that shows clicked once (as it shows by then), re-rendering after each. With what was
/// being done, and the saved lists it started from (`tasks` has 2 items, ...): no values, which
/// are the person's.
fn resumed(src: &str, saved: &str) -> Option<(applang::Diag, String, String)> {
    let mut app = App::new(applang::compile(src).ok()?, Limits::default(), 1);
    if saved.trim().is_empty() || app.saved().is_empty() {
        return None;
    }
    // As kept, whatever shows: restore would start afresh where they fault, hiding the fault.
    app.take_back(saved);
    let mut kept = String::new();
    for (s, (name, v)) in app.program().states().iter().zip(app.state()) {
        if let (true, Value::List(items)) = (s.saved, v) {
            let sep = if kept.is_empty() { "`" } else { ", `" };
            kept = kept + sep + name + "` has " + &items.len().to_string() + " items";
        }
    }
    let fail = |d, what: &str| Some((d, what.to_string(), kept.clone()));
    let mut shown = match app.render() {
        Ok(nodes) => nodes,
        Err(d) => return fail(d, "the first render"),
    };
    for (_, text) in buttons(&shown) {
        let Some(&(id, _)) = buttons(&shown).iter().find(|b| b.1 == text) else { continue };
        let what = ["clicking \"", &text, "\""].concat();
        if let Err(d) = app.handle(&Event::Click { id }) {
            return fail(d, &what);
        }
        match app.render() {
            Ok(nodes) => shown = nodes,
            Err(d) => return fail(d, &["the render after ", &what].concat()),
        }
    }
    None
}

/// Every (id, text) of `nodes`' buttons, in order.
fn buttons(nodes: &[Node]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::Button { text, id } => out.push((*id, text.clone())),
            Node::Row { children } | Node::Col { children } => out.extend(buttons(children)),
            _ => {}
        }
    }
    out
}

/// A diagnostic as `E0101 3:5 message`: line and column (in chars) from 1.
pub fn problem(d: &applang::Diag, src: &str) -> String {
    let code = d.code.map_or_else(|| "error".into(), |c| format!("E{c:04}"));
    let at = |s: applang::Span| {
        let before = src.get(..s.start).unwrap_or(src);
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!(" {}:{col}", before.matches('\n').count() + 1)
    };
    format!("{code}{} {}", d.span.map(at).unwrap_or_default(), clip(&d.message, 1024))
}

/// `s` cut to at most `max` bytes on a char boundary, ending in `…` if cut.
pub fn clip(s: &str, max: usize) -> String {
    let end = (0..=max.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    [&s[..end], if end < s.len() { "\u{2026}" } else { "" }].concat()
}

/// The corpus line for `program`, made for `prompt` by `model` in `attempts`, changing `base`
/// (none: "").
pub fn corpus_line(prompt: &str, program: &str, base: &str, attempts: u32, model: &str) -> String {
    let (prompt, program, model) = (quote(prompt), quote(program), quote(model));
    let base = if base.is_empty() { String::new() } else { [",\"base\":", &quote(base)].concat() };
    format!(
        "{{\"prompt\":{prompt},\"program\":{program},\"attempts\":{attempts},\"model\":{model}{base}}}\n"
    )
}

/// `path` as a person reads it: the home as `~`.
pub fn shown(path: &str) -> String {
    path.strip_prefix(HOME).map_or_else(|| path.into(), |rest| ["~", rest].concat())
}
