//! Remote: the window of a GUI program, a kernel process that describes it as a [`uiwire`] tree,
//! drawn here by [`uiview`] in the frame's theme; the window's input goes back as uiwire events.
//! The program starts at the first size ([`Event::Resize`]) with the roots `/`; until its first
//! frame the window shows a note, or why it failed (a first frame that does not decode: the
//! program is newer than the desktop). A frame's title is the window's and its requests are
//! honored (Size in the first only; Focus when the frame holds that Input, Code or Area; Feedback
//! goes to the page); a clean exit closes the window, and closing it sends [`Event::Close`]. The
//! window's focus goes to the program as [`Event::Focus`], and a prompt from the everything bar
//! as [`Event::Ask`], held until it starts. Edited text is owned as uiwire says, one
//! [`Event::Change`] out at a time: the next waits for a frame, or goes before any other event.
//! The wheel scrolls the Code under it, else what does not fit.

use std::mem;

use ui::icon::Glyph;
use ui::kernel::{Spawn, wire::Stdout};
use ui::{App, AppEvent, AppIcon, Code, Cx, Key, Mods, Rgba, Ui, WidgetId};
use uiview::{Area, Texts, View};
use uiwire::{Event, Frame, Node, Request};
use vfs::Vfs;

use crate::ai::Ai;

/// The Studio and Assistant programs.
pub const STUDIO: &str = "/bin/studio";
pub const ASSISTANT: &str = "/bin/assistant";
/// Studio's icon (braces on violet), and that of every `.app` it runs.
pub const STUDIO_ICON: AppIcon = AppIcon { glyph: Glyph::Studio, hue: Rgba::hex(0x8b7bff) };
pub const APP_ICON: AppIcon = AppIcon { glyph: Glyph::Window, hue: Rgba::hex(0xf59e0b) };
pub const ASSISTANT_ICON: AppIcon = AppIcon { glyph: Glyph::Assistant, hue: Rgba::hex(0xa78bfa) };
/// A system app as (name, title, icon, size, whether compact).
pub type SystemApp = (&'static str, &'static str, AppIcon, (f32, f32), bool);
/// About, Feedback and Files: one program, bin/system.wasm, run as the name of its /bin marker.
#[rustfmt::skip]
pub const SYSTEM: [SystemApp; 3] = [
    ("about", "About", icon(Glyph::About, 0xfbbf24), (560.0, 640.0), true),
    ("feedback", "Feedback", icon(Glyph::Feedback, 0x34d399), (520.0, 420.0), true),
    ("files", "Files", icon(Glyph::Folder, 0x60a5fa), (640.0, 480.0), false),
];
/// Studio's size: room for the app beside its prompt.
const STUDIO_SIZE: Option<(f32, f32)> = Some((880.0, 560.0));
/// Shown in place of a first frame that does not decode.
const NEWER: &str = "This program is newer than the desktop; reload the page";

const fn icon(glyph: Glyph, hue: u32) -> AppIcon {
    AppIcon { glyph, hue: Rgba::hex(hue) }
}

