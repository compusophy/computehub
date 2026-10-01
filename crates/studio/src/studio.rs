//! The Studio window: toolbar, editor and problem list.

use gfx::{RectF, Rgba};
use ui::{App, AppEvent, AppIcon, BUTTON_H, Cx, Key, Mods, PAD, RADIUS_SM, SPACING, Sense};
use ui::{Theme, Ui, WidgetId, theme::mix};
use vfs::Vfs;

use crate::{Editor, NEW_APP, Problem, SAMPLES, file_name, join, push_num};

const RUN: WidgetId = WidgetId(1);
const SAVE: WidgetId = WidgetId(2);
const NEW: WidgetId = WidgetId(3);
const EDITOR: WidgetId = WidgetId(4);
/// The first problem row; row `i` is `PROBLEM + i`.
const PROBLEM: u32 = 100;
const MAX_PROBLEMS: usize = 5;
const PROBLEM_H: f32 = 24.0;

/// The least room for the toolbar's note beside the buttons; with less it
/// goes on its own line under them.
const NOTE_MIN: f32 = 80.0;
/// Space between the editor's well and its text.
const INSET: f32 = 6.0;
const KEYWORDS: [&str; 12] = [
    "state", "label", "button", "input", "row", "col", "let", "if", "else", "repeat", "true",
    "false",
];

/// Where the last frame put the text, for clicks and scrolling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geo {
    /// The text area, relative to the corner of the [`Ui`] rect (the space
    /// of [`AppEvent::PointerDown`]): line `top`, column `left` starts at
    /// its corner.
    pub(crate) text: RectF,
    pub(crate) cell: f32,
    pub(crate) row: f32,
    /// Whole rows and columns that fit (at least one each).
    pub(crate) rows: usize,
    cols: usize,
}

/// The applang editor. Run compiles the text: on success it saves and asks
/// the shell to open the file (an [`crate::AppHost`] runs it); on failure
/// it lists the problems, and clicking one moves the caret there.
///
/// It draws in the frame's theme: a sunken well, the current line raised,
/// faint line numbers, keywords in the accent, strings and numbers in the
/// theme's terminal green and yellow, comments faint.
///
/// Keys: Enter keeps indentation, Tab inserts two spaces, Backspace,
/// Delete, arrows, Home, End, PageUp and PageDown edit and move; Ctrl+S (or
/// Cmd+S) saves and Ctrl+Enter runs. The wheel scrolls, taking the caret
/// along so it stays in view.
#[derive(Debug, Default)]
pub struct Studio {
    pub(crate) path: String,
    pub(crate) ed: Editor,
    loaded: bool,
    dirty: bool,
    pub(crate) problems: Vec<Problem>,
    status: String,
    /// The first line and column in view.
    pub(crate) top: usize,
    left: usize,
    /// Wheel pixels not yet a whole row.
    wheel: f32,
    /// The key of the event just before this one, to drop the `\n` or `\t`
    /// text a browser may send after Enter or Tab.
    last_key: Option<Key>,
    /// None until the first frame.
    pub(crate) geo: Option<Geo>,
}

impl Studio {
    /// Studio on `path`, not read yet: it loads on its first event or on
    /// [`Studio::load`].
    pub fn new(path: &str) -> Studio {
        let path = path.to_string();
        Studio { path, ..Studio::default() }
    }

    /// Reads the file into the editor. A missing sample starts as the
    /// sample; any other missing file starts empty.
    pub fn load(&mut self, vfs: &Vfs) {
        let text = match vfs.read(&self.path) {
            Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Err(_) => {
                self.status = "new file".to_string();
                let sample = SAMPLES.iter().find(|(p, _)| *p == self.path);
                sample.map_or("", |(_, src)| src).to_string()
            }
        };
        self.ed = Editor::new(&text);
        (self.loaded, self.dirty, self.top, self.left) = (true, false, 0, 0);
    }

    /// Writes the file (making its directory); it stays dirty on failure.
    fn save(&mut self, cx: &mut Cx<'_>) {
        if let Some((dir, _)) = self.path.rsplit_once('/').filter(|(d, _)| !d.is_empty()) {
            let _ = cx.vfs.mkdir_all(dir);
        }
        let saved = cx.vfs.write(&self.path, self.ed.text().as_bytes());
        self.dirty = saved.is_err();
        self.status = match saved {
            Ok(()) => "saved".to_string(),
            Err(e) => join(&["save failed: ", &e.to_string()]),
        };
    }

    fn run(&mut self, cx: &mut Cx<'_>) {
        let src = self.ed.text();
        if let Err(d) = applang::compile(&src) {
            self.problems = vec![Problem::new(&d, &src)];
            self.status = "did not compile".to_string();
            return;
        }
        self.problems.clear();
        self.save(cx);
        if !self.dirty {
            cx.open(&self.path);
            self.status = "saved and running".to_string();
        }
    }

