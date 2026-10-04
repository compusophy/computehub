use super::*;
use ai::{
    DEFAULT_MODEL, HOME, MAX_BODY, MAX_REPLY, blocks, failure, fault, free_path, problem, slug,
};
use edits::{Edit, Reply, apply, read};
use json::{Json, Stream, Usage, quote};

/// A clean program, one that does not compile (E0302 at 2:7) and one that faults as it first
/// renders (E0203 at 3:22).
const CLEAN: &str =
    "// Count: + adds one.\nstate n = 0;\nlabel \"Count\";\nbutton \"+\" { n += 1; }\nlabel n;\n";
const BROKEN: &str = "state n = 0;\nlabel nope;\n";
const FAULTS: &str = "state t = 0;\nstate c = 0;\nlabel \"avg \" + t / c;\n";
/// What every scripted reply cost.
const USAGE: &str = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3000,\"completion_tokens\":\
    1500,\"cost\":0.0081234,\"prompt_tokens_details\":{\"cached_tokens\":1900},\
    \"completion_tokens_details\":{\"reasoning_tokens\":400}}}\n\n";

/// An SSE chunk with `text` in its delta's `key`.
fn delta(key: &str, text: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"{key}\":{}}}}}]}}\r\n\n", quote(text))
}

/// A reply's body: some thinking, `content` in deltas of a few chars, how it finished (`length`:
/// cut off; else stop) and its usage.
fn sse(content: &str, finish: &str) -> Vec<u8> {
    sse_out(content, finish, 1500)
}

/// The same, its usage `output` tokens out.
fn sse_out(content: &str, finish: &str, output: u32) -> Vec<u8> {
    let mut out = delta("reasoning", "Hm, a grid\u{2026}");
    let chars: Vec<char> = content.chars().collect();
    out.extend(chars.chunks(5).map(|c| delta("content", &c.iter().collect::<String>())));
    out +=
        &format!("data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{finish}\"}}]}}\n\n");
    let usage = USAGE.replace(":1500,", &format!(":{output},"));
    [out.as_bytes(), usage.as_bytes(), b"data: [DONE]\n\n"].concat()
}

fn app(src: &str) -> String {
    format!("Here it is.\n```app\n{src}```\n")
}

fn edit(search: &str, replace: &str) -> String {
    format!("<<<<<<< SEARCH\n{search}=======\n{replace}>>>>>>> REPLACE\n")
}

fn task(ask: &str, base: &str) -> Task {
    Task { ask: ask.into(), base: base.into(), model: DEFAULT_MODEL.into(), kept: String::new() }
}

/// A make of `t` within `k`, each request answered by the next of `replies` (its content and
/// finish reason) a second after the last, in chunks of 61 bytes: the bodies asked, the
/// statuses seen, and its end.
fn run(t: Task, k: Knobs, replies: &[(&str, &str)]) -> (Vec<String>, Vec<String>, Done) {
    drive(t, k, replies, 61, |_| 1500)
}

/// The same in chunks of `size` bytes, each reply's usage the tokens `out` says for its body.
fn drive(
    t: Task,
    k: Knobs,
    replies: &[(&str, &str)],
    size: usize,
    out_of: impl Fn(&str) -> u32,
) -> (Vec<String>, Vec<String>, Done) {
    let (mut m, mut out) = Make::start(t, k, 0);
    let (mut bodies, mut seen, mut replies, mut now) = (Vec::new(), Vec::new(), replies.iter(), 0);
    loop {
        let body = match out {
            Out::Done(done) => return (bodies, seen, done),
            Out::Ask(body) => body,
            Out::Cancel => panic!("a cancel outside a reply"),
        };
        let tokens = out_of(&body);
        bodies.push(body);
        let Some((content, finish)) = replies.next() else { return (bodies, seen, m.stop(now)) };
        let (mut ended, data) = (None, sse_out(content, finish, tokens));
        for chunk in data.chunks(size) {
            now += 10;
            if let Some(o) = m.data(chunk, now) {
                assert!(matches!(o, Out::Cancel) && m.complete());
                ended = Some(None);
                break;
            }
            seen.push(m.status(now));
        }
        now += 1000;
        let end = ended.unwrap_or_else(|| m.end(200, "", now));
        out = match end {
            Some(o) => o,
            None => (m.status(now), m.check(now).unwrap()).1,
        };
    }
}

/// The user message of a request body.
fn user(body: &str) -> String {
    let v = Json::parse(body).unwrap();
    let m = v.get("messages").and_then(|m| m.at(1)).unwrap();
    m.get("content").and_then(Json::text).unwrap().to_string()
}

