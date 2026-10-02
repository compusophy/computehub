//! Editor: a text file, or a new note, to write and save.

use std::io::{self, ErrorKind};
use std::mem;

use uiwire::{Event, Frame, Key, Node, Request, Style, Variant, mods};
use vfs::Vfs;

use crate::{Disk, View, center, group, space, text};

/// Where a new note goes, under home.
pub(crate) const NOTES: &str = "/notes";
/// The most bytes it edits, as much as the desktop holds in a text field (`ui::CODE_MAX`); and
/// from how many on the count shows.
pub(crate) const MAX: usize = 65_000;
pub(crate) const NEAR: usize = 60_000;
/// Node ids: Save, a new note's name; the text is `AREA` plus the times the file was read
/// again, so a text read anew takes the frame's value.
pub(crate) const SAVE: u32 = 1;
pub(crate) const NAME: u32 = 2;
pub(crate) const AREA: u32 = 100;
const HINT: &str = "Write something";
/// The column's widest: lines of text a person reads at a glance.
const MAX_W: u16 = 720;
pub(crate) const LOST: &str = "An edit did not arrive whole: type anything to send the text \
again, then save.";

/// A plain text editor: the file's name (a field for a new note's, its suggestion from the
/// first line), Save (Ctrl+S, or Enter in the name), the folder and whether there are unsaved
/// changes (the title says so too, and Save is in the accent), and the text, in the UI font,
/// wrapping, the window's whole height; in one column, 720 px at most. A new note goes to `~/notes`, under the name typed
/// (`.txt` added if it has no extension) or one from its first line that is free there. Nothing
/// is replaced unawares: a file that changed since it was read, or a name already taken, is
/// replaced by a second Save. A file it cannot edit (not UTF-8, over 65,000 bytes, unreadable)
/// says why and is never saved over. A file with no edits is read again when the window gets
/// the keyboard back; closing with unsaved changes keeps them in a file of their own.
#[derive(Debug, Default)]
pub struct Editor {
    /// The file, absolute ("" for a new note until it is saved), and why it cannot be edited.
    pub(crate) path: String,
    pub(crate) refused: Option<String>,
    /// The text as the desktop last said; as the file holds it (last read or saved; "" while
    /// there is no file), and whether there is one.
    pub(crate) text: String,
    pub(crate) base: String,
    pub(crate) on_disk: bool,
    /// A new note's name as typed.
    pub(crate) name: String,
    /// What the last Save said, until the next edit; the file a second Save replaces.
    pub(crate) said: Option<(Style, String)>,
    pub(crate) confirm: Option<String>,
    /// Whether an event did not arrive whole since the last edit: one may be missing.
    pub(crate) lost: bool,
    /// The times the file was read again; whether it was read at all; the requests since the
    /// last frame.
    reads: u32,
    loaded: bool,
    requests: Vec<Request>,
}

impl Editor {
    /// Editor on `path` (`~` is home, relative is under home), read at its first event; with
    /// "", a new note.
    pub fn new(path: &str) -> Editor {
        let mut e = Editor { path: path.to_string(), ..Editor::default() };
        if !path.is_empty() {
            // Not a device: /dev/events is the window's own.
            match Vfs::normalize(Vfs::HOME, path) {
                Ok(p) if p != "/dev" && !p.starts_with("/dev/") => e.path = p,
                _ => e.refused = Some(["Not a path to a file: ", path].concat()),
            }
        }
        e
    }

    pub(crate) fn area(&self) -> u32 {
        AREA.wrapping_add(self.reads)
    }

    /// Whether the text is not what the file holds (with no file: whether there is any).
    pub(crate) fn dirty(&self) -> bool {
        self.refused.is_none() && self.text != self.base
    }

