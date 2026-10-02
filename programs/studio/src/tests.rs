use super::*;
use crate::edit::{CODE, MAKE, PROMPT, STOP, TOGGLE, spans};
use crate::run::{EDIT, TOO_BIG};
use coder::ai::{CORPUS, HOME, MAX_BODY};
use coder::json::{Json, quote};
use coder::receipt::MAKES;
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
    /// Types `prompt` into Studio's prompt (named in `f`) and makes it: the frame and request.
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
const NARROW: Event = Event::Resize { w: 411, h: 794 };

fn click(id: u32) -> Event {
    Event::Click { id }
}
fn change(id: u32, version: u32, text: &str) -> Event {
    Event::Change { id, version, text: text.into() }
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
/// The user message of a request body.
fn user(body: &Json) -> &str {
    let m = body.get("messages").and_then(|m| m.at(1)).expect("a user message");
    m.get("content").and_then(Json::text).unwrap()
}
/// The status line: the first Text of the row with `</>` or, before there is any code, the
/// Text before the prompt row.
fn status(f: &Frame) -> &str {
    let rows: Vec<&Node> = f.nodes.iter().filter(|n| matches!(n, Node::Row { .. })).collect();
    let row = rows.len().checked_sub(2).map(|i| rows[i]).expect("a status row");
    match row.children().first() {
        Some(Node::Text { text, .. }) => text,
        other => panic!("{other:?}"),
    }
}
fn classes<'t>(text: &'t str, spans: &[uiwire::Span]) -> Vec<(&'t str, Class)> {
    let at = |s: &uiwire::Span| &text[s.start as usize..(s.start + s.len) as usize];
    spans.iter().map(|s| (at(s), s.class)).collect()
}

const BROKEN: &str = "state n = 0;\nlabel nope;\n";

#[test]
fn opens_empty_with_only_the_prompt_centered() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    assert_eq!(f.title, "Studio");
    assert_eq!(f.requests, [Request::Size { w: 880, h: 560 }, Request::Focus { id: PROMPT }]);
    let [Node::Fill { .. }, Node::Row { children, .. }, Node::Fill { .. }, Node::Spacer { .. }] =
        &f.nodes[..]
    else {
        panic!("{:?}", f.nodes)
    };
    assert!(matches!(
        &children[..],
        [Node::Col { .. }, Node::Pane { w: 520, .. }, Node::Col { .. }]
    ));
    assert_eq!(input(&f), (PROMPT, "", "Describe an app"));
    assert_eq!(words(&f), ["Make"]);
    // A blank prompt makes nothing; `</>` without code does nothing.
    assert!(w.last(&[click(MAKE)]).requests.is_empty());
    assert!(w.send(&[click(TOGGLE)]).is_none());
}