#[test]
fn json_and_streams_read_replies_split_anywhere() {
    let src =
        r#" {"a": [1, -2.5e3, true, false, null], "s": "\"\\\/\n\u00e9\ud83d\ude00\ud800x"} "#;
    let v = Json::parse(src).unwrap();
    assert_eq!(v.get("a").and_then(|a| a.at(1)), Some(&Json::Num("-2.5e3".into())));
    assert_eq!(v.get("s").and_then(Json::text), Some("\"\\/\n\u{e9}\u{1f600}\u{fffd}x"));
    for bad in ["{", "[1,]", "[1 2]", "{\"a\" 1}", "tru", "\"\\x\"", "\"open\\", &"[".repeat(99)] {
        assert_eq!(Json::parse(bad), None, "{bad}");
    }
    let text = "a\"b\\c\n\u{1}\u{7f}\u{e9}";
    assert_eq!(quote(text), "\"a\\\"b\\\\c\\u000a\\u0001\u{7f}\u{e9}\"");
    assert_eq!(Json::parse(&quote(text)), Some(Json::Str(text.into())));
    // Three-byte chunks split lines, CRLFs and chars; reasoning is counted, never the reply; the
    // usage's own cost wins over the gateway's metadata before it.
    let reply = "H\u{e9}llo \u{2014} w\u{f6}rld \u{2713} \u{1f600} \"content\":\"no\"";
    let meta = "data: {\"choices\":[{\"delta\":{\"content\":\"\",\"providerMetadata\":{\"cost\":\"9\"}}}]}\n";
    let body = [sse(reply, "stop"), meta.as_bytes().to_vec()].concat();
    let (mut s, mut out) = (Stream::default(), String::new());
    body.chunks(3).for_each(|c| s.feed(c, &mut out, MAX_REPLY));
    s.end(&mut out, MAX_REPLY);
    assert_eq!((out.as_str(), s.thought, s.finish.as_str()), (reply, 11, "stop"));
    assert_eq!(s.usage, Some(Usage { input: 3000, cached: 1900, output: 1500, reasoning: 400 }));
    // A count that does not read as one (a sign, an exponent, too long) is no usage.
    for bad in ["-1", "1e3", "1.5", "12345678901"] {
        let (mut s, mut out) = (Stream::default(), String::new());
        let usage =
            format!("data: {{\"usage\":{{\"prompt_tokens\":{bad},\"completion_tokens\":1}}}}\n");
        s.feed(usage.as_bytes(), &mut out, MAX_REPLY);
        assert_eq!(s.usage, None, "{bad}");
    }
    // Errors, in a chunk or as the whole body; the reply kept to its room.
    let (mut s, mut out) = (Stream::default(), String::new());
    s.feed(b"{\n\"error\":{\"message\":\"no\"}}", &mut out, MAX_REPLY);
    s.end(&mut out, MAX_REPLY);
    assert_eq!((s.error.as_str(), out.as_str()), ("no", ""));
    let (mut s, mut out) = (Stream::default(), String::new());
    s.feed(b"data: {\"error\":\"busy\"}\n", &mut out, MAX_REPLY);
    s.feed(delta("content", "abcdef").as_bytes(), &mut out, 4);
    s.feed(delta("content", "gh").as_bytes(), &mut out, 4);
    assert_eq!((s.error.as_str(), out.as_str(), s.usage), ("busy", "abcdef", None));
    // Not the errors of the providers the gateway tried first, in its metadata (as it sent them,
    // live, once Fireworks answered 503 and Baseten the reply).
    let (mut s, mut out) = (Stream::default(), String::new());
    let tried = "data: {\"id\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\",\
                 \"providerMetadata\":{\"gateway\":{\"modelAttempts\":[{\"providerAttempts\":[{\
                 \"provider\":\"fireworks\",\"success\":false,\"error\":\"Service temporarily \
                 unavailable\",\"statusCode\":503},{\"provider\":\"baseten\",\"success\":true}]}],\
                 \"note\":\"a \\\"quoted\\\" {brace\"}}},\"finish_reason\":\"stop\"}],\
                 \"usage\":{\"prompt_tokens\":2501,\"completion_tokens\":999}}\n";
    s.feed(tried.as_bytes(), &mut out, MAX_REPLY);
    s.end(&mut out, MAX_REPLY);
    assert_eq!(
        (s.error.as_str(), s.finish.as_str(), s.usage.map(|u| u.output)),
        ("", "stop", Some(999))
    );
    let (mut s, mut out) = (Stream::default(), String::new());
    s.feed(
        b"data: {\"id\":\"g\", \"error\" : {\"code\":5,\"message\":\"cut\"}}\n",
        &mut out,
        MAX_REPLY,
    );
    assert_eq!(s.error, "cut");
}

#[test]
fn failures_are_coded() {
    let cases = [
        (401, "", "no", "E0901 the AI service refused the request: no"),
        (402, "", "", "E0902 the free AI is out of credit for now, try again later"),
        (429, "", "", "E0903 the free AI is busy, try again in a minute"),
        (404, "", "", "E0904 the model was not found or refused the request"),
        (503, "", "", "E0905 the AI is not available right now"),
        (0, "network", "", "E0905 network"),
        (200, "", "busy", "E0905 the AI stopped with an error: busy"),
    ];
    for (status, error, said, want) in cases {
        let code = want[1..5].parse::<u16>().unwrap();
        assert_eq!(failure(status, error, said), Some((code, want.to_string())));
    }
    assert_eq!(failure(0, "cancelled", ""), Some((0, "Stopped.".into())));
    assert_eq!(failure(200, "", ""), None);
}

#[test]
fn names_blocks_numbers_and_problems() {
    assert_eq!(slug("state n = 0; // label \"no\"\nlabel \"My  Todo-List!\";"), "my-todo-list");
    assert_eq!(slug("state n = 0; button \"x\" { n = 1; }"), "app");
    assert_eq!(slug(&format!("label \"{}\";", "ab ".repeat(20))).len(), 32);
    assert_eq!(blocks("```app \nlabel 1;"), [("label 1;", false)]);
    let two = blocks("a\n```app\nlabel 1;\n```\nb\n```app\n```rust\nx");
    assert_eq!(two, [("label 1;\n", true), ("", true)]);
    assert_eq!(blocks("```rust\nfn x\n```\n"), []);
    assert_eq!(ai::numbered("a\n\nb", 9), "  9| a\n 10| \n 11| b\n");
    let d = applang::compile(BROKEN).unwrap_err();
    assert_eq!(problem(&d, BROKEN), "E0302 2:7 `nope` is not a declared state or local variable");
    // A made app takes neither a file's name nor one whose saved states another app left.
    let taken = [format!("{HOME}/apps/todo.app"), format!("{HOME}/.appdata/todo-2.state")];
    let path = free_path("todo", &mut |p| taken.iter().any(|t| t == p));
    assert_eq!(path, Some(format!("{HOME}/apps/todo-3.app")));
    assert_eq!(ai::state_path("/apps/x.app"), format!("{HOME}/.appdata/x.state"));
    assert_eq!(ai::about("/* A: b */ // c\nlabel 1;"), "A: b c");
    assert_eq!(ai::about("// A\n// icon: dot 12 12 3\n// B\nlabel 1;"), "A B");
}

