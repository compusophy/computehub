//! Studio: the applang editor's window.

use crate::{Disk, NEW_APP, SAMPLES, View, file_name, problem, run::text};
use applang::Span;
use std::io::ErrorKind;
use uiwire::{Event, Frame, Key, Node, Request, Style, Variant, mods};

const RUN: u32 = 1;
const SAVE: u32 = 2;
const NEW: u32 = 3;
const CODE: u32 = 4;
/// The problem row under the editor.
const PROBLEM: u32 = 100;
/// The largest text Studio edits, in bytes: each frame carries all of it.
pub const MAX_TEXT: usize = 256 * 1024;
/// The most highlight spans a frame carries; the text past them is plain.
const MAX_SPANS: usize = 32 * 1024;

/// The applang editor. Run (Ctrl+Enter) saves and opens a file that
/// compiles, or underlines its problem; Save (Ctrl+S) and Run stay off while
/// the text may be stale (a lost or oversized edit) or was not read whole.
#[derive(Debug, Default)]
pub struct Studio {
    path: String,
    text: String,
    /// The desktop's edit count for the text, as the last Change gave it.
    version: u32,
    loaded: bool,
    dirty: bool,
    /// Why the file may not be saved over, if it may not.
    blocked: Option<String>,
    /// Whether an edit may have been lost since the last Change.
    lost: bool,
    problem: Option<String>,
    /// The problem's bytes while the text is still the one it was found in.
    mark: Option<Span>,
    status: String,
    requests: Vec<Request>,
    framed: bool,
}

impl Studio {
    /// Studio on `path`, read at its first event.
    pub fn new(path: &str) -> Studio {
        Studio { path: path.to_string(), ..Studio::default() }
    }

    fn load(&mut self, disk: &mut dyn Disk) {
        (self.loaded, self.version) = (true, 1);
        match disk.read(&self.path) {
            Ok(text) if text.len() > MAX_TEXT => self.block("the file is over 256 KiB".into()),
            Ok(text) => self.text = text,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let sample = SAMPLES.iter().find(|(p, _)| *p == self.path);
                (self.status, self.text) = ("new file".into(), sample.map_or("", |s| s.1).into());
            }
            Err(e) => self.block(format!("cannot read it: {e}")),
        }
    }

    fn block(&mut self, why: String) {
        (self.status, self.blocked) = (why.clone(), Some(why));
    }

    /// Why Save and Run are off, if they are.
    fn locked(&self) -> Option<&str> {
        let lost = self.lost.then_some("an edit was lost; edit again to send it");
        self.blocked.as_deref().or(lost)
    }

    fn change(&mut self, version: u32, text: &str) {
        if text.len() > MAX_TEXT {
            (self.lost, self.status) = (true, "the text is over 256 KiB".into());
        } else {
            (self.text, self.version, self.lost, self.dirty) = (text.into(), version, false, true);
            (self.mark, self.status) = (None, String::new());
        }
    }

    fn save(&mut self, disk: &mut dyn Disk) -> bool {
        if let Some(why) = self.locked() {
            self.status = ["not saved: ", why].concat();
            return false;
        }
        let saved = disk.write(&self.path, &self.text).map_err(|e| format!("save failed: {e}"));
        (self.dirty, self.status) = (saved.is_err(), saved.err().unwrap_or_else(|| "saved".into()));
        !self.dirty
    }

    fn run(&mut self, disk: &mut dyn Disk) {
        if let Some(why) = self.locked() {
            self.status = ["not run: ", why].concat();
            return;
        }
        if let Err(d) = applang::compile(&self.text) {
            (self.problem, self.mark) = (Some(problem(&d, &self.text)), d.span);
            self.status = "did not compile".into();
            return;
        }
        (self.problem, self.mark) = (None, None);
        if self.save(disk) {
            self.requests.push(Request::Open { name: self.path.clone() });
            self.status = "saved and running".into();
        }
    }

    fn new_file(&mut self, disk: &mut dyn Disk) {
        let name = |n| {
            if n == 1 { "/apps/untitled.app".into() } else { format!("/apps/untitled-{n}.app") }
        };
        let Some(path) = (1..100).map(name).find(|p: &String| !disk.exists(p)) else {
            self.status = "too many untitled apps".into();
            return;
        };
        self.status = match disk.write(&path, NEW_APP) {
            Ok(()) => {
                self.requests.push(Request::Open { name: ["studio:", &path].concat() });
                ["created ", &path].concat()
            }
            Err(e) => format!("new failed: {e}"),
        };
    }
}

