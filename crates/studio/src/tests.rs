use super::*;
use crate::edit::{CHECK, CHIP, CODE, MAKE, NEW, OPEN, PROMPT, STOP, TOGGLE, spans};
use crate::run::{EDIT, TOO_BIG};
use assistant::ai::{CORPUS, HOME};
use assistant::json::{Json, quote};
use std::collections::BTreeMap;
use uiwire::{Class, Key, Request, Variant, mods};

/// A disk in memory. Writes under `/ro/` fail, and so do reads under `/bad/`.
type Mem = BTreeMap<String, String>;

impl Disk for Mem {
    fn read(&mut self, path: &str) -> io::Result<String> {
        if path.starts_with("/bad/") {
            return Err(io::Error::other("device busy"));
        }
        self.get(path).cloned().ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no such file"))
    }
    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        if path.starts_with("/ro/") {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "read-only"));
        }
        self.insert(path.into(), text.into());
        Ok(())
    }
    fn append(&mut self, path: &str, text: &str) -> io::Result<()> {
        self.entry(path.into()).or_default().push_str(text);
        Ok(())
    }
    fn exists(&mut self, path: &str) -> bool {
        self.contains_key(path)
    }
}

const COUNTER: &str = include_str!("../samples/counter.app");
const GREETER: &str = "state name = \"\";\nstate waves = 0;\nlabel \"What is your name?\";\n\
    input name;\nif name != \"\" {\n  label \"Hello, \" + name + \"!\";\n  button \"Wave\" { waves = \
    waves + 1; }\n  label \"Waves: \" + waves;\n}\n";

fn with(files: &[(&str, &str)]) -> Mem {
    files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect()
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

/// The events device: one event per read, then the end.
fn feed(events: Vec<Vec<u8>>) -> impl Read {
    let none: Box<dyn Read> = Box::new(io::empty());
    events.into_iter().fold(none, |feed, ev| Box::new(feed.chain(io::Cursor::new(ev))))
}

/// A window: a view from program arguments, and its disk.
struct Win {
    view: Box<dyn View>,
    disk: Mem,
}

impl Win {
    fn new(args: &[&str], disk: Mem) -> Win {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        Win { view: view(&args).expect("a view"), disk }
    }
    /// Serves `events` over in-memory pipes, one per read; the frames sent.
    fn raw(&mut self, events: Vec<Vec<u8>>) -> Vec<Frame> {
        let mut sent = Vec::new();
        let mut ui = Client::new(feed(events), Sink(&mut sent));
        serve(&mut ui, self.view.as_mut(), &mut self.disk).unwrap();
        sent.iter().map(|f| Frame::decode(f).expect("a frame that decodes")).collect()
    }
    fn frames(&mut self, evs: &[Event]) -> Vec<Frame> {
        self.raw(evs.iter().map(Event::encode).collect())
    }
    /// The last frame `evs` brought, if any.
    fn send(&mut self, evs: &[Event]) -> Option<Frame> {
        self.frames(evs).pop()
    }
    fn last(&mut self, evs: &[Event]) -> Frame {
        self.send(evs).expect("a frame")
    }
    /// Types `prompt` into Studio's prompt (named in `f`) and makes it: the request.
    fn make(&mut self, f: &Frame, prompt: &str) -> (Frame, u32, Json) {
        let id = input(f).0;
        let f = self
            .last(&[Event::Change { id, version: 1, text: prompt.into() }, Event::Submit { id }]);
        let (rid, body) = ai(&f);
        (f, rid, body)
    }
    /// Answers request `id` with `reply` in deltas of 5 bytes, then the end; the frames.
    fn answer(&mut self, id: u32, reply: &str) -> Vec<Frame> {
        let mut evs: Vec<_> = reply.as_bytes().chunks(5).map(|c| content(id, c)).collect();
        evs.push(Event::AiData { id, data: b"data: [DONE]\n\n".to_vec() });
        evs.push(Event::AiEnd { id, status: 200, error: String::new() });
        self.frames(&evs)
    }
}

const WIDE: Event = Event::Resize { w: 860, h: 520 };
const NARROW: Event = Event::Resize { w: 420, h: 640 };

fn click(id: u32) -> Event {
    Event::Click { id }
}
fn change(id: u32, version: u32, text: &str) -> Event {
    Event::Change { id, version, text: text.into() }
}
fn config() -> Event {
    Event::Config { model: "m/x".into() }
}
/// An SSE event with a content delta of `bytes` (whole chars here).
fn content(id: u32, bytes: &[u8]) -> Event {
    let text = quote(std::str::from_utf8(bytes).unwrap());
    let data = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":{text}}}}}]}}\n\n");
    Event::AiData { id, data: data.into_bytes() }
}
fn think(id: u32, text: &str) -> Event {
    let data =
        format!("data: {{\"choices\":[{{\"delta\":{{\"reasoning\":{}}}}}]}}\n\n", quote(text));
    Event::AiData { id, data: data.into_bytes() }
}
fn app(src: &str) -> String {
    format!("Here it is.\n```app\n{src}```\n")
}

