//! AppHost: one applang program running in a window.

use applang::{Event, Limits, Node};
use gfx::RectF;
use ui::{App, AppEvent, AppIcon, BUTTON_H, Cx, FIELD_H, Key, PAD, RADIUS_SM, SPACING, Ui};
use ui::{WidgetId, theme::mix};
use vfs::Vfs;

use crate::studio::{px, snapped};
use crate::{Problem, file_name, join};

/// Inputs are numbered from here in render order; buttons keep applang's
/// ids, which count from 0.
const INPUT: u32 = 1 << 31;
/// "Edit in Studio", shown when the file cannot run.
const EDIT: WidgetId = WidgetId(INPUT - 1);
/// The fault bar: this tall, this far in from the bottom and sides.
const STATUS_H: f32 = 32.0;
const STATUS_INSET: f32 = 12.0;
/// The most one render may show, in widgets and in bytes of text. applang
/// bounds each string, not their sum, and every frame lays out all of it.
const MAX_WIDGETS: usize = 2048;
const MAX_TEXT: usize = 32 * 1024;
pub(crate) const TOO_BIG: &str =
    "renders more than 2048 widgets or 32 KiB of text; the rest is not shown";

#[derive(Debug, Default)]
pub(crate) enum Run {
    /// Not read yet.
    #[default]
    Pending,
    Live(applang::App),
    /// It could not be read or did not compile.
    Broken(Problem),
}

/// Runs one `.app` file with [`applang::App`] and [`Limits::default`],
/// compiled on its first event (or by [`AppHost::load`]). A press focuses an
/// input, whose edits go out as `Event::Input`s; a fault shows in a bar at
/// the bottom over the rolled-back state; a render past 2,048 widgets or
/// 32 KiB of text draws only what fits; a file that cannot run shows its
/// problem and a button to open it in Studio.
#[derive(Debug, Default)]
pub struct AppHost {
    path: String,
    src: String,
    pub(crate) run: Run,
    pub(crate) nodes: Vec<Node>,
    /// The offending source line and carets of a compile error.
    pub(crate) snippet: String,
    pub(crate) fault: Option<Problem>,
    /// The state name of the focused input.
    focus: Option<String>,
    scroll: f32,
    /// The content's and the window's height, from the last frame.
    content_h: f32,
    view_h: f32,
}

/// Each input's state and value, in render order.
fn inputs<'n>(nodes: &'n [Node], out: &mut Vec<(&'n str, &'n str)>) {
    for n in nodes {
        match n {
            Node::Input { state, value } => out.push((state, value)),
            Node::Row { children } | Node::Col { children } => inputs(children, out),
            Node::Label { .. } | Node::Button { .. } => {}
        }
    }
}

/// The value of input `state`, if one is drawn.
fn input_value<'n>(nodes: &'n [Node], state: &str) -> Option<&'n str> {
    let mut all = Vec::new();
    inputs(nodes, &mut all);
    all.into_iter().find(|(s, _)| *s == state).map(|(_, v)| v)
}

/// Keeps the widgets of `nodes`, in render order, while they fit in
/// `left` (widgets, text bytes); returns whether all of them did.
fn fit(nodes: &mut Vec<Node>, left: &mut (usize, usize)) -> bool {
    for i in 0..nodes.len() {
        let text = match &nodes[i] {
            Node::Label { text } | Node::Button { text, .. } => text.len(),
            Node::Input { state, value } => state.len() + value.len(),
            Node::Row { .. } | Node::Col { .. } => 0,
        };
        if left.0 == 0 || text > left.1 {
            nodes.truncate(i);
            return false;
        }
        *left = (left.0 - 1, left.1 - text);
        if let Node::Row { children } | Node::Col { children } = &mut nodes[i] {
            if !fit(children, left) {
                nodes.truncate(i + 1);
                return false;
            }
        }
    }
    true
}

fn union(a: Option<RectF>, b: RectF) -> RectF {
    let Some(a) = a else { return b };
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    let (right, bottom) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
    RectF::new(x, y, right - x, bottom - y)
}

/// The focused input's state, and how many inputs are drawn so far.
type Focus<'f> = (Option<&'f str>, u32);