    /// The file's text now: `None` if there is no file; else why Editor leaves it alone.
    fn fetch(&self, disk: &mut dyn Disk) -> Result<Option<String>, String> {
        let name = file_name(&self.path);
        match disk.read(&self.path) {
            Ok(b) if b.len() > MAX => Err([
                name,
                " is ",
                &group(b.len()),
                " bytes; Editor opens text up to ",
                &group(MAX),
                ".",
            ]
            .concat()),
            Ok(b) => String::from_utf8(b)
                .map(Some)
                .map_err(|_| [name, " is not UTF-8 text, so Editor leaves it as it is."].concat()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(["Can't open ", name, ": ", why(&e)].concat()),
        }
    }

    /// Reads the file, if there is one, and gives the text the keyboard.
    fn load(&mut self, disk: &mut dyn Disk) {
        if !self.path.is_empty() && self.refused.is_none() {
            match self.fetch(disk) {
                Ok(Some(t)) => (self.text, self.base, self.on_disk) = (t.clone(), t, true),
                Ok(None) => {}
                Err(why) => self.refused = Some(why),
            }
        }
        if self.refused.is_none() {
            self.requests.push(Request::Focus { id: self.area() });
        }
    }

    /// The window has the keyboard again: a file with no unsaved changes shows what is on disk
    /// now (another window may have saved it, or taken it away); whether that changed anything.
    fn reread(&mut self, disk: &mut dyn Disk) -> bool {
        if self.path.is_empty() || self.dirty() || self.refused.is_some() {
            return false;
        }
        let said = match self.fetch(disk) {
            Ok(Some(t)) if t != self.base || !self.on_disk => {
                (self.text, self.base, self.on_disk) = (t.clone(), t, true);
                self.reads = self.reads.wrapping_add(1);
                "read again: it changed elsewhere"
            }
            Ok(None) if self.on_disk => {
                (self.base, self.on_disk) = (String::new(), false);
                "gone from disk; Save puts it back"
            }
            _ => return false,
        };
        self.said = Some((Style::Small, said.into()));
        true
    }

    /// Where Save writes: the file; for a new note, the name typed in `~/notes` (`.txt` added
    /// if it has no extension), else a free one from its first line.
    fn target(&self, disk: &mut dyn Disk) -> Result<String, String> {
        if !self.path.is_empty() {
            return Ok(self.path.clone());
        }
        let notes = [Vfs::HOME, NOTES].concat();
        let typed = self.name.trim();
        if typed.is_empty() {
            let taken = || "Every name for it in ~/notes is taken".to_string();
            return free(&notes, &suggest(&self.text), disk).ok_or_else(taken);
        }
        let bad = || ["Not a name for a file: ", typed].concat();
        if typed.contains('/') || typed.bytes().all(|b| b == b'.') {
            return Err(bad());
        }
        let name = if typed.contains('.') { typed.to_string() } else { [typed, ".txt"].concat() };
        Vfs::normalize(&notes, &name).map_err(|_| bad())
    }

    /// Saves the text (see [`Editor`]): a file that is not as read, or a new note's name that
    /// is taken, waits for a second Save.
    fn save(&mut self, disk: &mut dyn Disk) {
        if self.refused.is_some() {
            return;
        }
        if self.lost {
            self.said = Some((Style::Error, LOST.into()));
            return;
        }
        let target = match self.target(disk) {
            Ok(target) => target,
            Err(why) => {
                self.said = Some((Style::Error, why));
                return;
            }
        };
        let was = (self.path == target && self.on_disk).then_some(self.base.as_bytes());
        let now = disk.read(&target).ok();
        let moved = now.as_deref() != was && now.as_deref() != Some(self.text.as_bytes());
        if moved && self.confirm.as_ref() != Some(&target) {
            let name = file_name(&target);
            let why = match was {
                None => [name, " is already in ", &shown(folder(&target))].concat(),
                Some(_) => [name, " changed since Editor read it"].concat(),
            };
            self.said = Some((Style::Error, [&why, "; Save again to replace it."].concat()));
            self.confirm = Some(target);
            return;
        }
        match disk.write(&target, self.text.as_bytes()) {
            Ok(()) => {
                // A new note's name field goes: the keyboard goes back to the text.
                if self.path.is_empty() {
                    self.requests.push(Request::Focus { id: self.area() });
                }
                (self.path, self.base, self.on_disk) = (target, self.text.clone(), true);
                (self.said, self.confirm) = (Some((Style::Small, "saved".into())), None);
            }
            Err(e) => self.said = Some((Style::Error, ["Couldn't save: ", why(&e)].concat())),
        }
    }

    /// The line under the bar: the folder and what the text is (unsaved, new, saved), or what
    /// went wrong; near the most bytes, how many.
    fn status(&self) -> Node {
        let at = match self.path.is_empty() {
            true => ["~", NOTES].concat(),
            false => shown(folder(&self.path)),
        };
        let state = match &self.said {
            Some((Style::Error, why)) => return text(Style::Error, why),
            Some((_, said)) => said.as_str(),
            None if self.refused.is_some() => "",
            None if self.dirty() => "unsaved changes",
            None if self.path.is_empty() => "new note",
            None if !self.on_disk => "new file",
            None => "",
        };
        let mut line = at;
        for part in [state.to_string(), count(self.text.len())] {
            if !part.is_empty() {
                line = [&line, " \u{b7} ", &part].concat();
            }
        }
        text(Style::Small, &line)
    }
}

/// How full the text is, from [`NEAR`] bytes on: `61,000 of 65,000 bytes`; `full` at the most.
fn count(n: usize) -> String {
    match n {
        _ if n >= MAX => ["full: ", &group(MAX), " bytes"].concat(),
        _ if n >= NEAR => [&group(n), " of ", &group(MAX), " bytes"].concat(),
        _ => String::new(),
    }
}

impl View for Editor {
    fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        let fresh = !mem::replace(&mut self.loaded, true);
        if fresh {
            self.load(disk);
        }
        let cmd = |m: u8| m & (mods::CTRL | mods::META) != 0;
        match *ev {
            Event::Click { id: SAVE } | Event::Submit { id: NAME } => self.save(disk),
            Event::Key { key: Key::Char, mods: m, ch: 's' | 'S', .. } if cmd(m) => self.save(disk),
            // Every Change gets a frame: the desktop sends the next one then.
            Event::Change { id, ref text, .. } => {
                if id == self.area() {
                    (self.text, self.lost) = (text.clone(), false);
                } else if id == NAME && self.path.is_empty() {
                    self.name.clone_from(text);
                }
                (self.said, self.confirm) = (None, None);
            }
            Event::Focus { on: true } => return self.reread(disk) || fresh,
            _ => return fresh,
        }
        true
    }