#[test]
fn faults_are_accounted_for_a_fix() {
    let f = fault(BROKEN, "", 3).unwrap();
    assert!(!f.compiles && f.said.starts_with("E0302 2:7"));
    let want = "Your program did not compile: E0302 at line 2, col 7: `nope` is not a declared state \
                or local variable\n  label nope;\n        ^^^^\nRule: use only declared states";
    assert!(f.account.starts_with(want), "{}", f.account);
    assert!(f.account.ends_with("fix every occurrence."));
    let f = fault(FAULTS, "", 3).unwrap();
    assert!(f.compiles && f.said.starts_with("E0203 3:"), "{}", f.said);
    let rule = "\nRule: guard every / and % so the divisor is never 0.\nIt came while the first \
                render.\n";
    assert!(f.account.contains(rule), "{}", f.account);
    assert!(fault(CLEAN, "", 3).is_none());
    // A program clean afresh that faults from the states its app kept, said so.
    let todo = applang::SHOTS[1].1;
    let kept = "tasks = [\"milk\", \"eggs\"];\ndone = [false, true];\n";
    let due = todo
        .replace("done = [false; 0];", "done = [false; 0];\nsaved state due = [\"\"; 0];")
        .replace("push(done, false);", "push(done, false); push(due, \" today\");")
        .replace("label tasks[i];", "label tasks[i] + due[i];");
    assert!(fault(&due, "", 3).is_none() && fault(todo, kept, 3).is_none());
    let f = fault(&due, kept, 3).unwrap();
    let from = "faults when it runs from its saved states: E0215 at line 35";
    let when = "It came while the first render, started from the states it keeps between runs.\n\
                Saved states come back as they were kept";
    assert!(f.account.contains(from) && f.account.contains(when), "{}", f.account);
    // A fault only another seed finds is found.
    let src = |k: &str| {
        format!("state x = 0;\nevery 100 {{ x = random(1000000000); }}\nlabel 10 / (x - {k});\n")
    };
    let mut app =
        applang::App::new(applang::compile(&src("1")).unwrap(), applang::Limits::default(), 2);
    app.handle(&applang::Event::Tick { ms: 100 }).unwrap();
    let x = app.state().next().unwrap().1.to_string();
    let only2 = src(&x);
    let smoke = |seed| applang::smoke(applang::compile(&only2).unwrap(), seed).fault.is_some();
    assert!(!smoke(1) && smoke(2));
    assert!(fault(&only2, "", 1).is_none() && fault(&only2, "", 3).is_some());
}

#[test]
fn replies_hold_programs_or_edits() {
    let e = |s: &str, r: &str| Edit { search: vec![s.into()], replace: vec![r.into()] };
    assert_eq!(read(&app(CLEAN), false), Reply::Program(CLEAN.into()));
    assert_eq!(read("```app\nlabel 1;", false), Reply::Program("label 1;".into()));
    assert_eq!(read("```app\nlabel 1;", true), Reply::Cut);
    // Edits anywhere, every fence around them ignored (one around all, or one each), CRLF read
    // as LF; a closed whole program wins, a snippet does not.
    let two = [edit("a\n", "b\n"), "text\n".into(), edit("c\n", "d\n")].concat();
    let want = Reply::Edits(vec![e("a", "b"), e("c", "d")]);
    assert_eq!(read(&two, false), want);
    assert_eq!(read(&["```app\n", &two, "```\n"].concat().replace('\n', "\r\n"), false), want);
    let each = app(&edit("a\n", "b\n")) + &app(&edit("c\n", "d\n"));
    assert_eq!(read(&each, false), want);
    assert_eq!(read(&[two.as_str(), &app(CLEAN)].concat(), false), Reply::Program(CLEAN.into()));
    assert_eq!(read(&[app("label 1;\n"), two.clone()].concat(), false), want);
    // The program is the first that begins as one does, with its comment, compiled or not (its
    // fix comes next), never an example before it; with none, the longest block; one open at a
    // cut is part of one.
    let example = "A label shows text:\n```app\nlabel \"Count\";\n```\n";
    for before in [example.into(), app(&CLEAN.replace("//", "/"))] {
        let reply = [before.as_str(), &app(CLEAN), &app("// Two.\nlabel 2;\n")].concat();
        assert_eq!(read(&reply, false), Reply::Program(CLEAN.into()), "{before}");
    }
    let broken = ["```app\n// No.\nlabel nope;\n```\n", &app(CLEAN)].concat();
    assert_eq!(read(&broken, false), Reply::Program("// No.\nlabel nope;\n".into()));
    let (long, short) = (app("state a = 0;\nlabel nope;\n"), app("label 1;\n"));
    let want = Reply::Program("state a = 0;\nlabel nope;\n".into());
    assert_eq!(read(&[short.as_str(), &long, &short].concat(), false), want);
    assert_eq!(
        read(&[short.as_str(), "```app\nstate a = 0;\nlabel 2;\n"].concat(), true),
        Reply::Cut
    );
    let open = "<<<<<<< SEARCH\na\n=======\nb\n";
    assert_eq!((read(open, false), read(open, true)), (Reply::Unclosed(1), Reply::Cut));
    let nested = [&edit("a\n", "b\n"), "<<<<<<< SEARCH\nx\n", open].concat();
    assert_eq!(read(&nested, false), Reply::Unclosed(2));
    assert_eq!(read("Sorry.", false), Reply::Nothing);
}

