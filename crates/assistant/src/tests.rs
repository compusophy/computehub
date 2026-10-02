use std::collections::BTreeMap;

use super::*;
use ai::{HOME, app_block, problem};
use json::{Json, quote};

/// A disk in memory; a full one fails every write.
#[derive(Default)]
struct Mem(BTreeMap<String, String>, bool);

impl Disk for Mem {
    fn put(&mut self, path: &str, text: &str, append: bool) -> io::Result<()> {
        (!self.1).then_some(()).ok_or_else(|| io::Error::other("disk full"))?;
        let file = self.0.entry(path.into()).or_default();
        *file = if append { [file.as_str(), text].concat() } else { text.into() };
        Ok(())
    }

    fn exists(&mut self, path: &str) -> bool {
        self.0.contains_key(path)
    }
}

/// The draw device: every write is one frame.
struct Sink<'a>(&'a mut Vec<Vec<u8>>);

impl Write for Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Win {
    a: Assistant,
    disk: Mem,
}

impl Win {
    /// A window that has drawn once and heard the model.
    fn new() -> Win {
        let mut w = Win::default();
        w.send(&[Event::Resize { w: 600, h: 600 }, config()]);
        w
    }

    /// Serves `events` over in-memory pipes, one per read; the frames sent.
    fn send(&mut self, events: &[Event]) -> Vec<Frame> {
        let none: Box<dyn Read> = Box::new(io::empty());
        let feed = events.iter().fold(none, |f, e| Box::new(f.chain(io::Cursor::new(e.encode()))));
        let mut sent = Vec::new();
        serve(&mut Client::new(feed, Sink(&mut sent)), &mut self.a, &mut self.disk).unwrap();
        sent.iter().map(|f| Frame::decode(f).expect("a frame that decodes")).collect()
    }

    fn last(&mut self, events: &[Event]) -> Frame {
        self.send(events).pop().expect("a frame")
    }

    /// Types and submits `prompt`; the request's id and body.
    fn ask(&mut self, prompt: &str) -> (u32, Json) {
        let (id, text) = (self.a.input_id(), prompt.into());
        ai(&self.last(&[Event::Change { id, version: 1, text }, Event::Submit { id }]))
    }

    /// Answers request `id` with `reply`, a delta per word, then `extra`, in
    /// `n`-byte chunks, then the end; the last frame.
    fn answer(&mut self, id: u32, reply: &str, extra: &str, n: usize) -> Frame {
        let body: String = reply.split_inclusive(' ').map(chunk).collect();
        let body = [&body, extra, "data: [DONE]\n\n"].concat();
        let mut evs: Vec<_> = body.as_bytes().chunks(n).map(|c| data(id, c)).collect();
        evs.push(Event::AiEnd { id, status: 200, error: String::new() });
        let frames = self.send(&evs);
        assert_eq!(frames.len(), evs.len(), "a frame per event of the request in flight");
        frames.into_iter().last().unwrap()
    }
}

fn config() -> Event {
    Event::Config { model: "m/x".into() }
}

fn data(id: u32, bytes: &[u8]) -> Event {
    Event::AiData { id, data: bytes.to_vec() }
}

/// An SSE event carrying one content delta, its line ended with CRLF.
fn chunk(text: &str) -> String {
    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\r\n\n", quote(text))
}

/// The Ai request a frame makes, which the free AI takes.
fn ai(f: &Frame) -> (u32, Json) {
    let ai = |r: &Request| match r {
        Request::Ai { id, body } if body.len() <= MAX_BODY => {
            Some((*id, Json::parse(body).expect("a JSON body")))
        }
        Request::Ai { body, .. } => panic!("a body of {} bytes", body.len()),
        _ => None,
    };
    f.requests.iter().find_map(ai).expect("an Ai request")
}

/// Every Text's and Button's text, depth first.
fn words(nodes: &[Node]) -> Vec<String> {
    let each = |n: &Node| match n {
        Node::Text { text, .. } | Node::Button { label: text, .. } => vec![text.clone()],
        n => words(n.children()),
    };
    nodes.iter().flat_map(each).collect()
}

