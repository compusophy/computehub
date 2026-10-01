use super::*;
use crate::{edit::spans, run::*};
use std::collections::BTreeMap;
use uiwire::{Class, Key, Node, Request, Style, Variant, mods};

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
    fn exists(&mut self, path: &str) -> bool {
        self.contains_key(path)
    }
}

/// The samples and `files`.
fn with(files: &[(&str, &str)]) -> Mem {
    SAMPLES.iter().chain(files).map(|(p, s)| (p.to_string(), s.to_string())).collect()
}
/// The events device: one event per read, then the end.
fn feed(events: Vec<Vec<u8>>) -> impl Read {
    let none: Box<dyn Read> = Box::new(io::empty());
    events.into_iter().fold(none, |feed, ev| Box::new(feed.chain(io::Cursor::new(ev))))
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

/// A window: a view from program arguments, and its disk.
struct Win {
    view: Box<dyn View>,
    disk: Mem,
}

impl Win {
    fn new(args: &[&str], disk: Mem) -> Win {
        Win { view: view(&strings(args)).expect("a view"), disk }
    }
    /// Serves `events` over in-memory pipes, one per read; the frames sent.
    fn raw(&mut self, events: Vec<Vec<u8>>) -> Vec<Frame> {
        let mut sent = Vec::new();
        let mut ui = Client::new(feed(events), Sink(&mut sent));
        serve(&mut ui, self.view.as_mut(), &mut self.disk).unwrap();
        sent.iter().map(|f| Frame::decode(f).expect("a frame that decodes")).collect()
    }
    /// One event; the frame it brought, if any.
    fn send(&mut self, ev: Event) -> Option<Frame> {
        let mut frames = self.raw(vec![ev.encode()]);
        assert!(frames.len() <= 1);
        frames.pop()
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}
const RESIZE: Event = Event::Resize { w: 640, h: 480 };
fn click(id: u32) -> Event {
    Event::Click { id }
}
fn change(id: u32, version: u32, text: &str) -> Event {
    Event::Change { id, version, text: text.into() }
}
fn key(key: Key, mods: u8, ch: char) -> Event {
    Event::Key { id: 4, key, mods, ch }
}
fn open(name: &str) -> Vec<Request> {
    vec![Request::Open { name: name.into() }]
}
/// Every Text's text, in order.
fn texts(nodes: &[Node]) -> Vec<String> {
    let each = |n: &Node| match n {
        Node::Text { text, .. } => vec![text.clone()],
        n => texts(n.children()),
    };
    nodes.iter().flat_map(each).collect()
}
/// Studio's toolbar note.
fn note(f: &Frame) -> String {
    texts(f.nodes[0].children()).concat()
}
/// Studio's Code node: version, text and spans.
fn code(f: &Frame) -> (u32, &str, &[uiwire::Span]) {
    let Node::Code { version, text, spans, .. } = &f.nodes[1].children()[0] else {
        panic!("no code in {:?}", f.nodes)
    };
    (*version, text, spans)
}
/// The problem rows under Studio's editor.
fn problems(f: &Frame) -> Vec<&str> {
    let rows = f.nodes.iter().filter_map(|n| match n {
        Node::Item { id: 100, text, .. } => Some(text.as_str()),
        _ => None,
    });
    rows.collect()
}
fn classes<'t>(text: &'t str, spans: &[uiwire::Span]) -> Vec<(&'t str, Class)> {
    let at = |s: &uiwire::Span| &text[s.start as usize..(s.start + s.len) as usize];
    spans.iter().map(|s| (at(s), s.class)).collect()
}

#[test]
fn samples_compile_and_arguments_pick_the_view() {
    for (path, src) in SAMPLES.into_iter().chain([("new", NEW_APP)]) {
        assert!(applang::compile(src).is_ok(), "{path}");
    }
    assert_eq!(SAMPLES[0].0, DEFAULT_FILE);
    #[rustfmt::skip]
    let titles: [(&[&str], &str); 5] = [(&[], "Studio — counter.app"),
        (&["edit", "/apps/greeter.app"], "Studio — greeter.app"),
        (&["edit", "clicker.app"], "Studio — clicker.app"),
        (&["run", "/apps/clicker.app"], "clicker.app"), (&["run", "x/studio.app"], "studio.app")];
    for (args, title) in titles {
        let f = Win::new(args, with(&[])).send(RESIZE).expect("a first frame");
        assert_eq!(f.title, title, "{args:?}");
    }
    let f = Win::new(&["edit", "clicker.app"], with(&[])).send(RESIZE).unwrap();
    assert_eq!(code(&f).1, SAMPLES[2].1, "a relative path is under /apps");
    let bad: [&[&str]; 5] =
        [&["edit"], &["run", ""], &["open", "a.app"], &["edit", "a", "b"], &[""]];
    assert!(bad.iter().all(|args| view(&strings(args)).is_none()));
}

#[test]
fn studio_highlights_follows_edits_and_marks_problems() {
    let mut w = Win::new(&["edit", DEFAULT_FILE], with(&[]));
    let f = w.send(RESIZE).expect("the first event draws");
    assert_eq!(f.requests, [Request::Size { w: 760, h: 540 }]);
    let button = |id, variant, label: &str| Node::Button { id, variant, label: label.into() };
    let (run, normal) = (button(1, Variant::Primary, "Run"), Variant::Normal);
    let bar = [run, button(2, normal, "Save"), button(3, normal, "New")];
    assert_eq!(f.nodes[0].children(), [&bar[..], &[text(Style::Small, DEFAULT_FILE)]].concat());
    assert!(matches!(f.nodes[1], Node::Fill { .. }) && f.nodes.len() == 2);
    assert!(matches!(f.nodes[1].children()[0], Node::Code { id: 4, line_numbers: true, .. }));
    let (version, src, spans) = code(&f);
    assert_eq!((version, src), (1, SAMPLES[0].1));
    let comment = ("// A counter: two buttons change one number.", Class::Comment);
    let state = [comment, ("state", Class::Keyword), ("count", Class::Name), ("=", Class::Punct)];
    let want = [&state[..], &[("0", Class::Number), (";", Class::Punct)]].concat();
    assert_eq!(classes(src, &spans[..6]), want);
    // A resize changes nothing here, so it draws nothing.
    assert!(w.send(RESIZE).is_none());
    // An edit comes back highlighted at the desktop's version.
    let f = w.send(change(4, 7, "label \"hi\";")).unwrap();
    let (version, src, spans) = code(&f);
    assert_eq!((version, src, f.requests.len()), (7, "label \"hi\";", 0));
    let want = [("label", Class::Keyword), ("\"hi\"", Class::String), (";", Class::Punct)];
    assert_eq!(classes(src, spans), want);
    assert_eq!(note(&f), "/apps/counter.app (modified)");
    // Plain keys and problem clicks draw nothing. Every Change is answered
    // (the desktop holds the next until a frame comes), even another id's.
    for ev in [key(Key::Char, 0, 's'), key(Key::Enter, 0, '\0'), click(100)] {
        assert!(w.send(ev).is_none());
    }
    assert_eq!(code(&w.send(change(5, 8, "x")).unwrap()).1, "label \"hi\";");
    // A failed run lists the problem and underlines it.
    let bad = "state count = 0;\nlabel count;\nlabel nope;";
    let mut w = Win::new(&["edit", "/apps/bad.app"], with(&[("/apps/bad.app", bad)]));
    assert_eq!(w.send(click(1)).unwrap().requests[1..], []);
    let f = w.send(click(1)).unwrap();
    assert!(f.requests.is_empty());
    let [problem] = problems(&f)[..] else { panic!("{:?}", f.nodes) };
    assert!(problem.starts_with("E0302 3:7 "), "{problem}");
    assert_eq!(note(&f), "/apps/bad.app · did not compile");
    let (_, src, spans) = code(&f);
    assert!(classes(src, spans).ends_with(&[("nope", Class::Error), (";", Class::Punct)]));
    // An edit takes the underline away at once; the problem stays until the next Run.
    let fixed = "state count = 0;\nlabel count;\nlabel count;";
    let f = w.send(change(4, 2, fixed)).unwrap();
    assert!(code(&f).2.iter().all(|s| s.class != Class::Error) && problems(&f).len() == 1);
    // Ctrl+Enter: compiled, saved, then opened in a window.
    let f = w.send(key(Key::Enter, mods::CTRL, '\0')).unwrap();
    assert_eq!((&f.requests, &w.disk["/apps/bad.app"][..]), (&open("/apps/bad.app"), fixed));
    assert!(problems(&f).is_empty() && note(&f) == "/apps/bad.app · saved and running");
    // Columns count chars, not bytes; the lexer's error shows as typed.
    let f = w.send(change(4, 3, "label 1;\nlabel \"é\" $;")).unwrap();
    assert!(code(&f).2.contains(&uiwire::Span { start: 20, len: 1, class: Class::Error }));
    assert!(problems(&w.send(click(1)).unwrap())[0].starts_with("E0001 2:11 "));
}

#[test]
fn keys_save_new_makes_untitled_apps_and_stale_text_is_never_saved() {
    let mut w = Win::new(&["edit", "/tmp/deep/x.app"], with(&[]));
    let f = w.send(RESIZE).unwrap();
    assert_eq!((code(&f).1, note(&f)), ("", "/tmp/deep/x.app · new file".into()));
    w.send(change(4, 2, "row {\n  label 1;\n}")).unwrap();
    assert!(!w.disk.exists("/tmp/deep/x.app"));
    let f = w.send(key(Key::Char, mods::CTRL, 's')).unwrap();
    assert_eq!(w.disk["/tmp/deep/x.app"], "row {\n  label 1;\n}");
    assert_eq!(note(&f), "/tmp/deep/x.app · saved");
    w.send(change(4, 3, "label 2;"));
    w.send(key(Key::Char, mods::META | mods::SHIFT, 'S'));
    assert_eq!(w.disk["/tmp/deep/x.app"], "label 2;");
    // New writes the starter to the first free name and edits it.
    assert_eq!(w.send(click(3)).unwrap().requests, open("studio:/apps/untitled.app"));
    assert_eq!(w.disk["/apps/untitled.app"], NEW_APP);
    assert_eq!(w.send(click(3)).unwrap().requests, open("studio:/apps/untitled-2.app"));
    // A save that fails says why and leaves the file modified; so does Run.
    let mut w = Win::new(&["edit", "/ro/x.app"], with(&[]));
    let f = (w.send(change(4, 2, "label 1;")), w.send(click(2)).unwrap()).1;
    assert_eq!(note(&f), "/ro/x.app (modified) · save failed: read-only");
    assert!(w.send(click(1)).unwrap().requests.is_empty());
    // Save and Run stay off while the text may be stale.
    let mut w = Win::new(&["edit", DEFAULT_FILE], with(&[]));
    w.send(RESIZE);
    // An event that does not decode.
    let f = w.raw(vec![vec![2, 4, 0]]).pop().unwrap();
    assert_eq!(note(&f), "/apps/counter.app · an event did not arrive whole");
    for ev in [click(1), key(Key::Char, mods::CTRL, 's')] {
        assert!(w.send(ev).unwrap().requests.is_empty());
    }
    let f = w.send(click(2)).unwrap();
    assert!(note(&f).ends_with("not saved: an edit was lost; edit again to send it"));
    assert_eq!(w.disk[DEFAULT_FILE], SAMPLES[0].1);
    // The next edit brings them back; one too big for a frame is lost.
    let _ = (w.send(change(4, 5, "label 5;")), w.send(click(2)));
    assert_eq!(w.disk[DEFAULT_FILE], "label 5;");
    let big = "x".repeat(MAX_TEXT + 1);
    let f = w.send(change(4, 6, &big)).unwrap();
    assert_eq!((code(&f).0, code(&f).1), (5, "label 5;"));
    assert!(note(&f).ends_with("the text is over 256 KiB"));
    let lost = "not run: an edit was lost; edit again to send it";
    assert!(note(&w.send(click(1)).unwrap()).ends_with(lost));
    // A file too big to edit, or one that cannot be read, is never saved over.
    let (disk, too_big) = (with(&[("/apps/big.app", &big)]), "the file is over 256 KiB");
    for (path, why) in [("/apps/big.app", too_big), ("/bad/x.app", "cannot read it: device busy")] {
        let mut w = Win::new(&["edit", path], disk.clone());
        assert_eq!(code(&w.send(RESIZE).unwrap()).1, "");
        w.send(change(4, 2, "label 1;"));
        let f = w.send(key(Key::Char, mods::CTRL, 's')).unwrap();
        assert_eq!(note(&f), [path, " (modified) · not saved: ", why].concat());
        assert_eq!(w.disk.get(path), disk.get(path));
    }
}

#[test]
fn host_clicks_and_inputs_reach_the_app_and_serve_frames_until_close() {
    let mut w = Win::new(&["run", DEFAULT_FILE], with(&[]));
    let f = w.send(RESIZE).unwrap();
    assert_eq!((f.title.as_str(), f.requests.len()), ("counter.app", 0));
    let b = |id, label: &str| Node::Button { id, variant: Variant::Normal, label: label.into() };
    let t = |s: &str| text(Style::Body, s);
    let row = Node::Row { id: 0, gap: 8, children: vec![b(1, "-"), t("0"), b(2, "+")] };
    assert_eq!(f.nodes, [t("Counter"), row, b(3, "Reset")]);
    for (id, count) in [(2, "1"), (2, "2"), (1, "1")] {
        assert_eq!(texts(&w.send(click(id)).unwrap().nodes), ["Counter", count]);
    }
    for ev in [click(0), click(EDIT), RESIZE, Event::Submit { id: 1 }] {
        assert!(w.send(ev).is_none());
    }
    // A click no button answers is the app's coded fault.
    let f = w.send(click(9)).unwrap();
    assert!(texts(&f.nodes)[2].starts_with("E0213 "), "{:?}", f.nodes);
    // Inputs round-trip.
    let mut w = Win::new(&["run", "/apps/greeter.app"], with(&[]));
    let field = Node::Input { id: INPUT, value: String::new(), placeholder: "name".into() };
    assert_eq!(w.send(RESIZE).unwrap().nodes[1], field);
    let f = w.send(change(INPUT, 3, "Ada")).unwrap();
    assert_eq!(texts(&f.nodes), ["What is your name?", "Hello, Ada!"]);
    assert!(matches!(&f.nodes[1], Node::Input { value, .. } if value == "Ada"));
    // The Wave button that appeared is applang's button 0, so 1 here.
    let f = w.send(click(1)).unwrap();
    assert_eq!(texts(&f.nodes)[2], "You waved 1 times.");
    // A Change for no input is still answered, with the window as it was.
    assert_eq!(w.send(change(INPUT + 9, 4, "x")), Some(f));
    // Serve frames each change, numbered, until Close.
    let mut w = Win::new(&["run", DEFAULT_FILE], with(&[]));
    let evs = [RESIZE, RESIZE, click(2), Event::Close, click(2)];
    let frames = w.raw(evs.iter().map(Event::encode).collect());
    assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(texts(&frames[1].nodes), ["Counter", "1"]);
    // A draw device that fails ends the program with its error.
    let mut ui = Client::new(feed(vec![click(2).encode()]), &mut [0u8; 0][..]);
    let err = serve(&mut ui, w.view.as_mut(), &mut w.disk).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WriteZero);
}