    fn garbled(&mut self) -> bool {
        self.lost = true;
        true
    }

    fn close(&mut self, disk: &mut dyn Disk) {
        if !self.dirty() {
            return;
        }
        // Never over a file: a new note as Save names it, a file's as `<name> (unsaved)`.
        let note = || [Vfs::HOME, NOTES, "/", &suggest(&self.text)].concat();
        let at = match self.path.is_empty() {
            true => self.target(disk).unwrap_or_else(|_| note()),
            false if !self.on_disk => self.path.clone(),
            false => {
                let (stem, ext) = split(&self.path);
                [stem, " (unsaved)", ext].concat()
            }
        };
        let Some((dir, name)) = at.rsplit_once('/') else { return };
        if let Some(path) = free(dir, name, disk) {
            _ = disk.write(&path, self.text.as_bytes());
        }
    }

    fn frame(&mut self) -> Frame {
        let (dirty, new) = (self.dirty(), self.path.is_empty());
        let name = if new { "new note" } else { file_name(&self.path) };
        let title = ["Editor \u{2014} ", name, if dirty { " (unsaved)" } else { "" }].concat();
        // The bar, a touch target tall: the name (a field for a new note's), Save at the right.
        let mut bar = match new {
            true => {
                let (value, placeholder) = (self.name.clone(), suggest(&self.text));
                vec![Node::Input { id: NAME, value, placeholder }]
            }
            false => {
                let room = Node::Col { id: 0, gap: 0, children: Vec::new() };
                vec![text(Style::Subheading, name), room]
            }
        };
        if self.refused.is_none() {
            let variant = if dirty { Variant::Primary } else { Variant::Normal };
            bar.push(Node::Button { id: SAVE, variant, label: "Save".into() });
        }
        bar.push(Node::Pane { id: 0, w: 0, children: vec![space(44)] });
        let body = match &self.refused {
            Some(why) => text(Style::Error, why),
            None => {
                let (id, value) = (self.area(), self.text.clone());
                let area = Node::Area { id, value, placeholder: HINT.into() };
                Node::Fill { id: 0, children: vec![area] }
            }
        };
        // One column, centered, at most as wide as reads well.
        let column = vec![Node::Row { id: 0, gap: 8, children: bar }, self.status(), body];
        let nodes = vec![center(Node::Pane { id: 0, w: MAX_W, children: column })];
        Frame { seq: 0, title, requests: mem::take(&mut self.requests), nodes }
    }
}