#[test]
fn makes_through_the_coder_then_saves_runs_and_records() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    assert!(w.send(&[Event::Config { model: "m/x".into() }]).is_none());
    let (f, id, body) = w.make(&f, "  a counter ");
    let s = |k| body.get(k).and_then(Json::text);
    assert_eq!(
        [s("model"), s("max_tokens"), s("temperature")],
        [Some("m/x"), Some("6144"), Some("0.3")]
    );
    assert_eq!((body.get("reasoning"), user(&body)), (None, "Make: a counter"));
    let system = body.get("messages").and_then(|m| m.at(0)?.get("content")?.text()).unwrap();
    assert_eq!(system, coder::system());
    // While it makes: Stop, its status, the draft; never the model's name.
    assert_eq!(status(&f), "asking");
    assert!(has(&f, "Stop") && !has(&f, "Make") && !has(&f, "m/x") && code(&f).0 == 0);
    // Thinking shows as such, at most a frame a second; the program streams into the draft.
    let frames = w.frames(&[think(id, "Hm\u{e9}. "), think(id, "Ok.")]);
    assert_eq!((frames.len(), status(&frames[0])), (1, "thinking \u{b7} 0 s"));
    let frames = w.answer(id, &app(COUNTER));
    assert!(frames.iter().any(|f| status(f) == "writing \u{b7} 5 lines" && code(f).2.starts_with("// A counter")));
    // Its block's end stops the request; testing shows, then the program runs clean.
    let n = frames.len();
    assert_eq!(
        (frames[n - 2].requests.as_slice(), status(&frames[n - 2])),
        (&[Request::AiCancel { id }][..], "testing")
    );
    let f = &frames[n - 1];
    let path = [HOME, "/apps/counter.app"].concat();
    assert_eq!(w.disk.get(&path).map(String::as_str), Some(COUNTER));
    assert!(status(f).starts_with("ready \u{b7} ") && f.title == "Studio \u{2014} counter.app");
    assert_eq!(f.nodes[0], text(Style::Small, "A counter: two buttons change one number."));
    // Full-bleed, no card: its label, its row of buttons; then the status with `</>`.
    assert_eq!(f.nodes[1], text(Style::Body, "Counter"));
    assert!(has(f, "</>") && !all(&f.nodes).iter().any(|n| matches!(n, Node::Card { .. })));
    assert_eq!((input(f).1, input(f).2), ("", "Change it"));
    assert!(f.requests.contains(&Request::Focus { id: input(f).0 }));
    let line = format!(
        "{{\"prompt\":\"a counter\",\"program\":{},\"attempts\":1,\"model\":\"m/x\"}}\n",
        quote(COUNTER)
    );
    assert_eq!(w.disk[CORPUS], line);
    let made = &w.disk[MAKES];
    let want = format!(
        "{{\"v\":1,\"path\":{},\"kind\":\"make\",\"outcome\":\"ready\",\"version\":1,\"lines\":13,",
        quote(&path)
    );
    assert!(
        made.starts_with(&want)
            && made.ends_with(",\"fault\":\"\"}\n")
            && made.lines().count() == 1,
        "{made}"
    );
    // Its buttons work in place: "+" is the app's second button.
    assert!(words(&w.last(&[click(APP + 1)])).contains(&"1"));
    // A second first make of a counter would not overwrite it.
    let mut w2 = Win { view: Box::new(Studio::new("")), disk: w.disk.clone() };
    let f = w2.last(&[WIDE]);
    let (_, id, _) = w2.make(&f, "another counter");
    w2.answer(id, &app(COUNTER));
    assert!(w2.disk.contains_key(&[HOME, "/apps/counter-2.app"].concat()));
}

#[test]
fn problems_go_back_as_fixes_once_testing_shows() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    let (_, id, _) = w.make(&f, "count");
    let frames = w.answer(id, &app(BROKEN));
    let f = frames.last().unwrap();
    let (fix, body) = ai(f);
    assert!(
        fix != id
            && user(&body)
                .starts_with("You were asked: count\n\nThe program, numbered:\n  1| state n = 0;")
    );
    // The program being fixed shows whole, numbered, its problem marked.
    assert_eq!(status(f), "fixing line 2 \u{b7} 0 s");
    let (cid, _, src, spans) = code(f);
    assert!(cid == 0 && src == BROKEN && classes(src, spans).contains(&("nope", Class::Error)));
    let edit = "<<<<<<< SEARCH\nlabel nope;\n=======\nlabel n;\n>>>>>>> REPLACE\n";
    let f = w.answer(fix, edit).pop().unwrap();
    assert!(
        status(&f).starts_with("ready") && w.disk.contains_key(&[HOME, "/apps/app.app"].concat())
    );
    assert!(w.disk[CORPUS].contains("\"attempts\":2"));
}

