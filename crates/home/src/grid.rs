//! The home screen's grid as it behaves (where cells are and how an icon draws: [`icons`]):
//! every app, each in the cell the person put it in ([`place`], kept as [`icons::PREF`]); icons
//! carried to a cell (a mouse's once it travels 4 px, a finger's once held, then 8 px), those in
//! the way making room while they hover there, or below the grid, onto the bottom row, which
//! keeps their apps (they go back); the selection box; Enter and Escape for them; each icon
//! sliding to its cell.

use std::mem;

use gfx::{DrawList, RectF};
use host::motion::{Tween, Vis};
use host::{Effect, Entry};
use ui::{Key, Mods, TextSystem, Theme};

use crate::icons::{self, State};
use crate::place::{self, Dims, Place};

/// The built-in apps in the home screen's first order (those the registry knows).
#[rustfmt::skip]
pub const APPS: [&str; 9] = ["studio", "assistant", "terminal", "files", "editor",
    "settings", "feedback", "about", "welcome"];
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
/// where the one pressed would land (the grid's: a cell's position; the dock's: its slot). A new
/// listing that changes the icons drops them.
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
}

/// What a button-0 press lands on, as the grid sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Icon(usize),
    Desktop,
    Menu,
    Other,
}

/// The grid: its icons (in their order: as kept, new ones last), where each is kept (once
/// listed, by the icons' order; before, as stored) and where each shows (a cell's position in
/// reading order: [`place::resolve`]), the files' generation they were listed at,
/// the icons selected, carried, the selection box's corner, where each icon slides, and the
/// area it lays out in (whether a phone's).
#[derive(Default)]
pub struct Grid {
    pub icons: Vec<Entry>,
    pub places: Vec<Place>,
    pub spots: Vec<usize>,
    listed: Option<u64>,
    pub selected: Vec<String>,
    pub carry: Option<Carry>,
    pub lasso: Option<(f32, f32)>,
    pub cells: Vec<Tween<Vis>>,
    pub area: RectF,
    pub narrow: bool,
}

impl Grid {
    /// A grid where a stored preference keeps the icons (none: the apps' own order, packed).
    pub fn new(stored: Option<&str>) -> Grid {
        Grid { places: place::parse(stored.unwrap_or("")), ..Grid::default() }
    }

    /// The grid's shape in its area.
    pub fn dims(&self) -> Dims {
        icons::dims(self.area, self.narrow)
    }

    /// Finds where each icon shows, in the grid's area as it is now (none before the first
    /// listing).
    fn lay(&mut self) {
        let n = self.icons.len().min(self.places.len());
        self.spots = place::resolve(&self.places[..n], self.dims());
    }

