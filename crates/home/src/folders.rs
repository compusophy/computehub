//! Folders on the home screen: three ([`NAMES`]), each holding the apps the person moved into it
//! (an app's menu: Move to). One that holds any shows as an icon, its tile a plate of its first
//! four apps' tiles, and opens as a panel over the screen: its name over its apps, four across.
//! Which folder each app is in is kept as [`PREF`]: a line an app, the folder's number, a space,
//! the app. Before any move, System holds the OS's own apps and Productivity the Editor and
//! Files.

use gfx::{DrawList, RectF};
use host::paint::{cap_baseline, px};
use host::{Effect, Entry};
use ui::icon::{Glyph, PHI};
use ui::{AppIcon, FontId, Rgba, TextStyle, TextSystem, Theme};

use crate::bar;
use crate::icons::{self, CELL, State};

/// The preference that keeps the folders, and the folders' names.
pub const PREF: &str = "folders";
pub const NAMES: [&str; 3] = ["System", "Games", "Productivity"];
/// The folders before any move.
const FIRST: &str = "0 activity\n0 settings\n0 feedback\n0 about\n0 welcome\n2 editor\n2 files\n\
    1 shop\n1 grandfather\n1 clock";
/// The panel's padding, the band its name takes, and its corner radius.
const PAD: f32 = 21.0;
const HEAD: f32 = 55.0;
const RADIUS: f32 = 21.0;

/// Which folder each app is in, and how many times that changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folders {
    pub of: Vec<(String, usize)>,
    pub changes: u64,
}

impl Folders {
    /// The folders as stored (none: the first ones); a line that does not read is left out.
    pub fn new(stored: Option<&str>) -> Folders {
        let line = |l: &str| match l.split_once(' ')? {
            (k @ ("0" | "1" | "2"), app) => Some((app.into(), usize::from(k.as_bytes()[0] - b'0'))),
            _ => None,
        };
        Folders { of: stored.unwrap_or(FIRST).lines().filter_map(line).collect(), changes: 0 }
    }

    /// Moves `app` into folder `k` (past the last: out of any), keeping where all are (to `fx`).
    pub fn put(&mut self, app: &str, k: usize, fx: &mut Vec<Effect>) {
        self.of.retain(|o| o.0 != app);
        if k < NAMES.len() {
            self.of.push((app.to_string(), k));
        }
        self.changes += 1;
        let mut value = String::new();
        for (app, k) in &self.of {
            value.extend([char::from(b'0' + *k as u8), ' ']);
            value.push_str(app);
            value.push('\n');
        }
        fx.push(Effect::Pref { key: PREF.to_string(), value });
    }

    /// The home screen's icons of `apps`: those in no folder, then each folder that holds any
    /// (named `folder:<k>`); and each folder's apps.
    pub fn group(&self, apps: Vec<Entry>) -> (Vec<Entry>, [Vec<Entry>; 3]) {
        let (mut top, mut inside) = (Vec::new(), <[Vec<Entry>; 3]>::default());
        for e in apps {
            match self.of.iter().find(|o| o.0 == e.name) {
                Some(o) => inside[o.1].push(e),
                None => top.push(e),
            }
        }
        for (k, apps) in inside.iter().enumerate().filter(|f| !f.1.is_empty()) {
            let mut name = String::from("folder:");
            name.push(char::from(b'0' + k as u8));
            let icon = AppIcon { glyph: Glyph::Folder, hue: apps[0].icon.hue };
            let label = NAMES[k].to_string();
            top.push(Entry { name, label, icon, mark: None });
        }
        (top, inside)
    }
}

/// The folder an icon named `name` opens, if it is a folder's.
pub fn index(name: &str) -> Option<usize> {
    match name.strip_prefix("folder:")?.as_bytes() {
        [k @ b'0'..=b'2'] => Some(usize::from(k - b'0')),
        _ => None,
    }
}