/// The app for a window name: About, Feedback or Files ([`SYSTEM`]; `"files:<dir>"` is Files at
/// that folder), the Assistant for `"assistant"`, Studio with nothing open for `"studio"` or on
/// `<path>` for `"studio:<path>"`, or running a `.app` path (relative: in `/apps`).
pub fn open(name: &str, ai: &Ai) -> Option<Box<dyn App>> {
    let (head, dir) = name.strip_prefix("files:").map_or((name, None), |d| ("files", Some(d)));
    if let Some(&(prog, title, icon, size, compact)) = SYSTEM.iter().find(|s| s.0 == head) {
        let argv = [prog].into_iter().chain(dir).map(String::from).collect();
        let mut r = Remote::new(&["/bin/", prog].concat(), argv, ai);
        (r.title, r.icon, r.size, r.compact) = (title.into(), icon, Some(size), compact);
        return Some(Box::new(r));
    }
    let abs = |p: &str| Vfs::normalize("/apps", p).ok().filter(|_| !p.is_empty());
    let file = |p: &str| p.rsplit('/').next().unwrap_or_default().to_string();
    let (argv, title, icon, size) = match name.strip_prefix("studio:") {
        _ if name == "assistant" => {
            (vec![name.into()], "Assistant".into(), ASSISTANT_ICON, Some((560.0, 600.0)))
        }
        _ if name == "studio" => (vec![name.into()], "Studio".into(), STUDIO_ICON, STUDIO_SIZE),
        Some(p) => {
            let path = abs(p)?;
            let title = ["Studio \u{2014} ", &file(&path)].concat();
            (["studio", "edit", &path].map(String::from).into(), title, STUDIO_ICON, STUDIO_SIZE)
        }
        None if name.ends_with(".app") => {
            let path = abs(name)?;
            (["studio", "run", &path].map(String::from).into(), file(&path), APP_ICON, None)
        }
        None => return None,
    };
    let program = if name == "assistant" { ASSISTANT } else { STUDIO };
    let mut r = Remote::new(program, argv, ai);
    (r.title, r.icon, r.size, r.view.follow) = (title, icon, size, name == "assistant");
    Some(Box::new(r))
}

/// A GUI program in a window: see the module docs.
#[derive(Default)]
pub struct Remote {
    /// The title until a frame names one, the icon, the preferred size, whether it is a small
    /// card ([`App::compact`]), and how it is scrolled.
    pub title: String,
    pub icon: AppIcon,
    pub size: Option<(f32, f32)>,
    pub compact: bool,
    pub view: View,
    program: String,
    argv: Vec<String>,
    pid: Option<u32>,
    ai: Ai,
    /// Why nothing runs (empty while it does), and its last output.
    note: String,
    log: String,
    frame: Option<Frame>,
    texts: Texts,
    /// Edited ids whose Change waits, and whether one is out unanswered.
    dirty: Vec<u32>,
    waiting: bool,
    /// The size last told, whether Close was sent, the last key.
    told: Option<(u16, u16)>,
    closed: bool,
    last_key: Option<Key>,
    /// Prompts from the everything bar that wait for the program to start.
    asks: Vec<String>,
    /// The content's corner as last drawn.
    origin: (f32, f32),
}

impl Remote {
    /// The window of `program` (a wasm file or marker in the VFS) run with `argv`, sharing `ai`.
    pub fn new(program: &str, argv: Vec<String>, ai: &Ai) -> Remote {
        Remote { program: program.to_string(), argv, ai: ai.clone(), ..Remote::default() }
    }

    fn start(&mut self, cx: &mut Cx<'_>) {
        let (argv, cwd, roots, stdout) =
            (self.argv.clone(), "/".into(), vec!["/".into()], Stdout::Console);
        let pid = guest::program(cx.vfs, "/", &self.program)
            .map_err(|missing| if missing { "not found" } else { "cannot execute" })
            .and_then(|program| {
                cx.kernel.spawn(Spawn { argv, program, cwd, tty: None, stdout, roots })
            });
        match pid {
            Ok(pid) => self.pid = Some(pid),
            Err(e) => self.note = [&self.program, ": ", e].concat(),
        }
    }

    fn post(&mut self, ev: Event, cx: &mut Cx<'_>) {
        if let Some(pid) = self.pid {
            cx.kernel.post_event(pid, &ev.encode());
        }
    }

    /// Sends the waiting Changes, then `ev`; false: nothing here to redraw.
    fn send(&mut self, ev: Event, cx: &mut Cx<'_>) -> bool {
        self.flush(cx);
        self.post(ev, cx);
        false
    }