/// Every node, depth first.
fn all(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().flat_map(|n| std::iter::once(n).chain(all(n.children()))).collect()
}
/// Every Text's text and Button's label, depth first.
fn words(f: &Frame) -> Vec<&str> {
    fn word(n: &Node) -> Option<&str> {
        match n {
            Node::Text { text, .. } | Node::Button { label: text, .. } => Some(text),
            _ => None,
        }
    }
    all(&f.nodes).into_iter().filter_map(word).collect()
}
fn has(f: &Frame, s: &str) -> bool {
    words(f).iter().any(|w| w.contains(s))
}
/// Studio's prompt: its id, value and placeholder.
fn input(f: &Frame) -> (u32, &str, &str) {
    let found = all(&f.nodes).into_iter().find_map(|n| match n {
        Node::Input { id, value, placeholder } if (PROMPT..CODE).contains(id) => {
            Some((*id, value.as_str(), placeholder.as_str()))
        }
        _ => None,
    });
    found.expect("a prompt")
}
/// Studio's Code: id, version, text and spans.
fn code(f: &Frame) -> (u32, u32, &str, &[uiwire::Span]) {
    let found = all(&f.nodes).into_iter().find_map(|n| match n {
        Node::Code { id, version, text, spans, .. } => {
            Some((*id, *version, text.as_str(), &spans[..]))
        }
        _ => None,
    });
    found.expect("a code editor")
}
/// The Ai request a frame makes.
fn ai(f: &Frame) -> (u32, Json) {
    let ai = |r: &Request| match r {
        Request::Ai { id, body } => Some((*id, Json::parse(body).expect("a JSON body"))),
        _ => None,
    };
    f.requests.iter().find_map(ai).expect("an Ai request")
}
/// The `i`th message of a request body: (role, content).
fn message(body: &Json, i: usize) -> (&str, &str) {
    let m = body.get("messages").and_then(|m| m.at(i)).expect("a message");
    (m.get("role").and_then(Json::text).unwrap(), m.get("content").and_then(Json::text).unwrap())
}
fn messages(body: &Json) -> usize {
    (0..).take_while(|i| body.get("messages").and_then(|m| m.at(*i)).is_some()).count()
}
fn classes<'t>(text: &'t str, spans: &[uiwire::Span]) -> Vec<(&'t str, Class)> {
    let at = |s: &uiwire::Span| &text[s.start as usize..(s.start + s.len) as usize];
    spans.iter().map(|s| (at(s), s.class)).collect()
}

#[test]
fn opens_empty_asking_what_to_make() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    assert_eq!(f.title, "Studio");
    let focus = Request::Focus { id: PROMPT };
    assert_eq!(f.requests, [Request::Size { w: 880, h: 560 }, focus]);
    // Centered: room above and below, room either side of a 520 px column.
    let [Node::Fill { .. }, Node::Row { children, .. }, Node::Fill { .. }, Node::Spacer { .. }] =
        &f.nodes[..]
    else {
        panic!("{:?}", f.nodes)
    };
    assert!(matches!(
        &children[..],
        [Node::Col { .. }, Node::Pane { w: 520, .. }, Node::Col { .. }]
    ));
    assert_eq!(input(&f), (PROMPT, "", "Describe an app\u{2026}"));
    let chips: Vec<_> = all(&f.nodes)
        .into_iter()
        .filter_map(|n| match n {
            Node::Button { id, variant: Variant::Chip, label } => Some((*id, label.as_str())),
            _ => None,
        })
        .collect();
    let want: Vec<_> = (CHIP..)
        .zip(["a tip calculator", "a pomodoro timer", "a habit tracker", "a dice roller"])
        .collect();
    assert_eq!(chips, want);
    assert_eq!(
        words(&f),
        [
            "What do you want to make?",
            "Make",
            "a tip calculator",
            "a pomodoro timer",
            "a habit tracker",
            "a dice roller"
        ]
    );
    // A blank prompt makes nothing; neither do the buttons of an app not yet made.
    assert!(w.last(&[click(MAKE)]).requests.is_empty());
    assert!(w.send(&[click(TOGGLE), click(OPEN), click(CHECK)]).is_none());
    // A chip fills the prompt and makes it; the chips go while it runs.
    let f = w.last(&[config(), click(CHIP + 3)]);
    let (_, body) = ai(&f);
    assert_eq!(message(&body, 1), ("user", "a dice roller"));
    assert!(f.requests.contains(&Request::Focus { id: PROMPT + 1 }));
    assert_eq!(input(&f).1, "a dice roller");
    assert!(has(&f, "Stop") && !has(&f, "Make") && !has(&f, "a tip calculator"));
    assert!(has(&f, "Asking m/x\u{2026}"));
    // New opens another Studio.
    assert_eq!(w.last(&[click(NEW)]).requests, [Request::Open { name: "studio".into() }]);
}

