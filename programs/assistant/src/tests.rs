use ai::{HOME, MAX_REPLY, app_block, failure, fault, fenced, free_path, problem, slug};
use json::{Json, quote};

use super::*;

/// An SSE event carrying one content delta, its line ended with CRLF.
fn chunk(text: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\r\n\n", quote(text))
}

#[test]
fn streams_read_a_reply_split_anywhere() {
    // Reasoning is counted, never taken as the reply.
    let (mut s, mut out) = (json::Stream::default(), String::new());
    let think = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Hm\u{e9}.\"}}]}\n\n";
    s.feed(think.as_bytes(), &mut out, MAX_REPLY);
    assert!(out.is_empty() && s.thought == 4);
    // Three-byte chunks split lines, CRLFs and chars; usage comes last, with the reasoning it
    // counted (streamed or not).
    let reply = "H\u{e9}llo \u{2014} w\u{f6}rld \u{2713} \u{1f600}";
    let usage = "data: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5,\
                 \"completion_tokens_details\":{\"reasoning_tokens\":3}}}\n\n";
    let body: String = reply.split_inclusive(' ').map(chunk).collect();
    let body = [&body, usage, "data: [DONE]\n\n"].concat();
    body.as_bytes().chunks(3).for_each(|c| s.feed(c, &mut out, MAX_REPLY));
    s.end(&mut out, MAX_REPLY);
    assert_eq!((out.as_str(), s.usage.clone()), (reply, Some(("12".into(), "5".into()))));
    assert_eq!(s.reasoning, 3);
    // An error body is read at its end.
    let (mut s, mut out) = (json::Stream::default(), String::new());
    s.feed(b"{\n\"error\":{\"message\":\"no\"}}", &mut out, MAX_REPLY);
    s.end(&mut out, MAX_REPLY);
    assert_eq!((s.error.as_str(), out.as_str()), ("no", ""));
}

#[test]
fn failures_are_coded() {
    // An HTTP status, the host's error and what the AI service said, as the person reads them.
    let cases = [
        (401, "", "no", "E0901 the AI service refused the request: no"),
        (403, "", "", "E0901 the AI service refused the request"),
        (402, "", "", "E0902 the free AI is out of credit for now, try again later"),
        (429, "", "", "E0903 the free AI is busy, try again in a minute"),
        (404, "", "", "E0904 the model was not found or refused the request"),
        (503, "", "", "E0905 the AI is not available right now"),
        (502, "", "", "E0905 couldn't reach the AI service"),
        (0, "network", "", "E0905 network"),
        (200, "", "busy", "E0905 the AI stopped with an error: busy"),
    ];
    for (status, error, said, want) in cases {
        assert_eq!(failure(status, error, said).map(|f| f.1).as_deref(), Some(want));
    }
    let stopped = failure(0, "cancelled", "").map(|f| f.1);
    assert_eq!((failure(200, "", ""), stopped.as_deref()), (None, Some("Stopped.")));
}

#[test]
fn json_reads_and_quotes() {
    let src =
        r#" {"a": [1, -2.5e3, true, false, null], "s": "\"\\\/\n\u00e9\ud83d\ude00\ud800x"} "#;
    let v = Json::parse(src).unwrap();
    assert_eq!(v.get("a").and_then(|a| a.at(1)), Some(&Json::Num("-2.5e3".into())));
    assert_eq!(v.get("s").and_then(Json::text), Some("\"\\/\n\u{e9}\u{1f600}\u{fffd}x"));
    for bad in ["{", "[1,]", "[1 2]", "{\"a\" 1}", "tru", "\"\\x\"", "\"open\\", &"[".repeat(99)] {
        assert_eq!(Json::parse(bad), None, "{bad}");
    }
    let text = "a\"b\\c\n\u{1}\u{7f}\u{e9}";
    assert_eq!(Json::parse(&quote(text)), Some(Json::Str(text.into())));
    // The stream keeps why the reply ended; a null finish_reason leaves it as it was.
    let (mut s, mut out) = (json::Stream::default(), String::new());
    let end =
        |f: &str| format!("data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":{f}}}]}}\n");
    s.feed(end("null").as_bytes(), &mut out, 64);
    assert_eq!(s.finish, "");
    s.feed([end("\"length\""), end("null")].concat().as_bytes(), &mut out, 64);
    assert_eq!((s.finish.as_str(), out.as_str()), ("length", ""));
}

#[test]
fn programs_run_from_the_states_they_keep_and_never_start_on_another_apps() {
    // Fine afresh, but a list kept at its old length makes a click fault: said with the states
    // it started from, and what a fix must keep to.
    let src = "saved state xs = [0; 8];\nlabel \"n \" + len(xs);\n\
               button \"Clear\" { for i in 0..8 { xs[i] = 0; } }\n";
    for kept in ["", "xs = [0; 8];", "ys = 1;\n"] {
        assert!(fault(src, kept).is_none(), "{kept}");
    }
    let (what, said, account) = fault(src, "xs = [0, 0, 0, 0];\n").unwrap();
    assert!(what[1] == "faulting from its saved states" && said.starts_with("E0215 3:"), "{said}");
    let when = "\nIt came while clicking \"Clear\", started from the states it keeps between runs: \
                `xs` has 4 items.\nSaved states come back as they were kept, also after";
    assert!(account.contains(when), "{account}");
    // A made app takes neither a file's name nor one whose saved states another app left.
    let taken = [format!("{HOME}/apps/todo.app"), format!("{HOME}/.appdata/todo-2.state")];
    let path = free_path("todo", |p| taken.iter().any(|t| t == p));
    assert_eq!(path, Some(format!("{HOME}/apps/todo-3.app")));
}

#[test]
fn slugs_blocks_and_problems() {
    assert_eq!(slug("state n = 0; // label \"no\"\nlabel \"My  Todo-List!\";"), "my-todo-list");
    assert_eq!(slug("state n = 0; button \"x\" { n = 1; }"), "app");
    assert_eq!(slug(&format!("label \"{}\";", "ab ".repeat(20))).len(), 32);
    assert_eq!(app_block("```app \nlabel 1;"), Some("label 1;"));
    assert_eq!(app_block("```rust\nfn x\n```\n"), None);
    assert_eq!(fenced("```app \nlabel 1;"), Some(("label 1;", false)));
    assert_eq!(fenced("a\n```app\nlabel 1;\n```\nb"), Some(("label 1;\n", true)));
    let src = "state n = 0;\n  label;";
    let d = applang::compile(src).unwrap_err();
    assert!(problem(&d, src).starts_with(&format!("E{:04} 2:", d.code.unwrap())));
}