/// Draws one node, in a row whose controls are `Some(band)` tall or not in
/// a row; returns the rect around everything it drew.
fn node(ui: &mut Ui<'_>, n: &Node, f: &mut Focus<'_>, in_row: Option<f32>) -> RectF {
    match n {
        // Beside controls, a label centers on their height.
        Node::Label { text } => match in_row {
            Some(band) if band > 0.0 => {
                let ((x, y), body, r) = (ui.cursor(), ui.theme().body(), ui.rect());
                let ts = ui.text_system();
                let lines = ts.wrap(text, body, r.x + r.w - PAD - x).len() as f32;
                let dy = ts.snap(((band - lines * ts.line_height(body)) / 2.0).max(0.0));
                ui.set_cursor(x, y + dy);
                let got = ui.label(text);
                ui.set_cursor(ui.cursor().0, y);
                got
            }
            _ => ui.label(text),
        },
        Node::Button { text, id } => ui.button(WidgetId(*id), text),
        Node::Input { state, value } => {
            let id = WidgetId(INPUT + f.1);
            f.1 += 1;
            ui.text_field(id, value, f.0 == Some(state.as_str()), state)
        }
        Node::Row { children } => {
            // The tallest control directly in the row; 0 for none.
            let h = |c: &Node| match c {
                Node::Button { .. } => BUTTON_H,
                Node::Input { .. } => FIELD_H,
                _ => 0.0,
            };
            let (mut b, band) = (None, children.iter().map(h).fold(0.0, f32::max));
            let r = ui.row(|ui| {
                for c in children {
                    b = Some(union(b, node(ui, c, f, Some(band))));
                }
            });
            union(b, r)
        }
        Node::Col { children } => {
            let (x, y) = ui.cursor();
            col(ui, children, f, in_row.is_some()).unwrap_or(RectF::new(x, y, 0.0, 0.0))
        }
    }
}

/// Stacks `children` top to bottom from the cursor, then leaves the cursor
/// right of them (in a row) or below them.
fn col(ui: &mut Ui<'_>, children: &[Node], f: &mut Focus<'_>, in_row: bool) -> Option<RectF> {
    let (x0, y0) = ui.cursor();
    let mut b: Option<RectF> = None;
    for c in children {
        ui.set_cursor(x0, b.map_or(y0, |b| b.y + b.h + SPACING));
        b = Some(union(b, node(ui, c, f, None)));
    }
    match (b, in_row) {
        (Some(b), true) => ui.set_cursor(b.x + b.w + SPACING, y0),
        (Some(b), false) => ui.set_cursor(x0, b.y + b.h + SPACING),
        (None, _) => ui.set_cursor(x0, y0),
    }
    b
}

impl AppHost {
    /// A host for the file at `path`, not read yet.
    pub fn new(path: &str) -> AppHost {
        AppHost { path: path.to_string(), ..AppHost::default() }
    }

    /// Reads and compiles the file and renders it once.
    pub fn load(&mut self, vfs: &Vfs) {
        (self.nodes, self.fault, self.focus) = (Vec::new(), None, None);
        let src = match vfs.read(&self.path) {
            Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Err(e) => {
                let message = join(&["cannot read ", &self.path, ": ", &e.to_string()]);
                self.run = Run::Broken(Problem::plain(message));
                return;
            }
        };
        match applang::compile(&src) {
            Ok(program) => {
                self.run = Run::Live(applang::App::new(program, Limits::default()));
                self.src = src;
                self.render();
            }
            Err(d) => {
                self.run = Run::Broken(Problem::new(&d, &src));
                let snip = d.span.and_then(|s| lang::diag::render_snippet(&src, s));
                let snip = snip.unwrap_or_default();
                self.snippet = snip.split_once('\n').map_or("", |(_, s)| s).to_string();
                self.src = src;
            }
        }
    }

    fn render(&mut self) {
        let Run::Live(app) = &self.run else { return };
        match app.render() {
            Ok(mut nodes) => {
                if !fit(&mut nodes, &mut (MAX_WIDGETS, MAX_TEXT)) {
                    self.fault = Some(Problem::plain(TOO_BIG.to_string()));
                }
                self.nodes = nodes;
            }
            Err(d) => {
                self.nodes.clear();
                self.fault = Some(Problem::new(&d, &self.src));
            }
        }
        if self.focus.as_deref().is_some_and(|s| input_value(&self.nodes, s).is_none()) {
            self.focus = None;
        }
    }

    fn fire(&mut self, ev: &Event) {
        if let Run::Live(app) = &mut self.run {
            self.fault = app.handle(ev).err().map(|d| Problem::new(&d, &self.src));
            self.render();
        }
    }