    /// Text `id` was edited: its Change goes now, or after the next frame.
    fn changed(&mut self, id: u32, cx: &mut Cx<'_>) -> bool {
        if !self.dirty.contains(&id) {
            self.dirty.push(id);
        }
        if !self.waiting {
            self.flush(cx);
        }
        true
    }

    fn flush(&mut self, cx: &mut Cx<'_>) {
        for id in mem::take(&mut self.dirty) {
            let t = &self.texts;
            let code = t.codes.iter().find(|c| c.0 == id).map(|c| (c.1.version, c.1.ed.text()));
            let input = || t.inputs.iter().find(|i| i.0 == id).map(|i| (i.2, i.1.clone()));
            let area =
                || t.areas.iter().find(|a| a.0 == id).map(|a| (a.1.version, a.1.text.clone()));
            if let Some((version, text)) = code.or_else(input).or_else(area) {
                self.post(Event::Change { id, version, text }, cx);
                self.waiting = true;
            }
        }
    }

    /// A key: an edit in the focused Input, Code or Area, else a Key event for
    /// Escape (which also leaves the editor), Enter and chords.
    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        let (chord, id) = (mods.ctrl || mods.alt || mods.meta, self.texts.focus);
        let (plain, t) = (!chord && key != Key::Escape, &mut self.texts);
        let edited = t.codes.iter_mut().find(|c| c.0 == id && plain).and_then(|c| c.1.key(key));
        let edited = edited.or_else(|| t.areas.iter_mut().find(|a| a.0 == id && plain)?.1.key(key));
        match (edited, self.texts.inputs.iter_mut().find(|i| i.0 == id && plain), key) {
            (Some(edited), ..) => return !edited || self.changed(id, cx),
            (_, Some(_), Key::Enter) => return self.send(Event::Submit { id }, cx),
            (_, Some(i), Key::Backspace) => {
                let popped = i.1.pop().is_some();
                i.2 = i.2.wrapping_add(u32::from(popped));
                return popped && self.changed(id, cx);
            }
            _ => {}
        }
        // The wire's key codes: these seven from 1, then Char.
        const KEYS: [Key; 7] =
            [Key::Enter, Key::Escape, Key::Tab, Key::Up, Key::Down, Key::Left, Key::Right];
        let (code, ch) = match key {
            Key::Char(c) => (8, c),
            Key::Space => (8, ' '),
            k => (KEYS.iter().position(|&x| x == k).map_or(0, |i| i as u8 + 1), '\0'),
        };
        let wire = uiwire::Key::from_u8(code).filter(|_| chord || code <= 2);
        let Some(wire) = wire else { return false };
        let bits = [mods.meta, mods.alt, mods.ctrl, mods.shift];
        let mods = bits.iter().fold(0, |m, &b| m << 1 | u8::from(b));
        self.send(Event::Key { id, key: wire, mods, ch }, cx);
        key == Key::Escape && mem::take(&mut self.texts.focus) != 0
    }

    /// Typed or pasted text for the focused Input, Code or Area, unless it
    /// echoes the Enter or Tab just handled.
    fn type_text(&mut self, s: &str, last: Option<Key>, cx: &mut Cx<'_>) -> bool {
        let echo = matches!((last, s), (Some(Key::Enter), "\n" | "\r\n") | (Some(Key::Tab), "\t"));
        let (id, t) = (self.texts.focus, &mut self.texts);
        let area = t.areas.iter_mut().find(|a| a.0 == id).map(|a| !echo && a.1.insert(s));
        let edited = match t.codes.iter_mut().find(|c| c.0 == id) {
            Some(c) => !echo && c.1.insert(s),
            None if area.is_some() => area == Some(true),
            None => t.inputs.iter_mut().find(|i| i.0 == id).is_some_and(|i| {
                let n = i.1.len();
                i.1.extend(s.chars().filter(|c| !c.is_control()));
                i.1.truncate(if i.1.len() > ui::CODE_MAX { n } else { i.1.len() });
                i.2 = i.2.wrapping_add(u32::from(i.1.len() != n));
                i.1.len() != n
            }),
        };
        edited && self.changed(id, cx)
    }

    /// A press focuses the Input, Code or Area under it (a Code or Area takes the caret there),
    /// or none.
    fn press(&mut self, x: f32, y: f32, id: Option<WidgetId>) -> bool {
        let (t, id) = (&mut self.texts, id.map_or(0, |w| w.0));
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        if let Some(c) = t.codes.iter_mut().find(|c| c.0 == id && id != 0) {
            c.1.click(x, y);
        }
        if let Some(a) = t.areas.iter_mut().find(|a| a.0 == id && id != 0) {
            a.1.click(x, y);
        }
        let focus = if id != 0 && t.has(id) { id } else { 0 };
        mem::replace(&mut t.focus, focus) != focus || focus != 0
    }

    /// The wheel scrolls the Code under it, else the window.
    fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        if let Some(c) = self.texts.codes.iter_mut().find(|c| c.1.contains(x, y)) {
            return c.1.wheel(dy);
        }
        let v = &mut self.view;
        let s = (v.scroll + dy).min(v.heights.0 - v.heights.1).max(0.0);
        dy.is_finite() && mem::replace(&mut v.scroll, s) != s
    }

    /// Output (the last kept for a failure) and the exit: clean closes, failed says why.
    fn io(&mut self, cx: &mut Cx<'_>) -> bool {
        let Some(pid) = self.pid else { return false };
        let out = cx.kernel.take_output(pid);
        if !out.is_empty() {
            self.log = String::from_utf8_lossy(&out).into_owned();
        }
        let Some(status) = cx.kernel.reap(pid) else { return false };
        (self.pid, _) = (None, self.ai.ask(pid, Request::Close));
        if status == 0 {
            cx.close_self();
            return false;
        }
        self.note = [&self.argv[0], " stopped with status "].concat();
        ui::push_num(&mut self.note, status as u32 as usize);
        self.frame = None;
        true
    }

    /// Takes `frame` as the window's tree, keeping the text the user edits.
    fn take(&mut self, mut frame: Frame, cx: &mut Cx<'_>) {
        let mut old = mem::take(&mut self.texts);
        each(&frame.nodes, &mut |n| match n {
            Node::Input { id, value, .. } if *id != 0 => {
                let i = old.inputs.iter().position(|i| i.0 == *id).map(|i| old.inputs.remove(i));
                let mut i = i.unwrap_or((*id, String::new(), 0));
                if old.focus != *id {
                    i.1.clone_from(value);
                }
                self.texts.inputs.push(i);
            }
            Node::Code { id, version, text, spans, .. } if *id != 0 => {
                let c = old.codes.iter().position(|c| c.0 == *id).map(|c| old.codes.remove(c).1);
                let mut c = c.unwrap_or_else(|| Code::new(text, *version));
                if *version > c.version {
                    c.set_text(text, *version);
                }
                if *version == c.version {
                    let s: Vec<_> = spans.iter().map(|s| (s.start, s.len, s.class as u8)).collect();
                    c.set_spans(&s);
                }
                self.texts.codes.push((*id, c));
            }
            Node::Area { id, value, .. } if *id != 0 => {
                let a = old.areas.iter().position(|a| a.0 == *id).map(|a| old.areas.remove(a));
                let mut a = a.unwrap_or_else(|| (*id, Area::new(value)));
                if old.focus != *id {
                    a.1.set(value);
                }
                self.texts.areas.push(a);
            }
            _ => {}
        });
        if self.texts.has(old.focus) {
            self.texts.focus = old.focus;
        }
        for r in mem::take(&mut frame.requests) {
            match r {
                Request::Open { name } => cx.open(&name),
                Request::Close => cx.close_self(),
                Request::Size { w, h } if self.frame.is_none() => cx.set_size(w, h),
                Request::Size { .. } => {}
                Request::Focus { id } if self.texts.has(id) => self.texts.focus = id,
                Request::Focus { .. } => {}
                Request::Feedback { kind, text, context } => cx.feedback(&kind, &text, context),
                r => self.ai.ask(self.pid.unwrap_or_default(), r),
            }
        }
        (self.frame, self.waiting) = (Some(frame), false);
        self.flush(cx);
    }
}