#[test]
fn a_change_edits_the_open_app_and_never_makes_it_worse() {
    let disk = with(&[("/apps/counter.app", COUNTER)]);
    let mut w = Win::new(&["edit", "counter.app"], disk);
    let f = w.last(&[WIDE]);
    assert_eq!((f.title.as_str(), status(&f)), ("Studio \u{2014} counter.app", ""));
    let (_, id, body) = w.make(&f, "add a double button");
    let asked = user(&body);
    assert!(asked.starts_with("The program, numbered:\n  1| // A counter: two buttons change one number.\n  2| state count = 0;"));
    assert!(asked.ends_with("\nChange it: add a double button\nKeep its first comment true."));
    let add = "<<<<<<< SEARCH\nbutton \"Reset\" { count = 0; }\n=======\nbutton \"Reset\" { count = 0; }\nbutton \"x2\" { count = count * 2; }\n>>>>>>> REPLACE\n";
    let f = w.answer(id, add).pop().unwrap();
    let doubled = COUNTER.replace(
        "button \"Reset\" { count = 0; }\n",
        "button \"Reset\" { count = 0; }\nbutton \"x2\" { count = count * 2; }\n",
    );
    assert_eq!(w.disk["/apps/counter.app"], doubled);
    assert!(has(&f, "x2") && status(&f).starts_with("ready"));
    assert!(w.disk[CORPUS].ends_with(&format!(",\"base\":{}}}\n", quote(COUNTER))));
    assert!(w.disk[MAKES].contains("\"kind\":\"change\",\"outcome\":\"ready\""));
    // The AI failing, and Stop, leave the working program as it was.
    let id = ai(&w.last(&[change(input(&f).0, 1, "break it"), click(MAKE)])).0;
    let f = w.last(&[Event::AiEnd { id, status: 429, error: String::new() }]);
    assert_eq!(status(&f), "AI busy \u{b7} try in a minute");
    assert!(has(&f, "x2") && has(&f, "Make") && input(&f).1 == "break it");
    let id = ai(&w.last(&[click(MAKE)])).0;
    let f = w.last(&[content(id, b"```app\n"), click(STOP)]);
    assert_eq!((f.requests.as_slice(), status(&f)), (&[Request::AiCancel { id }][..], "stopped"));
    assert!(w.send(&[Event::AiEnd { id, status: 0, error: "cancelled".into() }]).is_none());
    // A change that never runs clean: the app runs on as it was, saying why.
    let id = ai(&w.last(&[click(MAKE)])).0;
    let worse = app(&doubled.replace("label count;", "label 1 / (count - count);"));
    let f = w.answer(id, &worse).pop().unwrap();
    let id = ai(&f).0;
    let f = w.answer(id, &worse).pop().unwrap();
    let id = ai(&f).0;
    let f = w.answer(id, &worse).pop().unwrap();
    assert_eq!(status(&f), "couldn't change it \u{b7} E0203 line 7");
    assert!(w.disk["/apps/counter.app"] == doubled && has(&f, "x2") && f.requests.is_empty());
    let outcomes: Vec<&str> =
        w.disk[MAKES].lines().map(|l| &l[l.find("\"outcome\"").unwrap()..][..18]).collect();
    assert_eq!(
        outcomes,
        [
            "\"outcome\":\"ready\",",
            "\"outcome\":\"failed\"",
            "\"outcome\":\"stopped",
            "\"outcome\":\"broken\""
        ]
    );
}

#[test]
fn a_new_app_that_never_compiles_leaves_its_draft_marked() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    let (_, mut id, _) = w.make(&f, "count");
    for _ in 0..2 {
        id = ai(w.answer(id, &app(BROKEN)).last().unwrap()).0;
    }
    let f = w.answer(id, &app(BROKEN)).pop().unwrap();
    assert_eq!((status(&f), f.title.as_str()), ("couldn't \u{b7} E0302 line 2", "Studio"));
    assert!(
        !w.disk.keys().any(|p| p.ends_with(".app"))
            && w.disk[MAKES].contains("\"outcome\":\"broken\"")
    );
    // `</>` shows it, its problem marked; fixed there, `</>` saves and runs it under its name.
    let f = w.last(&[click(TOGGLE)]);
    let (cid, _, src, spans) = code(&f);
    assert!(src == BROKEN && classes(src, spans).contains(&("nope", Class::Error)));
    let f = w.last(&[change(cid, 2, "state n = 0;\nlabel \"Tally\";\nlabel n;\n"), click(TOGGLE)]);
    assert!(status(&f).starts_with("saved ~/apps/tally.app") && has(&f, "Tally"));
}

/// A program that compiles but faults as it first renders (E0203 at line 4).
const AVG: &str =
    "// Avg: what you add, averaged.\nstate t = 0;\nstate c = 0;\nlabel \"avg \" + t / c;\n";