impl View for Studio {
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool {
        let fresh = !self.loaded;
        if fresh {
            self.load(disk);
        }
        let cmd = |m: u8| m & (mods::CTRL | mods::META) != 0;
        match ev {
            None => (self.lost, self.status) = (true, "an event did not arrive whole".into()),
            Some(Event::Click { id: RUN }) => self.run(disk),
            Some(Event::Click { id: SAVE }) => _ = self.save(disk),
            Some(Event::Click { id: NEW }) => self.new_file(disk),
            Some(Event::Change { id: CODE, version, text }) => self.change(*version, text),
            // Every Change gets a frame: the desktop sends the next one then.
            Some(Event::Change { .. }) => {}
            Some(&Event::Key { key: Key::Char, mods, ch: 's' | 'S', .. }) if cmd(mods) => {
                _ = self.save(disk);
            }
            Some(&Event::Key { key: Key::Enter, mods, .. }) if cmd(mods) => self.run(disk),
            _ => return fresh,
        }
        true
    }

    fn frame(&mut self) -> Frame {
        let modified = if self.dirty { " (modified)" } else { "" };
        let dot = if self.status.is_empty() { "" } else { " · " };
        let note = text(Style::Small, &[&self.path, modified, dot, &self.status].concat());
        let button = |id, variant, label: &str| Node::Button { id, variant, label: label.into() };
        let (run, normal) = (button(RUN, Variant::Primary, "Run"), Variant::Normal);
        let toolbar = vec![run, button(SAVE, normal, "Save"), button(NEW, normal, "New"), note];
        let (spans, text) = (spans(&self.text, self.mark), self.text.clone());
        let code = Node::Code { id: CODE, version: self.version, line_numbers: true, text, spans };
        let row = Node::Row { id: 0, gap: 8, children: toolbar };
        let mut nodes = vec![row, Node::Fill { id: 0, children: vec![code] }];
        let item = |text| Node::Item { id: PROBLEM, text, detail: "".into(), selected: false };
        nodes.extend(self.problem.iter().cloned().map(item));
        let mut requests = std::mem::take(&mut self.requests);
        if !std::mem::replace(&mut self.framed, true) {
            requests.insert(0, Request::Size { w: 760, h: 540 });
        }
        let title = ["Studio — ", file_name(&self.path)].concat();
        Frame { seq: 0, title, requests, nodes }
    }
}

/// `text`'s highlight as at most [`MAX_SPANS`] wire spans, with `mark` (if
/// non-empty and on char boundaries) an error over whatever it covers.
pub(crate) fn spans(text: &str, mark: Option<Span>) -> Vec<uiwire::Span> {
    let mark = mark.filter(|m| m.start < m.end && text.get(m.start..m.end).is_some());
    // No mark is one past every token, and puts nothing.
    let (ms, me) = mark.map_or((usize::MAX, usize::MAX), |m| (m.start, m.end));
    let (mut out, mut marked) = (Vec::new(), false);
    let mut put = |start: usize, end: usize, class| {
        if start < end && out.len() < MAX_SPANS {
            // Both fit: the text is at most MAX_TEXT bytes.
            let (start, len) = (start as u32, (end - start) as u32);
            out.push(uiwire::Span { start, len, class });
        }
    };
    use uiwire::Class as W;
    // applang's classes, in their order.
    const WIRE: [W; 7] =
        [W::Keyword, W::String, W::Number, W::Comment, W::Name, W::Punct, W::Error];
    // Each token before the mark, then (once) the mark, then the token after it.
    for (s, class) in applang::highlight(text) {
        put(s.start, s.end.min(ms), WIRE[class as usize]);
        if s.end > ms && !std::mem::replace(&mut marked, true) {
            put(ms, me, W::Error);
        }
        put(s.start.max(me), s.end, WIRE[class as usize]);
    }
    if !marked {
        put(ms, me, W::Error);
    }
    out
}
