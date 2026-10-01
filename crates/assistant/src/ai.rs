//! What every program that makes apps with the AI shares (the Assistant and Studio): the request
//! body, coded failures, the program in a reply, file names, diagnostics and the corpus line.

use applang::Class;
use uiwire::Style;

use crate::json::quote;

/// The guest's home, where made apps and the corpus go.
pub const HOME: &str = concat!("/home/", "guest");
/// The fine-tuning corpus: a JSON line per app that compiled.
pub const CORPUS: &str = concat!("/home/", "guest", "/.ai/corpus.jsonl");
/// The model until the desktop names one.
pub const DEFAULT_MODEL: &str = "zai/glm-5.3";
/// How many times a program that does not compile goes back to the model.
pub const RETRIES: u32 = 2;
/// The most bytes of a reply kept.
pub const MAX_REPLY: usize = 64 * 1024;
/// A program in the form replies give it, for system prompts.
pub const EXAMPLE: &str = "```app\nstate count = 0;\nlabel \"Counter\";\nrow {\n  button \"-\" { \
                           count = count - 1; }\n  label count;\n  button \"+\" { count = count + \
                           1; }\n}\n```";

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
    let (mut at, mut start) = (0, None);
    for line in reply.split_inclusive('\n') {
        let fence = line.trim().strip_prefix("```");
        match start {
            None if fence.map(str::trim) == Some("app") => start = Some(at + line.len()),
            Some(s) if fence.is_some() => return Some(&reply[s..at]),
            _ => {}
        }
        at += line.len();
    }
    start.map(|s| &reply[s..])
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