    /// Writes a starter app to the first free `/apps/untitled*.app` and
    /// opens Studio on it in a new window.
    fn new_file(&mut self, cx: &mut Cx<'_>) {
        let name = |n| {
            let mut path = String::from("/apps/untitled");
            if n > 1 {
                path.push('-');
                push_num(&mut path, n, 1);
            }
            path.push_str(".app");
            path
        };
        let Some(path) = (1..100).map(name).find(|p| !cx.vfs.exists(p)) else {
            self.status = "too many untitled apps".to_string();
            return;
        };
        let _ = cx.vfs.mkdir_all("/apps");
        self.status = match cx.vfs.write(&path, NEW_APP.as_bytes()) {
            Ok(()) => {
                cx.open(&join(&["studio:", &path]));
                join(&["created ", &path])
            }
            Err(e) => join(&["new failed: ", &e.to_string()]),
        };
    }

    /// Scrolls so the caret is in view; each frame does this.
    pub(crate) fn reveal(&mut self) {
        let Some(Geo { rows, cols, .. }) = self.geo else {
            return;
        };
        let (l, c) = self.ed.caret();
        self.top = self.top.clamp((l + 1).saturating_sub(rows), l);
        self.left = self.left.clamp((c + 1).saturating_sub(cols), c);
    }

    /// The wheel: whole rows, the caret kept on a row in view.
    fn scroll(&mut self, dy: f32) -> bool {
        let Some(g) = self.geo.filter(|_| dy.is_finite()) else {
            return false;
        };
        self.wheel += dy;
        let steps = (self.wheel / g.row).trunc();
        self.wheel -= steps * g.row;
        let max = self.ed.line_count().saturating_sub(g.rows) as f32;
        let top = (self.top as f32 + steps).max(0.0).min(max) as usize;
        if top == self.top {
            return false;
        }
        let line = self.ed.caret().0;
        self.top = top;
        self.ed.move_to_line(line.clamp(top, top + g.rows - 1));
        true
    }

    fn key(&mut self, key: Key, mods: Mods, cx: &mut Cx<'_>) -> bool {
        let cmd = mods.ctrl || mods.meta;
        let page = self.geo.map_or(20, |g| g.rows);
        let line = self.ed.caret().0;
        let ed = &mut self.ed;
        match key {
            Key::Char('s') if cmd => self.save(cx),
            Key::Enter if cmd => self.run(cx),
            _ if cmd || mods.alt => return false,
            Key::Enter => ed.newline(),
            Key::Tab => ed.insert("  "),
            Key::Backspace => ed.backspace(),
            Key::Delete => ed.delete(),
            Key::Left => ed.left(),
            Key::Right => ed.right(),
            Key::Up => ed.up(),
            Key::Down => ed.down(),
            Key::Home => ed.home(),
            Key::End => ed.end(),
            Key::PageUp => ed.move_to_line(line.saturating_sub(page)),
            Key::PageDown => ed.move_to_line(line + page),
            _ => return false,
        }
        self.dirty |= matches!(key, Key::Enter | Key::Tab | Key::Backspace | Key::Delete) && !cmd;
        true
    }

    /// Typed or pasted text, unless it echoes the Enter or Tab just handled.
    fn text(&mut self, t: &str, last: Option<Key>) {
        let echo = matches!(
            (last, t),
            (Some(Key::Enter), "\n" | "\r\n") | (Some(Key::Tab), "\t") | (_, "")
        );
        if !echo {
            self.ed.insert(t);
            self.dirty = true;
        }
    }

    /// Moves the caret to problem `i`.
    fn jump(&mut self, i: usize) {
        if let Some(&(l, c)) = self.problems.get(i).and_then(|p| p.pos.as_ref()) {
            self.ed.set_caret(l.saturating_sub(1), c.saturating_sub(1));
        }
    }

    /// A press at `(x, y)` in the editor, relative to the content's corner
    /// as the shell sends it.
    fn click(&mut self, x: f32, y: f32) {
        if let Some(g) = self.geo {
            let x = x - g.text.x + self.left as f32 * g.cell;
            let y = y - g.text.y + self.top as f32 * g.row;
            self.ed.click(x, y, g.cell, g.row);
        }
    }

