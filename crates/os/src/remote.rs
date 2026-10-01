//! Remote: the window of a GUI program, a kernel process that describes it as a [`uiwire`] tree,
//! drawn here with `ui` in the frame's theme; the window's input goes back as uiwire events. The
//! program starts at the first size ([`Event::Resize`]) with the roots `/`; until its first frame
//! the window shows a note, or why it failed. A frame's title is the window's and its requests are
//! honored (Size in the first only); a clean exit closes the window, and closing it sends
//! [`Event::Close`]. Edited text is owned as uiwire says, one [`Event::Change`] out at a time: the
//! next waits for a frame, or goes before any other event. Nodes stack [`PAD`] inside the content
//! rect, the wheel scrolling what does not fit.

use std::mem;

use gfx::RectF;
use ui::kernel::{Spawn, wire::Stdout};
use ui::{App, AppEvent, AppIcon, BUTTON_H, CARD_PAD, Code, Cx, FIELD_H, Key, Mods, PAD};
use ui::{RADIUS_SM, Rgba, SPACING, Sense, TextStyle, TextSystem, Theme, Ui, WidgetId};
use uiwire::{Event, Frame, Node, Request, Style, Variant};
use vfs::Vfs;

use crate::ai::Ai;

/// The Studio and Assistant programs, and the file `"studio"` edits.
pub const STUDIO: &str = "/bin/studio";
pub const ASSISTANT: &str = "/bin/assistant";
pub const DEFAULT_FILE: &str = "/apps/counter.app";
/// Studio's icon (braces on violet), and that of every `.app` it runs.
pub const STUDIO_ICON: AppIcon = AppIcon { glyph: "{ }", hue: Rgba::hex(0x8b7bff) };
pub const APP_ICON: AppIcon = AppIcon { glyph: "<>", hue: Rgba::hex(0xf59e0b) };
pub const ASSISTANT_ICON: AppIcon = AppIcon { glyph: "AI", hue: Rgba::hex(0xa78bfa) };
/// Studio's sample apps, from its `samples/`, as `(path, source)`.
pub const SAMPLES: [(&str, &str); 3] = [
    (DEFAULT_FILE, include_str!("../../studio/samples/counter.app")),
    ("/apps/greeter.app", include_str!("../../studio/samples/greeter.app")),
    ("/apps/clicker.app", include_str!("../../studio/samples/clicker.app")),
];
/// An Item's height.
const ITEM_H: f32 = 36.0;

/// The app for a window name: the Assistant for `"assistant"`, Studio editing [`DEFAULT_FILE`] for
/// `"studio"` or `<path>` for `"studio:<path>"`, or running a `.app` path (relative: in `/apps`).
pub fn open(name: &str, ai: &Ai) -> Option<Box<dyn App>> {
    let abs = |p: &str| Vfs::normalize("/apps", p).ok().filter(|_| !p.is_empty());
    if name == "assistant" {
        let mut r = Remote::new(ASSISTANT, vec![name.into()], ai);
        (r.title, r.icon, r.size) = ("Assistant".into(), ASSISTANT_ICON, Some((560.0, 600.0)));
        r.follow = true;
        return Some(Box::new(r));
    }
    let (edit, path) = match name.strip_prefix("studio:") {
        Some(p) => (true, abs(p)?),
        None if name == "studio" => (true, DEFAULT_FILE.to_string()),
        None if name.ends_with(".app") => (false, abs(name)?),
        None => return None,
    };
    let file = path.rsplit('/').next().unwrap_or_default().to_string();
    let mode = if edit { "edit" } else { "run" };
    let mut r = Remote::new(STUDIO, ["studio", mode, &path].map(String::from).into(), ai);
    (r.title, r.icon, r.size) = match edit {
        true => (["Studio \u{2014} ", &file].concat(), STUDIO_ICON, Some((760.0, 540.0))),
        false => (file, APP_ICON, None),
    };
    Some(Box::new(r))
}

/// The text the host owns: each Input's id, text and version, each Code by
/// id, and the focused one (0 for none).
#[derive(Debug, Default)]
struct Texts {
    inputs: Vec<(u32, String, u32)>,
    codes: Vec<(u32, Code)>,
    focus: u32,
}