#[test]
fn make_again_after_nothing_compiled_makes_a_new_app() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    let (_, mut id, _) = w.make(&f, "an average");
    for _ in 0..2 {
        id = ai(w.answer(id, &app(BROKEN)).last().unwrap()).0;
    }
    assert_eq!(status(&w.answer(id, &app(BROKEN)).pop().unwrap()), "couldn't \u{b7} E0302 line 2");
    // Make again with the words the prompt kept: a first write again, never a change of the draft
    // showing, so what compiles but faults is installed as a first make's is.
    let (mut id, body) = ai(&w.last(&[click(MAKE)]));
    assert_eq!(user(&body), "Make: an average");
    for _ in 0..2 {
        id = ai(w.answer(id, &app(AVG)).last().unwrap()).0;
    }
    let f = w.answer(id, &app(AVG)).pop().unwrap();
    assert_eq!(status(&f), "runs, but faults \u{b7} E0203 line 4");
    assert_eq!(w.disk.get(&[HOME, "/apps/avg.app"].concat()).map(String::as_str), Some(AVG));
    assert!(
        w.disk[MAKES]
            .lines()
            .last()
            .unwrap()
            .contains("\"kind\":\"make\",\"outcome\":\"faulting\"")
    );
}

#[test]
fn edits_fenced_one_by_one_all_apply_and_cant_saves_nothing() {
    // A change whose edits come each in its own app fence: all of them, not the first.
    let mut w = Win::new(&["edit", "/apps/c.app"], with(&[("/apps/c.app", COUNTER)]));
    let f = w.last(&[WIDE]);
    let (_, id, _) = w.make(&f, "add x2 and x10");
    let (reset, title) = ("button \"Reset\" { count = 0; }\n", "label \"Counter\";\n");
    let x2 = [reset, "button \"x2\" { count = count * 2; }\n"].concat();
    let x10 = [title, "button \"x10\" { count = count * 10; }\n"].concat();
    let edit = |s: &str, r: &str| format!("<<<<<<< SEARCH\n{s}=======\n{r}>>>>>>> REPLACE\n");
    let f = w.answer(id, &(app(&edit(reset, &x2)) + &app(&edit(title, &x10)))).pop().unwrap();
    assert!(status(&f).starts_with("ready") && has(&f, "x2") && has(&f, "x10"));
    assert_eq!(w.disk["/apps/c.app"], COUNTER.replace(reset, &x2).replace(title, &x10));
    // A new app whose write faults, then only a comment: can't make that, nothing saved.
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[WIDE]);
    let (_, id, _) = w.make(&f, "an average");
    let id = ai(w.answer(id, &app(AVG)).last().unwrap()).0;
    let f = w.answer(id, "```app\n// applang has no floats, so no average.\n```\n").pop().unwrap();
    assert_eq!(
        (status(&f), w.disk.keys().any(|p| p.ends_with(".app"))),
        ("can't make that", false)
    );
    assert!(w.disk[MAKES].contains("\"outcome\":\"cant\",\"version\":0"));
}