/// `f` for every node of `nodes` and their children, in pre-order.
fn each(nodes: &[Node], f: &mut dyn FnMut(&Node)) {
    for n in nodes {
        f(n);
        each(n.children(), f);
    }
}

impl App for Remote {
    fn title(&self) -> String {
        self.frame.iter().map(|f| &f.title).find(|t| !t.is_empty()).unwrap_or(&self.title).clone()
    }

    fn wants_text_input(&self) -> bool {
        self.texts.focus != 0
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        self.size
    }

    fn icon(&self) -> AppIcon {
        self.icon
    }

    fn compact(&self) -> bool {
        self.compact
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let key = if let AppEvent::Key { key, .. } = ev { Some(key) } else { None };
        let last = mem::replace(&mut self.last_key, key);
        match ev {
            AppEvent::Resized { w, h } => {
                if self.pid.is_none() && self.note.is_empty() && !self.closed {
                    self.start(cx);
                }
                // `as` saturates, and NaN is 0. The AI settings follow the first.
                let size = (w as u16, h as u16);
                let first = self.told.is_none();
                if self.told.replace(size) != Some(size) {
                    self.post(Event::Resize { w: size.0, h: size.1 }, cx);
                }
                if let Some(pid) = self.pid.filter(|_| first) {
                    self.post(self.ai.hello(pid), cx);
                    mem::take(&mut self.asks)
                        .into_iter()
                        .for_each(|text| self.post(Event::Ask { text }, cx));
                }
                true
            }
            // A prompt waits for the program's first size, which starts it.
            AppEvent::Ask(text) if self.told.is_none() => {
                self.asks.push(text);
                false
            }
            AppEvent::Ask(text) => self.send(Event::Ask { text }, cx),
            AppEvent::Click(WidgetId(id)) if id != 0 => self.send(Event::Click { id }, cx),
            AppEvent::PointerDown { x, y, id } => self.press(x, y, id),
            AppEvent::Key { key, mods } => self.key(key, mods, cx),
            AppEvent::Text(s) => self.type_text(&s, last, cx),
            AppEvent::Wheel { x, y, dy } => self.wheel(x, y, dy),
            AppEvent::Io => self.io(cx),
            AppEvent::Focus(on) => {
                self.send(Event::Focus { on }, cx);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, pid: u32, frame: &[u8], cx: &mut Cx<'_>) -> bool {
        if self.pid != Some(pid) {
            return false;
        }
        match Frame::decode(frame) {
            Some(f) => self.take(f, cx),
            // A first frame this desktop cannot read: the program is newer (a stale tab).
            None if self.frame.is_none() => self.note = NEWER.into(),
            None => return false,
        }
        true
    }

    fn closing(&mut self, cx: &mut Cx<'_>) {
        if !mem::replace(&mut self.closed, true) {
            self.post(Event::Close, cx);
            self.ai.ask(self.pid.unwrap_or_default(), Request::Close);
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        self.origin = (r.x, r.y);
        let Some(frame) = &self.frame else {
            let t = ui.theme();
            if self.note.is_empty() {
                ui.small(&["Starting ", &self.title, "\u{2026}"].concat());
            } else {
                ui.subheading(&self.note);
                ui.wrapped(&self.log, t.mono().with_color(t.text_dim));
            }
            return;
        };
        uiview::draw(ui, &frame.nodes, &mut self.texts, &mut self.view);
    }
}

#[cfg(test)]
mod tests;