    /// The editor: gutter, highlighted text and caret, clipped to `well`.
    fn draw_text(&mut self, ui: &mut Ui<'_>, well: RectF, (row, asc): (f32, f32)) {
        let (t, line_px) = (ui.theme(), px(ui, 1.0));
        let mono = t.mono();
        let count = self.ed.line_count();
        let cell = ui.text_system().cell_width(mono.size);
        let mut num = String::new();
        push_num(&mut num, count, 2);
        let gutter = (num.len() + 2) as f32 * cell;
        let (w, h) = (well.w - gutter - INSET, well.h - 2.0 * INSET);
        let text = RectF::new(well.x + gutter, well.y + INSET, w.max(cell), h.max(row));
        let (rows, cols) = ((text.h / row) as usize, (text.w / cell) as usize);
        let (rows, cols) = (rows.max(1), cols.max(1));
        let r = ui.rect();
        self.geo = Some(Geo {
            text: RectF::new(text.x - r.x, text.y - r.y, text.w, text.h),
            cell,
            row,
            rows,
            cols,
        });
        self.reveal();
        let (cl, cc) = self.ed.caret();
        ui.push_clip(well);
        let sep = ui.text_system().snap(well.x + gutter - 0.625 * cell);
        ui.fill(RectF::new(sep, well.y, line_px, well.h), 0.0, t.border);
        let (mut buf, mut toks) = ([0u8; 4], Vec::new());
        for i in self.top..count.min(self.top + rows + 1) {
            let y = text.y + (i - self.top) as f32 * row;
            let base = ui.text_system().snap(y + asc);
            if i == cl {
                let lit = RectF::new(well.x + line_px, y, well.w - 2.0 * line_px, row);
                ui.fill(lit, 0.0, t.surface_hi);
            }
            num.clear();
            push_num(&mut num, i + 1, 1);
            let nx = well.x + gutter - 1.25 * cell - ui.text_system().measure(&num, mono);
            let color = if i == cl { t.text_dim } else { t.text_faint };
            ui.text(nx, base, &num, mono.with_color(color));
            let line = self.ed.line(i);
            paint(line, &mut toks);
            let cells = line.char_indices().skip(self.left).take(cols + 1);
            for (k, (at, c)) in cells.enumerate().filter(|(_, (_, c))| *c != ' ') {
                let (x, style) = (text.x + k as f32 * cell, mono.with_color(ink(toks[at], t)));
                ui.text(x, base, c.encode_utf8(&mut buf), style);
            }
        }
        let (top, left) = (self.top, self.left);
        if (top..top + rows).contains(&cl) && (left..=left + cols).contains(&cc) {
            let w = px(ui, 2.0);
            let x = ui.text_system().snap(text.x + (cc - left) as f32 * cell - w / 2.0);
            let y = text.y + (cl - top) as f32 * row;
            let color = if ui.state().focused { t.accent } else { t.text_faint };
            ui.fill(RectF::new(x, y, w, row), 0.0, color);
        }
        ui.pop_clip();
        ui.hit(EDITOR, well, Sense::Text);
    }
}

/// What a char of applang is, for its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Plain,
    Keyword,
    Str,
    Number,
    Comment,
}

/// The color of a [`Tok`] in `t`. Plain code is a shade softer than body
/// text, so keywords in the accent stand out even where the accent is
/// white.
fn ink(tok: Tok, t: &Theme) -> Rgba {
    match tok {
        Tok::Plain => mix(t.text, t.text_dim, 0.2),
        Tok::Keyword => t.accent,
        Tok::Str => t.ansi[2],
        Tok::Number => t.ansi[3],
        Tok::Comment => t.text_faint,
    }
}

/// `v` logical pixels as whole device pixels, at least one: a stroke.
pub(crate) fn px(ui: &mut Ui<'_>, v: f32) -> f32 {
    let d = ui.text_system().dpr();
    (v * d).round().max(1.0) / d
}

/// `r` with every edge on the nearest device pixel.
pub(crate) fn snapped(ui: &mut Ui<'_>, r: RectF) -> RectF {
    let ts = ui.text_system();
    let (x, y) = (ts.snap(r.x), ts.snap(r.y));
    RectF::new(x, y, ts.snap(r.x + r.w) - x, ts.snap(r.y + r.h) - y)
}

/// Fills `out` with a [`Tok`] for each byte of one line of applang (a char
/// takes its first byte's): keywords, strings, numbers and `//` comments.
/// Everything that starts or ends a token is ASCII, so bytes will do.
pub(crate) fn paint(line: &str, out: &mut Vec<Tok>) {
    let (b, n) = (line.as_bytes(), line.len());
    out.clear();
    out.resize(n, Tok::Plain);
    let word = |i: usize| b.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
    let mut i = 0;
    while i < n {
        let start = i;
        i += 1;
        let tok = match b[start] {
            b'/' if b.get(i) == Some(&b'/') => {
                i = n;
                Tok::Comment
            }
            b'"' => {
                while i < n && b[i] != b'"' {
                    i += 1 + usize::from(b[i] == b'\\');
                }
                i = (i + 1).min(n);
                Tok::Str
            }
            c if word(start) => {
                while word(i) {
                    i += 1;
                }
                match c.is_ascii_digit() {
                    true => Tok::Number,
                    false if KEYWORDS.iter().any(|k| k.as_bytes() == &b[start..i]) => Tok::Keyword,
                    false => Tok::Plain,
                }
            }
            _ => Tok::Plain,
        };
        out[start..i].fill(tok);
    }
}