#[test]
fn narrow_windows_show_the_make_with_the_keyboard_away() {
    let mut w = Win::new(&[], Mem::new());
    let f = w.last(&[NARROW]);
    let (f, id, _) = w.make(&f, "a long one");
    assert!(f.requests.contains(&Request::Focus { id: 0 }));
    // The draft shows its newest lines, as many as the window holds: (794 - 140) / 18.
    let long: String = (0..80).map(|i| format!("label {i};\n")).collect();
    let f = w.last(&[content(id, ["```app\n", &long].concat().as_bytes())]);
    let (_, _, tail, _) = code(&f);
    assert_eq!((tail.lines().count(), tail.lines().next()), (36, Some("label 44;")));
    // One column: the draft, the status, the prompt; no `</>` while it makes.
    let f = w.last(&[click(STOP)]);
    assert!(matches!(f.nodes.last(), Some(Node::Row { .. })) && has(&f, "</>"));
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
    // Unedited, `</>` goes back; edited with a problem, it stays and marks it, the app as it was.
    assert!(
        !has(&w.last(&[click(TOGGLE)]), "</> ")
            && w.last(&[click(TOGGLE)])
                .nodes
                .iter()
                .any(|n| matches!(n, Node::Fill { children, .. } if !children.is_empty()))
    );
    let bad = "state count = 0;\nlabel count;\nlabel nope;";
    assert_eq!(status(&w.last(&[change(id, 2, bad)])), "edited");
    let f = w.last(&[click(TOGGLE)]);
    assert!(status(&f).starts_with("E0302 3:7 ") && w.disk["/apps/x.app"] == COUNTER);
    let (_, version, src, spans) = code(&f);
    assert_eq!(version, 2);
    assert!(classes(src, spans).ends_with(&[("nope", Class::Error), (";", Class::Punct)]));
    // An edit takes the underline away; Ctrl+Enter checks, saves and runs it.
    let fixed = "state count = 0;\nlabel \"n = \" + count;";
    let f = w.last(&[change(id, 3, fixed)]);
    assert!(code(&f).3.iter().all(|s| s.class != Class::Error));
    let f = w.last(&[Event::Key { id, key: Key::Enter, mods: mods::CTRL, ch: '\0' }]);
    assert!(
        status(&f) == "saved /apps/x.app" && w.disk["/apps/x.app"] == fixed && has(&f, "n = 0")
    );
    // Ctrl+S checks too; a save that fails says why.
    let mut w = Win::new(&["edit", "/ro/x.app"], Mem::new());
    let f = w.last(&[WIDE]);
    assert_eq!(status(&f), "/ro/x.app \u{b7} new file");
    let f = w.last(&[change(input(&f).0, 1, "x")]);
    assert!(w.send(&[click(TOGGLE)]).is_none() && !has(&f, "</>"));
}

#[test]
fn stale_or_unreadable_text_is_never_sent_or_saved() {
    let mut w = Win::new(&["edit", "/apps/x.app"], with(&[("/apps/x.app", COUNTER)]));
    let f = w.last(&[WIDE, click(TOGGLE)]);
    let id = code(&f).0;
    // An event that does not decode: the text may be stale.
    let f = w.raw(vec![vec![2, 4, 0]]).pop().unwrap();
    assert_eq!(status(&f), "an event did not arrive whole");
    w.send(&[
        change(id, 2, "label 2;"),
        Event::Key { id, key: Key::Char, mods: mods::META, ch: 's' },
    ]);
    assert_eq!(w.disk["/apps/x.app"], "label 2;");
    // Saved, it shows the app; `</>` shows the code again.
    w.send(&[click(TOGGLE)]);
    let f = w.last(&[change(id, 6, &"x".repeat(MAX_TEXT + 1))]);
    assert!(status(&f).contains("over 256 KiB") && code(&f).2 == "label 2;");
    let f = w.last(&[change(input(&f).0, 1, "more"), click(MAKE)]);
    assert!(status(&f).starts_with("not made: an edit was lost") && f.requests.is_empty());
    let f = w.last(&[Event::Key { id, key: Key::Char, mods: mods::CTRL, ch: 's' }]);
    assert!(status(&f).starts_with("not checked: an edit was lost"));
    // A file too big to edit, or one that cannot be read, is never saved over.
    let big = "x".repeat(MAX_TEXT + 1);
    for (path, why) in [
        ("/apps/big.app", "the file is over 256 KiB"),
        ("/bad/x.app", "cannot read it: device busy"),
    ] {
        let mut w = Win::new(&["edit", path], with(&[("/apps/big.app", &big)]));
        let f = w.last(&[WIDE]);
        assert_eq!(status(&f), why);
        let f = w.last(&[change(input(&f).0, 1, "a tip calculator"), click(MAKE)]);
        assert!(status(&f).starts_with("not made: ") && f.requests.is_empty());
        assert_eq!(
            w.disk.get(path).map(String::len),
            (path == "/apps/big.app").then_some(big.len())
        );
    }
}

