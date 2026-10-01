use std::collections::BTreeMap;

use super::*;
use json::Json;

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

/// The Ai request a frame makes.
fn ai(f: &Frame) -> (u32, Json) {
    let ai = |r: &Request| match r {
        Request::Ai { id, body } => Some((*id, Json::parse(body).expect("a JSON body"))),
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
    assert_eq!(f.requests, [Request::Size { w: 560, h: 640 }]);
    assert!(f.title == "Assistant" && has(&f, DEFAULT_MODEL) && has(&f, "Send"));
    assert!(w.send(&[Event::Resize { w: 700, h: 700 }]).is_empty());
    // No key to ask for: the model the desktop names, and nothing else above the prompt.
    let f = w.last(&[config()]);
    assert!(has(&f, "m/x") && !has(&f, DEFAULT_MODEL) && f.requests.is_empty());
    assert_eq!(words(&f.nodes), ["Assistant", "m/x", "Send"]);
    // A blank prompt sends nothing.
    assert!(w.last(&[Event::Click { id: SEND }]).requests.is_empty());
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
    assert_eq!((messages(&body), message(&body, 1)), (2, ("user", "Hi there")));
    let f = w.a.frame();
    assert!(has(&f, "Hi there") && has(&f, "\u{2026}") && has(&f, "Stop") && !has(&f, "Send"));
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
    assert_eq!(f.requests, [Request::Open { name: path }]);
    assert!(has(&f, "Built counter.app \u{2713}") && has(&f, "Enjoy."));
    let mono = Node::Text { id: 0, style: Style::Mono, text: COUNTER.trim().into() };
    assert!(f.nodes.contains(&mono));
    let row = format!("{{\"prompt\":\"make me a counter app\",\"program\":{},", quote(COUNTER));
    let corpus = &w.disk.0[&[HOME, "/.ai/corpus.jsonl"].concat()];
    assert_eq!(*corpus, row + "\"attempts\":1,\"model\":\"m/x\"}\n");
}

#[test]
fn a_program_that_does_not_compile_goes_back_twice() {
    let mut w = Win::new();
    let (mut id, _) = w.ask("a todo app");
    for attempt in 1..=3 {
        let f = w.answer(id, "Sure.\n```app\nstate n = 0;\nlabel;\n```", "", 7);
        assert!(has(&f, "The program did not compile: E0"));
        if attempt == 3 {
            assert!(f.requests.is_empty() && has(&f, "Send") && w.disk.0.is_empty());
            let last = f.nodes.iter().rev().nth(1);
            assert!(matches!(last, Some(Node::Text { style: Style::Error, .. })));
            break;
        }
        let body;
        (id, body) = ai(&f);
        let (role, text) = message(&body, messages(&body) - 1);
        assert!(role == "user" && text.contains(" 2:") && text.ends_with("in one app block."));
    }
    // A fix on the retry is built, its attempts counted against the prompt.
    let mut w = Win::new();
    let (id, _) = w.ask("a counter app");
    let f = w.answer(id, "```app\nlabel\n```", "", 64);
    assert!(has(&w.answer(ai(&f).0, &format!("```app\n{COUNTER}"), "", 64), "Built counter.app"));
    let corpus = &w.disk.0[&[HOME, "/.ai/corpus.jsonl"].concat()];
    assert!(corpus.contains("\"prompt\":\"a counter app\"") && corpus.contains("\"attempts\":2"));
    // A disk that cannot take it says so.
    let mut w = Win { disk: Mem(BTreeMap::new(), true), ..Win::new() };
    let (id, _) = w.ask("again");
    assert!(has(&w.answer(id, &format!("```app\n{COUNTER}```"), "", 64), "Couldn't save"));
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
}

#[test]
fn slugs_blocks_and_problems() {
    assert_eq!(slug("state n = 0; // label \"no\"\nlabel \"My  Todo-List!\";"), "my-todo-list");
    assert_eq!(slug("state n = 0; button \"x\" { n = 1; }"), "app");
    assert_eq!(slug(&format!("label \"{}\";", "ab ".repeat(20))).len(), 32);
    assert_eq!(app_block("```app \nlabel 1;"), Some("label 1;"));
    assert_eq!(app_block("```rust\nfn x\n```\n"), None);
    let src = "state n = 0;\n  label;";
    let d = applang::compile(src).unwrap_err();
    assert!(problem(&d, src).starts_with(&format!("E{:04} 2:", d.code.unwrap())));
    let many: String = (0..40).map(|i| format!("p{i}\n```\nc{i}\n```\n")).collect();
    assert_eq!(blocks(&many).len(), MAX_BLOCKS);
}