#[test]
fn makes_checks_fixes_and_saves() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    assert!(w.send(&[config()]).is_none());
    let (f, id, body) = w.make(&f, "  a counter ");
    let s = |k| body.get(k).and_then(Json::text);
    assert_eq!(
        [s("model"), s("max_tokens"), s("temperature")],
        [Some("m/x"), Some("8192"), Some("0.3")]
    );
    let effort = body.get("reasoning").and_then(|r| r.get("effort")).and_then(Json::text);
    assert_eq!((effort, body.get("stream")), (Some("low"), Some(&Json::Bool(true))));
    let (role, system) = message(&body, 0);
    assert!(role == "system" && system.contains(applang::REFERENCE) && system.contains("```app\n"));
    assert_eq!((messages(&body), message(&body, 1)), (2, ("user", "a counter")));
    assert!(has(&f, "Asking m/x\u{2026}") && has(&f, "Stop"));
    // Reasoning shows as thinking, never as the program.
    let f = w.last(&[think(id, "Hm\u{e9}. "), think(id, "Ok.")]);
    assert!(has(&f, "Thinking\u{2026} 8 chars"), "{:?}", words(&f));
    // A reply that does not compile goes back with its problem, once checked.
    let frames = w.answer(id, &app("label;\n"));
    assert!(has(&frames[0], "Writing\u{2026} 5 chars"));
    let n = frames.len();
    assert!(has(&frames[n - 2], "Checking\u{2026}") && frames[n - 2].requests.is_empty());
    assert!(has(&frames[n - 1], "Fixing (1 of 2) \u{2014} asking m/x\u{2026}"));
    let (id, body) = ai(&frames[n - 1]);
    assert_eq!(messages(&body), 4);
    assert_eq!(message(&body, 2), ("assistant", app("label;\n").as_str()));
    let (role, fix) = message(&body, 3);
    assert!(
        role == "user" && fix.starts_with("The program did not compile: E0") && fix.contains(" 1:")
    );
    assert!(fix.ends_with("in one app block."));
    // A reply with no program goes back too.
    let f = w.answer(id, "Sorry.").pop().unwrap();
    let (id, body) = ai(&f);
    assert!(
        has(&f, "Fixing (2 of 2)")
            && message(&body, 3).1.starts_with("Your reply held no app block.")
    );
    // The fix compiles: saved under its first label's name, running, in the corpus.
    let f = w.answer(id, &app(COUNTER)).pop().unwrap();
    let path = [HOME, "/apps/counter.app"].concat();
    assert_eq!(w.disk.get(&path).map(String::as_str), Some(COUNTER));
    assert!(has(&f, "Ready \u{2713} \u{2014} saved ~/apps/counter.app"), "{:?}", words(&f));
    assert_eq!(f.title, "Studio \u{2014} counter.app");
    let line = format!(
        "{{\"prompt\":\"a counter\",\"program\":{},\"attempts\":3,\"model\":\"m/x\"}}\n",
        quote(COUNTER)
    );
    assert_eq!(w.disk[CORPUS], line);
    // The prompt empties, keeps the keyboard and is remembered; the app runs in a card.
    assert_eq!(input(&f).1, "");
    assert!(f.requests.contains(&Request::Focus { id: input(&f).0 }));
    assert!(has(&f, "\u{201c}a counter\u{201d}") && has(&f, "Counter") && has(&f, "Make"));
    let card = all(&f.nodes).into_iter().find(|n| matches!(n, Node::Card { .. })).unwrap();
    assert!(words(&Frame { nodes: vec![card.clone()], ..Frame::default() }).contains(&"Reset"));
    // Its buttons work in place: "+" is the app's second button.
    let f = w.last(&[click(APP + 1)]);
    assert!(words(&f).contains(&"1"));
    // A second first make of a counter would not overwrite it.
    let mut w2 = Win { view: Box::new(Studio::new("")), disk: w.disk.clone() };
    let f = w2.last(&[WIDE]);
    let (_, id, _) = w2.make(&f, "another counter");
    w2.answer(id, &app(COUNTER));
    assert!(w2.disk.contains_key(&[HOME, "/apps/counter-2.app"].concat()));
}

