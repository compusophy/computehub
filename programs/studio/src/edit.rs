//! Studio's window: what it holds, and what each event does to it. [`crate::make`] runs the
//! make, [`crate::view`] draws it.

use crate::make::Making;
use crate::{Disk, Live, View};
use applang::Span;
use coder::ai::{DEFAULT_MODEL, clip, free_path, problem, shown, slug};

/// Why a new app was not saved.
pub(crate) const TAKEN: &str = "not saved: every name for it in ~/apps is taken";
use std::io::ErrorKind;
use uiwire::{Event, Frame, Key, Request, Style, mods};

/// Studio's own widgets, all below [`crate::APP`], where the app's begin: Make, Stop (the
/// overlay finds these as [`coder::ids`]), the code view's toggle, the prompt and the code.
pub(crate) const MAKE: u32 = 1;
pub(crate) const STOP: u32 = coder::ids::STOP;
pub(crate) const TOGGLE: u32 = 3;
/// The prompt Input is this plus how many times Studio set its text, and the Code this plus how
/// many times Studio replaced the program: a fresh id takes the frame's text.
pub(crate) const PROMPT: u32 = coder::ids::PROMPT;
pub(crate) const CODE: u32 = coder::ids::PROMPT_END;
/// The largest text Studio edits, in bytes: each frame carries all of it.
pub const MAX_TEXT: usize = 256 * 1024;
/// The largest prompt, in bytes.
const MAX_PROMPT: usize = 16 * 1024;

/// Where apps are made: the app running live (or its code, or while it is made the program
/// streaming in), a status, and the prompt. With no file it asks what to make; the first make
/// names the file (`~/apps/<slug>.app`), and every make after changes it. `</>` shows the code;
/// from the code it runs and saves it as edited (so do Ctrl+Enter and Ctrl+S), or marks its
/// problem.
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
    /// The prompt being typed, and how many times Studio set it.
    pub(crate) prompt: String,
    pub(crate) set: u32,
    /// The model, as the desktop's last Config said.
    model: String,
    pub(crate) make: Option<Making>,
    pub(crate) last_id: u32,
    /// What the line under the app says, and how; what the program's first comment says.
    pub(crate) status: (Style, String),
    pub(crate) caption: String,
    /// The make's status as the last frame showed it (it moves with each line the draft gains).
    pub(crate) seen: String,
    /// The content size, as the last Resize said.
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) requests: Vec<Request>,
    pub(crate) framed: bool,
    /// The timer and keys asked for the app.
    pub(crate) asked: (u32, bool),
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
        match disk.read(&self.path) {
            Ok(text) if text.len() > MAX_TEXT => self.block("the file is over 256 KiB".into()),
            Ok(text) => self.replace(text, disk),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                self.status = (Style::Small, [&shown(&self.path), " \u{b7} new file"].concat())
            }
            Err(e) => self.block(["cannot read it: ", &e.to_string()].concat()),
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
    pub(crate) fn replace(&mut self, text: String, disk: &mut dyn Disk) {
        self.edits = self.edits.wrapping_add(1);
        (self.version, self.dirty, self.lost, self.mark) = (1, false, false, None);
        self.live = Some(Live::new(&text, &self.path, disk));
        self.text = text;
    }

    /// Names a new app's file after `src`, its program (its label: `~/apps/<slug>.app`, a free
    /// one) if it has none; whether it has one now.
    pub(crate) fn name(&mut self, src: &str, disk: &mut dyn Disk) -> bool {
        if self.path.is_empty() {
            let path = free_path(&slug(src), &mut |p| disk.exists(p));
            self.path = path.unwrap_or_default();
        }
        !self.path.is_empty()
    }

    /// Saves the text: what to say about it, `done` and the path on success.
    pub(crate) fn save(&mut self, disk: &mut dyn Disk, done: &str) -> (Style, String) {
        let at = shown(&self.path);
        match disk.write(&self.path, &self.text) {
            Ok(()) => {
                self.dirty = false;
                (Style::Small, [done, &at].concat())
            }
            Err(e) => (Style::Error, ["couldn't save ", &at, ": ", &e.to_string()].concat()),
        }
    }

    /// Runs the code as edited if it compiles, saving it (a new app takes a name) and showing
    /// the app (saying so if it faults as it starts); else marks its problem.
    fn check(&mut self, disk: &mut dyn Disk) {
        if let Some(why) = self.locked() {
            self.status = (Style::Error, ["not checked: ", why].concat());
        } else if let Err(d) = applang::compile(&self.text) {
            (self.mark, self.status) = (d.span, (Style::Error, problem(&d, &self.text)));
        } else {
            if !self.name(&self.text.clone(), disk) {
                self.status = (Style::Error, TAKEN.into());
                return;
            }
            let live = Live::new(&self.text, &self.path, disk);
            let faults = live.faults();
            (self.live, self.mark) = (Some(live), None);
            let done = if faults { "faults as it starts \u{b7} saved " } else { "saved " };
            self.status = self.save(disk, done);
            if faults {
                self.status.0 = Style::Error;
            }
            self.code = false;
        }
    }

    fn change(&mut self, ev: &Event, disk: &mut dyn Disk) {
        let Event::Change { id, version, text } = ev else { return };
        if *id == self.prompt_id() {
            self.prompt = clip(text, MAX_PROMPT);
        } else if *id != self.code_id() {
            if let Some(live) = &mut self.live {
                live.event(ev, disk);
            }
        } else if text.len() > MAX_TEXT {
            self.lost = true;
            self.status = (Style::Error, "the code is over 256 KiB; this edit was not kept".into());
        } else {
            (self.text, self.version, self.lost, self.dirty) =
                (text.clone(), *version, false, true);
            self.mark = None;
            self.status = (Style::Small, "edited".into());
        }
    }

    /// `</>`: the code; from the code, back to the app, running and saving the code first if it
    /// was edited (staying, its problem marked, if it does not compile).
    fn toggle(&mut self, disk: &mut dyn Disk) {
        match self.code && self.dirty {
            true => self.check(disk),
            false => self.code = !self.code,
        }
    }
}