/// A GUI program in a window: see the module docs.
#[derive(Default)]
pub struct Remote {
    /// The title until a frame names one, the icon, the preferred size, and whether a view at
    /// the bottom stays there as the content grows (the Assistant's transcript).
    pub title: String,
    pub icon: AppIcon,
    pub size: Option<(f32, f32)>,
    pub follow: bool,
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
    /// As last drawn: pixels scrolled, content and view height, content corner.
    scroll: f32,
    heights: (f32, f32),
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
            if let Some((version, text)) = code.or_else(input) {
                self.post(Event::Change { id, version, text }, cx);
                self.waiting = true;
            }
        }
    }

    /// A key: an edit in the focused Input or Code, else a Key event for
    /// Escape (which also leaves the editor), Enter and chords.
    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        let (chord, id) = (mods.ctrl || mods.alt || mods.meta, self.texts.focus);
        let plain = !chord && key != Key::Escape;
        let edited = self.texts.codes.iter_mut().find(|c| c.0 == id && plain);
        let edited = edited.and_then(|c| c.1.key(key));
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

    /// Typed or pasted text for the focused Input or Code, unless it echoes
    /// the Enter or Tab just handled.
    fn type_text(&mut self, s: &str, last: Option<Key>, cx: &mut Cx<'_>) -> bool {
        let echo = matches!((last, s), (Some(Key::Enter), "\n" | "\r\n") | (Some(Key::Tab), "\t"));
        let (id, t) = (self.texts.focus, &mut self.texts);
        let edited = match t.codes.iter_mut().find(|c| c.0 == id) {
            Some(c) => !echo && c.1.insert(s),
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

    /// A press focuses the Input or Code under it (a Code takes the caret there), or none.
    fn press(&mut self, x: f32, y: f32, id: Option<WidgetId>) -> bool {
        let (t, id) = (&mut self.texts, id.map_or(0, |w| w.0));
        let code = t.codes.iter_mut().find(|c| c.0 == id && id != 0);
        let focus =
            if code.is_some() || id != 0 && t.inputs.iter().any(|i| i.0 == id) { id } else { 0 };
        if let Some(c) = code {
            c.1.click(x + self.origin.0, y + self.origin.1);
        }
        mem::replace(&mut t.focus, focus) != focus || focus != 0
    }

    /// The wheel scrolls the Code under it, else the window.
    fn wheel(&mut self, x: f32, y: f32, dy: f32) -> bool {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        if let Some(c) = self.texts.codes.iter_mut().find(|c| c.1.contains(x, y)) {
            return c.1.wheel(dy);
        }
        let s = (self.scroll + dy).min(self.heights.0 - self.heights.1).max(0.0);
        dy.is_finite() && mem::replace(&mut self.scroll, s) != s
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
            _ => {}
        });
        let t = &mut self.texts;
        if t.inputs.iter().any(|i| i.0 == old.focus) || t.codes.iter().any(|c| c.0 == old.focus) {
            t.focus = old.focus;
        }
        for r in mem::take(&mut frame.requests) {
            match r {
                Request::Open { name } => cx.open(&name),
                Request::Close => cx.close_self(),
                Request::Size { w, h } if self.frame.is_none() => cx.set_size(w, h),
                Request::Size { .. } => {}
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
                }
                true
            }
            AppEvent::Click(WidgetId(id)) if id != 0 => self.send(Event::Click { id }, cx),
            AppEvent::PointerDown { x, y, id } => self.press(x, y, id),
            AppEvent::Key { key, mods } => self.key(key, mods, cx),
            AppEvent::Text(s) => self.type_text(&s, last, cx),
            AppEvent::Wheel { x, y, dy } => self.wheel(x, y, dy),
            AppEvent::Io => self.io(cx),
            AppEvent::Focus(_) => true,
            _ => false,
        }
    }

    fn frame(&mut self, pid: u32, frame: &[u8], cx: &mut Cx<'_>) -> bool {
        let frame = Frame::decode(frame).filter(|_| self.pid == Some(pid));
        frame.map(|f| self.take(f, cx)).is_some()
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
        let (w, view, texts) = ((r.w - 2.0 * PAD).max(0.0), r.h - 2.0 * PAD, &mut self.texts);
        let mut lay = Lay { t: ui.theme(), texts, sizes: Vec::new(), extra: 0.0, fills: 0, i: 0 };
        let ts = ui.text_system();
        let mut h = lay.stack(ts, &frame.nodes, w, SPACING, None);
        if lay.fills > 0 && h < view {
            (lay.extra, lay.sizes) = ((view - h) / lay.fills as f32, Vec::new());
            h = lay.stack(ts, &frame.nodes, w, SPACING, None);
        }
        let end = self.follow && self.scroll >= self.heights.0 - self.heights.1;
        self.heights = (h + 2.0 * PAD, r.h);
        self.scroll = if end { f32::MAX } else { self.scroll }.min(self.heights.0 - r.h).max(0.0);
        let (x, y) = (r.x + PAD, r.y + PAD - self.scroll);
        lay.draw_stack(ui, &frame.nodes, (x, y), SPACING);
    }
}