#[test]
fn studio_ids_stay_below_the_apps_and_the_overlay_finds_them() {
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
    assert!([MAKE, STOP, TOGGLE, PROMPT, CODE].iter().all(|id| *id < APP));
    assert_eq!(
        (STOP, PROMPT, CODE),
        (coder::ids::STOP, coder::ids::PROMPT, coder::ids::PROMPT_END)
    );
    const { assert!(PROMPT + 0xFF_FFFF < CODE && CODE + 0xFF_FFFF < APP && APP < INPUT) };
    let f = w.last(&[click(TOGGLE)]);
    assert!((CODE..APP).contains(&code(&f).0));
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
fn games_tick_take_keys_and_taps_and_keep_their_saved_state() {
    let src = "saved state best = 0; state x = 0; state cells = [0; 4];
        every 100 { x += 1; best = max(best, x); }
        on key \"left\" { x -= 10; } on key \"escape\" { x = 0; }
        label \"x \" + x;
        grid 2, cells { cells[cell] = 1; }";
    let (path, state) = ("/apps/game.app", [HOME, "/.appdata/game.state"].concat());
    let mut w = Win::new(&["run", path], with(&[(path, src), (&state, "best = 7;\n")]));
    // It asks for ticks and keys; its grid taps as the first handler shown.
    let f = w.last(&[WIDE]);
    assert_eq!(f.requests, [Request::Timer { ms: 100 }, Request::Keys { on: true }]);
    let grid = |f: &Frame| {
        let g = all(&f.nodes).into_iter().find(|n| matches!(n, Node::Grid { .. }));
        let Some(Node::Grid { id, cells, .. }) = g else { panic!("a grid") };
        (*id, cells.clone())
    };
    assert_eq!(grid(&f), (APP, vec![0; 4]));
    // Every tick answers; one that makes the every due runs it.
    assert!(has(&w.last(&[Event::Tick { ms: 100 }]), "x 1"));
    assert!(has(&w.last(&[Event::Tick { ms: 50 }]), "x 1"));
    // A plain key is the app's, a chord the window's; a tap is its grid's, and gives the app the
    // keyboard.
    let key = |id, key, mods| Event::Key { id, key, mods, ch: '\0' };
    assert!(has(&w.last(&[key(0, Key::Left, 0)]), "x -9"));
    assert!(w.send(&[key(0, Key::Left, mods::CTRL)]).is_none());
    let f = w.last(&[Event::Tap { id: APP, cell: 3 }]);
    assert!(grid(&f).1 == [0, 0, 0, 1] && f.requests == [Request::Focus { id: 0 }]);
    // The saved best came back (7), and is kept once it passes it.
    assert_eq!(w.disk[&state], "best = 7;\n");
    w.frames(&vec![Event::Tick { ms: 100 }; 20]);
    assert_eq!(w.disk[&state], "best = 11;\n");
    // In Studio the preview ticks; Escape leaving the prompt is not the app's; showing the code
    // stops its timer.
    let mut s = Win::new(&["edit", path], w.disk.clone());
    assert!(s.last(&[WIDE]).requests.contains(&Request::Timer { ms: 100 }));
    assert!(s.send(&[key(PROMPT, Key::Escape, 0)]).is_none());
    assert!(s.last(&[click(TOGGLE)]).requests.contains(&Request::Timer { ms: 0 }));
}

#[test]
fn runs_of_one_app_keep_each_others_saved_states() {
    // Its window and Studio's preview: what one wrote comes back before the other's next event
    // but a tick, whose write keeps what it did not change (and another version's states).
    let src = "saved state n = 0; saved state t = 0; every 100 { t += 1; }\n\
               button \"+\" { n += 1; } label \"n \" + n + \" t \" + t;";
    let (path, state) = ("/apps/two.app", [HOME, "/.appdata/two.state"].concat());
    let mut a = Win::new(&["run", path], with(&[(path, src), (&state, "old = [1];\n")]));
    let mut b = Win::new(&["edit", path], a.disk.clone());
    b.send(&[WIDE]);
    a.send(&[WIDE, click(APP), click(APP)]);
    b.disk.clone_from(&a.disk);
    assert!(has(&b.last(&[Event::Tick { ms: 100 }]), "n 2 t 1"));
    assert_eq!(b.disk[&state], "n = 2;\nt = 1;\nold = [1];\n");
    a.disk.clone_from(&b.disk);
    assert!(has(&a.last(&[click(APP)]), "n 3 t 1"));
    b.disk.clone_from(&a.disk);
    assert!(has(&b.last(&[click(APP)]), "n 4 t 1"));
    a.disk.clone_from(&b.disk);
    assert!(has(&a.last(&[Event::Focus { on: true }]), "n 4 t 1"));
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
