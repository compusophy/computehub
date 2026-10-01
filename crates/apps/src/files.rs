//! Files: the filesystem, a folder at a time.

use gfx::RectF;
use ui::icon::Glyph;
use ui::{App, AppEvent, AppIcon, Cx, RADIUS_SM, Sense, Ui, WidgetId};
use vfs::{Entry, Vfs};

use crate::kit::{self, ROW_H, Row, Scroll};

/// Widget ids: Up, crumb `i` is `CRUMB + i`, entry `i` is `ENTRY + i`.
const UP: u32 = 1;
const CRUMB: u32 = 10;
const ENTRY: u32 = 1000;
/// The path bar's height (a touch target), and the list's side margin.
const BAR_H: f32 = 44.0;
const SIDE: f32 = 13.0;
const EMPTY: &str = "This folder is empty.";

/// A folder's contents as rows (folders first, then files with their sizes) under a path bar: Up,
/// then the path as crumbs from `~` (or `/` outside home), each one a click away. A folder opens
/// in place, a `.app` runs, any other file opens in Studio. The list scrolls.
#[derive(Debug)]
pub struct Files {
    /// The folder shown, absolute, and its entries as of `seen`: the [`Vfs::generation`] and
    /// folder they were listed at.
    pub(crate) dir: String,
    entries: Vec<Entry>,
    seen: Option<(u64, String)>,
    scroll: Scroll,
    /// The entries' paths as last drawn (row `i` is `ENTRY + i`), and the one the last press
    /// landed on: a click opens what was under it, though the folder was listed again since.
    shown: Vec<String>,
    pressed: Option<String>,
}

impl Default for Files {
    fn default() -> Files {
        Files::new("~")
    }
}

impl Files {
    /// Files at `dir` (`~` is home, relative is under home; one that is not a folder shows home).
    pub fn new(dir: &str) -> Files {
        let dir = Vfs::normalize(Vfs::HOME, dir).unwrap_or_else(|_| Vfs::HOME.to_string());
        let (entries, shown, scroll) = (Vec::new(), Vec::new(), Scroll::default());
        Files { dir, entries, seen: None, scroll, shown, pressed: None }
    }

    /// Lists the folder again if it or the filesystem changed; whether it did.
    fn refresh(&mut self, fs: &Vfs) -> bool {
        let now = (fs.generation(), self.dir.clone());
        if self.seen.as_ref() == Some(&now) {
            return false;
        }
        match fs.list(&self.dir) {
            // Folders first, each part in the order listed (no sort: it costs boot bytes).
            Ok(list) => {
                self.entries.clear();
                for dirs in [true, false] {
                    for e in list.iter().filter(|e| e.is_dir == dirs) {
                        self.entries.push(e.clone());
                    }
                }
            }
            Err(_) if self.dir != Vfs::HOME => {
                self.go(Vfs::HOME.to_string());
                return self.refresh(fs);
            }
            Err(_) => self.entries.clear(),
        }
        self.seen = Some(now);
        true
    }

    fn go(&mut self, dir: String) {
        (self.dir, self.scroll) = (dir, Scroll::default());
    }

    /// The path of entry `name` in the folder shown.
    fn path(&self, name: &str) -> String {
        [self.dir.trim_end_matches('/'), "/", name].concat()
    }

