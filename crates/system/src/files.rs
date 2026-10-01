//! Files: the filesystem, a folder at a time.

use icons::Glyph;
use uiwire::{Event, Frame, Node, Request, Style, Variant};
use vfs::{Entry, Vfs};

use crate::{Disk, View, space, text};

/// Node ids: Up, crumb `i` is `CRUMB + i`, entry `i` is `ENTRY + i`.
pub(crate) const UP: u32 = 1;
pub(crate) const CRUMB: u32 = 10;
pub(crate) const ENTRY: u32 = 1000;
const EMPTY: &str = "This folder is empty.";
/// The tiles' hues: a folder's, a plain file's.
const FOLDER: u32 = 0x60a5fa;
const FILE: u32 = 0x94a3b8;
/// About how wide a crumb's char is (Inter at 14 px averages less), what a crumb adds to its
/// label (its padding and the chevron before it), and what the bar holds besides the crumbs (the
/// window's padding, Up).
const CHAR_W: usize = 8;
const CRUMB_W: usize = 16 + 13;
const BESIDE: usize = 40 + 25;

/// A folder's contents as rows (folders first, then files with their sizes) under a path bar: Up,
/// then the path as crumbs from `~` (or `/` outside home), each one a click away (on a narrow
/// window the first after `~` or `/` give way to an ellipsis). A folder opens in place, a `.app`
/// runs, any other file opens in Studio. The folder is listed again at every event (a focus
/// brings one), so what changed elsewhere shows.
#[derive(Debug)]
pub struct Files {
    /// The folder shown, absolute, and its entries as last listed.
    pub(crate) dir: String,
    pub(crate) entries: Vec<Entry>,
    /// The entries' paths as last framed (row `i` is `ENTRY + i`): a click opens what was shown.
    shown: Vec<String>,
    /// The content width the last Resize gave, and the requests since the last frame.
    width: u16,
    requests: Vec<Request>,
}

impl Files {
    /// Files at `dir` (`~` is home, relative is under home; one that is not a folder shows home).
    pub fn new(dir: &str) -> Files {
        let dir = Vfs::normalize(Vfs::HOME, dir).unwrap_or_else(|_| Vfs::HOME.to_string());
        Files { dir, entries: Vec::new(), shown: Vec::new(), width: 0, requests: Vec::new() }
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

    /// The path bar: Up (not at `/`), then the crumbs between chevrons, the last one plain; on a
    /// narrow window the first after the root give way to an ellipsis.
    fn bar(&self) -> Node {
        let mut bar = Vec::new();
        if self.dir != "/" {
            bar.push(Node::Button { id: UP, variant: Variant::Quiet, label: "\u{2191}".into() });
        }
        let crumbs = self.crumbs();
        let wide = |c: &(&str, &str)| c.0.chars().count() * CHAR_W + CRUMB_W;
        let room = usize::from(self.width).saturating_sub(BESIDE);
        // How many crumbs after the root give way to the ellipsis (which takes a crumb's room).
        let mut gone = 0;
        let mut need: usize = crumbs.iter().map(wide).sum();
        while need > room && gone + 2 < crumbs.len() {
            gone += 1;
            need -= wide(&crumbs[gone]) - if gone == 1 { CHAR_W + CRUMB_W } else { 0 };
        }
        let chevron = || [space(2), Node::Glyph { glyph: Glyph::Chevron as u8, size: 9 }, space(2)];
        for (i, (label, _)) in crumbs.iter().enumerate().filter(|(i, _)| *i == 0 || *i > gone) {
            if i > 0 {
                bar.extend(chevron());
            }
            if i > 0 && i == gone + 1 && gone > 0 {
                bar.extend([text(Style::Dim, "\u{2026}")].into_iter().chain(chevron()));
            }
            let id = CRUMB + i as u32;
            bar.push(match i + 1 == crumbs.len() {
                true => text(Style::Body, label),
                false => Node::Button { id, variant: Variant::Quiet, label: label.to_string() },
            });
        }
        // A touch target's height (Up's on a phone), with no Up (at `/`) too: rows never jump.
        bar.push(space(44));
        Node::Row { id: 0, gap: 0, children: bar }
    }
}

/// A row's tile: a folder (home's own), a `.app` on its name's hue, or a file.
fn tile(e: &Entry, path: &str) -> (Glyph, u32) {
    match (e.is_dir, e.name.ends_with(".app")) {
        (true, _) if path == Vfs::HOME => (Glyph::Home, FOLDER),
        (true, _) => (Glyph::Folder, FOLDER),
        (false, true) => (Glyph::Window, tint(&e.name)),
        (false, false) => (Glyph::File, FILE),
    }
}

impl View for Files {
    fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        // The folder as it is now; then the click on what was shown.
        let (fresh, dir) = (self.refresh(disk), self.dir.clone());
        let changed = match *ev {
            Event::Resize { w, .. } => std::mem::replace(&mut self.width, w) != w,
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
            Event::Click { id } if id >= ENTRY => {
                let path = self.shown.get((id - ENTRY) as usize).cloned();
                path.is_some_and(|path| self.open(path))
            }
            _ => false,
        };
        let moved = self.dir != dir && self.refresh(disk);
        fresh || changed || moved
    }

    fn frame(&mut self) -> Frame {
        self.shown = self.entries.iter().map(|e| self.path(&e.name)).collect();
        let rows = self.entries.iter().zip(&self.shown).enumerate().map(|(i, (e, path))| {
            let (glyph, hue) = tile(e, path);
            let detail = if e.is_dir { String::new() } else { size(e.size) };
            let (id, text, more) = (ENTRY + i as u32, e.name.clone(), e.is_dir);
            Node::Entry { id, glyph: glyph as u8, hue, text, detail, more }
        });
        let mut nodes = vec![self.bar(), Node::Separator];
        nodes.push(match self.entries.is_empty() {
            true => {
                Node::Center { id: 0, gap: 0, children: vec![space(26), text(Style::Small, EMPTY)] }
            }
            false => Node::Col { id: 0, gap: 0, children: rows.collect() },
        });
        let requests = std::mem::take(&mut self.requests);
        Frame { seq: 0, title: "Files".into(), requests, nodes }
    }
}

/// FNV-1a of `name` as a hue, as the desktop tints a `.app` file's tile (`ui::theme::app_tint`):
/// 0xRRGGBB.
pub(crate) fn tint(name: &str) -> u32 {
    let id =
        name.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    // Millidegrees in integers, so the hue is exact for every id.
    let h = ((u64::from(id) * 137_508) % 360_000) as f32 / 60_000.0;
    let (v, s) = (0.9, 0.45);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let sixths = [(c, x, 0.0), (x, c, 0.0), (0.0, c, x), (0.0, x, c), (x, 0.0, c), (c, 0.0, x)];
    let (r, g, b) = sixths[(h as usize).min(5)];
    let byte = |f: f32| u32::from(((f + v - c) * 255.0).round() as u8);
    byte(r) << 16 | byte(g) << 8 | byte(b)
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