fn has(f: &Frame, s: &str) -> bool {
    words(&f.nodes).iter().any(|w| w.contains(s))
}

/// The `i`th message of a request body: (role, content).
fn message(body: &Json, i: usize) -> (&str, &str) {
    let m = body.get("messages").and_then(|m| m.at(i)).expect("a message");
    (m.get("role").and_then(Json::text).unwrap(), m.get("content").and_then(Json::text).unwrap())
}

fn messages(body: &Json) -> usize {
    (0..).take_while(|i| body.get("messages").and_then(|m| m.at(*i)).is_some()).count()
}

const COUNTER: &str = "state count = 0;\nlabel \"Counter\";\nbutton \"+\" { count = count + 1; }\n";

#[test]
fn opens_ready_and_shows_the_model() {
    let mut w = Win::default();
    let f = w.last(&[Event::Resize { w: 600, h: 600 }]);
    assert_eq!(f.requests, [Request::Size { w: 560, h: 640 }, Request::Focus { id: INPUT }]);
    assert!(f.title == "Assistant" && has(&f, DEFAULT_MODEL) && has(&f, "Send"));
    assert!(w.send(&[Event::Resize { w: 700, h: 700 }]).is_empty());
    // No key to ask for: the model the desktop names, and nothing else above the prompt.
    let f = w.last(&[config()]);
    assert!(has(&f, "m/x") && !has(&f, DEFAULT_MODEL) && f.requests.is_empty());
    assert_eq!(words(&f.nodes), ["Assistant", "m/x", "Send"]);
    // A blank prompt sends nothing; a sent one empties the prompt, which keeps the keyboard.
    assert!(w.last(&[Event::Click { id: SEND }]).requests.is_empty());
    let f = w.last(&[
        Event::Change { id: INPUT, version: 1, text: "hi".into() },
        Event::Submit { id: INPUT },
    ]);
    assert_eq!(f.requests[1..], [Request::Focus { id: INPUT + 1 }]);
}

#[test]
fn streams_a_reply_split_anywhere() {
    let mut w = Win::new();
    let (id, body) = w.ask("  Hi there ");
    let s = |k| body.get(k).and_then(Json::text);
    assert_eq!([s("model"), s("max_tokens")], [Some("m/x"), Some("4096")]);
    assert_eq!(s("temperature"), Some("0.3"));
    let usage = body.get("stream_options").and_then(|o| o.get("include_usage"));
    assert_eq!([body.get("stream"), usage], [Some(&Json::Bool(true)); 2]);
    let (role, system) = message(&body, 0);
    assert!(role == "system" && system.contains(applang::REFERENCE));
    // No reasoning field (the free AI turns thinking off); the program says what it is.
    assert!(body.get("reasoning").is_none() && system.contains(ai::HONEST));
    assert!(system.ends_with(&[ai::HONEST, ai::EXAMPLE].concat()));
    assert!(ai::EXAMPLE.starts_with("```app\n// Counter: - and + change the number.\nstate"));
    assert_eq!((messages(&body), message(&body, 1)), (2, ("user", "Hi there")));
    let f = w.a.frame();
    assert!(has(&f, "Hi there") && has(&f, "\u{2026}") && has(&f, "Stop") && !has(&f, "Send"));
    // Reasoning is counted, never taken as the reply.
    let think = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Hm\u{e9}.\"}}]}\n\n";
    let f = w.last(&[data(id, think.as_bytes())]);
    assert!(has(&f, "\u{2026}") && !has(&f, "Hm") && w.a.run.as_ref().unwrap().stream.thought == 4);
    // Three-byte chunks split lines, CRLFs and chars; usage comes last.
    let reply = "H\u{e9}llo \u{2014} w\u{f6}rld \u{2713} \u{1f600}";
    let usage = "data: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\n";
    let f = w.answer(id, reply, usage, 3);
    assert!(has(&f, reply) && has(&f, "12 tokens in, 5 out") && has(&f, "Send"));
    let input = f.nodes.last().map(|row| row.children()[0].clone());
    assert!(matches!(input, Some(Node::Input { id: 101, value, .. }) if value.is_empty()));
    let (_, body) = w.ask("again");
    assert_eq!((messages(&body), message(&body, 2)), (4, ("assistant", reply)));
}

