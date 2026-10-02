use std::io::{self, Read, Write};

use uiwire::{Event, Frame, Key, Node, REVEAL, Request, SIGIL, Style, Variant, mods};
use vfs::Vfs;

use super::*;
use crate::activity::{BACK, DESKTOP, END, FILES, ROW, bytes, dollars, percent, tokens};
use crate::feedback::{AREA, BOX, GO, KIND, MAX, NEAR, THANKS};
use crate::files::{CRUMB, ENTRY, LIST, MAX_ROWS, UP, size};

impl Disk for Vfs {
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>> {
        Vfs::list(self, path).map_err(err)
    }

    fn read(&mut self, path: &str) -> io::Result<Vec<u8>> {
        Vfs::read(self, path).map(<[u8]>::to_vec).map_err(err)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        _ = self.mkdir_all(path.rsplit_once('/').map_or("/", |d| d.0));
        Vfs::write(self, path, data).map_err(err)
    }
}

/// A VFS error as WASI gives it.
fn err(e: vfs::VfsError) -> io::Error {
    use vfs::VfsError::*;
    let kind = match e {
        NotFound => io::ErrorKind::NotFound,
        NotADir => io::ErrorKind::NotADirectory,
        IsADir => io::ErrorKind::IsADirectory,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e)
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

/// An app run as `argv` over a VFS, a window the desktop draws.
struct Win {
    view: Box<dyn View>,
    fs: Vfs,
}

impl Win {
    fn new(argv: &str) -> Win {
        let argv: Vec<String> = argv.split(' ').map(String::from).collect();
        Win { view: view(&argv).expect("an app").1, fs: Vfs::new() }
    }

    /// Serves `events` over in-memory pipes, one per read; the frames sent.
    fn send(&mut self, events: &[Event]) -> Vec<Frame> {
        self.feed(events.iter().map(Event::encode).collect())
    }

    /// The same for events as bytes, which may not decode.
    fn feed(&mut self, events: Vec<Vec<u8>>) -> Vec<Frame> {
        let none: Box<dyn Read> = Box::new(io::empty());
        let feed = events.into_iter().fold(none, |f, e| Box::new(f.chain(io::Cursor::new(e))));
        let mut sent = Vec::new();
        serve(&mut Client::new(feed, Sink(&mut sent)), self.view.as_mut(), &mut self.fs).unwrap();
        sent.iter().map(|f| Frame::decode(f).expect("a frame that decodes")).collect()
    }

    /// The frame after `events`, which must bring one.
    fn last(&mut self, events: &[Event]) -> Frame {
        self.send(events).pop().expect("a frame")
    }

    fn click(&mut self, id: u32) -> Vec<Frame> {
        self.send(&[Event::Click { id }])
    }
}

fn resize(w: u16) -> Event {
    Event::Resize { w, h: 480 }
}

/// Every node of `nodes` and their children, in pre-order.
fn all(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().flat_map(|n| [vec![n], all(n.children())].concat()).collect()
}

/// The texts of `nodes`, in order: Texts, Buttons' labels, Entries' names.
fn texts(nodes: &[Node]) -> Vec<&str> {
    fn text(n: &Node) -> Option<&str> {
        match n {
            Node::Text { text, .. } | Node::Entry { text, .. } => Some(text),
            Node::Button { label, .. } => Some(label),
            _ => None,
        }
    }
    all(nodes).into_iter().filter_map(text).collect()
}

/// The ids of the Buttons and Entries of `nodes`, in order.
fn ids(nodes: &[Node]) -> Vec<u32> {
    let id = |n: &Node| match n {
        Node::Button { id, .. } | Node::Entry { id, .. } => Some(*id),
        _ => None,
    };
    all(nodes).into_iter().filter_map(id).collect()
}

#[test]
fn the_name_it_runs_as_picks_the_app() {
    let argv = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
    for (ok, name) in [
        ("about", "about"),
        ("editor", "editor"),
        ("system editor ~/notes/a.txt", "editor"),
        ("/bin/feedback", "feedback"),
        ("files", "files"),
        ("files ~/apps", "files"),
        ("system about", "about"),
        ("bin/system.wasm files /", "files"),
        ("/bin/welcome", "welcome"),
    ] {
        assert_eq!(view(&argv(ok)).map(|v| v.0), Some(name), "{ok}");
    }
    let no = [
        "",
        "system",
        "toolbox",
        "about more",
        "files a b",
        "editor a b",
        "system system about",
        "welcome x",
    ];
    for no in no {
        assert!(view(&argv(no)).is_none(), "{no:?}");
    }
    assert!(view(&[]).is_none());
    // In a terminal there is no window: it says so, and how to open one, where it is seen.
    assert_eq!(hint("files"), "files: an app with a window; open it with: open files");
    assert_eq!([hint_to([true; 3]), hint_to([false, true, false])], [Some(1); 2]);
    // `about > out.txt`, and `about < in.txt > out.txt`: stdout a file, still a terminal.
    assert_eq!([hint_to([true, false, true]), hint_to([false, false, true])], [Some(2); 2]);
    assert_eq!(hint_to([true, false, false]), Some(2));
    assert_eq!(hint_to([false; 3]), None, "a window's console is no terminal");
}

#[test]
fn about_shows_the_mark_the_name_the_stack_and_credits() {
    let mut w = Win::new("about");
    let f = w.last(&[resize(560), Event::Config { model: "m".into() }]);
    assert!(f.title == "About" && f.requests.is_empty());
    // The head, centered: the mark, the name, the version and build.
    let Node::Center { children: head, .. } = &f.nodes[0] else { panic!("{:?}", f.nodes[0]) };
    assert_eq!(head[1], Node::Glyph { glyph: icons::Glyph::Mark as u8, size: 89 });
    let said = texts(&f.nodes);
    let version = [about::VERSION, " \u{b7} build ", about::BUILD].concat();
    assert_eq!(said[..2], ["compusophy", version.as_str()]);
    assert!(said.contains(&"The stack") && said.contains(&"Credits"));
    assert_eq!(said.last(), Some(&"github.com/compusophy/computehub"));
    // The stack names every crate, once; roles beside the names, under them when narrow.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let mut crates: Vec<String> = ["crates", "programs"]
        .iter()
        .flat_map(|d| std::fs::read_dir([root, d].concat()).unwrap())
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let mut named: Vec<String> = about::STACK.iter().map(|s| s.0.to_string()).collect();
    crates.sort();
    named.sort();
    assert_eq!(named, crates);
    let rows =
        |f: &Frame| all(&f.nodes).iter().filter(|n| matches!(n, Node::Pane { w: 144, .. })).count();
    assert_eq!(rows(&f), about::STACK.len());
    assert!(w.send(&[resize(500), Event::Focus { on: true }]).is_empty(), "nothing moves");
    let narrow = w.last(&[resize(360)]);
    assert_eq!(rows(&narrow), 0);
    assert_eq!(texts(&narrow.nodes).len(), texts(&f.nodes).len());
}

#[test]
fn feedback_sends_its_kind_text_and_context_and_thanks() {
    let mut w = Win::new("feedback");
    let f = w.last(&[resize(520)]);
    assert_eq!(
        (f.title.as_str(), f.requests.clone()),
        ("Feedback", vec![Request::Focus { id: AREA }])
    );
    let chips = |f: &Frame| {
        let on = |n: &&Node| matches!(n, Node::Button { variant: Variant::On, .. });
        all(&f.nodes)
            .into_iter()
            .filter(on)
            .map(|n| texts(std::slice::from_ref(n))[0].to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(chips(&f), ["Idea"]);
    let area =
        |f: &Frame| all(&f.nodes).into_iter().find(|n| matches!(n, Node::Area { .. })).cloned();
    let empty = Node::Area {
        id: AREA,
        value: "".into(),
        placeholder: "What happened, or what would make it better?".into(),
    };
    assert_eq!(area(&f), Some(empty));
    let toggle = |f: &Frame| {
        all(&f.nodes).into_iter().find_map(|n| match n {
            Node::Toggle { id: BOX, on, .. } => Some(*on),
            _ => None,
        })
    };
    assert_eq!(toggle(&f), Some(true));
    let send = |f: &Frame| {
        all(&f.nodes).into_iter().find_map(|n| match n {
            Node::Button { id: GO, variant, .. } => Some(*variant),
            _ => None,
        })
    };
    assert_eq!(send(&f), Some(Variant::Normal), "nothing to send yet");
    assert!(w.click(GO).is_empty() && w.send(&[resize(300)]).is_empty());
    // Every Change gets a frame; the kind and the context switch.
    let change = |id, text: &str| Event::Change { id, version: 1, text: text.into() };
    let f = w.last(&[change(AREA, "  Hello\nworld  ")]);
    assert_eq!(send(&f), Some(Variant::Primary));
    assert!(w.click(KIND + 1).is_empty(), "Idea already");
    let f = w.last(&[Event::Click { id: KIND }, Event::Click { id: BOX }]);
    assert_eq!((chips(&f), toggle(&f)), (vec!["Bug".to_string()], Some(false)));
    // Send: the request, a fresh empty text with the keyboard in it, and the thanks.
    let f = w.click(GO).pop().unwrap();
    let sent =
        Request::Feedback { kind: "bug".into(), text: "Hello\nworld".into(), context: false };
    assert_eq!(f.requests, [sent, Request::Focus { id: AREA + 1 }]);
    assert!(
        matches!(area(&f), Some(Node::Area { id, value, .. }) if id == AREA + 1 && value.is_empty())
    );
    assert!(texts(&f.nodes).contains(&THANKS));
    // A late Change of the old text is no text; a new one hides the thanks. Near the most a
    // count shows; past it Send waits (nothing is cut); at the most, Ctrl+Enter sends.
    let f = w.last(&[change(AREA, "stale")]);
    assert!(texts(&f.nodes).contains(&THANKS) && send(&f) == Some(Variant::Normal));
    let said = |f: &Frame| {
        let t = texts(&f.nodes).into_iter().find(|t| t.contains(" bytes")).map(String::from);
        let error =
            all(&f.nodes).iter().any(|n| matches!(n, Node::Text { style: Style::Error, .. }));
        (t, error, send(f))
    };
    let f = w.last(&[change(AREA + 1, &"x".repeat(NEAR - 1))]);
    assert!(!texts(&f.nodes).contains(&THANKS) && said(&f).0.is_none() && f.title == "Feedback");
    let f = w.last(&[change(AREA + 1, &"x".repeat(NEAR))]);
    assert_eq!(said(&f), (Some("7,000 of 8,000 bytes".into()), false, Some(Variant::Primary)));
    // The title says it too: a long text grows the box, and what is under it, below the fold.
    assert_eq!(f.title, "Feedback \u{2014} 7,000 of 8,000 bytes");
    let enter = |m| Event::Key { id: AREA + 1, key: Key::Enter, mods: m, ch: '\0' };
    let long = "é".repeat(MAX);
    let f = w.last(&[change(AREA + 1, &long)]);
    let over = Some("Too long to send: 16,000 of 8,000 bytes".into());
    assert_eq!(said(&f), (over, true, Some(Variant::Normal)));
    assert_eq!(f.title, "Feedback \u{2014} too long: 16,000 of 8,000 bytes");
    assert!(w.send(&[enter(mods::CTRL), Event::Click { id: GO }]).is_empty(), "nothing cut");
    assert!(w.send(&[enter(0), enter(mods::SHIFT)]).is_empty());
    let f = w.last(&[change(AREA + 1, &long[..MAX]), enter(mods::META)]);
    let Request::Feedback { kind, text, context: false } = &f.requests[0] else {
        panic!("{:?}", f.requests)
    };
    assert_eq!((kind.as_str(), text.len()), ("bug", MAX));
}

/// A VFS with folders and files under home.
fn home() -> Vfs {
    let mut fs = Vfs::new();
    let h = Vfs::HOME;
    for d in ["/apps", "/zed"] {
        fs.mkdir_all(&[h, d].concat()).unwrap();
    }
    fs.write(&[h, "/apps/clock.app"].concat(), b"app").unwrap();
    fs.write(&[h, "/notes.txt"].concat(), &[b'x'; 1536]).unwrap();
    fs
}

#[test]
fn files_walks_folders_and_opens_what_it_finds() {
    let (mut w, h) = (Win::new("files"), Vfs::HOME);
    w.fs = home();
    let f = w.last(&[resize(640)]);
    assert_eq!(f.title, "Files");
    assert_eq!(texts(&f.nodes), ["\u{2191}", "~", "apps", "zed", "notes.txt"], "folders first");
    assert_eq!(ids(&f.nodes), [UP, 1000, 1001, 1002]);
    let entry = |f: &Frame, i: usize| {
        all(&f.nodes).into_iter().filter(|n| matches!(n, Node::Entry { .. })).nth(i).cloned()
    };
    let folder = Node::Entry {
        id: ENTRY,
        glyph: icons::Glyph::Folder as u8,
        hue: 0x60a5fa,
        text: "apps".into(),
        detail: "".into(),
        more: true,
    };
    assert_eq!(entry(&f, 0), Some(folder));
    assert!(
        matches!(entry(&f, 2), Some(Node::Entry { detail, more: false, .. }) if detail == "1.5 KB")
    );
    // Into a folder, the crumbs follow; a .app runs, other files open in Editor.
    let f = w.click(ENTRY).pop().unwrap();
    assert_eq!(texts(&f.nodes), ["\u{2191}", "~", "apps", "clock.app"]);
    // A .app's tile is its sigil, seeded as the home screen seeds it.
    let seed = host::sigil(&[h, "/apps/clock.app"].concat()).expect("a sigil");
    let app = Node::Entry {
        id: ENTRY,
        glyph: SIGIL,
        hue: seed,
        text: "clock.app".into(),
        detail: "3 B".into(),
        more: false,
    };
    assert_eq!(entry(&f, 0), Some(app));
    let open = |name: String| vec![Request::Open { name }];
    assert_eq!(w.click(ENTRY).pop().unwrap().requests, open([h, "/apps/clock.app"].concat()));
    assert_eq!(texts(&w.click(CRUMB).pop().unwrap().nodes)[1..3], ["~", "apps"]);
    assert_eq!(
        w.click(ENTRY + 2).pop().unwrap().requests,
        open(["editor:", h, "/notes.txt"].concat())
    );
    // Up, out of home: crumbs from /, home's own tile; at the root no Up.
    let f = w.click(UP).pop().unwrap();
    assert_eq!(texts(&f.nodes), ["\u{2191}", "/", "home", "guest"]);
    assert!(
        matches!(entry(&f, 0), Some(Node::Entry { glyph, .. }) if glyph == icons::Glyph::Home as u8)
    );
    let f = w.click(UP).pop().unwrap();
    assert!(texts(&f.nodes)[0] == "/" && texts(&f.nodes).contains(&"home"));
    assert!(w.click(UP).is_empty() && w.click(CRUMB).is_empty(), "already there");
    // A change to the files shows at the next event, as a focus brings.
    w.fs.write("/top.txt", b"x").unwrap();
    assert_eq!(w.send(&[Event::Focus { on: true }, Event::Focus { on: false }]).len(), 1);
}

#[test]
fn files_opens_what_was_shown_and_starts_where_asked() {
    // files <dir> opens there; a folder that is not falls back to home; an empty one says so.
    let mut w = Win::new("files ~/apps");
    w.fs = home();
    assert_eq!(texts(&w.last(&[resize(640)]).nodes), ["\u{2191}", "~", "apps", "clock.app"]);
    let mut gone = Win::new("files /nope");
    gone.fs = Vfs::new();
    let f = gone.last(&[resize(640)]);
    assert_eq!(texts(&f.nodes), ["\u{2191}", "~", "This folder is empty."]);
    // A click opens the row as framed, though the folder changed since; a row whose entry went
    // opens nothing.
    let mut w = Win::new("files");
    w.fs = home();
    w.send(&[resize(640)]);
    w.fs.mkdir(&[Vfs::HOME, "/b"].concat()).unwrap();
    let f = w.click(ENTRY + 2).pop().unwrap();
    assert_eq!(f.requests, [Request::Open { name: ["editor:", Vfs::HOME, "/notes.txt"].concat() }]);
    assert_eq!(texts(&f.nodes)[2..], ["apps", "b", "zed", "notes.txt"]);
    w.fs.remove(&[Vfs::HOME, "/notes.txt"].concat(), false).unwrap();
    let f = w.click(ENTRY + 3).pop().unwrap();
    assert!(f.requests.is_empty() && !texts(&f.nodes).contains(&"notes.txt"));
    // A deep path: every crumb, on one line (the desktop slides it, the last in view), the last
    // plain; a new size brings no new frame.
    let deep = "~/apps/a/very/deep/folder/indeed";
    w.fs.mkdir_all(&deep.replace('~', Vfs::HOME)).unwrap();
    let mut d = Win::new(&["files ", deep].concat());
    d.fs = std::mem::replace(&mut w.fs, Vfs::new());
    let f = d.last(&[resize(320)]);
    let Node::Row { children: bar, .. } = &f.nodes[0] else { panic!("{:?}", f.nodes[0]) };
    let Node::Strip { children: crumbs, .. } = &bar[1] else { panic!("{:?}", bar[1]) };
    let said = ["~", "apps", "a", "very", "deep", "folder", "indeed"];
    assert_eq!(texts(crumbs), said);
    assert_eq!(ids(crumbs), (CRUMB..CRUMB + 6).collect::<Vec<_>>());
    assert!(matches!(crumbs.last(), Some(Node::Text { style: Style::Body, .. })));
    assert!(d.send(&[resize(900)]).is_empty());
}

#[test]
fn files_keeps_its_bar_up_starts_each_folder_at_the_top_and_counts_what_it_cannot_show() {
    // The bar and its rule stay; the list scrolls below them, a new Scroll for a new folder.
    let mut w = Win::new("files");
    w.fs = home();
    fn scroll(f: &Frame) -> (u32, Vec<&str>) {
        match &f.nodes[..] {
            [Node::Row { .. }, Node::Separator, Node::Scroll { id, children }] => {
                (*id, texts(children))
            }
            n => panic!("{n:?}"),
        }
    }
    let f = w.last(&[resize(640)]);
    assert_eq!(scroll(&f), (LIST + 1, vec!["apps", "zed", "notes.txt"]));
    let f = w.last(&[Event::Focus { on: true }, Event::Click { id: ENTRY }]);
    assert_eq!(scroll(&f), (LIST + 2, vec!["clock.app"]));
    w.fs.write(&[Vfs::HOME, "/apps/b.app"].concat(), b"").unwrap();
    assert_eq!(scroll(&w.last(&[Event::Focus { on: true }])).0, LIST + 2, "the same folder");
    assert_eq!(scroll(&w.click(UP).pop().unwrap()).0, LIST + 3);
    // A folder of thousands: the first MAX_ROWS rows, then how many more; the frame holds.
    let big = [Vfs::HOME, "/big"].concat();
    w.fs.mkdir(&big).unwrap();
    for i in 0..MAX_ROWS + 4321 {
        w.fs.write(&[&big, "/f", &i.to_string()].concat(), b"").unwrap();
    }
    w.send(&[Event::Focus { on: true }]);
    let f = w.click(ENTRY + 1).pop().expect("a frame that holds");
    let (_, said) = scroll(&f);
    assert_eq!((said.len(), said.last().copied()), (MAX_ROWS + 1, Some("+4,321 more")));
    assert!(f.encode_checked().is_some());
    assert_eq!(*ids(&f.nodes).last().unwrap(), ENTRY + MAX_ROWS as u32 - 1);
}

#[test]
fn welcome_lists_the_apps_under_the_mark_it_reveals() {
    let mut w = Win::new("welcome");
    let f = w.last(&[Event::Resize { w: 520, h: 768 }]);
    assert!(f.title == "Welcome" && f.requests.is_empty());
    // The mark, revealed, 1/φ of the shorter side at most 144; the name, a line, the hint.
    let revealed = icons::Glyph::Mark as u8 | REVEAL;
    let marks = |f: &Frame| {
        let mark = |n: &&Node| matches!(n, Node::Glyph { glyph, .. } if *glyph == revealed);
        all(&f.nodes).into_iter().filter(mark).cloned().collect::<Vec<_>>()
    };
    assert_eq!(marks(&f), [Node::Glyph { glyph: revealed, size: 144 }]);
    let said = texts(&f.nodes);
    assert_eq!(
        said[..3],
        [
            "compusophy",
            "a computer in your browser \u{2014} free AI, nothing to install.",
            welcome::HINT
        ]
    );
    assert!(welcome::HINT.contains("home screen") && welcome::HINT.contains("bottom right"));
    assert!(all(&f.nodes).iter().any(|n| matches!(n, Node::Text { style: Style::Display, .. })));
    // The apps, a row each: name over what it is for, its icon, a chevron; a click opens it.
    let rows: Vec<_> = said[3..].iter().map(|t| t.split('\n').next().unwrap()).collect();
    assert_eq!(rows, ["Studio", "Assistant", "Terminal", "Files", "Settings", "About", "Feedback"]);
    assert!(
        said[3..].iter().all(|t| t.contains('\n')) && ids(&f.nodes) == (1..=7).collect::<Vec<_>>()
    );
    let opened: Vec<Request> = (1..=7).flat_map(|id| w.click(id).pop().unwrap().requests).collect();
    let names = welcome::APPS.map(|a| Request::Open { name: a.0.into() });
    assert_eq!(opened, names);
    assert!(w.click(8).is_empty() && w.send(&[Event::Resize { w: 600, h: 900 }]).is_empty());
    // A small window, a smaller mark: 1/φ of its shorter side.
    let f = w.last(&[Event::Resize { w: 360, h: 200 }]);
    assert!(matches!(marks(&f)[..], [Node::Glyph { size: 123, .. }]), "{:?}", marks(&f));
    assert!(welcome::MARK_MAX == 144 && w.send(&[Event::Focus { on: true }]).is_empty());
}

#[test]
fn sizes_and_counts_read_as_people_say_them() {
    assert_eq!([size(5), size(1536), size(3_500_000)], ["5 B", "1.5 KB", "3.3 MB"]);
    let counts = [0, 7, 999, 1000, 8000, 65_536, 1_234_567].map(group);
    assert_eq!(counts, ["0", "7", "999", "1,000", "8,000", "65,536", "1,234,567"]);
    // Text styles the apps use exist on the wire.
    assert!(Style::from_u8(Style::Display as u8).is_some());
}

/// An Editor frame's column (centered, at most 720 wide): its bar, status line and text.
fn column(f: &Frame) -> &[Node] {
    match &f.nodes[..] {
        [Node::Row { children, .. }] => match &children[..] {
            [_, Node::Pane { w: 720, children, .. }, _] => children,
            n => panic!("{n:?}"),
        },
        n => panic!("{n:?}"),
    }
}

/// Its status line, as its style and text.
fn status(f: &Frame) -> (Style, &str) {
    match &column(f)[1] {
        Node::Text { style, text, .. } => (*style, text.as_str()),
        n => panic!("{n:?}"),
    }
}

/// Its Save button's look, if it has one.
fn save(f: &Frame) -> Option<Variant> {
    all(&f.nodes).into_iter().find_map(|n| match n {
        Node::Button { id: editor::SAVE, variant, .. } => Some(*variant),
        _ => None,
    })
}

/// Its text's id and value, if it has one (in a Fill: the window's height).
fn text_of(f: &Frame) -> Option<(u32, &str)> {
    match &column(f)[2] {
        Node::Fill { children, .. } => match &children[..] {
            [Node::Area { id, value, .. }] => Some((*id, value.as_str())),
            n => panic!("{n:?}"),
        },
        _ => None,
    }
}

/// Its name field's value and placeholder, for a new note.
fn name_field(f: &Frame) -> Option<(&str, &str)> {
    all(&f.nodes).into_iter().find_map(|n| match n {
        Node::Input { id: editor::NAME, value, placeholder } => Some((&**value, &**placeholder)),
        _ => None,
    })
}

fn edit(id: u32, text: &str) -> Event {
    Event::Change { id, version: 1, text: text.into() }
}

fn key_s(mods: u8) -> Event {
    Event::Key { id: editor::AREA, key: Key::Char, mods, ch: 's' }
}

#[test]
fn editor_writes_a_new_note_names_it_and_saves_it_in_notes() {
    use editor::{AREA, NAME, NOTES, SAVE};
    let (mut w, notes) = (Win::new("editor"), [Vfs::HOME, NOTES].concat());
    let focus = [Request::Focus { id: AREA }];
    let f = w.last(&[Event::Resize { w: 411, h: 700 }]);
    assert_eq!((f.title.as_str(), &f.requests[..]), ("Editor \u{2014} new note", &focus[..]));
    assert_eq!((name_field(&f), text_of(&f)), (Some(("", "Untitled.txt")), Some((AREA, ""))));
    let new = (Style::Small, "~/notes \u{b7} new note");
    assert_eq!((save(&f), status(&f)), (Some(Variant::Normal), new));
    // Typed: unsaved (the title says so, Save is in the accent), named after its first words.
    let f = w.last(&[edit(AREA, "\n  # Shopping /  list\neggs")]);
    assert_eq!(f.title, "Editor \u{2014} new note (unsaved)");
    assert_eq!(name_field(&f), Some(("", "Shopping list.txt")));
    let unsaved = "~/notes \u{b7} unsaved changes";
    assert_eq!((save(&f), status(&f).1), (Some(Variant::Primary), unsaved));
    // Saved: the file in ~/notes, its name in the bar, the keyboard back in the text.
    let f = w.click(SAVE).pop().unwrap();
    let path = [&notes, "/Shopping list.txt"].concat();
    assert_eq!(w.fs.read(&path).unwrap(), b"\n  # Shopping /  list\neggs");
    let title = "Editor \u{2014} Shopping list.txt";
    assert_eq!((f.title.as_str(), &f.requests[..]), (title, &focus[..]));
    assert!(name_field(&f).is_none() && texts(&f.nodes)[0] == "Shopping list.txt");
    assert_eq!((save(&f), status(&f).1), (Some(Variant::Normal), "~/notes \u{b7} saved"));
    // Ctrl+S and Cmd+S save too; a plain S is no save.
    w.send(&[edit(AREA, "list"), key_s(mods::SHIFT)]);
    assert_eq!(w.fs.read(&path).unwrap(), b"\n  # Shopping /  list\neggs");
    let f = w.last(&[key_s(mods::CTRL)]);
    assert_eq!((w.fs.read(&path).unwrap(), status(&f).1), (&b"list"[..], "~/notes \u{b7} saved"));
    w.send(&[edit(AREA, "list\nmilk"), key_s(mods::META)]);
    assert_eq!(w.fs.read(&path).unwrap(), b"list\nmilk");
    // Another note of the same name takes the next free one.
    let mut v = Win::new("editor");
    v.fs = std::mem::replace(&mut w.fs, Vfs::new());
    v.send(&[resize(411), edit(AREA, "Shopping list"), Event::Click { id: SAVE }]);
    assert_eq!(v.fs.read(&[&notes, "/Shopping list 2.txt"].concat()).unwrap(), b"Shopping list");
    // A name typed (Enter saves): `.txt` added if it has none; one taken is replaced only by a
    // second Save, and an edit asks again.
    let mut n = Win::new("editor");
    n.fs = std::mem::replace(&mut v.fs, Vfs::new());
    let named =
        [resize(411), edit(AREA, "x"), edit(NAME, " Shopping list "), Event::Submit { id: NAME }];
    let f = n.last(&named);
    let taken =
        (Style::Error, "Shopping list.txt is already in ~/notes; Save again to replace it.");
    assert_eq!((status(&f), n.fs.read(&path).unwrap()), (taken, &b"list\nmilk"[..]));
    let f = n.last(&[edit(AREA, "xy"), Event::Click { id: SAVE }]);
    assert_eq!((status(&f), n.fs.read(&path).unwrap()), (taken, &b"list\nmilk"[..]));
    let f = n.last(&[Event::Click { id: SAVE }]);
    assert_eq!((status(&f).1, n.fs.read(&path).unwrap()), ("~/notes \u{b7} saved", &b"xy"[..]));
    // Names that are no file's say so; one with an extension keeps it.
    let mut b = Win::new("editor");
    b.send(&[resize(411)]);
    let long = "n".repeat(300);
    for bad in ["a/b", "..", &long] {
        let f = b.last(&[edit(NAME, bad), Event::Click { id: SAVE }]);
        assert_eq!(status(&f), (Style::Error, &*["Not a name for a file: ", bad].concat()));
    }
    let f = b.last(&[edit(NAME, "plan.md"), Event::Submit { id: NAME }]);
    assert!(b.fs.is_file(&[&notes, "/plan.md"].concat()) && f.title == "Editor \u{2014} plan.md");
}

#[test]
fn editor_opens_text_files_and_leaves_alone_what_it_cannot_hold() {
    use editor::{AREA, MAX, NEAR};
    let h = Vfs::HOME;
    let mut fs = home();
    fs.write(&[h, "/a.bin"].concat(), &[b'f', 0xff, 0xfe]).unwrap();
    fs.write(&[h, "/big.log"].concat(), &vec![b'x'; MAX + 1]).unwrap();
    fs.write(&[h, "/full.txt"].concat(), &vec![b'y'; MAX]).unwrap();
    let open = |path: &str| {
        let mut w = Win::new(&["editor ", path].concat());
        w.fs = fs.clone();
        let f = w.last(&[resize(640)]);
        (w, f)
    };
    // A file: its text, its name; nothing unsaved. Relative is under home.
    let xs = "x".repeat(1536);
    for path in ["~/notes.txt", "notes.txt"] {
        let (_, f) = open(path);
        assert_eq!(
            (f.title.as_str(), status(&f)),
            ("Editor \u{2014} notes.txt", (Style::Small, "~"))
        );
        assert_eq!((text_of(&f), save(&f)), (Some((AREA, xs.as_str())), Some(Variant::Normal)));
        assert_eq!(f.requests, [Request::Focus { id: AREA }]);
    }
    // A file not there yet: empty; Save makes it, and its folder.
    let (mut w, f) = open("~/drafts/new.md");
    assert_eq!((status(&f).1, text_of(&f)), ("~/drafts \u{b7} new file", Some((AREA, ""))));
    w.send(&[edit(AREA, "hi"), Event::Click { id: editor::SAVE }]);
    assert_eq!(w.fs.read(&[h, "/drafts/new.md"].concat()).unwrap(), b"hi");
    // Not UTF-8, too big, a folder, a device: why, and no text, no Save; nothing written, even on close.
    let refusals = [
        ("a.bin", "a.bin is not UTF-8 text, so Editor leaves it as it is."),
        ("big.log", "big.log is 65,001 bytes; Editor opens text up to 65,000."),
        ("zed", "Can't open zed: it is a folder"),
        ("/tmp/x\0", "Not a path to a file: /tmp/x\0"),
        ("/dev/events", "Not a path to a file: /dev/events"),
    ];
    for (path, why) in refusals {
        let (mut w, f) = open(path);
        assert_eq!(column(&f)[2], Node::Text { id: 0, style: Style::Error, text: why.into() });
        assert!(save(&f).is_none() && f.requests.is_empty(), "{path}");
        w.send(&[key_s(mods::CTRL), Event::Click { id: editor::SAVE }, Event::Close]);
        assert!(w.fs == fs, "{path}");
    }
    // At the most (as much as the desktop's text holds) it opens, and says it is full; near
    // it, how near.
    assert_eq!(MAX, ui::CODE_MAX);
    let (mut w, f) = open("full.txt");
    assert_eq!(status(&f).1, "~ \u{b7} full: 65,000 bytes");
    let f = w.last(&[edit(AREA, &"z".repeat(NEAR))]);
    assert_eq!(status(&f).1, "~ \u{b7} unsaved changes \u{b7} 60,000 of 65,000 bytes");
    assert!(f.encode_checked().is_some());
}

#[test]
fn editor_replaces_only_what_it_read_and_reads_again_what_changed() {
    use editor::{AREA, SAVE};
    let path = [Vfs::HOME, "/notes.txt"].concat();
    let mut w = Win::new("editor notes.txt");
    w.fs = home();
    w.fs.write(&path, b"one").unwrap();
    w.send(&[resize(640)]);
    // Saved elsewhere meanwhile (another window): Save says so; a second one replaces it.
    w.fs.write(&path, b"theirs").unwrap();
    let f = w.last(&[edit(AREA, "mine"), Event::Click { id: SAVE }]);
    let why = (Style::Error, "notes.txt changed since Editor read it; Save again to replace it.");
    assert_eq!((status(&f), w.fs.read(&path).unwrap()), (why, &b"theirs"[..]));
    let f = w.last(&[Event::Click { id: SAVE }]);
    assert_eq!((status(&f).1, w.fs.read(&path).unwrap()), ("~ \u{b7} saved", &b"mine"[..]));
    // Nothing unsaved: the window's focus reads it again, into a fresh text; a late Change of
    // the old one is no edit. With unsaved changes it waits for Save.
    w.fs.write(&path, b"newer").unwrap();
    let f = w.last(&[Event::Focus { on: true }]);
    let again = "~ \u{b7} read again: it changed elsewhere";
    assert_eq!((text_of(&f), status(&f).1), (Some((AREA + 1, "newer")), again));
    assert!(w.send(&[Event::Focus { on: true }, Event::Focus { on: false }]).is_empty());
    let f = w.last(&[edit(AREA, "stale")]);
    let title = "Editor \u{2014} notes.txt";
    assert_eq!((text_of(&f), f.title.as_str()), (Some((AREA + 1, "newer")), title));
    w.send(&[edit(AREA + 1, "edited")]);
    w.fs.write(&path, b"other").unwrap();
    assert!(w.send(&[Event::Focus { on: true }]).is_empty());
    // Taken away while it shows: unsaved now, and Save puts it back.
    let mut d = Win::new("editor notes.txt");
    d.fs = home();
    d.send(&[resize(640)]);
    d.fs.remove(&path, false).unwrap();
    let f = d.last(&[Event::Focus { on: true }]);
    let gone = "~ \u{b7} gone from disk; Save puts it back";
    assert_eq!((status(&f).1, save(&f)), (gone, Some(Variant::Primary)));
    d.click(SAVE);
    assert_eq!(d.fs.read(&path).unwrap(), &[b'x'; 1536][..]);
}

#[test]
fn editor_keeps_unsaved_text_when_it_closes_and_says_what_went_wrong() {
    use editor::{AREA, LOST, NAME, NOTES, SAVE};
    let (h, notes) = (Vfs::HOME, [Vfs::HOME, NOTES].concat());
    // Closed with unsaved changes: a new note as Save would name it, a file's beside it, never
    // over anything.
    let mut w = Win::new("editor");
    w.fs = home();
    w.send(&[resize(640), edit(AREA, "Idea\nmore"), Event::Close]);
    assert_eq!(w.fs.read(&[&notes, "/Idea.txt"].concat()).unwrap(), b"Idea\nmore");
    let mut f = Win::new("editor notes.txt");
    f.fs = std::mem::replace(&mut w.fs, Vfs::new());
    f.send(&[resize(640), edit(AREA, "edited"), Event::Close]);
    assert_eq!(f.fs.read(&[h, "/notes (unsaved).txt"].concat()).unwrap(), b"edited");
    assert_eq!(f.fs.read(&[h, "/notes.txt"].concat()).unwrap().len(), 1536);
    let mut t = Win::new("editor");
    t.fs = f.fs.clone();
    t.send(&[resize(640), edit(AREA, "again"), edit(NAME, "Idea"), Event::Close]);
    assert_eq!(t.fs.read(&[&notes, "/Idea 2.txt"].concat()).unwrap(), b"again");
    // Nothing unsaved, nothing kept.
    for argv in ["editor", "editor notes.txt"] {
        let mut c = Win::new(argv);
        c.fs = f.fs.clone();
        let text = if argv == "editor" { "" } else { &*"x".repeat(1536) };
        c.send(&[resize(640), edit(AREA, "Idea\nmore"), edit(AREA, text), Event::Close]);
        assert!(c.fs == f.fs, "{argv}");
    }
    // A save that fails says why, and the text stays unsaved: a file where ~/notes would be.
    let mut e = Win::new("editor");
    e.fs.write(&notes, b"").unwrap();
    let f = e.last(&[resize(640), edit(AREA, "x"), Event::Click { id: SAVE }]);
    let failed = (Style::Error, "Couldn't save: a file is in the way");
    assert_eq!((status(&f), f.title.as_str()), (failed, "Editor \u{2014} new note (unsaved)"));
    // An event that did not arrive whole may have been an edit: no Save until the next one.
    let mut l = Win::new("editor");
    l.send(&[resize(640), edit(AREA, "a")]);
    let f = l.feed(vec![vec![99], Event::Click { id: SAVE }.encode()]);
    assert_eq!((f.len(), status(&f[1])), (2, (Style::Error, LOST)));
    assert!(!l.fs.exists(&notes));
    l.send(&[edit(AREA, "ab"), Event::Click { id: SAVE }]);
    assert!(l.fs.is_file(&[&notes, "/ab.txt"].concat()));
    // The other apps pass over such an event.
    assert!(Win::new("about").feed(vec![vec![99]]).is_empty());
}

/// A process as a sample lists it: pid, window, state, argv, DRAWs; then its meters (ms busy,
/// KB), if it has them.
type Row = (u32, u32, u8, &'static str, u32, Option<(u32, u32)>);

/// The desktop's sample at `at` ms: `loud` counts by index (the rest 0), the processes, the
/// desktop's µs, and Activity's own meters (12 ms, 9,000 KB).
fn sample(at: u32, loud: &[(usize, u32)], rows: &[Row], us: u32) -> Event {
    use uiwire::stat::{LOUD, Proc, Stats};
    let mut counts = vec![0; LOUD];
    loud.iter().for_each(|&(i, n)| counts[i] = n);
    let argv = |s: &str| s.split(' ').map(String::from).collect();
    let proc =
        |r: &Row| Proc { pid: r.0, window: r.1, state: r.2, argv: argv(r.3), counts: vec![r.4] };
    let meters = rows.iter().filter_map(|r| r.5.map(|(b, kb)| (r.0, vec![b, kb]))).collect();
    let (procs, quiet) = (rows.iter().map(proc).collect(), vec![1, 2, 3, us, 18_000]);
    let s = Stats { at, loud: counts, procs, meters, quiet, own: vec![12, 9_000] };
    Event::Stats { data: s.encode() }
}

/// Activity at its first size, then `samples`: the last frame.
fn activity(samples: &[Event]) -> (Win, Frame) {
    let mut w = Win::new("activity");
    let mut all = vec![resize(440)];
    all.extend_from_slice(samples);
    let f = w.last(&all);
    (w, f)
}

/// The word and the line under it.
fn word(f: &Frame) -> [&str; 2] {
    let said = texts(&f.nodes);
    [said[0], said[1]]
}

#[test]
fn activity_watches_from_its_first_size_and_draws_only_what_changes() {
    let mut w = Win::new("activity");
    let f = w.last(&[resize(440)]);
    assert_eq!(
        (f.title.as_str(), f.requests.clone()),
        ("Activity", vec![Request::Watch { on: true }])
    );
    assert_eq!(word(&f), ["Measuring", "Asking the desktop."]);
    // On a desktop it watches whatever has the focus: a monitor beside a busy window.
    assert!(w.send(&[resize(600), Event::Focus { on: false }]).is_empty());
    // The first sample lists what runs; rates come with the second, a second later.
    let still = |at| sample(at, &[], &[], 0);
    let f = w.last(&[still(1000)]);
    assert_eq!(word(&f)[0], "Measuring");
    assert!(texts(&f.nodes).contains(&"The desktop\nthe home screen and windows \u{b7} 18 MB"));
    assert_eq!(word(&w.last(&[still(2000)])), ["Still", "Nothing is drawing."]);
    // The same again: nothing it shows changed, so no frame.
    assert!(w.send(&[still(3000), still(4000)]).is_empty());
    assert!(w.send(&[Event::Stats { data: vec![9] }]).is_empty(), "a sample that does not decode");
}

#[test]
fn on_a_phone_focus_pauses_and_resumes_the_watch() {
    use uiwire::stat::RUNS;
    // Narrower than on a desktop, it fills a phone's screen: shown exactly while focused.
    let mut w = Win::new("activity");
    assert_eq!(w.last(&[resize(409)]).requests, [Request::Watch { on: true }]);
    let spin = |busy| -> Row { (7, 2, RUNS, "spin", 0, Some((busy, 64))) };
    let f = w.last(&[sample(1000, &[], &[spin(0)], 0), sample(2000, &[], &[spin(990)], 0)]);
    assert_eq!(word(&f)[0], "Busy");
    let paused = w.last(&[Event::Focus { on: false }]);
    assert_eq!((paused.requests, paused.nodes), (vec![Request::Watch { on: false }], f.nodes));
    assert!(w.send(&[Event::Focus { on: false }]).is_empty());
    // Back: watching again; no rate spans the pause, so the first look only lists (here what it
    // listed: no frame).
    let back = w.last(&[Event::Focus { on: true }]);
    assert_eq!(
        (&back.requests[..], word(&back)[0]),
        (&[Request::Watch { on: true }][..], "Measuring")
    );
    assert!(w.send(&[sample(60_000, &[], &[spin(1990)], 0)]).is_empty());
    let f = w.last(&[sample(61_000, &[], &[spin(2980)], 0)]);
    assert_eq!(word(&f), ["Busy", "spin is using 99% of a core."]);
    // Made wide (a phone turned), it watches whatever has the focus.
    w.send(&[Event::Focus { on: false }, resize(600)]);
    assert!(w.send(&[Event::Focus { on: false }, Event::Focus { on: true }]).is_empty());
}

#[test]
fn the_word_says_busy_drawing_resting_or_still() {
    use uiwire::stat::{GRAIN_ON, IDLE, INPUT, MOTION, PROGRAMS, RUNS};
    let pair = |loud: &[(usize, u32)], rows: &[Row], gap: u32| {
        let none: Vec<Row> =
            rows.iter().map(|r| (r.0, r.1, r.2, r.3, 0, r.5.map(|m| (0, m.1)))).collect();
        let f = activity(&[sample(1000, &[], &none, 0), sample(1000 + gap, loud, rows, 0)]).1;
        word(&f).map(String::from)
    };
    let said = |w: &str, l: &str| [w.to_string(), l.to_string()];
    assert_eq!(pair(&[], &[], 1000), said("Still", "Nothing is drawing."));
    let resting = "Only the living grain draws, 8 frames a second. Settings\u{a0}\u{203a} Appearance can still it.";
    assert_eq!(pair(&[(GRAIN_ON, 1)], &[], 1000), said("Resting", resting));
    // Drawing, by the largest cause, input first; the program that drew the most is named.
    let input = pair(&[(INPUT, 12), (MOTION, 12)], &[], 1000);
    assert_eq!(input, said("Drawing", "12 frames a second, following you."));
    let moving = pair(&[(MOTION, 60), (PROGRAMS, 4)], &[], 1000);
    assert_eq!(moving, said("Drawing", "60 frames a second: something is moving."));
    let snake: Row = (6, 4, IDLE, "studio run /apps/snake.app", 10, Some((3, 900)));
    let drawn = pair(&[(PROGRAMS, 5)], &[snake], 500);
    assert_eq!(drawn, said("Drawing", "10 frames a second: snake.app is drawing."));
    assert_eq!(
        pair(&[(PROGRAMS, 1)], &[], 1000),
        said("Drawing", "1 frame a second: A program is drawing.")
    );
    // Samples further apart than 3 s give no rate: no number.
    assert_eq!(pair(&[(INPUT, 3)], &[], 5000), said("Drawing", "Following you."));
    // Busy: another program used half a core or more.
    let spin: Row = (7, 2, RUNS, "spin 100", 0, Some((990, 1024)));
    let busy = pair(&[(INPUT, 3)], &[spin, (8, 2, RUNS, "bench", 0, Some((600, 64)))], 1000);
    assert_eq!(busy, said("Busy", "spin is using 99% of a core. And 1 more."));
}

#[test]
fn rows_name_programs_by_command_line_and_say_what_they_do() {
    use uiwire::stat::{ENDED, IDLE, RUNS};
    #[rustfmt::skip]
    let rows: [Row; 6] = [(4, 3, IDLE, "files ~", 0, Some((2, 2048))),
        (5, 0, IDLE, "/bin/assistant", 0, Some((0, 3072))),
        (6, 9, IDLE, "studio run /apps/snake.app", 9, Some((5, 900))),
        (7, 2, RUNS, "spin 100", 0, Some((500, 1024))), (8, 2, RUNS, "nap 5", 0, None),
        (9, 6, ENDED, "studio edit ~/notes.txt", 0, None)];
    let first: Vec<Row> =
        rows.iter().map(|r| (r.0, r.1, r.2, r.3, 0, r.5.map(|m| (0, m.1)))).collect();
    let (_, f) = activity(&[sample(1000, &[], &first, 0), sample(2000, &[], &rows, 20_000)]);
    let entries: Vec<&str> = texts(&f.nodes).into_iter().filter(|t| t.contains('\n')).collect();
    let dot = " \u{b7} ";
    let want = [
        format!("The desktop\nthe home screen and windows{dot}18 MB"),
        format!("Files\nidle{dot}2.1 MB"),
        format!("Assistant\nidle{dot}3.1 MB"),
        format!("snake.app\ndrawing 9 a second{dot}922 KB"),
        format!("spin\nin Terminal{dot}working{dot}1.0 MB"),
        format!("nap\nin Terminal{dot}working{dot}\u{2014}"),
        format!("Studio: notes.txt\nended{dot}\u{2014}"),
        format!("Activity\nthis window{dot}9.2 MB"),
    ];
    assert_eq!(entries[..8], want.iter().map(String::as_str).collect::<Vec<_>>()[..]);
    // Their tiles, a share of a core at the right, and which rows open a page.
    let rows: Vec<_> = all(&f.nodes)
        .into_iter()
        .filter_map(|n| match n {
            Node::Entry { id, glyph, hue, detail, more, .. } => {
                Some((*id, *glyph, *hue, detail.as_str(), *more))
            }
            _ => None,
        })
        .collect();
    let seed = "snake.app"
        .bytes()
        .fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    assert_eq!(rows[0], (DESKTOP, icons::Glyph::Mark as u8, 0x94a3b8, "2%", true));
    assert_eq!(rows[3], (ROW + 6, SIGIL, seed, "<1%", true));
    assert_eq!(rows[4], (ROW + 7, icons::Glyph::Terminal as u8, 0x2dd4bf, "50%", true));
    assert_eq!((rows[7].0, rows[7].1, rows[7].4), (0, icons::Glyph::Pulse as u8, false));
    assert_eq!(rows[8], (FILES, icons::Glyph::Folder as u8, 0x60a5fa, "", true));
}

#[test]
fn end_comes_only_from_a_program_page() {
    use uiwire::stat::{IDLE, RUNS};
    let rows: [Row; 2] =
        [(5, 0, IDLE, "assistant", 0, Some((0, 64))), (7, 2, RUNS, "spin 9", 0, Some((0, 64)))];
    let (mut w, _) = activity(&[sample(1000, &[], &rows, 0)]);
    let f = w.click(ROW + 7).pop().unwrap();
    let said = texts(&f.nodes);
    assert_eq!(said[..4], ["\u{2039} Running", "spin", "spin 9", "Running in a Terminal."]);
    assert!(
        said.contains(&"End spin")
            && said.contains(&"It stops at once, as Ctrl+C would. What it had not saved is lost.")
    );
    // Escape and Back return to the list; Activity's own row and the files open no page.
    assert_eq!(
        word(&w.last(&[Event::Key { id: 0, key: Key::Escape, mods: 0, ch: '\0' }]))[0],
        "Measuring"
    );
    assert!(w.click(ROW).is_empty() && w.click(0).is_empty());
    assert_eq!(w.click(FILES).pop().unwrap().requests, [Request::Open { name: "files".into() }]);
    // A command line that only says the name is not repeated.
    let page = texts(&w.click(ROW + 5).pop().unwrap().nodes).join("|");
    assert!(page.starts_with("\u{2039} Running|Assistant|Idle"), "{page}");
    assert!(page.contains("|Ends the Assistant now. It starts again when you call it."));
    assert_eq!(word(&w.click(BACK).pop().unwrap())[0], "Measuring");
    // The desktop's page has no End; End asks once and the row leaves until the desktop agrees.
    let desk = w.click(DESKTOP).pop().unwrap();
    assert!(!ids(&desk.nodes).contains(&END) && texts(&desk.nodes)[1] == "The desktop");
    assert!(w.click(END).is_empty(), "nothing to end on the desktop's page");
    w.click(ROW + 7);
    let f = w.click(END).pop().unwrap();
    assert_eq!(f.requests, [Request::End { pid: 7 }]);
    assert!(!ids(&f.nodes).contains(&(ROW + 7)) && w.click(END).is_empty());
    // Still listed: still left out; gone: a page for it shows the list.
    assert!(
        w.send(&[sample(2000, &[], &rows, 0)]).iter().all(|f| !ids(&f.nodes).contains(&(ROW + 7)))
    );
    w.send(&[sample(3000, &[], &rows[..1], 0)]);
    assert!(w.click(ROW + 7).iter().all(|f| texts(&f.nodes)[0] != "\u{2039} Running"));
}

#[test]
fn files_ai_and_numbers_read_as_people_say_them() {
    use uiwire::stat::{ASKED, FAILED, HOME, MICROUSD, TOKENS_IN, TOKENS_OUT, UNKEPT, UNMETERED};
    assert_eq!(
        [740, 212_000, 999_999, 1_300_000, 18_400_000].map(bytes),
        ["740 B", "212 KB", "1.0 MB", "1.3 MB", "18 MB"]
    );
    assert_eq!(
        [None, Some(5), Some(120), Some(1000)].map(percent),
        ["\u{2014}", "<1%", "12%", "100%"]
    );
    assert_eq!([940, 12_400, 1_234_567].map(tokens), ["940", "12.4k", "1.2M"]);
    let squares = |f: &Frame| {
        all(&f.nodes).into_iter().find_map(|n| match n {
            Node::Grid { cells, .. } => Some(cells.clone()),
            _ => None,
        })
    };
    let (_, f) = activity(&[sample(1000, &[(HOME, 212_000)], &[], 0)]);
    let said = texts(&f.nodes);
    assert!(
        said.contains(&"Your files\n212 KB of about 5 MB") && said.contains(&"No requests yet.")
    );
    assert_eq!(squares(&f), Some([vec![2; 2], vec![0; 22]].concat()));
    // Nearly full: yellow; unkept: red, and why.
    let (_, f) = activity(&[sample(1000, &[(HOME, 4_200_000)], &[], 0)]);
    assert_eq!(squares(&f).map(|s| (s[20], s[21])), Some((3, 0)));
    let (_, f) = activity(&[sample(1000, &[(HOME, 9), (UNKEPT, 1)], &[], 0)]);
    assert_eq!(squares(&f).map(|s| s[0]), Some(1));
    assert!(texts(&f.nodes).contains(
        &"This browser refused to keep your files. Changes since then are lost at reload."
    ));
    // The AI by its receipts: requests, tokens, cost; what failed, what had no receipt.
    #[rustfmt::skip]
    let ai = [(ASKED, 14), (TOKENS_IN, 11_460), (TOKENS_OUT, 940), (MICROUSD, 19_400), (FAILED, 2),
        (UNMETERED, 1)];
    let said = texts(&activity(&[sample(1000, &ai, &[], 0)]).1.nodes).join("|");
    let line =
        "14 requests \u{b7} 12.4k tokens \u{b7} about $0.02|2 failed|1 answer had no receipt";
    assert!(said.contains(line), "{said}");
    let one = [(ASKED, 1), (UNMETERED, 3)];
    let said = texts(&activity(&[sample(1000, &one, &[], 0)]).1.nodes).join("|");
    assert!(said.contains("|1 request|3 answers had no receipt|"), "{said}");
    assert_eq!(
        [1, 4_999, 5_000, 19_400, 1_254_999].map(dollars),
        ["under $0.01", "under $0.01", "about $0.01", "about $0.02", "about $1.25"]
    );
}