/// A folder's tile in the square `r`: a plate (rounded as an app's) holding its first four apps'
/// tiles, two across.
pub fn tile(list: &mut DrawList, text: &mut TextSystem, r: RectF, apps: &[Entry], theme: &Theme) {
    let radius = r.w / (PHI * PHI * PHI);
    list.fill(r, radius, theme.surface_hi);
    list.border(r, radius, px(text, 1.0), theme.border);
    let (pad, gap) = (r.w * 0.14, r.w * 0.08);
    let s = (r.w - 2.0 * pad - gap) / 2.0;
    for (i, e) in apps.iter().take(4).enumerate() {
        let at = |n: usize| (n as f32 * (s + gap)).round();
        let mini = RectF::new(r.x + pad + at(i % 2), r.y + pad + at(i / 2), s, s);
        crate::tile(list, text, mini, (e.icon, e.mark.as_ref()), theme);
    }
}

/// The panel of a folder of `n` apps on a screen of `size`: centered, four across (fewer if it
/// holds fewer, and narrower cells on a narrow screen), its name's band over them.
pub fn panel(size: (f32, f32), n: usize) -> RectF {
    let cols = n.clamp(1, 4) as f32;
    let cell = ((size.0 - 32.0 - 2.0 * PAD) / cols).min(CELL.0).floor();
    let (w, h) = (cols * cell + 2.0 * PAD, n.div_ceil(4).max(1) as f32 * CELL.1 + HEAD + PAD);
    RectF::new(((size.0 - w) / 2.0).round(), ((size.1 - h) / 2.0).round(), w, h)
}

/// App `i`'s cell in the panel `p` of `n` apps.
pub fn cell(p: RectF, n: usize, i: usize) -> RectF {
    let w = (p.w - 2.0 * PAD) / n.clamp(1, 4) as f32;
    let (col, row) = ((i % 4) as f32, (i / 4) as f32);
    RectF::new(p.x + PAD + col * w, p.y + HEAD + row * CELL.1, w, CELL.1)
}

/// What of a folder of `n` apps' panel is at `(x, y)`: app `i`'s cell (`Some(Some(i))`), the
/// rest of the panel (`Some(None)`), or nothing.
pub fn at(size: (f32, f32), n: usize, x: f32, y: f32) -> Option<Option<usize>> {
    let p = panel(size, n);
    p.contains(x, y).then(|| (0..n).find(|&i| cell(p, n, i).contains(x, y)))
}

/// Folder `k` of `apps` open on a screen of `size`: the screen dimmed but the top bar's mark,
/// which answers above it (`bar`: the bar's button under the pointer), the panel raised over
/// it, its name, its apps (`hover`: the one under the pointer, and whether held).
pub fn draw(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    (size, k, apps): ((f32, f32), usize, &[Entry]),
    (hover, bar): (Option<(usize, bool)>, bar::Hover),
) {
    let (n, base) = (apps.len(), theme.base);
    list.fill(RectF::new(0.0, 0.0, size.0, size.1), 0.0, Rgba(base.0, base.1, base.2, 153));
    bar::button(list, text, theme, bar::buttons(size.0)[0], bar);
    let p = panel(size, n);
    list.shadow_offset(p, RADIUS, 34.0, 8.0, theme.shadow);
    list.fill(p, RADIUS, theme.surface);
    list.border(p, RADIUS, px(text, 1.0), theme.border);
    let style = TextStyle::new(FontId::SansBold, 17.0, theme.text);
    let w = text.measure(NAMES[k], style);
    let x = text.snap(p.x + (p.w - w) / 2.0);
    text.draw_text(list, x, cap_baseline(text, p.y + PAD, 21.0, 17.0), NAMES[k], style);
    for (i, e) in apps.iter().enumerate() {
        let hover = hover.filter(|h| h.0 == i).map(|h| h.1);
        let state = State { hover, ..State::default() };
        icons::draw(
            list,
            text,
            theme,
            cell(p, n, i),
            (e.icon, e.mark.as_ref(), &e.label, &[]),
            state,
        );
    }
}