#[test]
fn builds_an_app_and_feeds_the_corpus() {
    let mut w = Win::new();
    let (id, _) = w.ask("make me a counter app");
    let f = w.answer(id, &format!("Here it is.\n```app\n{COUNTER}```\nEnjoy."), "", 5);
    let path = [HOME, "/apps/counter.app"].concat();
    assert_eq!(w.disk.0.get(&path).map(String::as_str), Some(COUNTER));
    assert_eq!(f.requests, [Request::Open { name: ["studio:", &path].concat() }]);
    assert!(has(&f, "Built counter.app \u{2713}") && has(&f, "Enjoy."));
    let mono = Node::Text { id: 0, style: Style::Mono, text: COUNTER.trim().into() };
    assert!(f.nodes.contains(&mono));
    let row = format!("{{\"prompt\":\"make me a counter app\",\"program\":{},", quote(COUNTER));
    let corpus = &w.disk.0[CORPUS];
    assert_eq!(*corpus, row + "\"attempts\":1,\"model\":\"m/x\"}\n");
    // Another counter (here, or one made in Studio) is never saved over: it takes the next name.
    w.disk.0.insert(path.clone(), "label \"Studio's\";".into());
    let (id, _) = w.ask("a counter again");
    let f = w.answer(id, &format!("```app\n{COUNTER}```"), "", 64);
    let two = [HOME, "/apps/counter-2.app"].concat();
    assert_eq!(
        (w.disk.0[&path].as_str(), w.disk.0[&two].as_str()),
        ("label \"Studio's\";", COUNTER)
    );
    assert_eq!(f.requests, [Request::Open { name: ["studio:", &two].concat() }]);
    assert!(has(&f, "Built counter-2.app \u{2713}"));
}

#[test]
fn a_program_that_does_not_run_goes_back_three_times_with_its_line_and_rule() {
    let mut w = Win::new();
    let (mut id, _) = w.ask("a todo app");
    for attempt in 1..=4 {
        let f = w.answer(id, "Sure.\n```app\nstate n = 0;\nif n == 0 { label n }\n```", "", 7);
        assert!(has(&f, "The program did not compile: E0101 2:21"));
        if attempt == 4 {
            assert!(f.requests.is_empty() && has(&f, "Send") && w.disk.0.is_empty());
            let last = f.nodes.iter().rev().nth(1);
            assert!(matches!(last, Some(Node::Text { style: Style::Error, .. })));
            break;
        }
        let body;
        (id, body) = ai(&f);
        // After the reply: what broke, its line with a caret, the rule and every occurrence.
        let (role, text) = message(&body, messages(&body) - 1);
        let want = "Your program did not compile: E0101 at line 2, col 21: expected `;`, found \
                    `}`\n  if n == 0 { label n }\n                      ^\nRule: label, input,";
        assert!(role == "user" && text.starts_with(want), "{text}");
        assert!(text.contains("fix every occurrence.\nReply with the corrected full program"));
        assert_eq!(message(&body, messages(&body) - 2).0, "assistant");
    }
    // One that compiles but faults when it first renders goes back too.
    let (id, _) = w.ask("an average");
    let f = w.answer(id, "```app\nstate n = 0;\nlabel 1 / n;\n```", "", 64);
    assert!(has(&f, "The program faults when it first renders: E0203 2:7"));
    let (_, body) = ai(&f);
    let fix = message(&body, messages(&body) - 1).1;
    assert!(fix.contains("E0203 at line 2, col 7: ") && fix.contains("Rule: guard every /"));
    // A fix on the retry is built, its attempts counted against the prompt.
    let mut w = Win::new();
    let (id, _) = w.ask("a counter app");
    let f = w.answer(id, "```app\nlabel\n```", "", 64);
    assert!(has(&w.answer(ai(&f).0, &format!("```app\n{COUNTER}"), "", 64), "Built counter.app"));
    let corpus = &w.disk.0[CORPUS];
    assert!(corpus.contains("\"prompt\":\"a counter app\"") && corpus.contains("\"attempts\":2"));
    // A disk that cannot take it says so.
    let mut w = Win { disk: Mem(BTreeMap::new(), true), ..Win::new() };
    let (id, _) = w.ask("again");
    assert!(has(&w.answer(id, &format!("```app\n{COUNTER}```"), "", 64), "Couldn't save"));
}