    /// Appends `typed` (control chars dropped) to the focused input's
    /// value, or with `None` deletes its last char, and sends the result.
    fn edit(&mut self, typed: Option<&str>) -> bool {
        let Some(state) = self.focus.clone() else {
            return false;
        };
        let old = input_value(&self.nodes, &state).unwrap_or("");
        let mut text = old.to_string();
        match typed {
            Some(t) => text.extend(t.chars().filter(|c| !c.is_control())),
            None => _ = text.pop(),
        }
        if text == old {
            return false;
        }
        self.fire(&Event::Input { state, text });
        true
    }
}

impl App for AppHost {
    fn title(&self) -> String {
        file_name(&self.path).to_string()
    }

    fn wants_text_input(&self) -> bool {
        self.focus.is_some()
    }

    fn icon(&self) -> AppIcon {
        crate::APP_ICON
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let fresh = matches!(self.run, Run::Pending);
        if fresh {
            self.load(cx.vfs);
        }
        let redraw = match ev {
            AppEvent::Click(EDIT) if matches!(self.run, Run::Broken(_)) => {
                cx.open(&join(&["studio:", &self.path]));
                false
            }
            AppEvent::Click(WidgetId(id)) if id < EDIT.0 => {
                self.fire(&Event::Click { id });
                true
            }
            AppEvent::PointerDown { id, .. } => {
                let mut all = Vec::new();
                inputs(&self.nodes, &mut all);
                let i = id.and_then(|w| w.0.checked_sub(INPUT));
                let focus = i.and_then(|i| all.get(i as usize)).map(|(s, _)| s.to_string());
                std::mem::replace(&mut self.focus, focus.clone()) != focus
            }
            AppEvent::Text(t) => self.edit(Some(&t)),
            AppEvent::Key { key: Key::Backspace, .. } => self.edit(None),
            AppEvent::Key { key: Key::Escape, .. } => self.focus.take().is_some(),
            AppEvent::Wheel { dy, .. } => {
                let max = (self.content_h - self.view_h).max(0.0);
                let s = (self.scroll + dy).max(0.0).min(max);
                dy.is_finite() && std::mem::replace(&mut self.scroll, s) != s
            }
            AppEvent::Resized { .. } | AppEvent::Focus(_) => true,
            _ => false,
        };
        redraw || fresh
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        let bar = STATUS_H + 2.0 * STATUS_INSET - PAD;
        self.view_h = r.h - if self.fault.is_some() { bar } else { 0.0 };
        match &self.run {
            Run::Pending => _ = ui.small(&join(&["Loading ", &self.path, "…"])),
            Run::Broken(p) => {
                ui.heading(&join(&[file_name(&self.path), " cannot run"]));
                ui.wrapped(&p.line(), t.mono().with_color(t.danger));
                if !self.snippet.is_empty() {
                    ui.wrapped(&self.snippet, t.mono().with_color(t.text_dim));
                }
                ui.space(SPACING);
                ui.button_primary(EDIT, "Edit in Studio");
            }
            Run::Live(_) => {
                self.scroll = self.scroll.min((self.content_h - self.view_h).max(0.0));
                let (x, y) = ui.cursor();
                ui.set_cursor(x, y - self.scroll);
                let b = col(ui, &self.nodes, &mut (self.focus.as_deref(), 0), false);
                let bottom = b.map_or(y - self.scroll, |b| b.y + b.h);
                self.content_h = bottom + self.scroll - r.y + PAD;
            }
        }
        let Some(fault) = &self.fault else { return };
        // The fault bar: the code in the danger color, then the position
        // and the message, cut to fit.
        let (line, inset) = (px(ui, 1.0), STATUS_INSET);
        let at = RectF::new(r.x + inset, r.y + r.h - inset - STATUS_H, r.w - 2.0 * inset, STATUS_H);
        let bar = snapped(ui, at);
        ui.fill(bar, RADIUS_SM, mix(t.surface_hi, t.danger, 0.12));
        ui.border(bar, RADIUS_SM, line, t.danger.with_alpha(110));
        let (code, pos) = fault.head();
        let style = t.small().with_color(t.text);
        let ts = ui.text_system();
        let (a, d) = (ts.ascent(style), ts.descent(style));
        let base = ts.snap(bar.y + (bar.h - a - d) / 2.0 + a);
        let rest = join(&[&pos, if pos.is_empty() { "" } else { "  " }, &fault.message]);
        let x = bar.x + 12.0;
        ui.push_clip(bar);
        let w = ui.text(x, base, &code, style.with_color(t.danger)) + 10.0;
        let room = bar.x + bar.w - 12.0 - (x + w);
        let shown = ui.text_system().ellipsize(&rest, style, room);
        ui.text(x + w, base, &shown, style);
        ui.pop_clip();
    }
}