    /// Lists `apps` again if the files changed since (`generation`), each where it was kept, new
    /// ones in the first free cells; one new since the first listing keeps where the icons show
    /// (the preference to `fx`), so it stays there. Whether it listed.
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
        let names = |icons: &[Entry]| icons.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        let before = names(&self.icons);
        let (icons, places, new) = place::arrange(&self.places, apps(), |e| &e.name);
        (self.icons, self.places) = (icons, places);
        self.lay();
        if !first && new {
            self.keep(false, fx);
        }
        let now = names(&self.icons);
        if now != before {
            self.carry = None;
            self.cells.clear();
        }
        self.selected.retain(|n| now.contains(n));
        true
    }

    /// Keeps where the icons show as their cells in this layout (the preference to `fx`): every
    /// one's if `all` (a drop: what the person sees stays), else but those kept for a larger
    /// screen ([`place::keep`]).
    fn keep(&mut self, all: bool, fx: &mut Vec<Effect>) {
        let dims = self.dims();
        place::keep(&mut self.places, &self.spots, dims, all);
        let value = place::format(&self.places);
        fx.push(Effect::Pref { key: icons::PREF.to_string(), value });
    }

    /// Where every icon shows while `c` is carried: the carried landing where the one pressed is
    /// headed, the others making room ([`place::plan`]).
    fn plan(&self, c: &Carry) -> Vec<usize> {
        place::plan(&self.spots, &c.icons, c.lead, c.slot, self.dims())
    }

    /// Each icon's cell (its position): where it shows, but while icons are carried (`None` for
    /// them) where the rest would be with them put down where they are headed.
    pub fn cells(&self) -> Vec<Option<usize>> {
        let Some(c) = self.carry.as_ref().filter(|c| c.lifted) else {
            return self.spots.iter().map(|&s| Some(s)).collect();
        };
        let plan = self.plan(c).into_iter().enumerate();
        plan.map(|(i, s)| (!c.icons.contains(&i)).then_some(s)).collect()
    }

    /// The icon under `(x, y)`, unless icons are carried.
    pub fn at(&self, x: f32, y: f32) -> Option<usize> {
        let free = self.carry.as_ref().is_none_or(|c| !c.lifted);
        icons::at(&self.spots, self.area, self.narrow, x, y).filter(|_| free)
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
        let (all, slot) = (self.selected.contains(&self.icons.get(i)?.name), *self.spots.get(i)?);
        let with = |k: usize, e: &Entry| k == i || all && self.selected.contains(&e.name);
        let picked = self.icons.iter().enumerate().filter(|(k, e)| with(*k, e)).map(|p| p.0);
        let r = icons::cell(slot, self.area, self.narrow);
        let (icons, off, lifted) = (picked.collect(), (at.0 - r.x, at.1 - r.y), touch);
        Some(Carry { icons, lead: i, from: at, off, touch, lifted, moved: false, slot })
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

    /// The pointer moved to `at`: carried icons follow it once it traveled far enough, headed
    /// for the cell under them (below the grid, the one they came from), those in the way
    /// making room. Whether they just started to move (a press is no click then).
    pub fn carry_to(&mut self, at: Option<(f32, f32)>) -> bool {
        let beneath = self.beneath(at);
        let (Some(at), Some(c)) = (at, &mut self.carry) else { return false };
        let start = c.travel(at);
        if let (true, Some(&home)) = (c.moved, self.spots.get(c.lead)) {
            let r = icons::cell(0, self.area, self.narrow);
            let center = (at.0 - c.off.0 + r.w / 2.0, at.1 - c.off.1 + r.h / 2.0);
            c.slot = if beneath { home } else { icons::slot(self.area, self.narrow, center) };
        }
        start
    }

    /// Puts carried icons down: if `keep` and they moved, where they are headed, those in the way
    /// moved aside, and every icon kept where it then shows (the preference to `fx`) if any
    /// moved; else (or below the grid) back. Each slides there from where it showed with the
    /// pointer at `at`.
    pub fn drop(&mut self, keep: bool, at: Option<(f32, f32)>, fx: &mut Vec<Effect>) {
        for (i, r) in self.carried(at) {
            if let Some(t) = self.cells.get_mut(i) {
                *t = Tween::new(Vis::at(r));
            }
        }
        let keep = keep && !self.beneath(at);
        let Some(c) = self.carry.take() else { return };
        let spots = self.plan(&c);
        if keep && c.moved && spots != self.spots {
            self.spots = spots;
            self.keep(true, fx);
        }
    }

    /// The selection box follows the pointer to `at` and selects the icons it touches.
    pub fn lasso_to(&mut self, at: Option<(f32, f32)>) {
        let (Some(from), Some(at)) = (self.lasso, at) else { return };
        let b = icons::boxed(from, at);
        let touches = |s: usize| {
            let r = icons::cell(s, self.area, self.narrow).inset(4.0).intersect(b);
            r.w > 0.0 && r.h > 0.0
        };
        let picked = self.icons.iter().zip(&self.spots).filter(|(_, s)| touches(**s));
        self.selected = picked.map(|(e, _)| e.name.clone()).collect();
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

    /// Each icon heads for its cell, found anew for the area (a new one, or all on a new screen
    /// size, at once: `instant`); carried ones wait.
    pub fn sync(&mut self, now: f64, instant: bool) {
        self.lay();
        let (cells, m) = (self.cells(), &mut self.cells);
        m.truncate(cells.len());
        for (i, (cell, &spot)) in cells.into_iter().zip(&self.spots).enumerate() {
            let to = Vis::at(icons::cell(cell.unwrap_or(spot), self.area, self.narrow));
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
