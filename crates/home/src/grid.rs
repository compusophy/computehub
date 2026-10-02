//! The home screen's grid as it behaves (where cells are and how an icon draws: [`icons`]):
//! every app, listed in the person's order and the order kept as it changes ([`icons::PREF`]);
//! icons carried to a new place (a mouse's once it travels 4 px, a finger's once held, then 8 px)
//! or below the grid, onto the bottom row, which keeps their apps (they go back); the selection
//! box; Enter and Escape for them; each icon sliding to its cell.

use std::mem;

use gfx::{DrawList, RectF};
use host::motion::{Tween, Vis};
use host::{Effect, Entry};
use ui::{Key, Mods, TextSystem, Theme};

use crate::icons::{self, State};

/// The built-in apps in the home screen's first order (those the registry knows).
pub const APPS: [&str; 9] = [
    "studio",
    "assistant",
    "terminal",
    "files",
    "activity",
    "settings",
    "feedback",
    "about",
    "welcome",
];
/// Travel before a pressed icon is carried: a mouse's, and a finger's once it picked one up.
const MOUSE_PX: f32 = 4.0;
const FINGER_PX: f32 = 8.0;
/// How many more carried icons show behind the first, and how far apart; how long an icon slides.
const STACK: usize = 2;
const STEP: f32 = 5.0;
const SLIDE_MS: f32 = 180.0;

/// Icons carried, or pressed and about to be: their indices (in order) and the one pressed, where
/// the press (or the finger's pick-up) was and its offset in that icon's cell, whether a finger
/// holds them, whether they show lifted, whether they moved past the travel (and will drop), and
/// where among the rest they would. A new listing that changes the icons drops them.
#[derive(Clone, Debug, PartialEq)]
pub struct Carry {
    pub icons: Vec<usize>,
    pub lead: usize,
    pub from: (f32, f32),
    pub off: (f32, f32),
    pub touch: bool,
    pub lifted: bool,
    pub moved: bool,
    pub slot: usize,
}

impl Carry {
    /// The pointer moved to `at`: whether they start to move now, past their travel (a mouse's
    /// 4 px, a finger's 8 once it held them).
    pub fn travel(&mut self, at: (f32, f32)) -> bool {
        let far = (at.0 - self.from.0).abs().max((at.1 - self.from.1).abs());
        let travel = if self.touch { FINGER_PX } else { MOUSE_PX };
        let start = !self.moved && (self.lifted || !self.touch) && far >= travel;
        if start {
            (self.moved, self.lifted) = (true, true);
        }
        start
    }

    /// Where a finger held them, if it picked them up and lifts unmoved: their menu opens there.
    pub fn held(&self) -> Option<(f32, f32)> {
        (self.touch && self.lifted && !self.moved).then_some(self.from)
    }

    /// The slot they were picked up from, among the rest.
    pub fn home(&self) -> usize {
        (0..self.lead).filter(|k| !self.icons.contains(k)).count()
    }
}

/// What a button-0 press lands on, as the grid sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Icon(usize),
    Desktop,
    Menu,
    Other,
}

/// The grid: its icons in the person's order (and that order by name), the files' generation
/// they were listed at, the icons selected, carried, the selection box's corner, where each icon
/// slides, and the area it lays out in (whether a phone's).
#[derive(Default)]
pub struct Grid {
    pub icons: Vec<Entry>,
    pub order: Vec<String>,
    listed: Option<u64>,
    pub selected: Vec<String>,
    pub carry: Option<Carry>,
    pub lasso: Option<(f32, f32)>,
    pub cells: Vec<Tween<Vis>>,
    pub area: RectF,
    pub narrow: bool,
}

impl Grid {
    /// A grid in the order a stored preference names (none: the apps' own).
    pub fn new(stored: Option<&str>) -> Grid {
        Grid { order: crate::names(stored.unwrap_or("")), ..Grid::default() }
    }

    /// Lists `apps` again if the files changed since (`generation`): in the person's order, new
    /// ones last; one new since the first listing is kept in the order as it is now (so the
    /// newest stays last), its preference pushed to `fx`. Whether it listed.
    pub fn list(
        &mut self,
        generation: u64,
        apps: impl FnOnce() -> Vec<Entry>,
        fx: &mut Vec<Effect>,
    ) -> bool {
        let first = self.listed.is_none();
        if self.listed.replace(generation) == Some(generation) {
            return false;
        }
        let (icons, new) = icons::arrange(&self.order, apps(), |e| &e.name);
        let before = mem::take(&mut self.order);
        self.icons = icons;
        self.note_order(!first && new, fx);
        if self.order != before {
            self.carry = None;
            self.cells.clear();
        }
        let order = &self.order;
        self.selected.retain(|n| order.contains(n));
        true
    }

    /// Notes the icons' order, and keeps it (the preference, to `fx`) if `keep`.
    fn note_order(&mut self, keep: bool, fx: &mut Vec<Effect>) {
        self.order = self.icons.iter().map(|e| e.name.clone()).collect();
        if keep {
            let value = crate::joined(&self.order);
            fx.push(Effect::Pref { key: icons::PREF.to_string(), value });
        }
    }

