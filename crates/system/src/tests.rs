use std::io::{self, Read, Write};

use uiwire::{Event, Frame, Key, Node, Request, Style, Variant, mods};
use vfs::Vfs;

use super::*;
use crate::feedback::{AREA, BOX, GO, KIND, MAX, THANKS};
use crate::files::{CRUMB, ENTRY, UP, size, tint};

impl Disk for Vfs {
    fn list(&mut self, path: &str) -> io::Result<Vec<Entry>> {
        Vfs::list(self, path).map_err(|_| io::ErrorKind::NotFound.into())
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

/// An app run as `argv` over a VFS, a window the desktop draws.
struct Win {
    view: Box<dyn View>,
    fs: Vfs,
}

impl Win {
    fn new(argv: &str) -> Win {
        let argv: Vec<String> = argv.split(' ').map(String::from).collect();
        Win { view: view(&argv).expect("an app"), fs: Vfs::new() }
    }

    /// Serves `events` over in-memory pipes, one per read; the frames sent.
    fn send(&mut self, events: &[Event]) -> Vec<Frame> {
        let none: Box<dyn Read> = Box::new(io::empty());
        let feed = events.iter().fold(none, |f, e| Box::new(f.chain(io::Cursor::new(e.encode()))));
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
    for ok in [
        "about",
        "/bin/feedback",
        "files",
        "files ~/apps",
        "system about",
        "bin/system.wasm files /",
    ] {
        assert!(view(&argv(ok)).is_some(), "{ok}");
    }
    for no in ["", "system", "toolbox", "about more", "files a b", "system system about"] {
        assert!(view(&argv(no)).is_none(), "{no:?}");
    }
    assert!(view(&[]).is_none());
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
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let mut crates: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
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
    // A late Change of the old text is no text; a new one hides the thanks. Ctrl+Enter sends,
    // at most MAX bytes.
    let f = w.last(&[change(AREA, "stale")]);
    assert!(texts(&f.nodes).contains(&THANKS) && send(&f) == Some(Variant::Normal));
    let long = "é".repeat(MAX);
    let f = w.last(&[change(AREA + 1, &long)]);
    assert!(!texts(&f.nodes).contains(&THANKS));
    let enter = |m| Event::Key { id: AREA + 1, key: Key::Enter, mods: m, ch: '\0' };
    assert!(w.send(&[enter(0), enter(mods::SHIFT)]).is_empty());
    let f = w.last(&[enter(mods::META)]);
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
    // Into a folder, the crumbs follow; a .app runs, other files open in Studio.
    let f = w.click(ENTRY).pop().unwrap();
    assert_eq!(texts(&f.nodes), ["\u{2191}", "~", "apps", "clock.app"]);
    let app = Node::Entry {
        id: ENTRY,
        glyph: icons::Glyph::Window as u8,
        hue: tint("clock.app"),
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
        open(["studio:", h, "/notes.txt"].concat())
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
    assert_eq!(f.requests, [Request::Open { name: ["studio:", Vfs::HOME, "/notes.txt"].concat() }]);
    assert_eq!(texts(&f.nodes)[2..], ["apps", "b", "zed", "notes.txt"]);
    w.fs.remove(&[Vfs::HOME, "/notes.txt"].concat(), false).unwrap();
    let f = w.click(ENTRY + 3).pop().unwrap();
    assert!(f.requests.is_empty() && !texts(&f.nodes).contains(&"notes.txt"));
    // A deep path, narrow: the first crumbs after ~ give way to an ellipsis; the last stays.
    let deep = "~/apps/a/very/deep/folder/indeed";
    w.fs.mkdir_all(&deep.replace('~', Vfs::HOME)).unwrap();
    let mut d = Win::new(&["files ", deep].concat());
    d.fs = std::mem::replace(&mut w.fs, Vfs::new());
    let wide = texts(&d.last(&[resize(900)]).nodes).len();
    let f = d.last(&[resize(320)]);
    let said = texts(&f.nodes);
    assert!(said.len() < wide, "{said:?}");
    assert_eq!(said[1..5], ["~", "\u{2026}", "folder", "indeed"]);
    let crumbs: Vec<u32> =
        ids(&f.nodes).into_iter().filter(|i| (CRUMB..ENTRY).contains(i)).collect();
    assert_eq!(crumbs, [CRUMB, CRUMB + 5]);
}

#[test]
fn sizes_read_as_people_say_them_and_app_tiles_match_the_desktops() {
    assert_eq!([size(5), size(1536), size(3_500_000)], ["5 B", "1.5 KB", "3.3 MB"]);
    let fnv = |s: &str| {
        s.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619))
    };
    for name in ["clock.app", "a.app", "dice-roller.app", ""] {
        let ui::Rgba(r, g, b, _) = ui::theme::app_tint(fnv(name));
        assert_eq!(tint(name), u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b), "{name}");
    }
    assert_ne!(tint("a.app"), tint("b.app"));
    // Text styles the apps use exist on the wire.
    assert!(Style::from_u8(Style::Accent as u8).is_some());
}
