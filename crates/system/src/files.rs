//! Files: the filesystem, a folder at a time.

use icons::Glyph;
use uiwire::{Event, Frame, Node, Request, SIGIL, Style, Variant};
use vfs::{Entry, Vfs};

use crate::{Disk, View, center, group, space, text};

/// Node ids: Up, crumb `i` is `CRUMB + i`, entry `i` is `ENTRY + i`; the list's Scroll is `LIST`
/// plus the folders shown before, so each folder's list starts at its top.
pub(crate) const UP: u32 = 1;
pub(crate) const CRUMB: u32 = 10;
pub(crate) const ENTRY: u32 = 1000;
pub(crate) const LIST: u32 = 1 << 31;
/// The most rows a folder shows (a frame holds 4,096 nodes at most); the rest are counted.
pub(crate) const MAX_ROWS: usize = 1000;
const EMPTY: &str = "This folder is empty.";
/// The tiles' hues: a folder's, a plain file's.
const FOLDER: u32 = 0x60a5fa;
const FILE: u32 = 0x94a3b8;

/// A folder's contents as rows (folders first, then files with their sizes) under a path bar
/// that stays put while they scroll: Up, then the path as crumbs from `~` (or `/` outside home),
/// each one a click away, on one line (a long path slides left, its last crumb in view). A folder
/// opens in place, at its top; a `.app` runs; any other file opens in Studio. The folder is
/// listed again at every event (a focus brings one), so what changed elsewhere shows; past
/// 1,000 rows the rest are a count.
#[derive(Debug)]
pub struct Files {
    /// The folder shown, absolute, and its entries as last listed.
    pub(crate) dir: String,
    pub(crate) entries: Vec<Entry>,
    /// The entries' paths as last framed (row `i` is `ENTRY + i`): a click opens what was shown.
    shown: Vec<String>,
    /// The folder the last frame showed (empty before the first), and how many it showed before.
    framed: String,
    folders: u32,
    requests: Vec<Request>,
}

impl Files {
    /// Files at `dir` (`~` is home, relative is under home; one that is not a folder shows home).
    pub fn new(dir: &str) -> Files {
        let dir = Vfs::normalize(Vfs::HOME, dir).unwrap_or_else(|_| Vfs::HOME.to_string());
        let (entries, shown, framed) = (Vec::new(), Vec::new(), String::new());
        Files { dir, entries, shown, framed, folders: 0, requests: Vec::new() }
    }

    /// Lists the folder again (home instead, if it is gone); whether anything shown changed.
    fn refresh(&mut self, disk: &mut dyn Disk) -> bool {
        let list = match disk.list(&self.dir) {
            Err(_) if self.dir != Vfs::HOME => {
                self.dir = Vfs::HOME.to_string();
                self.refresh(disk);
                return true;
            }
            list => list.unwrap_or_default(),
        };
        // Folders first, each part in the order listed.
        let (mut dirs, files): (Vec<Entry>, Vec<Entry>) = list.into_iter().partition(|e| e.is_dir);
        dirs.extend(files);
        let changed = dirs != self.entries;
        self.entries = dirs;
        changed
    }

    /// The path of entry `name` in the folder shown.
    fn path(&self, name: &str) -> String {
        [self.dir.trim_end_matches('/'), "/", name].concat()
    }

    /// The folder's crumbs as (label, path): `~` and the folders under it, else `/` and each one.
    pub(crate) fn crumbs(&self) -> Vec<(&str, &str)> {
        let d = self.dir.as_str();
        let home = d.strip_prefix(Vfs::HOME).is_some_and(|r| r.is_empty() || r.starts_with('/'));
        let (mut out, from) =
            if home { (vec![("~", Vfs::HOME)], Vfs::HOME.len()) } else { (vec![("/", "/")], 0) };
        let mut start = from + 1;
        for (i, b) in d.bytes().enumerate().skip(from + 1).chain([(d.len(), b'/')]) {
            if b == b'/' {
                if i > start {
                    out.push((&d[start..i], &d[..i]));
                }
                start = i + 1;
            }
        }
        out
    }

    /// A click on the row framed at `path`: into a folder, or open a file; nothing if it is no
    /// longer in the folder.
    fn open(&mut self, path: String) -> bool {
        let Some(e) = self.entries.iter().find(|e| self.path(&e.name) == path) else {
            return false;
        };
        match (e.is_dir, path.ends_with(".app")) {
            (true, _) => self.dir = path,
            (false, true) => self.requests.push(Request::Open { name: path }),
            (false, false) => {
                self.requests.push(Request::Open { name: ["studio:", &path].concat() })
            }
        }
        true
    }