    /// Each icon's cell: its place in the order, but while icons are carried (`None` for them)
    /// the rest close up around a gap for them at the slot.
    pub fn cells(&self) -> Vec<Option<usize>> {
        let (carry, mut out, mut k) = (self.carry.as_ref().filter(|c| c.lifted), Vec::new(), 0);
        for i in 0..self.icons.len() {
            let cell = match carry {
                Some(c) if c.icons.contains(&i) => None,
                Some(c) if k >= c.slot => Some(k + c.icons.len()),
                _ => Some(k),
            };
            k += usize::from(cell.is_some());
            out.push(cell);
        }
        out
    }

    /// The icon under `(x, y)`, unless icons are carried.
    pub fn at(&self, x: f32, y: f32) -> Option<usize> {
        let free = self.carry.as_ref().is_none_or(|c| !c.lifted);
        icons::at(self.icons.len(), self.area, self.narrow, x, y).filter(|_| free)
    }

    /// A button-0 press at `at` (a finger's if `touch`): on an icon a mouse's may carry it (with
    /// the selection, if it is selected); on the bare desktop a mouse's starts a selection box.
    /// The selection ends but on its own icons and in menus.
    pub fn press(&mut self, on: Press, at: (f32, f32), touch: bool) {
        (self.carry, self.lasso) = (None, None);
        match on {
            Press::Icon(i) => {
                if !self.icons.get(i).is_some_and(|e| self.selected.contains(&e.name)) {
                    self.selected.clear();
                }
                if !touch {
                    self.carry = self.pick(i, at, false);
                }
            }
            Press::Desktop if !touch => {
                self.selected.clear();
                self.lasso = Some(at);
            }
            Press::Menu => {}
            _ => self.selected.clear(),
        }
    }

    /// Icon `i` (and the selection with it, if it is selected) pressed at `at`: lifted at once
    /// for a finger, which held it.
    pub fn pick(&self, i: usize, at: (f32, f32), touch: bool) -> Option<Carry> {
        let all = self.selected.contains(&self.icons.get(i)?.name);
        let (mut picked, mut slot) = (Vec::new(), 0);
        for (k, e) in self.icons.iter().enumerate() {
            match k == i || all && self.selected.contains(&e.name) {
                true => picked.push(k),
                false => slot += usize::from(k < i),
            }
        }
        let r = icons::cell(i, self.area, self.narrow);
        let (off, lifted) = ((at.0 - r.x, at.1 - r.y), touch);
        Some(Carry { icons: picked, lead: i, from: at, off, touch, lifted, moved: false, slot })
    }

    /// Whether `at` lies below the grid: over the bottom row, which keeps carried icons' apps
    /// (the icons go back).
    fn beneath(&self, at: Option<(f32, f32)>) -> bool {
        at.is_some_and(|at| at.1 >= self.area.y + self.area.h)
    }

    /// The apps of the icons carried (and moved) below the grid, the pointer at `at`.
    pub fn below(&self, at: Option<(f32, f32)>) -> Vec<String> {
        let c = self.carry.as_ref().filter(|c| c.moved && self.beneath(at));
        let icons = c.map_or(&[][..], |c| &c.icons[..]).iter().filter_map(|&i| self.icons.get(i));
        icons.map(|e| e.name.clone()).collect()
    }

    /// The pointer moved to `at`: carried icons follow it once it traveled far enough, and the
    /// slot under them opens (below the grid, the one they came from). Whether they just
    /// started to move (a press is no click then).
    pub fn carry_to(&mut self, at: Option<(f32, f32)>) -> bool {
        let beneath = self.beneath(at);
        let (Some(at), Some(c)) = (at, &mut self.carry) else { return false };
        let start = c.travel(at);
        if c.moved {
            let r = icons::cell(0, self.area, self.narrow);
            let center = (at.0 - c.off.0 + r.w / 2.0, at.1 - c.off.1 + r.h / 2.0);
            let rest = self.icons.len() - c.icons.len();
            c.slot =
                if beneath { c.home() } else { icons::slot(rest, self.area, self.narrow, center) };
        }
        start
    }

    /// Puts carried icons down: where they are headed if `keep` and they moved (keeping the
    /// order if it changed, its preference to `fx`), else (or below the grid) back; each slides
    /// there from where it showed with the pointer at `at`.
    pub fn drop(&mut self, keep: bool, at: Option<(f32, f32)>, fx: &mut Vec<Effect>) {
        for (i, r) in self.carried(at) {
            if let Some(t) = self.cells.get_mut(i) {
                *t = Tween::new(Vis::at(r));
            }
        }
        let keep = keep && !self.beneath(at);
        let Some(c) = self.carry.take() else { return };
        let order = icons::moved(self.icons.len(), &c.icons, c.slot);
        if keep && c.moved && order.iter().enumerate().any(|(k, &i)| k != i) {
            let (icons, cells) = (mem::take(&mut self.icons), mem::take(&mut self.cells));
            for i in order {
                self.icons.push(icons[i].clone());
                self.cells.extend(cells.get(i));
            }
            self.note_order(true, fx);
        }
    }