#[test]
fn edits_apply_atomically_where_they_match_one_place() {
    let src = "state n = 0;\nfn f() {\n    n = 1;\n}\nlabel n;\nlabel n;\n";
    let block = |s: &str, r: &str| match read(&edit(s, r), false) {
        Reply::Edits(e) => e,
        other => panic!("{other:?}"),
    };
    // Exact; numbers copied with the lines (and into the REPLACE); trailing spaces; indent drift.
    let want = src.replace("    n = 1;", "    n = 2;");
    for (s, r) in [
        ("    n = 1;\n", "    n = 2;\n"),
        ("  3|     n = 1;\n", "  3|     n = 2;\n"),
        ("    n = 1;   \n", "    n = 2;\n"),
    ] {
        assert_eq!(apply(src, &block(s, r)), Ok(want.clone()), "{s:?}");
    }
    assert_eq!(apply(src, &block("n = 1;\n", "n = 2;\n")), Ok(src.replace("    n = 1;", "n = 2;")));
    // Blank lines around a block are not lines to match; a blank REPLACE deletes.
    assert_eq!(apply(src, &block("\nfn f() {\n\n", "fn g() {\n")), Ok(src.replace("f()", "g()")));
    assert_eq!(apply(src, &block("state n = 0;\n", "")), Ok(src.replacen("state n = 0;\n", "", 1)));
    // Ambiguous, no match (the nearest lines), empty: the program is unchanged.
    let why = apply(src, &block("label n;\n", "x\n")).unwrap_err();
    assert_eq!(
        why,
        "SEARCH block 1 matched lines 5 and 6; quote a line around it so it matches one place."
    );
    let why = apply(src, &block("fn f() {\n  n = 9;\n", "x\n")).unwrap_err();
    let near =
        "SEARCH block 1 matched no lines. The nearest lines are:\n  2| fn f() {\n  3|     n = 1;\n";
    assert!(why.starts_with(near), "{why}");
    assert_eq!(
        apply(src, &block("nothing\n", "x\n")).unwrap_err(),
        "SEARCH block 1 matched no lines."
    );
    assert_eq!(apply(src, &block("\n\n", "x\n")).unwrap_err(), "SEARCH block 1 is empty.");
    // Blocks apply in order to the evolving text, all or none.
    let mut both = block("state n = 0;\n", "state n = 5;\n");
    both.extend(block("state n = 5;\n", "state n = 6;\n"));
    assert_eq!(apply(src, &both), Ok(src.replace("n = 0", "n = 6")));
    both.extend(block("gone\n", "x\n"));
    assert!(apply(src, &both).unwrap_err().starts_with("SEARCH block 3 matched no lines"));
}

#[test]
fn the_system_prompt_is_stable_and_whole() {
    let s = system();
    let shots = applang::SHOTS.map(|s| s.1);
    for part in [applang::REFERENCE, shots[0], shots[1], shots[2], ai::HONEST, ai::ICON] {
        assert!(s.contains(part));
    }
    assert!(s.contains("<<<<<<< SEARCH\nlines copied exactly") && !s.contains(DEFAULT_MODEL));
    assert!(!s.contains(HOME) && s.starts_with("You write apps for Studio"));
    // FNV-1a 64: a change to the prompt is a decision (and an eval), never a drift.
    let fnv = s
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    assert_eq!((fnv, s.len()), (FNV, s.len()), "{fnv:#x}");
    // A fix of a 200-line program fits what the free AI takes.
    let big: String =
        (0..200).map(|i| format!("label \"line {i} of a long program, padded\";\n")).collect();
    let (_, out) = Make::start(task("grow", &big), Knobs::default(), 0);
    let Out::Ask(body) = out else { panic!() };
    assert!(body.len() < MAX_BODY && user(&body).contains("200| label \"line 199"));
}

/// The system prompt's hash (see the test above).
const FNV: u64 = 0x8b2d_8925_97c7_62ad;

#[test]
fn a_clean_write_is_one_request() {
    let (bodies, seen, done) =
        run(task("a counter", ""), Knobs::default(), &[(&app(CLEAN), "stop")]);
    let b = Json::parse(&bodies[0]).unwrap();
    let s = |k| b.get(k).and_then(Json::text).map(str::to_string);
    assert_eq!(
        [s("model"), s("max_tokens"), s("temperature")],
        [Some(DEFAULT_MODEL.into()), Some("6144".into()), Some("0.3".into())]
    );
    assert_eq!((b.get("reasoning"), user(&bodies[0])), (None, "Make: a counter".into()));
    assert_eq!((done.outcome, done.install, done.draft.as_str()), (Outcome::Ready, true, CLEAN));
    assert_eq!(
        (done.plan.as_str(), done.said().as_str()),
        ("Count: + adds one.", "ready \u{b7} 1 s")
    );
    // Its status, as the reply streamed: thinking, then the lines it wrote; its block's end stopped
    // the request, whose usage is then estimated, the system prompt's part of it cached.
    assert_eq!(seen[0], "thinking \u{b7} 0 s");
    assert!(
        seen.contains(&"writing \u{b7} 1 line".into())
            && seen.contains(&"writing \u{b7} 4 lines".into())
    );
    let (r, sys) = (&done.receipt, quote(&system()).len() as u32 / 3);
    assert!(r.est && r.turns.len() == 1 && r.turns[0].input == bodies[0].len() as u32 / 3);
    assert!(r.turns[0].cached == sys && sys > 2000, "{sys}");
    // After a usage that found it uncached, none.
    let (mut m, _) = Make::start(task("count", ""), Knobs::default(), 0);
    let cold = String::from_utf8(sse(&app(BROKEN), "stop")).unwrap().replace(":1900}", ":0}");
    assert!(m.data(cold.as_bytes(), 1).is_none() && m.end(200, "", 2).is_none());
    assert!(matches!(m.check(3), Some(Out::Ask(_))));
    assert!(matches!(m.data(delta("content", &app(CLEAN)).as_bytes(), 4), Some(Out::Cancel)));
    let Some(Out::Done(done)) = m.check(5) else { panic!() };
    assert_eq!(
        (done.outcome, done.receipt.turns[0].cached, done.receipt.turns[1].cached),
        (Outcome::Ready, 0, 0)
    );
}