impl View for Studio {
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool {
        let fresh = !self.loaded;
        if fresh {
            self.load(disk);
        }
        let asked = self.make.as_ref().map(|m| m.id);
        let some = !self.text.is_empty();
        let cmd = |m: u8| m & (mods::CTRL | mods::META) != 0;
        match ev {
            None => {
                self.lost = true;
                self.status = (Style::Error, "an event did not arrive whole".into());
            }
            Some(&Event::Resize { w, h }) => {
                let was = (self.narrow(), self.height);
                (self.width, self.height) = (w, h);
                return fresh || was != (self.narrow(), h);
            }
            Some(Event::Config { model }) => {
                self.model.clone_from(model);
                return fresh;
            }
            Some(&Event::Click { id: MAKE }) => self.make(disk),
            Some(&Event::Submit { id }) if id == self.prompt_id() => self.make(disk),
            Some(&Event::Click { id: STOP }) => self.stop(disk),
            Some(&Event::Click { id: TOGGLE }) if some => self.toggle(disk),
            // Every Change gets a frame: the desktop sends the next one then.
            Some(ev @ Event::Change { .. }) => self.change(ev, disk),
            Some(&Event::Key { id, key: Key::Enter, mods, .. }) if cmd(mods) => {
                if id == self.prompt_id() {
                    self.make(disk)
                } else if some {
                    self.check(disk)
                }
            }
            Some(&Event::Key { key: Key::Char, mods, ch: 's' | 'S', .. }) if cmd(mods) && some => {
                self.check(disk)
            }
            Some(Event::AiData { id, data }) if asked == Some(*id) => {
                return self.data(data, disk) || fresh;
            }
            Some(Event::AiEnd { id, status, error }) if asked == Some(*id) => {
                self.end(*status, error, disk)
            }
            Some(ev) if self.live.as_mut().is_some_and(|live| live.event(ev, disk)) => {
                // A press on an app that takes keys gives it the keyboard.
                if matches!(ev, Event::Click { .. } | Event::Tap { .. }) && self.asked.1 {
                    self.requests.push(Request::Focus { id: 0 });
                }
            }
            _ => return fresh,
        }
        true
    }

    fn step(&mut self, disk: &mut dyn Disk) -> bool {
        self.verify(disk)
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