    /// The selection box follows the pointer to `at` and selects the icons it touches.
    pub fn lasso_to(&mut self, at: Option<(f32, f32)>) {
        let (Some(from), Some(at)) = (self.lasso, at) else { return };
        let b = icons::boxed(from, at);
        let touches = |i: usize| {
            let r = icons::cell(i, self.area, self.narrow).inset(4.0).intersect(b);
            r.w > 0.0 && r.h > 0.0
        };
        let picked = self.icons.iter().enumerate().filter(|(i, _)| touches(*i));
        self.selected = picked.map(|(_, e)| e.name.clone()).collect();
    }

    /// Escape puts carried icons back (the pointer at `at`) and forgets the press, else ends the
    /// selection; Enter opens the selected apps (their names, the selection ending). `None` if
    /// `key` is not theirs.
    pub fn key(&mut self, key: Key, m: Mods, at: Option<(f32, f32)>) -> Option<Vec<String>> {
        match key {
            Key::Escape if self.carry.is_some() || self.lasso.is_some() => {
                self.drop(false, at, &mut Vec::new());
                self.lasso = None;
                self.selected.clear();
            }
            Key::Escape if !self.selected.is_empty() => self.selected.clear(),
            Key::Enter if m == Mods::default() && !self.selected.is_empty() => {
                return Some(mem::take(&mut self.selected));
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// Each icon heads for its cell (a new one, or all on a new screen size, at once: `instant`);
    /// carried ones wait.
    pub fn sync(&mut self, now: f64, instant: bool) {
        let (cells, m) = (self.cells(), &mut self.cells);
        m.truncate(cells.len());
        for (i, cell) in cells.into_iter().enumerate() {
            let to = Vis::at(icons::cell(cell.unwrap_or(i), self.area, self.narrow));
            match m.get_mut(i) {
                None => m.push(Tween::new(to)),
                Some(_) if cell.is_none() => {}
                Some(t) if instant => *t = Tween::new(to),
                Some(t) => t.to(to, now, SLIDE_MS),
            }
        }
    }

    /// Starts the icons' pending slides as a frame begins.
    pub fn arm(&mut self, now: f64) {
        self.cells.iter_mut().for_each(|c| c.arm(now));
    }

    /// Whether an icon still slides.
    pub fn moving(&self, now: f64) -> bool {
        self.cells.iter().any(|c| c.is_running(now))
    }

    /// Every icon not carried, sliding to its cell, washed while hovered (`Some((icon, held))`)
    /// and ringed while selected; then the selection box to the pointer at `at`.
    pub fn draw(
        &self,
        list: &mut DrawList,
        text: &mut TextSystem,
        theme: &Theme,
        now: f64,
        hover: Option<(usize, bool)>,
        at: Option<(f32, f32)>,
    ) {
        for (i, cell) in self.cells().into_iter().enumerate() {
            let (Some(cell), e) = (cell, &self.icons[i]) else { continue };
            let slid = self.cells.get(i).map(|c| c.value(now).rect);
            let r = slid.unwrap_or_else(|| icons::cell(cell, self.area, self.narrow));
            let hover = hover.filter(|h| h.0 == i).map(|h| h.1);
            let state = State { hover, selected: self.selected.contains(&e.name), lift: 0.0 };
            icons::draw(list, text, theme, r, (e.icon, e.sigil, &e.label), state);
        }
        if let (Some(from), Some(at)) = (self.lasso, at) {
            icons::draw_box(list, text, theme, from, at);
        }
    }

    /// The icons carried, the pointer at `at`: the one pressed where the pointer holds it,
    /// lifted, the others' tiles stacked behind it.
    pub fn draw_carried(
        &self,
        list: &mut DrawList,
        text: &mut TextSystem,
        theme: &Theme,
        at: Option<(f32, f32)>,
    ) {
        for (k, (i, r)) in self.carried(at).into_iter().enumerate().rev() {
            let e = &self.icons[i];
            let label = if k == 0 { e.label.as_str() } else { "" };
            let state = State { lift: 1.0, ..State::default() };
            icons::draw(list, text, theme, r, (e.icon, e.sigil, label), state);
        }
    }

    /// Where the lifted icons show, the pointer at `at`: the one pressed under it as it was
    /// pressed, then up to two others, each a step behind.
    pub fn carried(&self, at: Option<(f32, f32)>) -> Vec<(usize, RectF)> {
        let (Some(c), Some(at)) = (self.carry.as_ref().filter(|c| c.lifted), at) else {
            return Vec::new();
        };
        let cell = icons::cell(0, self.area, self.narrow);
        let lead = RectF { x: at.0 - c.off.0, y: at.1 - c.off.1, ..cell };
        let mut out = vec![(c.lead, lead)];
        for &i in c.icons.iter().filter(|&&i| i != c.lead) {
            let k = out.len() as f32 * STEP;
            if out.len() <= STACK {
                out.push((i, RectF { x: lead.x + k, y: lead.y - k, ..lead }));
            }
        }
        out
    }
}