#[test]
fn problems_get_fixes_by_edits_and_the_best_so_far_stays() {
    // A program that does not compile, fixed by an edit to the line.
    let fix = edit("label nope;\n", "label n;\n");
    let (bodies, seen, done) =
        run(task("count", ""), Knobs::default(), &[(&app(BROKEN), "stop"), (&fix, "stop")]);
    let asked = user(&bodies[1]);
    let want = "You were asked: count\n\nThe program, numbered:\n  1| state n = 0;\n  2| label nope;\n\n\
                Your program did not compile: E0302 at line 2, col 7:";
    assert!(asked.starts_with(want) && asked.ends_with("Reply with edit blocks."), "{asked}");
    let b = Json::parse(&bodies[1]).unwrap();
    assert_eq!(b.get("max_tokens").and_then(Json::text), Some("4096"));
    assert!(
        seen.contains(&"fixing line 2 \u{b7} 0 s".into())
            && seen.contains(&"fixing line 2 \u{b7} 1 edit".into())
    );
    assert_eq!((done.outcome, done.draft.as_str()), (Outcome::Ready, "state n = 0;\nlabel n;\n"));
    // A write that does not compile is not the program a make stops at, so its usage came too.
    let r = &done.receipt;
    assert_eq!(
        (r.turns.len(), r.est, r.turns[1].usd_micros, r.turns[0].code),
        (2, false, 8634, 302)
    );
    assert_eq!(r.usd_micros, 2 * 8634);
    // The same problem twice is rewritten once; a third time ends with the best so far: a new
    // app's that compiles, installed faulting.
    let replies = [(app(FAULTS), "stop"), (app(FAULTS), "stop"), (app(FAULTS), "stop")];
    let replies: Vec<(&str, &str)> = replies.iter().map(|(a, f)| (a.as_str(), *f)).collect();
    let (bodies, _, done) = run(task("an average", ""), Knobs::default(), &replies);
    assert!(user(&bodies[2]).ends_with(
        "Two tries did not clear this. Write the program again, simpler, as one complete app block."
    ));
    assert_eq!((bodies.len(), done.outcome, done.install), (3, Outcome::Faulting, true));
    assert_eq!(
        (done.said().as_str(), done.draft.as_str()),
        ("runs, but faults \u{b7} E0203 line 3", FAULTS)
    );
    // A compiling program beats a later one that does not.
    let replies = [
        (app(FAULTS), "stop"),
        (app(BROKEN), "stop"),
        (app(BROKEN), "stop"),
        (app(BROKEN), "stop"),
    ];
    let replies: Vec<(&str, &str)> = replies.iter().map(|(a, f)| (a.as_str(), *f)).collect();
    let (_, _, done) = run(task("x", ""), Knobs::default(), &replies);
    assert_eq!((done.outcome, done.draft.as_str()), (Outcome::Faulting, FAULTS));
}

#[test]
fn edits_that_miss_go_back_once_then_the_program_is_rewritten() {
    let miss = edit("label gone;\n", "label n;\n");
    let replies =
        [(app(BROKEN), "stop"), (miss.clone(), "stop"), (miss, "stop"), (app(CLEAN), "stop")];
    let replies: Vec<(&str, &str)> = replies.iter().map(|(a, f)| (a.as_str(), *f)).collect();
    let (bodies, _, done) = run(task("count", ""), Knobs::default(), &replies);
    let missed = user(&bodies[2]);
    assert!(missed.starts_with(&user(&bodies[1])) && missed.contains("Your edits did not apply, so the program is unchanged: SEARCH block 1 matched no lines."));
    assert!(
        user(&bodies[3])
            .contains("SEARCH block 1 matched no lines.\nTwo tries did not clear this.")
    );
    assert_eq!((bodies.len(), done.outcome), (4, Outcome::Ready));
    assert_eq!(
        done.receipt.turns.iter().map(|t| t.turn).collect::<Vec<_>>(),
        [Turn::Write, Turn::Fix, Turn::Missed, Turn::Rewrite]
    );
    // A fix that drops a quarter of the program is not applied.
    let long = [CLEAN, "label 1;\nlabel 2;\nlabel nope;\n"].concat();
    let gut = edit(&long, "label 1;\n");
    let (bodies, _, _) =
        run(task("count", ""), Knobs::default(), &[(&app(&long), "stop"), (&gut, "stop")]);
    assert!(user(&bodies[2]).contains("Your edits dropped over a quarter of the program's lines."));
}