#[test]
fn a_reply_out_of_room_asks_for_the_same_app_shorter_then_says_e0907() {
    let long = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n";
    let mut w = Win::new();
    let (mut id, _) = w.ask("a tetris game");
    for attempt in 1..=4 {
        let f = w.answer(id, "Here.\n```app\nstate n = 0;\nlabel \"Tet", long, 64);
        if attempt == 4 {
            assert!(has(&f, ROOM) && f.requests.is_empty() && w.disk.0.is_empty());
            break;
        }
        assert!(has(&f, "The program ran out of room; asking for a shorter one."));
        let body;
        (id, body) = ai(&f);
        assert_eq!(message(&body, messages(&body) - 1), ("user", SHORTER));
    }
    // A cut-off reply that still holds a whole program is built; a cut-off chat says so.
    let notes = |w: &Win| -> Vec<String> {
        w.a.turns.last().unwrap().notes.iter().map(|n| n.1.clone()).collect()
    };
    let (id, _) = w.ask("a counter");
    w.answer(id, &format!("```app\n{COUNTER}```\nIt counts"), long, 64);
    assert_eq!(notes(&w), ["Built counter.app \u{2713} \u{2014} open in Studio to change it"]);
    let (id, _) = w.ask("a long story");
    let f = w.answer(id, "Once upon", long, 64);
    assert!(has(&f, "Once upon") && notes(&w) == [ROOM] && f.requests.is_empty());
    // All thinking and no reply: out of room, not "No reply.", and the prompt leaves the history.
    let (id, _) = w.ask("think hard");
    let think = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"Hm\"}}]}\n\n";
    w.answer(id, "", &[think, long].concat(), 64);
    assert_eq!(notes(&w), [ROOM]);
    let (_, body) = w.ask("next");
    assert_eq!(message(&body, messages(&body) - 2), ("assistant", "Once upon"));
    // Cut off inside its block, a reply holds part of a program, even one that compiles: it is
    // asked for shorter, never built. A whole program that faults gets a fix.
    let mut w = Win::new();
    let (id, _) = w.ask("a counter");
    let f = w.answer(id, &format!("```app\n{COUNTER}"), long, 64);
    assert!(has(&f, "The program ran out of room; asking for a shorter one.") && !has(&f, "Built"));
    let (id, body) = ai(&f);
    assert!(message(&body, messages(&body) - 1) == ("user", SHORTER) && w.disk.0.is_empty());
    let f = w.answer(id, "```app\nlabel\n```\nIt shows", long, 64);
    assert!(has(&f, "The program did not compile: E") && has(&f, "; asking for a fix."));
}

#[test]
fn failures_are_coded() {
    let no = "{\n\"error\":{\"message\":\"no\"}}";
    let cases = [
        (401, "", no, "E0901 the AI service refused the request: no"),
        (403, "", "", "E0901 the AI service refused the request"),
        (402, "", "", "E0902 the free AI is out of credit for now, try again later"),
        (429, "", "", "E0903 the free AI is busy, try again in a minute"),
        (404, "", "", "E0904 the model was not found or refused the request"),
        (503, "", "", "E0905 the AI is not available right now"),
        (502, "", "", "E0905 couldn't reach the AI service"),
        (0, "network", "", "E0905 network"),
        (200, "", "data: {\"error\":\"busy\"}\n", "E0905 the AI stopped with an error: busy"),
    ];
    let mut w = Win::new();
    for (status, error, body, want) in cases {
        let (id, req) = w.ask("hello");
        assert_eq!(messages(&req), 2, "an unanswered prompt leaves the history");
        let end = Event::AiEnd { id, status, error: error.into() };
        let f = w.last(&[data(id, body.as_bytes()), end]);
        assert!(has(&f, want), "{want}: {:?}", words(&f.nodes));
    }
}