/// How a Text of `style` is set.
fn style_of(style: Style, t: &Theme) -> TextStyle {
    match style {
        Style::Body => t.body(),
        Style::Title => t.title(),
        Style::Heading => t.heading(),
        Style::Subheading => t.subheading(),
        Style::Small => t.small(),
        Style::Mono => t.mono(),
        Style::Dim => t.body().with_color(t.text_dim),
        Style::Error => t.body().with_color(t.danger),
        Style::Success => t.body().with_color(t.ansi[2]),
    }
}

/// One frame's layout in theme `t` over the host's text: each node's size in
/// pre-order, the height each Fill adds, the Fill count, the next size to draw.
struct Lay<'t> {
    t: &'t Theme,
    texts: &'t mut Texts,
    sizes: Vec<(f32, f32)>,
    extra: f32,
    fills: usize,
    i: usize,
}

impl Lay<'_> {
    /// Measures `ns` stacked `gap` apart in width `w`; their height. Each Code
    /// takes `g` more height (in a Fill).
    fn stack(&mut self, ts: &mut TextSystem, ns: &[Node], w: f32, gap: f32, g: Option<f32>) -> f32 {
        let h = ns.iter().map(|n| self.measure(ts, n, w, g).1 + gap).sum::<f32>();
        h - if ns.is_empty() { 0.0 } else { gap }
    }

    /// Measures `n` given width `w` (Buttons and Spacers take their own).
    fn measure(&mut self, ts: &mut TextSystem, n: &Node, w: f32, grow: Option<f32>) -> (f32, f32) {
        let (at, t) = (self.sizes.len(), self.t);
        self.sizes.push((0.0, 0.0));
        let size = match n {
            Node::Col { gap, children: c, .. } => (w, self.stack(ts, c, w, f32::from(*gap), None)),
            Node::Card { children, .. } => {
                (w, self.stack(ts, children, w - 2.0 * CARD_PAD, SPACING, None) + 2.0 * CARD_PAD)
            }
            Node::Fill { children, .. } => {
                self.fills += 1;
                let codes = children.iter().filter(|c| matches!(c, Node::Code { .. })).count();
                let grow = Some(self.extra / codes.max(1) as f32);
                (w, self.stack(ts, children, w, SPACING, grow) + [self.extra, 0.0][codes.min(1)])
            }
            // Buttons and Spacers take their width, each Text its own up to
            // an even share of the rest, the others share what is left.
            Node::Row { gap, children, .. } => {
                let (gap, mut x, mut h) = (f32::from(*gap), 0.0, 0.0f32);
                let mut own: Vec<_> = children.iter().map(|c| own_width(ts, t, c)).collect();
                let gaps = gap * children.len().saturating_sub(1) as f32;
                let mut room = (w - own.iter().flatten().sum::<f32>() - gaps).max(0.0);
                let mut flex = own.iter().filter(|o| o.is_none()).count();
                let share = room / flex.max(1) as f32;
                for (c, o) in children.iter().zip(&mut own) {
                    if let Node::Text { style, text, .. } = c {
                        let style = style_of(*style, t);
                        let lines = ts.wrap(text, style, f32::MAX);
                        let one = lines.iter().map(|l| ts.measure(l, style)).fold(0.0, f32::max);
                        // A pixel more: snapping must not wrap it.
                        let one = (one.ceil() + 1.0).min(share);
                        (*o, room, flex) = (Some(one), room - one, flex - 1);
                    }
                }
                let share = room / flex.max(1) as f32;
                for (c, o) in children.iter().zip(own) {
                    let s = self.measure(ts, c, o.unwrap_or(share), None);
                    (x, h) = (x + s.0 + gap, h.max(s.1));
                }
                (x - if children.is_empty() { 0.0 } else { gap }, h)
            }
            // 3 to 12 rows as the text has lines; 3 and what is left in a Fill.
            Node::Code { id, text, .. } => {
                let n = self.texts.codes.iter().find(|c| c.0 == *id).map(|c| c.1.ed.line_count());
                let rows = grow.map_or(n.unwrap_or(text.lines().count()).clamp(3, 12), |_| 3);
                (w, rows as f32 * ts.line_height(t.mono()) + 12.0 + grow.unwrap_or(0.0))
            }
            Node::Text { style, text, .. } => {
                let style = style_of(*style, t);
                (w, ts.wrap(text, style, w).len() as f32 * ts.line_height(style))
            }
            Node::Button { label, .. } => (ui::button_width(ts, t, label), BUTTON_H),
            Node::Spacer { px } => (f32::from(*px), f32::from(*px)),
            Node::Input { .. } => (w, FIELD_H),
            Node::Item { .. } => (w, ITEM_H),
            Node::Separator => (w, 1.0),
        };
        self.sizes[at] = size;
        size
    }

    /// The next size to draw; `take` moves past it.
    fn next(&mut self, take: bool) -> (f32, f32) {
        self.i += usize::from(take);
        self.sizes.get(self.i - usize::from(take)).copied().unwrap_or_default()
    }

    /// Draws `nodes` down from `(x, y)`, `gap` apart.
    fn draw_stack(&mut self, ui: &mut Ui<'_>, nodes: &[Node], (x, mut y): (f32, f32), gap: f32) {
        for n in nodes {
            let h = self.next(false).1;
            self.draw(ui, n, x, y);
            y += h + gap;
        }
    }

    /// Draws `n` at `(x, y)` in its measured size.
    fn draw(&mut self, ui: &mut Ui<'_>, n: &Node, x: f32, y: f32) {
        let ((w, h), t) = (self.next(true), self.t);
        let r = ui.snapped(RectF::new(x, y, w, h));
        let at = |ui: &mut Ui<'_>, f: &mut dyn FnMut(&mut Ui<'_>)| ui.within(r, |ui| f(ui));
        match n {
            Node::Col { gap, children: c, .. } => self.draw_stack(ui, c, (x, y), f32::from(*gap)),
            Node::Fill { children, .. } => self.draw_stack(ui, children, (x, y), SPACING),
            Node::Card { children, .. } => {
                ui.raised(r);
                self.draw_stack(ui, children, (x + CARD_PAD, y + CARD_PAD), SPACING);
            }
            // Leaves center on the row's height.
            Node::Row { gap, children, .. } => {
                let mut cx = x;
                for c in children {
                    let (cw, ch) = self.next(false);
                    let leaf = c.children().is_empty() && !matches!(c, Node::Code { .. });
                    let cy = if leaf { y + (h - ch) / 2.0 } else { y };
                    self.draw(ui, c, cx, cy);
                    cx += cw + f32::from(*gap);
                }
            }
            Node::Text { style, text, .. } => {
                at(ui, &mut |ui| _ = ui.wrapped(text, style_of(*style, t)))
            }
            Node::Button { id, variant, label } => at(ui, &mut |ui| {
                _ = match variant {
                    Variant::Normal => ui.button(WidgetId(*id), label),
                    Variant::Primary => ui.button_primary(WidgetId(*id), label),
                    Variant::Danger => ui.button_danger(WidgetId(*id), label),
                }
            }),
            Node::Input { id, placeholder, .. } => {
                let value = self.texts.inputs.iter().find(|i| i.0 == *id).map_or("", |i| &i.1);
                let focus = self.texts.focus == *id && *id != 0;
                at(ui, &mut |ui| _ = ui.text_field(WidgetId(*id), value, focus, placeholder));
            }
            Node::Code { id, line_numbers, text, .. } => {
                let focus = self.texts.focus == *id;
                match self.texts.codes.iter_mut().find(|c| c.0 == *id) {
                    Some(c) => c.1.draw(ui, WidgetId(*id), r, *line_numbers, focus),
                    None => at(ui, &mut |ui| _ = ui.wrapped(text, t.mono())),
                }
            }
            Node::Item { id, text, detail, selected } => {
                item(ui, WidgetId(*id), r, [text, detail], *selected)
            }
            Node::Separator => ui.fill(RectF { h: ui.px(1.0), ..r }, 0.0, t.border),
            Node::Spacer { .. } => {}
        }
    }
}