    /// The folder's crumbs as (label, path): `~` and the folders under it, else `/` and each one.
    fn crumbs(&self) -> Vec<(&str, &str)> {
        let d = self.dir.as_str();
        let home = d.strip_prefix(Vfs::HOME).is_some_and(|r| r.is_empty() || r.starts_with('/'));
        let (mut out, from) =
            if home { (vec![("~", Vfs::HOME)], Vfs::HOME.len()) } else { (vec![("/", "/")], 0) };
        // Each name ends at a `/` or the end (by bytes: a char pattern costs boot bytes).
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

    /// A click on the entry drawn at `path`: into a folder, or open a file; nothing if it is no
    /// longer in the folder shown.
    fn open(&mut self, path: String, cx: &mut Cx<'_>) -> bool {
        let Some(e) = self.entries.iter().find(|e| self.path(&e.name) == path) else {
            return false;
        };
        let dir = e.is_dir;
        match (dir, path.ends_with(".app")) {
            (true, _) => self.go(path),
            (false, true) => cx.open(&path),
            (false, false) => cx.open(&["studio:", &path].concat()),
        }
        dir
    }

    /// The path bar across the top of `r`: Up (not at `/`), the crumbs, a hairline under them.
    fn bar(&self, ui: &mut Ui<'_>, r: RectF) {
        let t = ui.theme();
        let mut x = r.x + SIDE - 8.0;
        if self.dir != "/" {
            let up = ui.snapped(RectF::new(x, r.y, BAR_H, BAR_H));
            let (hover, down) = kit::pointer(ui, WidgetId(UP));
            if hover {
                let wash = ui.snapped(up.inset(5.0));
                ui.fill(wash, RADIUS_SM, t.wash(down));
            }
            let style = t.body().with_color(if hover { t.text } else { t.text_dim });
            let w = ui.text_system().measure("\u{2191}", style);
            let base = kit::cap_base(ui, up.y, up.h, style);
            let ax = ui.text_system().snap(up.x + (up.w - w) / 2.0);
            ui.text(ax, base, "\u{2191}", style);
            ui.hit(WidgetId(UP), up, Sense::Click);
            x = up.x + up.w;
        }
        let (crumbs, style) = (self.crumbs(), t.body());
        let mut total = -13.0;
        for c in &crumbs {
            total += ui.text_system().measure(c.0, style) + 16.0 + 13.0;
        }
        // The last crumb stays in view: a long path slides left, under the clip.
        let room = (r.x + r.w - SIDE - x).max(0.0);
        ui.push_clip(RectF::new(x, r.y, room, BAR_H));
        x -= (total - room).max(0.0);
        for (i, (label, _)) in crumbs.iter().enumerate() {
            if i > 0 {
                let at = ui.snapped(RectF::new(x + 2.0, r.y + (BAR_H - 9.0) / 2.0, 9.0, 9.0));
                ui.glyph(at, Glyph::Chevron, t.text_dim);
                x += 13.0;
            }
            let id = WidgetId(CRUMB + i as u32);
            let w = ui.text_system().measure(label, style) + 16.0;
            let at = ui.snapped(RectF::new(x, r.y + 5.0, w, BAR_H - 10.0));
            let (hover, down) = kit::pointer(ui, id);
            if hover {
                ui.fill(at, RADIUS_SM, t.wash(down));
            }
            let last = i + 1 == crumbs.len();
            let ink = style.with_color(if last || hover { t.text } else { t.text_dim });
            let base = kit::cap_base(ui, at.y, at.h, ink);
            ui.text(at.x + 8.0, base, label, ink);
            ui.hit(id, at, Sense::Click);
            x = at.x + at.w;
        }
        ui.pop_clip();
        kit::rule(ui, r.x, r.y + BAR_H, r.w);
    }
}

impl App for Files {
    fn title(&self) -> String {
        "Files".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        self.bar(ui, r);
        let top = ui.text_system().snap(r.y + BAR_H) + ui.px(1.0);
        let view = RectF::new(r.x, top, r.w, (r.y + r.h - top).max(0.0));
        ui.push_clip(view);
        let (x, w, y) = (r.x + SIDE, (r.w - 2.0 * SIDE).max(0.0), view.y + 8.0 - self.scroll.y);
        if self.seen.is_some() && self.entries.is_empty() {
            kit::para(ui, EMPTY, t.small(), (x, w), view.y + 34.0, true);
        }
        self.shown = self.entries.iter().map(|e| self.path(&e.name)).collect();
        for (i, e) in self.entries.iter().enumerate() {
            let at = RectF::new(x, y + i as f32 * ROW_H, w, ROW_H);
            if at.y + at.h < view.y || at.y > view.y + view.h {
                continue;
            }
            let icon = match (e.is_dir, e.name.ends_with(".app")) {
                (true, _) if self.path(&e.name) == Vfs::HOME => {
                    AppIcon { glyph: Glyph::Home, ..kit::FILES }
                }
                (true, _) => kit::FILES,
                (false, true) => AppIcon { glyph: Glyph::Window, hue: kit::tint(&e.name) },
                (false, false) => kit::FILE,
            };
            let size = if e.is_dir { String::new() } else { kit::size(e.size) };
            let row = Row { icon, name: &e.name, line: "", aside: &size, more: e.is_dir };
            let at = ui.snapped(at);
            kit::row(ui, WidgetId(ENTRY + i as u32), at, &row);
        }
        ui.pop_clip();
        self.scroll.measure(self.entries.len() as f32 * ROW_H + 16.0, view.h);
        self.scroll.thumb(ui, view);
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        // What a press landed on, as drawn, before the folder is listed again.
        if let AppEvent::PointerDown { id, .. } = ev {
            let row = id.and_then(|id| id.0.checked_sub(ENTRY));
            self.pressed = row.and_then(|i| self.shown.get(i as usize)).cloned();
        }
        let fresh = self.refresh(cx.vfs);
        let changed = match ev {
            AppEvent::Click(WidgetId(UP)) => {
                let up = self.dir.rsplit_once('/').map_or("/", |(p, _)| p);
                self.go([up, "/"][usize::from(up.is_empty())].to_string());
                true
            }
            AppEvent::Click(WidgetId(id @ CRUMB..ENTRY)) => {
                let crumb = self.crumbs().get((id - CRUMB) as usize).map(|c| c.1.to_string());
                crumb.is_some_and(|path| {
                    let new = path != self.dir;
                    self.go(path);
                    new
                })
            }
            AppEvent::Click(WidgetId(id @ ENTRY..)) => {
                let drawn = self.shown.get((id - ENTRY) as usize).cloned();
                self.pressed.take().or(drawn).is_some_and(|path| self.open(path, cx))
            }
            AppEvent::Wheel { dy, .. } => self.scroll.wheel(dy),
            AppEvent::Resized { .. } => true,
            _ => false,
        };
        let moved = self.refresh(cx.vfs);
        fresh || changed || moved
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((640.0, 480.0))
    }

    fn icon(&self) -> AppIcon {
        kit::FILES
    }
}