#[test]
fn stop_cancels_and_ignores_the_rest() {
    let mut w = Win::new();
    let (id, _) = w.ask("long story");
    w.send(&[data(id, chunk("Once ").as_bytes())]);
    let f = w.last(&[Event::Click { id: STOP }]);
    assert_eq!(f.requests, [Request::AiCancel { id }]);
    assert!(has(&f, "Once") && has(&f, "Stopped.") && has(&f, "Send"));
    let end = Event::AiEnd { id, status: 0, error: "cancelled".into() };
    assert!(w.send(&[data(id, chunk("upon").as_bytes()), end]).is_empty());
    let (_, body) = w.ask("next");
    assert_eq!(message(&body, 2), ("assistant", "Once "));
}

#[test]
fn asks_from_the_everything_bar_are_sent_in_turn() {
    let mut w = Win::new();
    let ask = |text: &str| Event::Ask { text: text.into() };
    assert!(w.send(&[ask(" ")]).is_empty());
    let (id, body) = ai(&w.last(&[ask("first")]));
    assert_eq!(message(&body, 1), ("user", "first"));
    // One asked while a reply streams waits for it, then goes as if typed.
    let f = w.last(&[ask("second")]);
    assert!(f.requests.is_empty() && has(&f, "Stop"));
    // A draft typed meanwhile stays, in the same Input: an ask never goes through it.
    w.send(&[Event::Change { id: INPUT, version: 1, text: "my draft".into() }]);
    let f = w.answer(id, "One.", "", 64);
    let (_, body) = ai(&f);
    assert_eq!((messages(&body), message(&body, 3)), (4, ("user", "second")));
    assert!(has(&f, "second") && has(&f, "Stop"));
    let input = f.nodes.last().map(|row| row.children()[0].clone());
    assert!(matches!(input, Some(Node::Input { id: INPUT, value, .. }) if value == "my draft"));
}

#[test]
fn history_keeps_the_last_twelve_messages() {
    let mut w = Win::new();
    for i in 0..9 {
        let (id, _) = w.ask(&format!("q{i}"));
        w.answer(id, &format!("a{i}"), "", 64);
    }
    let (_, body) = w.ask("last");
    assert_eq!(messages(&body), 13);
    assert_eq!((message(&body, 1), message(&body, 12)), (("assistant", "a3"), ("user", "last")));
}

#[test]
fn requests_fit_what_the_free_ai_takes() {
    // Long prompts: each request (checked by `ai`) keeps the newest history that fits.
    let mut w = Win::new();
    for i in 0..7 {
        let prompt = format!("{i}{}", "x".repeat(16_000));
        let (id, body) = w.ask(&prompt);
        assert_eq!(message(&body, messages(&body) - 1), ("user", prompt.as_str()));
        w.answer(id, "ok", "", 64);
    }
    // One too long to send even alone is noted, not sent, and leaves the history.
    let long = Event::Change { id: w.a.input_id(), version: 1, text: "\u{1}".repeat(16 << 10) };
    let f = w.last(&[long, Event::Submit { id: w.a.input_id() }]);
    assert!(!f.requests.iter().any(|r| matches!(r, Request::Ai { .. })) && has(&f, "Send"));
    assert!(has(&f, "Not sent: too long for the AI (80 KiB at most)"));
    let (_, body) = w.ask("short");
    assert_eq!(message(&body, messages(&body) - 2), ("assistant", "ok"));
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
    let many: String = (0..40).map(|i| format!("p{i}\n```\nc{i}\n```\n")).collect();
    assert_eq!(blocks(&many).len(), MAX_BLOCKS);
}
