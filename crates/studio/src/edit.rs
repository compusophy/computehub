//! Studio's window: what it holds, and what each event does to it. [`crate::make`] asks the AI,
//! [`crate::view`] draws it.

use crate::make::{EXAMPLES, Make};
use crate::{Disk, Live, View};
use applang::Span;
use assistant::ai::{DEFAULT_MODEL, clip, problem, shown};
use std::io::ErrorKind;
use uiwire::{Event, Frame, Key, Request, Style, mods};

/// Studio's own widgets, all below [`crate::APP`], where the app's begin: buttons, then the
/// example chips (this plus their place in [`EXAMPLES`]), the prompt and the code.
pub(crate) const MAKE: u32 = 1;
pub(crate) const STOP: u32 = 2;
pub(crate) const TOGGLE: u32 = 3;
pub(crate) const OPEN: u32 = 4;
pub(crate) const NEW: u32 = 5;
pub(crate) const CHECK: u32 = 6;
pub(crate) const CHIP: u32 = 10;
/// The prompt Input is this plus how many times Studio set its text, and the Code this plus how
/// many times Studio replaced the program: a fresh id takes the frame's text.
pub(crate) const PROMPT: u32 = 1 << 24;
pub(crate) const CODE: u32 = 2 << 24;
/// The largest text Studio edits, in bytes: each frame carries all of it.
pub const MAX_TEXT: usize = 256 * 1024;
/// The largest prompt, in bytes.
const MAX_PROMPT: usize = 16 * 1024;

/// Where apps are made: a prompt, the app running live (or its code), and the AI's progress.
/// With no file it asks what to make; the first make names the file (`~/apps/<slug>.app`), and
/// every make after changes it. Check (Ctrl+Enter or Ctrl+S) runs and saves the code as edited.
/// A prompt from the desktop's everything bar ([`Event::Ask`]) is made as if typed, after the
/// make under way if there is one.
#[derive(Debug, Default)]
pub struct Studio {
    /// The app's file; "" until the first make picks one.
    pub(crate) path: String,
    /// The program as edited, the desktop's edit count for it, and how many times Studio
    /// replaced it.
    pub(crate) text: String,
    pub(crate) version: u32,
    pub(crate) edits: u32,
    /// The program running in the preview: the text as last loaded, made or checked.
    pub(crate) live: Option<Live>,
    loaded: bool,
    /// Showing the code (else the app), and whether it was edited since it was saved.
    pub(crate) code: bool,
    pub(crate) dirty: bool,
    /// Why the file may not be saved over, if it may not.
    blocked: Option<String>,
    /// Whether an edit may have been lost since the last Change.
    lost: bool,
    /// The problem's bytes while the text is still the one it was found in.
    pub(crate) mark: Option<Span>,
    /// The prompt being typed, how many times Studio set it, and the prompts made, oldest first.
    pub(crate) prompt: String,
    pub(crate) set: u32,
    pub(crate) made: Vec<String>,
    /// The model, as the desktop's last Config said.
    model: String,
    pub(crate) make: Option<Make>,
    /// Prompts from the everything bar, made in turn once no make runs.
    asks: Vec<String>,
    pub(crate) last_id: u32,
    /// What the line under the app says, and how.
    pub(crate) status: (Style, String),
    /// The content width, as the last Resize said.
    pub(crate) width: u16,
    pub(crate) requests: Vec<Request>,
    pub(crate) framed: bool,
}

impl Studio {
    /// Studio on `path`, read at its first event; with "", nothing open.
    pub fn new(path: &str) -> Studio {
        Studio { path: path.to_string(), ..Studio::default() }
    }

    pub(crate) fn prompt_id(&self) -> u32 {
        PROMPT + (self.set & 0xFF_FFFF)
    }

    pub(crate) fn code_id(&self) -> u32 {
        CODE + (self.edits & 0xFF_FFFF)
    }

    pub(crate) fn model(&self) -> &str {
        if self.model.is_empty() { DEFAULT_MODEL } else { &self.model }
    }

    /// Puts `text` in the prompt and the keyboard there.
    pub(crate) fn set_prompt(&mut self, text: &str) {
        (self.prompt, self.set) = (text.into(), self.set.wrapping_add(1));
        self.requests.push(Request::Focus { id: self.prompt_id() });
    }

    fn load(&mut self, disk: &mut dyn Disk) {
        self.loaded = true;
        self.requests.push(Request::Focus { id: self.prompt_id() });
        if self.path.is_empty() {
            return;
        }
        self.status = (Style::Small, shown(&self.path));
        match disk.read(&self.path) {
            Ok(text) if text.len() > MAX_TEXT => self.block("the file is over 256 KiB".into()),
            Ok(text) => self.replace(text),
            Err(e) if e.kind() == ErrorKind::NotFound => self.status.1.push_str(" \u{b7} new file"),
            Err(e) => self.block(format!("cannot read it: {e}")),
        }
    }

    fn block(&mut self, why: String) {
        self.status = (Style::Error, why.clone());
        self.blocked = Some(why);
    }

    /// Why the text may not be checked, saved or sent, if it may not.
    pub(crate) fn locked(&self) -> Option<&str> {
        let lost = self.lost.then_some("an edit was lost; edit again to send it");
        self.blocked.as_deref().or(lost)
    }

    /// Takes `text` as the program: a fresh Code, and the preview running it.
    pub(crate) fn replace(&mut self, text: String) {
        self.edits = self.edits.wrapping_add(1);
        (self.version, self.dirty, self.lost, self.mark) = (1, false, false, None);
        self.live = Some(Live::new(&text));
        self.text = text;
    }