#[test]
fn cut_formatless_and_honest_replies() {
    // Cut off inside its program: the same app, shorter, once; then E0907, nothing installed.
    let cut = "```app\nstate a = 0;\nlabel \"Tet";
    let (bodies, _, done) =
        run(task("tetris", ""), Knobs::default(), &[(cut, "length"), (cut, "length")]);
    assert!(user(&bodies[1]).starts_with("Make: tetris\n\nYour reply ran out of room"));
    assert_eq!((done.outcome, done.install, done.code), (Outcome::Broken, false, 907));
    assert_eq!(
        (done.said().as_str(), done.draft.as_str()),
        ("couldn't \u{b7} E0907", "state a = 0;\nlabel \"Tet")
    );
    // No program and no edits: asked for them once, then E0906.
    let (bodies, _, done) =
        run(task("x", ""), Knobs::default(), &[("Sorry.", "stop"), ("Hm.", "stop")]);
    assert!(
        user(&bodies[1])
            .ends_with("Reply with one complete app block, or with edit blocks, and nothing else.")
    );
    assert_eq!((bodies.len(), done.code, done.outcome), (2, 906, Outcome::Broken));
    // Only a comment: applang can make nothing close, said why; nothing installed.
    for reply in [
        "```app\n// applang has no network: a chat needs one.\n```\n",
        "// applang has no network: a chat needs one.",
    ] {
        let (_, _, done) = run(task("a chat", ""), Knobs::default(), &[(reply, "stop")]);
        assert_eq!(
            (done.outcome, done.install, done.said().as_str()),
            (Outcome::Cant, false, "can't make that")
        );
        assert_eq!(done.plan, "applang has no network: a chat needs one.");
    }
    // So after a write that compiles but faults: nothing installed, nothing shown or marked.
    let (cant, faults) =
        ("```app\n// applang has no floats: averages need them.\n```\n", app(FAULTS));
    let replies = [(faults.as_str(), "stop"), (cant, "stop")];
    let (_, _, done) = run(task("an average", ""), Knobs::default(), &replies);
    assert_eq!(
        (done.outcome, done.install, done.draft.as_str(), done.mark, done.code),
        (Outcome::Cant, false, "", None, 0)
    );
    assert_eq!(done.said(), "can't make that");
}

#[test]
fn a_change_cut_off_asks_for_edits_never_the_app_cut_down() {
    // A 25-line app to change and a whole program back, cut off by its length (not a runaway):
    // asked for edit blocks, never for the app shorter, its extras left out.
    let buttons: String = (0..20).map(|i| format!("button \"+{i}\" {{ n += {i}; }}\n")).collect();
    let base = [CLEAN, &buttons].concat();
    let cut = ["```app\n", &base].concat();
    let small = app("// Count.\nstate n = 0;\nlabel n;\n");
    let fix = edit("label n;\n", "label n;\nlabel \"Big\";\n");
    let replies = [(cut.as_str(), "length"), (&small, "stop"), (&fix, "stop")];
    let (bodies, _, done) = run(task("make it bigger", &base), Knobs::default(), &replies);
    let asked = user(&bodies[1]);
    assert!(asked.starts_with(&user(&bodies[0])) && !asked.contains("100 lines"), "{asked}");
    assert!(asked.ends_with("Reply with edit blocks only, never the whole program."));
    // The app cut down that came back anyway was not taken; the edits that came next were.
    assert!(user(&bodies[2]).contains("dropped over a quarter of the program's lines"));
    assert_eq!(
        (done.outcome, done.draft),
        (Outcome::Ready, base.replace("label n;\n", "label n;\nlabel \"Big\";\n"))
    );
    let turns: Vec<Turn> = done.receipt.turns.iter().map(|t| t.turn).collect();
    assert_eq!(turns, [Turn::Change, Turn::Shorter, Turn::Missed]);
}