    /// The path bar: Up (not at `/`), then the crumbs between chevrons on one line, the last one
    /// plain, as tall as a touch target with no Up too: the rows never jump.
    fn bar(&self) -> Node {
        let all = self.crumbs();
        let mut crumbs = Vec::new();
        for (i, (label, _)) in all.iter().enumerate() {
            if i > 0 {
                let chevron = Node::Glyph { glyph: Glyph::Chevron as u8, size: 9 };
                crumbs.extend([space(2), chevron, space(2)]);
            }
            crumbs.push(match i + 1 == all.len() {
                true => text(Style::Body, label),
                false => {
                    let (id, label) = (CRUMB + i as u32, label.to_string());
                    Node::Button { id, variant: Variant::Quiet, label }
                }
            });
        }
        let mut bar = Vec::new();
        if self.dir != "/" {
            bar.push(Node::Button { id: UP, variant: Variant::Quiet, label: "\u{2191}".into() });
        }
        bar.push(Node::Strip { id: 0, gap: 0, children: crumbs });
        // A strut: no width, a touch target's height.
        bar.push(Node::Pane { id: 0, w: 0, children: vec![space(44)] });
        Node::Row { id: 0, gap: 0, children: bar }
    }
}

/// A row's tile as (glyph, hue): a folder (home's own), a `.app`'s sigil (its seed the FNV-1a of
/// its name, as the home screen's), or a file.
fn tile(e: &Entry, path: &str) -> (u8, u32) {
    match (e.is_dir, e.name.ends_with(".app")) {
        (true, _) if path == Vfs::HOME => (Glyph::Home as u8, FOLDER),
        (true, _) => (Glyph::Folder as u8, FOLDER),
        (false, true) => (
            SIGIL,
            e.name.bytes().fold(2_166_136_261, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619)),
        ),
        (false, false) => (Glyph::File as u8, FILE),
    }
}

impl View for Files {
    fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        // The folder as it is now; then the click on what was shown.
        let (fresh, dir) = (self.refresh(disk), self.dir.clone());
        let changed = match *ev {
            Event::Resize { .. } => self.framed.is_empty(),
            Event::Click { id: UP } => {
                let up = self.dir.rsplit_once('/').map_or("/", |(p, _)| p);
                self.dir = [up, "/"][usize::from(up.is_empty())].to_string();
                self.dir != dir
            }
            Event::Click { id } if (CRUMB..ENTRY).contains(&id) => {
                let crumb = self.crumbs().get((id - CRUMB) as usize).map(|c| c.1.to_string());
                crumb.is_some_and(|path| {
                    path != dir && {
                        self.dir = path;
                        true
                    }
                })
            }
            Event::Click { id } if (ENTRY..LIST).contains(&id) => {
                let path = self.shown.get((id - ENTRY) as usize).cloned();
                path.is_some_and(|path| self.open(path))
            }
            _ => false,
        };
        let moved = self.dir != dir && self.refresh(disk);
        fresh || changed || moved
    }

    fn frame(&mut self) -> Frame {
        if self.framed != self.dir {
            (self.framed, self.folders) = (self.dir.clone(), self.folders.wrapping_add(1));
        }
        let shown = self.entries.iter().take(MAX_ROWS);
        self.shown = shown.map(|e| self.path(&e.name)).collect();
        let rows = self.entries.iter().zip(&self.shown).zip(ENTRY..).map(|((e, path), id)| {
            let (glyph, hue) = tile(e, path);
            let detail = if e.is_dir { String::new() } else { size(e.size) };
            let (text, more) = (e.name.clone(), e.is_dir);
            Node::Entry { id, glyph, hue, text, detail, more }
        });
        let mut list: Vec<Node> = rows.collect();
        if let Some(more) = self.entries.len().checked_sub(MAX_ROWS).filter(|n| *n > 0) {
            list.push(center(text(Style::Small, &["+", &group(more), " more"].concat())));
        }
        if list.is_empty() {
            list.push(Node::Center {
                id: 0,
                gap: 0,
                children: vec![space(26), text(Style::Small, EMPTY)],
            });
        }
        let (id, list) =
            (LIST.wrapping_add(self.folders), Node::Col { id: 0, gap: 0, children: list });
        let nodes = vec![self.bar(), Node::Separator, Node::Scroll { id, children: vec![list] }];
        let requests = std::mem::take(&mut self.requests);
        Frame { seq: 0, title: "Files".into(), requests, nodes }
    }
}

/// `n` bytes for a person: `340 B`, `1.2 KB`, `3.4 MB`, in whole tenths.
pub(crate) fn size(n: u64) -> String {
    let (unit, scale) = match n {
        0..1024 => ("B", 1),
        1024..1_048_576 => ("KB", 1024),
        _ => ("MB", 1_048_576),
    };
    let tenths = n * 10 / scale;
    match scale {
        1 => format!("{n} {unit}"),
        _ => format!("{}.{} {unit}", tenths / 10, tenths % 10),
    }
}
