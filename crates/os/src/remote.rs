//! Remote: the window of a GUI program, a kernel process that describes it as a [`uiwire`] tree,
//! drawn here by [`uiview`] in the frame's theme; the window's input goes back as uiwire events.
//! The program starts at the first size ([`Event::Resize`]) with the roots `/`; until its first
//! frame the window shows a note, or why it failed (a first frame that does not decode: the
//! program is newer than the desktop). A frame's title is the window's and its requests are
//! honored (Size in the first only; Focus when the frame holds that Input, Code or Area; Feedback
//! goes to the page); a clean exit closes the window, and closing it sends [`Event::Close`]. The
//! window's focus goes to the program as [`Event::Focus`], and a prompt from the everything bar
//! as [`Event::Ask`], held until it starts. Edited text is owned as uiwire says ([`Texts`]), one
//! [`Event::Change`] out at a time: the next waits for a frame, or goes before any other event.
//! The wheel scrolls the Code under it, else the Scroll, else what does not fit
//! ([`uiview::wheel`]); a revealed mark asks for frames while it comes in. The overlay's acts and
//! status go to the host ([`Cx::agent`]), which answers through [`AppEvent::Agent`] (a tap the
//! overlay makes goes to the program as it is); the window is busy while its program starts, and
//! from a click, submit, tap or plain key it was sent until it draws that: the program answers
//! each Tick and Change with a frame too, in the order it reads them, so the frames of those
//! out before it count first. A program that asked for a timer ([`Request::Timer`]) gets an
//! [`Event::Tick`] with the ms passed each time one is due and the last was answered (or a second
//! went by), and a frame then while the window shows (by the page's timer, none between); one
//! that asked for keys ([`Request::Keys`]) gets plain keys while none of its text fields has the
//! keyboard. A press on a Grid's square is an [`Event::Tap`], and so is each square the mouse
//! then drags across, once the last tap is drawn ([`uiview::Play::tap`]); the press's Click is
//! none.

use std::mem;

use ui::icon::Glyph;
use ui::kernel::{Spawn, wire::Stdout};
use ui::{App, AppEvent, AppIcon, Cx, Key, Mods, Rgba, Ui, WidgetId};
use uiview::{Play, Texts, View};
use uiwire::{Event, Frame, Request};
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
/// About, Feedback, Files and Welcome: one program, bin/system.wasm, run as the name of its /bin
/// marker.
#[rustfmt::skip]
pub const SYSTEM: [SystemApp; 4] = [
    ("about", "About", icon(Glyph::About, 0xfbbf24), (560.0, 640.0), true),
    ("feedback", "Feedback", icon(Glyph::Bug, 0x34d399), (520.0, 420.0), true),
    ("files", "Files", icon(Glyph::Folder, 0x60a5fa), (640.0, 480.0), false),
    ("welcome", "Welcome", icon(Glyph::Mark, 0xf472b6), (520.0, 768.0), true),
];
/// Studio's size: room for the app beside its prompt.
const STUDIO_SIZE: Option<(f32, f32)> = Some((880.0, 560.0));
/// Shown in place of a first frame that does not decode.
const NEWER: &str = "This program is newer than the desktop; reload the page";

const fn icon(glyph: Glyph, hue: u32) -> AppIcon {
    AppIcon { glyph, hue: Rgba::hex(hue) }
}