impl App for Studio {
    fn title(&self) -> String {
        join(&["Studio — ", file_name(&self.path)])
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((760.0, 540.0))
    }

    fn icon(&self) -> AppIcon {
        crate::STUDIO_ICON
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let fresh = !self.loaded;
        if fresh {
            self.load(cx.vfs);
        }
        let key = if let AppEvent::Key { key, .. } = ev { Some(key) } else { None };
        let last = std::mem::replace(&mut self.last_key, key);
        match ev {
            AppEvent::Click(RUN) => self.run(cx),
            AppEvent::Click(SAVE) => self.save(cx),
            AppEvent::Click(NEW) => self.new_file(cx),
            AppEvent::Click(WidgetId(n)) if n >= PROBLEM => self.jump((n - PROBLEM) as usize),
            AppEvent::PointerDown { x, y, id } if id == Some(EDITOR) => self.click(x, y),
            AppEvent::Key { key, mods } => return self.key(key, mods, cx) || fresh,
            AppEvent::Text(t) => self.text(&t, last),
            AppEvent::Wheel { dy, .. } => return self.scroll(dy) || fresh,
            AppEvent::Resized { .. } | AppEvent::Focus(_) => {}
            _ => return fresh,
        }
        true
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let dirty = if self.dirty { " (modified)" } else { "" };
        let sep = if self.status.is_empty() { "" } else { " · " };
        let note = join(&[&self.path, dirty, sep, &self.status]);
        let (r, t, line_px) = (ui.rect(), ui.theme(), px(ui, 1.0));
        let (mono, small) = (t.mono(), t.small());
        // One line right of the buttons, or under them if too little fits.
        let mut room = 0.0;
        ui.row(|ui| {
            ui.button_primary(RUN, "Run");
            ui.button(SAVE, "Save");
            let b = ui.button(NEW, "New");
            let x = b.x + b.w + SPACING;
            room = r.x + r.w - PAD - x;
            if room >= NOTE_MIN {
                let ts = ui.text_system();
                let (a, d) = (ts.ascent(small), ts.descent(small));
                let base = ts.snap(b.y + (BUTTON_H - a - d) / 2.0 + a);
                let shown = ts.ellipsize(&note, small, room);
                ui.text(x, base, &shown, small);
            }
        });
        if room < NOTE_MIN {
            let room = ui.width();
            let shown = ui.text_system().ellipsize(&note, small, room);
            ui.small(&shown);
        }
        let y0 = ui.cursor().1;
        let ts = ui.text_system();
        let (row, asc) = (ts.line_height(mono), ts.ascent(mono));
        let shown = self.problems.len().min(MAX_PROBLEMS);
        // The problem rows, and a gap above them when there are any.
        let panel = shown.min(1) as f32 * SPACING + shown as f32 * PROBLEM_H;
        let well_h = (r.y + r.h - PAD - panel - y0).max(row + 2.0 * INSET);
        let well = snapped(ui, RectF::new(r.x + PAD, y0, ui.width(), well_h));
        ui.fill(well, RADIUS_SM, t.surface_lo);
        self.draw_text(ui, well, (row, asc));
        ui.border(well, RADIUS_SM, line_px, t.border);

        let mut y = well.y + well.h + SPACING;
        for (i, p) in self.problems.iter().take(shown).enumerate() {
            let id = WidgetId(PROBLEM + i as u32);
            let rect = RectF::new(well.x, y, well.w, PROBLEM_H);
            let (hover, down) = (ui.state().hover == Some(id), ui.state().pressed == Some(id));
            if hover {
                ui.fill(rect, 6.0, t.wash(down));
            }
            let base = ui.text_system().snap(y + (PROBLEM_H - row) / 2.0 + asc);
            let (code, pos) = p.head();
            ui.push_clip(rect);
            let mut x = rect.x + 8.0;
            for (part, color) in [(&code, t.danger), (&pos, t.text_dim), (&p.message, t.text)] {
                if !part.is_empty() {
                    x += ui.text(x, base, part, mono.with_color(color)) + 8.0;
                }
            }
            ui.pop_clip();
            ui.hit(id, rect, Sense::Click);
            y += PROBLEM_H;
        }
        ui.advance_to(y);
    }
}