#[test]
fn budgets_failures_stops_and_runaways_end_with_the_best_so_far() {
    // Fixes back and forth until a budget is spent (requests, output tokens, dollars, time): the
    // best so far, a new app's that compiles, is installed faulting; one whose time ran out while
    // its reply streamed never checks that reply.
    let (broken, faults) = (app(BROKEN), app(FAULTS));
    let six: Vec<(&str, &str)> =
        (0..6).map(|i| ([&broken, &faults][i % 2].as_str(), "stop")).collect();
    let k = Knobs::default();
    for (k, n, said) in [
        (k, 5, "runs, but faults \u{b7} E0203 line 3"),
        (Knobs { out_tokens: 6000, ..k }, 3, "runs, but faults \u{b7} E0203 line 3"),
        (Knobs { ms: 1250, ..k }, 2, "couldn't \u{b7} E0302 line 2"),
    ] {
        let (bodies, _, done) = run(task("x", ""), k, &six);
        assert_eq!((bodies.len(), done.said().as_str()), (n, said), "{k:?}");
    }
    // The AI failing mid-make ends it with what it had; so does Stop.
    let (mut m, _) = Make::start(task("x", ""), k, 0);
    assert!(m.data(&sse(&app(FAULTS), "stop"), 10).is_none() && m.end(200, "", 15).is_none());
    assert!(matches!(m.check(20), Some(Out::Ask(_))));
    let Some(Out::Done(done)) = m.end(402, "", 30) else { panic!() };
    assert_eq!(
        (done.outcome, done.install, done.said().as_str()),
        (Outcome::Failed, true, "out of AI for today")
    );
    let (mut m, _) = Make::start(task("x", ""), k, 0);
    m.data(&sse(&app(FAULTS), "stop"), 10);
    m.end(200, "", 15);
    m.check(20);
    m.data(delta("reasoning", "so").as_bytes(), 30);
    let done = m.stop(40);
    assert_eq!((done.outcome, done.install, done.draft.as_str()), (Outcome::Stopped, true, FAULTS));
    assert!(done.receipt.est && done.receipt.turns.len() == 2);
    // Thinking past its budget with nothing written: asked once more, for less and never with the
    // same body (the gateway sends a body it has seen to the provider that answered it), then
    // E0909.
    let (mut m, _) = Make::start(task("x", ""), Knobs { runaway: 10, ..k }, 0);
    for round in 0..2 {
        assert!(m.data(delta("reasoning", &"x".repeat(20)).as_bytes(), 5).is_none());
        assert!(matches!(
            m.data(delta("reasoning", &"x".repeat(20)).as_bytes(), 6),
            Some(Out::Cancel)
        ));
        match m.check(7).unwrap() {
            Out::Ask(body) => assert!(user(&body).starts_with("Make: x\n\nYour reply ran out")),
            Out::Done(done) => {
                assert_eq!((round, done.said().as_str()), (1, "couldn't \u{b7} E0909"))
            }
            Out::Cancel => panic!(),
        }
    }
    // A fix has less room, so less thinking is a runaway: here 20 tokens, half a write's 40; it is
    // asked again told so.
    let k = Knobs { runaway: 40, ..k };
    let (mut m, _) = Make::start(task("x", ""), Knobs { fix_tokens: k.write_tokens / 2, ..k }, 0);
    m.data(&sse(&app(BROKEN), "stop"), 1);
    let Some(Out::Ask(fix)) = m.end(200, "", 2).or_else(|| m.check(2)) else { panic!() };
    assert!(m.data(delta("reasoning", &"x".repeat(60)).as_bytes(), 3).is_none());
    assert!(matches!(m.data(delta("reasoning", &"x".repeat(10)).as_bytes(), 4), Some(Out::Cancel)));
    let Some(Out::Ask(again)) = m.check(5) else { panic!() };
    let rush = "\n\nYou thought past your room and wrote nothing. Decide quickly, then reply.";
    assert_eq!(user(&again), user(&fix) + rush);
}

#[test]
fn budgets_hold_with_every_reply_at_its_whole_room() {
    // Each reply uses all the room its request had (as runaways on providers that ignore the
    // thinking budget do): a request goes out only while its whole room and its input fit what is
    // left, so neither the dollars (GLM 5.3) nor the output tokens (Flash) are ever overrun.
    let room = |b: &str| Json::parse(b).and_then(|b| b.get("max_tokens")?.text()?.parse().ok());
    let room = |b: &str| room(b).unwrap();
    let nope = |n: u32| format!("label nope{n};\n");
    let write = ["```app\n// x\nstate n = 0;\n", &nope(1)].concat();
    let fixes: Vec<String> = (1..6).map(|n| edit(&nope(n), &nope(n + 1))).collect();
    let mut replies = vec![(write.as_str(), "stop")];
    replies.extend(fixes.iter().map(|f| (f.as_str(), "stop")));
    let k = Knobs::default();
    for (model, n) in [(DEFAULT_MODEL, 3), ("zai/glm-5.3-flash", 4)] {
        let t = Task { model: model.into(), ..task("x", "") };
        let (bodies, _, done) = drive(t, k, &replies, 61, room);
        let out: u32 = done.receipt.turns.iter().map(|t| t.output).sum();
        let r = &done.receipt;
        assert!(r.usd_micros <= k.usd_micros && out <= k.out_tokens, "{model}: {r:?}");
        assert_eq!((bodies.len(), done.outcome), (n, Outcome::Broken), "{model}");
    }
    // A request's room shrinks to what is left, down to half its usual.
    let k = Knobs { out_tokens: 6144 + 3000, ..k };
    let (bodies, _, _) = drive(task("x", ""), k, &replies, 61, room);
    let rooms: Vec<u32> = bodies.iter().map(|b| room(b)).collect();
    assert_eq!(rooms, [6144, 3000]);
}

#[test]
fn a_reply_reads_the_same_however_it_is_fenced_or_chunked() {
    // A change's edits, each in its own app fence (as models used to Aider fence them), and a
    // write with an example before its program: the whole reply is read, in one chunk or many.
    let reset = "button \"+\" { n += 1; }\nbutton \"Reset\" { n = 0; }\n";
    let minus = "label n;\nbutton \"-\" { n -= 1; }\n";
    let each = app(&edit("button \"+\" { n += 1; }\n", reset)) + &app(&edit("label n;\n", minus));
    let both = CLEAN.replace("button \"+\" { n += 1; }\n", reset).replace("label n;\n", minus);
    let example =
        ["A label shows text:\n```app\nlabel \"Count\";\n```\nSo:\n", &app(CLEAN)].concat();
    for size in [1 << 20, 61, 7] {
        let t = task("add Reset and minus buttons", CLEAN);
        let (_, _, done) = drive(t, Knobs::default(), &[(&each, "stop")], size, |_| 1500);
        assert_eq!((done.outcome, done.draft.as_str()), (Outcome::Ready, both.as_str()), "{size}");
        let t = task("a counter", "");
        let (_, _, done) = drive(t, Knobs::default(), &[(&example, "stop")], size, |_| 1500);
        assert_eq!((done.outcome, done.draft.as_str()), (Outcome::Ready, CLEAN), "{size}");
    }
}