#[test]
fn a_change_sends_the_program_and_keeps_its_file() {
    let disk = with(&[("/apps/counter.app", COUNTER)]);
    let mut w = Win::new(&["edit", "counter.app"], disk);
    let f = w.last(&[WIDE]);
    assert_eq!(f.title, "Studio \u{2014} counter.app");
    assert!(has(&f, "/apps/counter.app") && input(&f).2 == "Describe a change\u{2026}");
    let (_, id, body) = w.make(&f, "add a double button");
    let (role, ask) = message(&body, 1);
    let want = format!(
        "The program now:\n```app\n{}\n```\nChange it: add a double button\nReply with the complete new program in one app block.",
        COUNTER.trim_end()
    );
    assert_eq!((role, ask), ("user", want.as_str()));
    let doubled = [COUNTER, "button \"x2\" { count = count * 2; }\n"].concat();
    let f = w.answer(id, &app(&doubled)).pop().unwrap();
    assert_eq!(w.disk["/apps/counter.app"], doubled);
    assert!(has(&f, "Ready \u{2713} \u{2014} saved /apps/counter.app") && has(&f, "x2"));
    let corpus = &w.disk[CORPUS];
    assert!(
        corpus.contains("\"attempts\":1")
            && corpus.ends_with(&format!(",\"base\":{}}}\n", quote(COUNTER)))
    );
    // A make that fails leaves the working program as it was.
    let (_, mut id, _) = w.make(&f, "break it");
    for _ in 0..3 {
        let f = w.answer(id, &app("label nope;\n")).pop().unwrap();
        if has(&f, "E0906") {
            assert!(
                has(&f, "E0906 still not compiling after 2 fixes: E0302 1:7 "),
                "{:?}",
                words(&f)
            );
            assert!(has(&f, "x2") && input(&f).1 == "break it" && f.requests.is_empty());
            break;
        }
        id = ai(&f).0;
    }
    assert_eq!(w.disk["/apps/counter.app"], doubled);
    // So does an AI that fails, and Stop.
    let f = w.last(&[click(MAKE)]);
    let id = ai(&f).0;
    let f = w.last(&[Event::AiEnd { id, status: 429, error: String::new() }]);
    assert!(has(&f, "E0903 the free AI is busy, try again in a minute") && has(&f, "Make"));
    let id = ai(&w.last(&[click(MAKE)])).0;
    let f = w.last(&[content(id, b"```app\n"), click(STOP)]);
    assert_eq!(f.requests, [Request::AiCancel { id }]);
    assert!(has(&f, "Stopped.") && has(&f, "Make"));
    assert!(w.send(&[Event::AiEnd { id, status: 0, error: "cancelled".into() }]).is_none());
    assert_eq!(w.disk["/apps/counter.app"], doubled);
    // Open in window runs the file.
    assert_eq!(
        w.last(&[click(OPEN)]).requests,
        [Request::Open { name: "/apps/counter.app".into() }]
    );
}