/// The width a Button or Spacer takes; `None` for the others.
fn own_width(ts: &mut TextSystem, t: &Theme, n: &Node) -> Option<f32> {
    match n {
        Node::Button { label, .. } => Some(ui::button_width(ts, t, label)),
        Node::Spacer { px } => Some(f32::from(*px)),
        _ => None,
    }
}

/// A list row in `r`: `text`, then `detail` dim at the right; washed under
/// the pointer, tinted when `selected`; a [`Sense::Click`] hit.
fn item(ui: &mut Ui<'_>, id: WidgetId, r: RectF, [text, detail]: [&String; 2], selected: bool) {
    let (t, s) = (ui.theme(), ui.state());
    if selected || s.hover == Some(id) {
        ui.fill(r, RADIUS_SM, if selected { t.selection } else { t.wash(s.pressed == Some(id)) });
    }
    let (body, small) = (t.body(), t.small());
    let ts = ui.text_system();
    let (base, dw) = (ts.snap(r.y + (r.h + 0.727 * body.size) / 2.0), ts.measure(detail, small));
    let right = ts.snap(r.x + r.w - 12.0 - dw);
    ui.push_clip(RectF { w: (right - r.x - 12.0).max(0.0), ..r });
    ui.text(r.x + 12.0, base, text, body);
    ui.pop_clip();
    ui.push_clip(r);
    ui.text(right, base, detail, small);
    ui.pop_clip();
    ui.hit(id, r, Sense::Click);
}

#[cfg(test)]
mod tests;