#[test]
fn every_end_shows_its_fault_or_a_code_of_the_makes_own() {
    // Edits that never close, for a new app: no program, so the make's own code (gave up), never
    // applang's (E0001 is an unexpected character).
    let open = "<<<<<<< SEARCH\nlabel n;\n=======\nlabel m;\n";
    let (_, _, done) = run(task("x", ""), Knobs::default(), &[(open, "stop"), (open, "stop")]);
    assert_eq!((done.outcome, done.said().as_str()), (Outcome::Broken, "couldn't \u{b7} E0919"));
    let (broken, faults, cut) = (app(BROKEN), app(FAULTS), "```app\nstate a = 0;\nlabel \"T");
    let ends: [&[(&str, &str)]; 7] = [
        &[(open, "stop"), (open, "stop")],
        &[("Sorry.", "stop"), ("Hm.", "stop")],
        &[(cut, "length"), (cut, "length")],
        &[(&broken, "stop"), (&broken, "stop"), (&broken, "stop")],
        &[(&faults, "stop"), (&faults, "stop"), (&faults, "stop")],
        &[("// applang has no network.", "stop")],
        &[(&broken, "stop")],
    ];
    for replies in ends {
        let (_, _, d) = run(task("x", ""), Knobs::default(), replies);
        let shown = fault(&d.draft, "", 3).and_then(|f| f.diag.code).filter(|_| d.mark.is_some());
        let own = matches!(d.code, 0 | 901..=910 | 919) && d.ended == d.code;
        let ended = matches!((d.outcome, d.ended), (Outcome::Stopped, 0) | (_, 906..=910 | 919));
        assert!(own || shown == Some(d.code) && ended, "{replies:?}: {} {}", d.code, d.said());
    }
}

#[test]
fn a_change_never_makes_an_app_worse() {
    // Asked with the program numbered; an edit to it that runs clean is the app now.
    let t = task("count by two", CLEAN);
    let by2 = edit("button \"+\" { n += 1; }\n", "button \"+\" { n += 2; }\n");
    let (bodies, seen, done) = run(t.clone(), Knobs::default(), &[(&by2, "stop")]);
    let want = "The program, numbered:\n  1| // Count: + adds one.\n  2| state n = 0;";
    assert!(
        user(&bodies[0]).starts_with(want)
            && user(&bodies[0]).ends_with("Change it: count by two\nKeep its first comment true.")
    );
    assert!(seen.contains(&"changing \u{b7} 1 edit".into()), "{seen:?}");
    assert_eq!((done.outcome, done.install, done.change), (Outcome::Ready, true, true));
    assert_eq!(done.draft, CLEAN.replace("n += 1", "n += 2"));
    // One that faults is fixed; one that never runs clean leaves the app as it was.
    let bad = edit("label n;\n", "label 1 / (n - n);\n");
    let worse = app(&CLEAN.replace("label n;", "label 1 / (n - n);"));
    let replies = [(bad.as_str(), "stop"), (worse.as_str(), "stop"), (worse.as_str(), "stop")];
    let (_, _, done) = run(t, Knobs::default(), &replies);
    assert_eq!(
        (done.outcome, done.install, done.said().as_str()),
        (Outcome::Broken, false, "couldn't change it \u{b7} E0203 line 5")
    );
}

#[test]
fn makes_replay_bit_for_bit() {
    let replies = [(app(BROKEN), "stop"), (edit("label nope;\n", "label n;\n"), "stop")];
    let replies: Vec<(&str, &str)> = replies.iter().map(|(a, f)| (a.as_str(), *f)).collect();
    let a = run(task("count", ""), Knobs::default(), &replies);
    let b = run(task("count", ""), Knobs::default(), &replies);
    assert_eq!((a.0, a.1, format!("{:?}", a.2)), (b.0, b.1, format!("{:?}", b.2)));
}

#[test]
fn receipts_are_priced_and_recorded() {
    let (_, _, done) = run(task("a counter", ""), Knobs::default(), &[(&app(CLEAN), "stop")]);
    let line = receipt::line("/x.app", 1, 5, &done);
    let usd = receipt::usd(done.receipt.usd_micros);
    let want = format!(
        "{{\"v\":1,\"path\":\"/x.app\",\"kind\":\"make\",\"outcome\":\"ready\",\"version\":1,\"lines\":5,\"plan\":\"Count: + adds one.\",\"requests\":1,\"usd\":{usd},\"est\":true,\"ms\":"
    );
    assert!(line.starts_with(&want) && line.ends_with(",\"fault\":\"\"}\n"), "{line}");
    assert_eq!(
        Json::parse(line.trim_end()).map(|j| j.get("requests").cloned()),
        Some(Some(Json::Num("1".into())))
    );
    assert_eq!(
        [receipt::usd(26_249), receipt::usd(80_000), receipt::usd(1)],
        ["0.0262", "0.0800", "0.0000"]
    );
    // The newest half, from a line's start, once over 256 KiB.
    let text: String = (0..5000).map(|i| format!("{{\"n\":{i:052}}}\n")).collect();
    assert!(receipt::rotate(&text[..200_000]).is_none());
    let kept = receipt::rotate(&text).unwrap();
    assert!(
        kept.len() <= receipt::MAX_MAKES / 2
            && text.ends_with(&kept)
            && kept.starts_with("{\"n\":")
    );
    // Prices and the free AI's own: its MODELS line, read where it lives.
    let proxy = include_str!("../../../api/ai.mjs");
    let models = "const MODELS = { 'zai/glm-5.3': [1.4, 4.4], 'zai/glm-5.3-flash': [0.15, 0.5] };";
    assert!(proxy.contains(models));
    // As the gateway charged a reply of 1,961 tokens in (986 cached) and 2,751 out: $0.01372576.
    let u = Usage { input: 1961, cached: 986, output: 2751, reasoning: 1037 };
    assert_eq!(
        (receipt::price("zai/glm-5.3", &u), receipt::price("zai/glm-5.3-flash", &u)),
        (13726, 1549)
    );
}