#[test]
fn studio_ids_stay_below_the_apps() {
    let mut w = Win::new(&["edit", "/apps/greeter.app"], with(&[("/apps/greeter.app", GREETER)]));
    w.send(&[WIDE]);
    let f = w.last(&[change(INPUT, 2, "Ada"), click(APP)]);
    assert!(has(&f, "Hello, Ada!") && has(&f, "Waves: 1"));
    let ids: Vec<u32> = all(&f.nodes)
        .iter()
        .filter_map(|n| match n {
            Node::Button { id, .. } | Node::Input { id, .. } | Node::Code { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    let card = all(&f.nodes).into_iter().find(|n| matches!(n, Node::Card { .. })).unwrap();
    for n in all(&f.nodes) {
        let (Node::Button { id, .. } | Node::Input { id, .. }) = n else { continue };
        let inside = all(card.children()).iter().any(|c| std::ptr::eq(*c, n));
        assert_eq!(*id >= APP, inside, "{n:?}");
    }
    assert!(
        [MAKE, STOP, TOGGLE, OPEN, NEW, CHECK, CHIP + 3, PROMPT, CODE].iter().all(|id| *id < APP)
    );
    const { assert!(PROMPT + 0xFF_FFFF < CODE && CODE + 0xFF_FFFF < APP && APP < INPUT) };
    // The code view's editor is Studio's too.
    let f = w.last(&[click(TOGGLE)]);
    assert!((CODE..APP).contains(&code(&f).0) && has(&f, "Preview") && has(&f, "Check"));
}

#[test]
fn narrow_windows_stack_with_the_prompt_last() {
    let mut w = Win::new(&["edit", "/apps/counter.app"], with(&[("/apps/counter.app", COUNTER)]));
    let f = w.last(&[WIDE]);
    let [Node::Row { children: head, .. }, Node::Row { children: body, .. }] = &f.nodes[..] else {
        panic!("{:?}", f.nodes)
    };
    assert!(
        matches!(&head[..2], [Node::Text { style: Style::Heading, text, .. }, Node::Col { .. }] if text == "Counter")
    );
    assert!(matches!(&body[..], [Node::Card { .. }, Node::Pane { w: 280, .. }]));
    // Narrower than 600: one column, the app keeping its height and the prompt at the bottom.
    let f = w.last(&[NARROW]);
    assert!(matches!(&f.nodes[..3], [Node::Text { .. }, Node::Row { .. }, Node::Card { .. }]));
    assert!(matches!(f.nodes[3], Node::Fill { .. }));
    let Some(Node::Row { children, .. }) = f.nodes.last() else { panic!("{:?}", f.nodes) };
    assert!(matches!(&children[..], [Node::Input { .. }, Node::Button { id: MAKE, .. }]));
    // A size on the same side draws nothing; the code fills the column itself.
    assert!(w.send(&[Event::Resize { w: 599, h: 300 }]).is_none());
    let f = w.last(&[click(TOGGLE)]);
    assert!(
        matches!(&f.nodes[2], Node::Col { children, .. } if matches!(children[0], Node::Fill { .. }))
    );
    assert!(!f.nodes.iter().any(|n| matches!(n, Node::Fill { .. })));
    assert!(w.send(&[Event::Resize { w: 600, h: 300 }]).is_some());
}

#[test]
fn code_edits_check_save_and_mark_problems() {
    let mut w = Win::new(&["edit", "/apps/x.app"], with(&[("/apps/x.app", COUNTER)]));
    w.send(&[WIDE]);
    let f = w.last(&[click(TOGGLE)]);
    let (id, version, text, spans) = code(&f);
    assert_eq!((id, version, text), (CODE + 1, 1, COUNTER));
    let comment = ("// A counter: two buttons change one number.", Class::Comment);
    assert_eq!(classes(text, &spans[..2]), [comment, ("state", Class::Keyword)]);
    // An edit with a problem: Check marks it and leaves the app running as it was.
    let bad = "state count = 0;\nlabel count;\nlabel nope;";
    let f = w.last(&[change(id, 2, bad)]);
    assert!(has(&f, "Edited \u{2014} Check (Ctrl+Enter) runs and saves it"));
    let f = w.last(&[click(CHECK)]);
    assert!(has(&f, "E0302 3:7 ") && w.disk["/apps/x.app"] == COUNTER);
    let (_, version, src, spans) = code(&f);
    assert_eq!(version, 2);
    assert!(classes(src, spans).ends_with(&[("nope", Class::Error), (";", Class::Punct)]));
    // An edit takes the underline away; Ctrl+Enter checks, saves and runs it.
    let fixed = "state count = 0;\nlabel \"n = \" + count;";
    let f = w.last(&[change(id, 3, fixed)]);
    assert!(code(&f).3.iter().all(|s| s.class != Class::Error));
    let f = w.last(&[Event::Key { id, key: Key::Enter, mods: mods::CTRL, ch: '\0' }]);
    assert!(
        has(&f, "Checked \u{2713} \u{2014} saved /apps/x.app") && w.disk["/apps/x.app"] == fixed
    );
    assert!(has(&w.last(&[click(TOGGLE)]), "n = 0"));
    // Open in window says when it runs the saved file, not the edits.
    let f = w.last(&[change(id, 4, "label 4;"), click(OPEN)]);
    assert!(has(&f, "Opened the saved version") && f.requests.len() == 1);
    // Ctrl+S checks too; a save that fails says why.
    let mut w = Win::new(&["edit", "/ro/x.app"], Mem::new());
    let f = w.last(&[WIDE]);
    assert!(has(&f, "/ro/x.app \u{b7} new file") && has(&f, "Nothing here yet"));
    let id = code(&w.last(&[click(TOGGLE)])).0;
    let f = w.last(&[
        change(id, 2, "label 1;"),
        Event::Key { id, key: Key::Char, mods: mods::META, ch: 's' },
    ]);
    assert!(has(&f, "Couldn't save /ro/x.app: read-only"));
}

#[test]
fn stale_or_unreadable_text_is_never_sent_or_saved() {
    let mut w = Win::new(&["edit", "/apps/x.app"], with(&[("/apps/x.app", COUNTER)]));
    let f = w.last(&[WIDE, click(TOGGLE)]);
    let id = code(&f).0;
    // An event that does not decode: the text may be stale.
    let f = w.raw(vec![vec![2, 4, 0]]).pop().unwrap();
    assert!(has(&f, "An event did not arrive whole"));
    let f = w.last(&[click(CHECK)]);
    assert!(has(&f, "Not checked: an edit was lost; edit again to send it"));
    let f = w.last(&[change(input(&f).0, 1, "more"), click(MAKE)]);
    assert!(has(&f, "Not made: an edit was lost") && f.requests.is_empty());
    // The next edit brings them back; one too big for a frame is lost.
    w.send(&[change(id, 5, "label 5;"), click(CHECK)]);
    assert_eq!(w.disk["/apps/x.app"], "label 5;");
    let f = w.last(&[change(id, 6, &"x".repeat(MAX_TEXT + 1))]);
    assert!(has(&f, "over 256 KiB") && code(&f).2 == "label 5;");
    // A file too big to edit, or one that cannot be read, is never saved over.
    let big = "x".repeat(MAX_TEXT + 1);
    for (path, why) in [
        ("/apps/big.app", "the file is over 256 KiB"),
        ("/bad/x.app", "cannot read it: device busy"),
    ] {
        let mut w = Win::new(&["edit", path], with(&[("/apps/big.app", &big)]));
        let f = w.last(&[WIDE]);
        let id = code(&w.last(&[click(TOGGLE)])).0;
        assert!(has(&f, why));
        let f = w.last(&[change(id, 2, "label 1;"), click(CHECK)]);
        assert!(has(&f, &["Not checked: ", why].concat()));
        assert_eq!(
            w.disk.get(path).map(String::len),
            (path == "/apps/big.app").then_some(big.len())
        );
    }
}

#[test]
fn asks_from_the_everything_bar_make_in_turn() {
    let mut w = Win::new(&[], Mem::new());
    w.send(&[WIDE]);
    let ask = |text: &str| Event::Ask { text: text.into() };
    assert!(w.send(&[ask("  ")]).is_none());
    let f = w.last(&[ask("a counter")]);
    let (id, body) = ai(&f);
    assert_eq!((message(&body, 1).1, input(&f).1), ("a counter", "a counter"));
    // One while a make runs waits for it, then changes what it made.
    let f = w.last(&[ask("add a double button")]);
    assert!(!f.requests.iter().any(|r| matches!(r, Request::Ai { .. })));
    let frames = w.answer(id, &app(COUNTER));
    let f = frames.last().unwrap();
    let (_, body) = ai(f);
    assert!(message(&body, 1).1.contains("Change it: add a double button"));
    assert!(
        has(&frames[frames.len() - 2], "Ready \u{2713}") && input(f).1 == "add a double button"
    );
}

#[test]
fn arguments_pick_the_view() {
    assert!(applang::compile(COUNTER).is_ok());
    #[rustfmt::skip]
    let titles: [(&[&str], &str); 4] = [(&[], "Studio"), (&["edit", "/apps/a.app"], "Studio \u{2014} a.app"),
        (&["run", "/apps/b.app"], "b.app"), (&["run", "x/c.app"], "c.app")];
    for (args, title) in titles {
        assert_eq!(Win::new(args, Mem::new()).last(&[WIDE]).title, title, "{args:?}");
    }
    let bad: [&[&str]; 5] =
        [&["edit"], &["run", ""], &["open", "a.app"], &["edit", "a", "b"], &[""]];
    assert!(
        bad.iter()
            .all(|args| view(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>()).is_none())
    );
}

#[test]
fn host_clicks_and_inputs_reach_the_app_and_serve_frames_until_close() {
    let mut w = Win::new(&["run", "/apps/counter.app"], with(&[("/apps/counter.app", COUNTER)]));
    let f = w.last(&[WIDE]);
    assert_eq!((f.title.as_str(), f.requests.len()), ("counter.app", 0));
    let b = |id, label: &str| Node::Button { id, variant: Variant::Normal, label: label.into() };
    let row = Node::Row {
        id: 0,
        gap: 8,
        children: vec![b(APP, "-"), text(Style::Body, "0"), b(APP + 1, "+")],
    };
    assert_eq!(f.nodes, [text(Style::Body, "Counter"), row, b(APP + 2, "Reset")]);
    for (id, count) in [(APP + 1, "1"), (APP + 1, "2"), (APP, "1")] {
        assert_eq!(words(&w.last(&[click(id)]))[2], count);
    }
    assert!(w.send(&[click(0), click(EDIT), WIDE, Event::Submit { id: APP }]).is_none());
    // A click no button answers is the app's coded fault.
    let f = w.last(&[click(APP + 8)]);
    assert!(words(&f).last().unwrap().starts_with("E0213 "), "{:?}", f.nodes);
    // Inputs round-trip; a Change for no input is still answered.
    let mut w = Win::new(&["run", "/apps/g.app"], with(&[("/apps/g.app", GREETER)]));
    let field = Node::Input { id: INPUT, value: String::new(), placeholder: "name".into() };
    assert_eq!(w.last(&[WIDE]).nodes[1], field);
    let f = w.last(&[change(INPUT, 3, "Ada")]);
    assert!(
        has(&f, "Hello, Ada!")
            && matches!(&f.nodes[1], Node::Input { value, .. } if value == "Ada")
    );
    assert_eq!(w.send(&[change(INPUT + 9, 4, "x")]), Some(f));
    // Serve frames each change, numbered, until Close.
    let mut w = Win::new(&["run", "/apps/counter.app"], with(&[("/apps/counter.app", COUNTER)]));
    let frames = w.frames(&[WIDE, WIDE, click(APP + 1), Event::Close, click(APP + 1)]);
    assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [0, 1]);
    // A draw device that fails ends the program with its error.
    let mut ui = Client::new(feed(vec![click(APP + 1).encode()]), &mut [0u8; 0][..]);
    let err = serve(&mut ui, w.view.as_mut(), &mut w.disk).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WriteZero);
}

#[test]
fn host_shows_faults_files_that_cannot_run_and_renders_too_big() {
    let src = "state n = 0;\nlabel \"n = \" + n;\nbutton \"inc\" { n = n + 1; }\n\
               button \"spin\" { repeat 1000000 { n = n + 1; } }";
    let mut w = Win::new(&["run", "/apps/spin.app"], with(&[("/apps/spin.app", src)]));
    let f = w.last(&[click(APP), click(APP + 1)]);
    let Some(Node::Text { style: Style::Error, text: fault, .. }) = f.nodes.last() else {
        panic!("{:?}", f.nodes)
    };
    assert!(fault.starts_with("E0206 4:") && words(&f)[0] == "n = 1", "{fault}");
    assert_eq!(words(&w.last(&[click(APP)]))[0], "n = 2");
    let mut w = Win::new(&["run", "/apps/missing.app"], Mem::new());
    let f = w.last(&[Event::Key { id: 0, key: Key::Escape, mods: 0, ch: '\0' }]);
    let edit = Node::Button { id: EDIT, variant: Variant::Primary, label: "Edit in Studio".into() };
    let error = text(Style::Error, "error cannot read /apps/missing.app: no such file");
    assert_eq!(f.nodes, [text(Style::Heading, "missing.app cannot run"), error, edit]);
    let typo = "state x = 1;\nlabel x + true;";
    let mut w = Win::new(&["run", "/apps/typo.app"], with(&[("/apps/typo.app", typo)]));
    let t: Vec<String> = words(&w.last(&[WIDE])).iter().map(|s| s.to_string()).collect();
    assert!(
        t[1].starts_with("E0303 2:")
            && t[2].starts_with("  label x + true;\n")
            && t[2].ends_with('^')
    );
    assert_eq!(
        w.last(&[click(EDIT)]).requests,
        [Request::Open { name: "studio:/apps/typo.app".into() }]
    );
    assert!(w.send(&[click(APP)]).is_none());
    // A message that quotes a huge name, and a huge line, are cut to fit.
    let long = ["label ", &"n".repeat(300_000), ";"].concat();
    let mut w = Win::new(&["run", "/apps/long.app"], with(&[("/apps/long.app", &long)]));
    let f = w.last(&[WIDE]);
    let t = words(&f);
    assert!(t[1].starts_with("E0302 1:7 ") && t[1].ends_with('\u{2026}') && t[1].len() <= 1040);
    assert!(t[2].ends_with('\u{2026}') && t[2].len() <= 4099);
    // A render too big for a frame is cut.
    fn depth(n: &Node) -> usize {
        1 + n.children().first().map_or(0, depth)
    }
    for (n, cut) in [(5000, true), (4000, false)] {
        let src = "label 1;".repeat(n);
        let mut w = Win::new(&["run", "/apps/many.app"], with(&[("/apps/many.app", &src)]));
        let f = w.last(&[WIDE]);
        assert_eq!(f.nodes.len(), 4000 + usize::from(cut));
        assert_eq!(f.nodes.last() == Some(&text(Style::Error, TOO_BIG)), cut);
    }
    let src = ["row {".repeat(40), "label 1;".into(), "}".repeat(40)].concat();
    let mut w = Win::new(&["run", "/apps/deep.app"], with(&[("/apps/deep.app", &src)]));
    let f = w.last(&[WIDE]);
    assert_eq!((f.nodes.len(), depth(&f.nodes[0])), (2, 31));
    assert_eq!(f.nodes[1], text(Style::Error, TOO_BIG));
    // In Studio the app sits deeper, so it is cut sooner, never past the frame's depth.
    let mut w = Win::new(&["edit", "/apps/deep.app"], with(&[("/apps/deep.app", &src)]));
    assert!(has(&w.last(&[WIDE]), TOO_BIG));
}

#[test]
fn spans_cut_tokens_around_the_mark_and_fs_reads_back() {
    use uiwire::Class::*;
    let s = |start, len, class| uiwire::Span { start, len, class };
    let mark = |a, b| Some(applang::Span::new(a, b));
    let src = "label nope;";
    assert_eq!(spans(src, mark(6, 10)), [s(0, 5, Keyword), s(6, 4, Error), s(10, 1, Punct)]);
    let want = [s(0, 2, Keyword), s(2, 6, Error), s(8, 2, Name), s(10, 1, Punct)];
    assert_eq!(spans(src, mark(2, 8)), want);
    let want = [s(0, 5, Keyword), s(6, 1, Number), s(7, 1, Punct), s(9, 1, Error)];
    assert_eq!(spans("label 1;  ", mark(9, 10)), want);
    // Empty marks, marks past the end and marks inside a char are left out.
    let src = "label \"é\";";
    for m in [mark(3, 3), mark(0, 99), mark(7, 8)] {
        assert_eq!(spans(src, m), spans(src, None));
    }
    assert_eq!(spans(&";".repeat(40_000), None).len(), 32 * 1024);
    // Fs makes directories, appends and reads back.
    let dir = std::env::temp_dir().join(format!("compusophy-studio-{}", std::process::id()));
    let file = dir.join("a").join("b.app");
    let path = file.to_str().unwrap();
    assert!(!Fs.exists(path) && Fs.write(path, "label 1;").is_ok());
    assert!(Fs.exists(path) && Fs.read(path).unwrap() == "label 1;");
    let log = dir.join("c").join("d.jsonl");
    let log = log.to_str().unwrap();
    assert!(Fs.append(log, "a\n").is_ok() && Fs.append(log, "b\n").is_ok());
    assert_eq!(Fs.read(log).unwrap(), "a\nb\n");
    std::fs::write(path, b"\xff1").unwrap();
    assert_eq!(Fs.read(path).unwrap(), "\u{fffd}1");
    let missing = dir.join("none");
    assert_eq!(Fs.read(missing.to_str().unwrap()).unwrap_err().kind(), ErrorKind::NotFound);
    std::fs::remove_dir_all(&dir).unwrap();
}