/// A new note's name from its text: its first line with something in it, at most 40 chars of
/// its words (no `/` or controls, no `#` or dots at its start), and `.txt`; `Untitled.txt` if
/// there is none.
pub(crate) fn suggest(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let (mut name, mut n) = (String::new(), 0);
    for c in line.chars().filter(|&c| c != '/' && !c.is_control()) {
        let space = c.is_whitespace();
        let lead = name.is_empty() && (space || c == '#' || c == '.');
        if n == 40 || lead || (space && name.ends_with(' ')) {
            continue;
        }
        (n, _) = (n + 1, name.push(if space { ' ' } else { c }));
    }
    let stem = name.trim_end();
    [if stem.is_empty() { "Untitled" } else { stem }, ".txt"].concat()
}

/// `want` in folder `dir`, or if that is taken the first of `<stem> 2<ext>` to `<stem>
/// 99<ext>` that is not; as a path.
pub(crate) fn free(dir: &str, want: &str, disk: &mut dyn Disk) -> Option<String> {
    let taken: Vec<String> =
        disk.list(dir).unwrap_or_default().into_iter().map(|e| e.name).collect();
    let (stem, ext) = split(want);
    let name = (1..100)
        .map(|n| match n {
            1 => want.to_string(),
            n => [stem, " ", &n.to_string(), ext].concat(),
        })
        .find(|name| !taken.contains(name))?;
    Some([dir, "/", &name].concat())
}

/// `name` as its stem and its extension (from its last dot, not a leading one).
fn split(name: &str) -> (&str, &str) {
    let base = name.rfind('/').map_or(0, |i| i + 1);
    name[base..].rfind('.').filter(|&i| i > 0).map_or((name, ""), |i| name.split_at(base + i))
}

/// The last name of `path`.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The folder holding `path`.
fn folder(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((dir, _)) => dir,
    }
}

/// `path` as people read it: home as `~`.
pub(crate) fn shown(path: &str) -> String {
    match path.strip_prefix(Vfs::HOME) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => ["~", rest].concat(),
        _ => path.to_string(),
    }
}

/// What an I/O error is, in a few words (not std's messages: their table would ship).
fn why(e: &io::Error) -> &'static str {
    match e.kind() {
        ErrorKind::NotFound => "not found",
        ErrorKind::PermissionDenied => "not allowed",
        ErrorKind::IsADirectory => "it is a folder",
        ErrorKind::NotADirectory => "a file is in the way",
        ErrorKind::StorageFull => "no room left",
        ErrorKind::InvalidInput => "not a valid path",
        _ => "it failed",
    }
}