    /// Saves the text; what to say about it, `done` and the path on success.
    pub(crate) fn save(&mut self, disk: &mut dyn Disk, done: &str) -> (Style, String) {
        let at = shown(&self.path);
        match disk.write(&self.path, &self.text) {
            Ok(()) => {
                self.dirty = false;
                (Style::Success, [done, &at].concat())
            }
            Err(e) => (Style::Error, format!("Couldn't save {at}: {e}")),
        }
    }

    /// Runs the code as edited if it compiles, saving it; else marks its problem.
    fn check(&mut self, disk: &mut dyn Disk) {
        if let Some(why) = self.locked() {
            self.status = (Style::Error, ["Not checked: ", why].concat());
        } else if let Err(d) = applang::compile(&self.text) {
            (self.mark, self.status) = (d.span, (Style::Error, problem(&d, &self.text)));
        } else {
            self.mark = None;
            self.live = Some(Live::new(&self.text));
            self.status = self.save(disk, "Checked \u{2713} \u{2014} saved ");
        }
    }

    fn change(&mut self, ev: &Event) {
        let Event::Change { id, version, text } = ev else { return };
        if *id == self.prompt_id() {
            self.prompt = clip(text, MAX_PROMPT);
        } else if *id != self.code_id() {
            if let Some(live) = &mut self.live {
                live.event(ev);
            }
        } else if text.len() > MAX_TEXT {
            self.lost = true;
            self.status = (Style::Error, "The code is over 256 KiB; this edit was not kept".into());
        } else {
            (self.text, self.version, self.lost, self.dirty) =
                (text.clone(), *version, false, true);
            self.mark = None;
            let edited = "Edited \u{2014} Check (Ctrl+Enter) runs and saves it";
            self.status = (Style::Small, edited.into());
        }
    }

    /// Makes the oldest prompt from the everything bar, if no make runs; whether there was one.
    fn next_ask(&mut self) -> bool {
        if self.make.is_some() || self.asks.is_empty() {
            return false;
        }
        let text = self.asks.remove(0);
        self.set_prompt(&text);
        self.make();
        true
    }

    fn open(&mut self) {
        self.requests.push(Request::Open { name: self.path.clone() });
        if self.dirty {
            let note = "Opened the saved version; Check saves your edits";
            self.status = (Style::Small, note.into());
        }
    }
}

impl View for Studio {
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool {
        let fresh = !self.loaded;
        if fresh {
            self.load(disk);
        }
        let (asked, file) = (self.make.as_ref().map(|m| m.id), !self.path.is_empty());
        let cmd = |m: u8| m & (mods::CTRL | mods::META) != 0;
        match ev {
            None => {
                self.lost = true;
                self.status = (Style::Error, "An event did not arrive whole".into());
            }
            Some(&Event::Resize { w, .. }) => {
                let narrow = self.narrow();
                self.width = w;
                return fresh || narrow != self.narrow();
            }
            Some(Event::Config { model }) => {
                self.model.clone_from(model);
                return fresh;
            }
            Some(&Event::Click { id: MAKE }) => self.make(),
            Some(&Event::Submit { id }) if id == self.prompt_id() => self.make(),
            Some(&Event::Click { id: STOP }) => self.stop(),
            Some(&Event::Click { id: TOGGLE }) if file => self.code = !self.code,
            Some(&Event::Click { id: OPEN }) if file => self.open(),
            Some(&Event::Click { id: NEW }) => {
                self.requests.push(Request::Open { name: "studio".into() })
            }
            Some(&Event::Click { id: CHECK }) if file => self.check(disk),
            Some(&Event::Click { id }) if id >= CHIP && id - CHIP < EXAMPLES.len() as u32 => {
                if self.make.is_none() {
                    self.set_prompt(EXAMPLES[(id - CHIP) as usize]);
                    self.make();
                }
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Some(ev @ Event::Change { .. }) => self.change(ev),
            Some(&Event::Key { id, key: Key::Enter, mods, .. }) if cmd(mods) => {
                if id == self.prompt_id() {
                    self.make()
                } else if file {
                    self.check(disk)
                }
            }
            Some(&Event::Key { key: Key::Char, mods, ch: 's' | 'S', .. }) if cmd(mods) && file => {
                self.check(disk)
            }
            Some(Event::AiData { id, data }) if asked == Some(*id) => self.data(data),
            Some(Event::AiEnd { id, status, error }) if asked == Some(*id) => {
                self.end(*status, error)
            }
            Some(Event::Ask { text }) if !text.trim().is_empty() => {
                self.asks.push(clip(text, MAX_PROMPT));
                self.next_ask();
            }
            Some(ev) if self.live.as_mut().is_some_and(|live| live.event(ev)) => {}
            _ => return fresh,
        }
        true
    }

    fn step(&mut self, disk: &mut dyn Disk) -> bool {
        self.verify(disk) || self.next_ask()
    }

    fn frame(&mut self) -> Frame {
        self.draw()
    }
}

/// `text`'s highlight as at most [`MAX_SPANS`] wire spans, with `mark` (if
/// non-empty and on char boundaries) an error over whatever it covers.
pub(crate) fn spans(text: &str, mark: Option<Span>) -> Vec<uiwire::Span> {
    /// The most highlight spans a frame carries; the text past them is plain.
    const MAX_SPANS: usize = 32 * 1024;
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