#[test]
fn host_shows_faults_files_that_cannot_run_and_renders_too_big() {
    let src = "state n = 0;\nlabel \"n = \" + n;\nbutton \"inc\" { n = n + 1; }\n\
               button \"spin\" { repeat 1000000 { n = n + 1; } }";
    let mut w = Win::new(&["run", "/apps/spin.app"], with(&[("/apps/spin.app", src)]));
    let f = (w.send(click(1)), w.send(click(2)).unwrap()).1;
    let Some(Node::Text { style: Style::Error, text: fault, .. }) = f.nodes.last() else {
        panic!("{:?}", f.nodes)
    };
    assert!(fault.starts_with("E0206 4:"), "{fault}");
    assert_eq!(texts(&f.nodes)[0], "n = 1");
    // A clean event clears it.
    assert_eq!(texts(&w.send(click(1)).unwrap().nodes), ["n = 2"]);
    let mut w = Win::new(&["run", "/apps/missing.app"], with(&[]));
    let f = w.send(Event::Key { id: 0, key: Key::Escape, mods: 0, ch: '\0' }).unwrap();
    let edit = Node::Button { id: EDIT, variant: Variant::Primary, label: "Edit in Studio".into() };
    let heading = text(Style::Heading, "missing.app cannot run");
    let error = text(Style::Error, "error cannot read /apps/missing.app: no such file");
    assert_eq!(f.nodes, [heading, error, edit]);
    let typo = "state x = 1;\nlabel x + true;";
    let mut w = Win::new(&["run", "/apps/typo.app"], with(&[("/apps/typo.app", typo)]));
    let f = w.send(RESIZE).unwrap();
    let t = texts(&f.nodes);
    assert!(t[1].starts_with("E0303 2:") && t[2].starts_with("  label x + true;\n"), "{t:?}");
    assert!(matches!(f.nodes[2], Node::Text { style: Style::Mono, .. }) && t[2].ends_with('^'));
    assert_eq!(w.send(click(EDIT)).unwrap().requests, open("studio:/apps/typo.app"));
    assert!(w.send(click(1)).is_none());
    // A message that quotes a huge name, and a huge line, are cut to fit.
    let long = ["label ", &"n".repeat(300_000), ";"].concat();
    let mut w = Win::new(&["run", "/apps/long.app"], with(&[("/apps/long.app", &long)]));
    let t = texts(&w.send(RESIZE).unwrap().nodes);
    assert!(t[1].starts_with("E0302 1:7 ") && t[1].ends_with('…') && t[1].len() <= 1027);
    assert!(t[2].ends_with('…') && t[2].len() <= 4099);
    // A render too big for a frame is cut.
    fn depth(n: &Node) -> usize {
        1 + n.children().first().map_or(0, depth)
    }
    for (n, cut) in [(5000, true), (4000, false)] {
        let src = "label 1;".repeat(n);
        let mut w = Win::new(&["run", "/apps/many.app"], with(&[("/apps/many.app", &src)]));
        let f = w.send(RESIZE).unwrap();
        assert_eq!(f.nodes.len(), 4000 + usize::from(cut));
        assert_eq!(f.nodes.last() == Some(&text(Style::Error, TOO_BIG)), cut);
    }
    let src = ["row {".repeat(40), "label 1;".into(), "}".repeat(40)].concat();
    let mut w = Win::new(&["run", "/apps/deep.app"], with(&[("/apps/deep.app", &src)]));
    let f = w.send(RESIZE).unwrap();
    assert_eq!((f.nodes.len(), depth(&f.nodes[0])), (2, 31));
    assert_eq!(f.nodes[1], text(Style::Error, TOO_BIG));
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
    // Fs makes directories and reads back.
    let dir = std::env::temp_dir().join(format!("compusophy-studio-{}", std::process::id()));
    let file = dir.join("a").join("b.app");
    let path = file.to_str().unwrap();
    assert!(!Fs.exists(path) && Fs.write(path, "label 1;").is_ok());
    assert!(Fs.exists(path) && Fs.read(path).unwrap() == "label 1;");
    std::fs::write(path, b"\xff1").unwrap();
    assert_eq!(Fs.read(path).unwrap(), "\u{fffd}1");
    let missing = dir.join("none");
    assert_eq!(Fs.read(missing.to_str().unwrap()).unwrap_err().kind(), ErrorKind::NotFound);
    std::fs::remove_dir_all(&dir).unwrap();
}