/// The app for a window name: About, Feedback, Files or Welcome ([`SYSTEM`]; `"files:<dir>"` is
/// Files at that folder), the Assistant for `"assistant"`, Studio with nothing open for
/// `"studio"` or on `<path>` for `"studio:<path>"`, or running a `.app` path (relative: in
/// `/apps`).
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
    /// The content's corner as last drawn, and when (page ms); the timer, keys, taps and
    /// answers (busy).
    origin: (f32, f32),
    drawn: f64,
    play: Play,
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
        if matches!(ev, Event::Click { .. } | Event::Submit { .. } | Event::Tap { .. }) {
            self.play.sent(self.waiting);
        }
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
        // Plain arrows, letters, digits and space too, for a program that asked for keys: the
        // play it answers with a frame.
        let plain = self.play.keys && id == 0 && code > 3 && !chord;
        let wire = uiwire::Key::from_u8(code).filter(|_| chord || code <= 2 || plain);
        let Some(wire) = wire else { return false };
        let bits = [mods.meta, mods.alt, mods.ctrl, mods.shift];
        let mods = bits.iter().fold(0, |m, &b| m << 1 | u8::from(b));
        self.send(Event::Key { id, key: wire, mods, ch }, cx);
        if plain {
            self.play.sent(self.waiting);
        }
        key == Key::Escape && mem::take(&mut self.texts.focus) != 0
    }

    /// The pointer pressed (`id` the hit under it), or dragged while pressed, at `(x, y)` in the
    /// window: the Grid's squares newly under it are tapped ([`Play::tap`]).
    fn tap(&mut self, id: Option<u32>, x: f32, y: f32, cx: &mut Cx<'_>) {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        let (id, cells) = self.play.tap(&self.view, id, x, y);
        for cell in cells {
            self.post(Event::Tap { id, cell }, cx);
        }
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
        // Failed: whatever it was doing for the person is over (the overlay's status says so).
        cx.agent(Request::Status { working: false });
        self.note = [&self.argv[0], " stopped with status "].concat();
        ui::push_num(&mut self.note, status as u32 as usize);
        self.frame = None;
        true
    }

    /// Takes `frame` as the window's tree, keeping the text the user edits.
    fn take(&mut self, mut frame: Frame, cx: &mut Cx<'_>) {
        self.texts.adopt(&frame.nodes, &self.dirty);
        for r in mem::take(&mut frame.requests) {
            match r {
                Request::Open { name } => cx.open(&name),
                Request::Close => cx.close_self(),
                Request::Size { w, h } if self.frame.is_none() => cx.set_size(w, h),
                Request::Size { .. } => {}
                // Focus 0 takes the keyboard from the text fields (to the program's keys).
                Request::Focus { id } if id == 0 || self.texts.has(id) => self.texts.focus = id,
                Request::Focus { .. } => {}
                Request::Feedback { kind, text, context } => cx.feedback(&kind, &text, context),
                r @ (Request::Act { .. } | Request::Status { .. }) => cx.agent(r),
                r if self.play.ask(&r) => {}
                r => self.ai.ask(self.pid.unwrap_or_default(), r),
            }
        }
        (self.frame, self.waiting) = (Some(frame), false);
        self.play.answered();
        self.flush(cx);
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

    fn frame_in(&self, now_ms: f64) -> Option<u32> {
        if self.view.animating(now_ms) {
            return Some(0);
        }
        self.pid.and(self.play.due_in(now_ms))
    }

    fn squares(&self, id: u32) -> Option<u32> {
        self.view.grids.iter().find(|b| b.id == id).map(|b| b.n)
    }

    fn busy(&self) -> bool {
        self.pid.is_some() && (self.frame.is_none() || self.play.busy())
    }

    fn ended(&self) -> bool {
        self.pid.is_none() && !self.note.is_empty()
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
            // A Grid's press was its tap, whatever now has its id.
            AppEvent::Click(WidgetId(id))
                if id != 0 && !self.play.pressed() && self.squares(id).is_none() =>
            {
                self.send(Event::Click { id }, cx)
            }
            AppEvent::PointerDown { x, y, id } => {
                self.tap(Some(id.map_or(0, |w| w.0)), x, y, cx);
                let (x, y) = (x + self.origin.0, y + self.origin.1);
                self.texts.press(id.map_or(0, |w| w.0), x, y)
            }
            AppEvent::Drag { x, y } => {
                self.tap(None, x, y, cx);
                false
            }
            AppEvent::Tick { now_ms } => {
                if let Some(tick) = self.play.tick(now_ms, self.drawn) {
                    self.send(tick, cx);
                }
                false
            }
            AppEvent::Key { key, mods } => self.key(key, mods, cx),
            AppEvent::Text(s) => self.type_text(&s, last, cx),
            AppEvent::Wheel { x, y, dy } => {
                let (x, y) = (x + self.origin.0, y + self.origin.1);
                uiview::wheel(&mut self.texts, &mut self.view, x, y, dy)
            }
            AppEvent::Io => self.io(cx),
            AppEvent::Focus(on) => {
                self.send(Event::Focus { on }, cx);
                true
            }
            AppEvent::Agent(ev) => self.send(ev, cx),
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
        (self.origin, self.drawn) = ((r.x, r.y), ui.state().now_ms);
        self.play.drew();
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
