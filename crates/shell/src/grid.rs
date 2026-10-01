//! The home screen's icons (see `home::icons`): every app, listed in the person's order and the
//! order kept as it changes; the selection box; icons carried to a new place; Enter and Escape
//! for them; drawing them.

use std::mem;

use gfx::{DrawList, RectF};
use home::icons::{self, State};
use host::motion::{Tween, Vis};
use host::rectf;
use ui::{Key, Mods, Theme};

use crate::desktop::Target;
use crate::{Effect, Response, Shell};

/// The built-in apps in the home screen's first order (those the registry knows).
pub(crate) const APPS: [&str; 8] =
    ["studio", "assistant", "terminal", "files", "settings", "feedback", "about", "welcome"];
/// Travel before a pressed icon is carried: a mouse's, and a finger's once it picked one up.
const MOUSE_PX: f32 = 4.0;
const FINGER_PX: f32 = 8.0;
/// How many more carried icons show behind the first, and how far apart.
const STACK: usize = 2;
const STEP: f32 = 5.0;

/// Icons carried, or pressed and about to be: their indices (in order) and the one pressed, where
/// the press (or the finger's pick-up) was and its offset in that icon's cell, whether a finger
/// holds them, whether they show lifted, whether they moved past the travel (and will drop), and
/// where among the rest they would. A new listing that changes the icons drops them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Carry {
    pub icons: Vec<usize>,
    pub lead: usize,
    pub from: (f32, f32),
    pub off: (f32, f32),
    pub touch: bool,
    pub lifted: bool,
    pub moved: bool,
    pub slot: usize,
}

impl Shell {
    /// The grid's area, and whether it is a phone's.
    fn grid(&self) -> (RectF, bool) {
        (rectf(self.host.wm().area()), self.host.narrow())
    }

    /// Lists the apps again in the person's order, new ones last; one new since the `first`
    /// listing is kept in the order as it is now (so the newest stays last).
    pub(crate) fn list_icons(&mut self, first: bool) {
        let (icons, new) = icons::arrange(&self.order, self.host.home(&APPS), |e| &e.name);
        let before = self.order.clone();
        self.icons = icons;
        self.note_order(!first && new);
        if self.order != before {
            self.carry = None;
            self.motion.cells.clear();
        }
        let order = &self.order;
        self.selected.retain(|n| order.contains(n));
    }

    /// Notes the icons' order, and keeps it (the preference) if `keep`.
    fn note_order(&mut self, keep: bool) {
        self.order.clear();
        for e in &self.icons {
            self.order.push(e.name.clone());
        }
        if keep {
            let value = home::joined(&self.order);
            self.pending.push(Effect::Pref { key: icons::PREF.to_string(), value });
        }
    }

    /// Each icon's cell: its place in the order, but while icons are carried (`None` for them)
    /// the rest close up around a gap for them at the slot.
    pub(crate) fn cells(&self) -> Vec<Option<usize>> {
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
    pub(crate) fn icon_at(&self, x: f32, y: f32) -> Option<usize> {
        let (a, narrow) = self.grid();
        let free = self.carry.as_ref().is_none_or(|c| !c.lifted);
        icons::at(self.icons.len(), a, narrow, x, y).filter(|_| free)
    }

    /// A button-0 press on the home screen (`hit`): on an icon a mouse's may carry it (with the
    /// selection, if it is selected); on the bare desktop a mouse's starts a selection box. The
    /// selection ends but on its own icons and in menus.
    pub(crate) fn press_home(&mut self, hit: Option<Target>, touch: bool) {
        let at = self.pointer.unwrap_or_default();
        (self.carry, self.lasso) = (None, None);
        match hit {
            Some(Target::Icon(i)) => {
                if !self.icons.get(i).is_some_and(|e| self.selected.contains(&e.name)) {
                    self.selected.clear();
                }
                if !touch {
                    self.carry = self.pick(i, at, false);
                }
            }
            Some(Target::Desktop) if !touch => {
                self.selected.clear();
                self.lasso = Some(at);
            }
            Some(Target::Menu(_) | Target::MenuPanel) => {}
            _ => self.selected.clear(),
        }
    }

    /// Icon `i` (and the selection with it, if it is selected) pressed at `at`: lifted at once
    /// for a finger, which held it.
    pub(crate) fn pick(&self, i: usize, at: (f32, f32), touch: bool) -> Option<Carry> {
        let all = self.selected.contains(&self.icons.get(i)?.name);
        let (mut picked, mut slot) = (Vec::new(), 0);
        for (k, e) in self.icons.iter().enumerate() {
            match k == i || all && self.selected.contains(&e.name) {
                true => picked.push(k),
                false => slot += usize::from(k < i),
            }
        }
        let r = icons::cell(i, self.grid().0, self.grid().1);
        let (off, lifted) = ((at.0 - r.x, at.1 - r.y), touch);
        Some(Carry { icons: picked, lead: i, from: at, off, touch, lifted, moved: false, slot })
    }

    /// The pointer moved: carried icons follow it once it traveled far enough, and the slot
    /// under them opens.
    pub(crate) fn carry_to(&mut self) {
        let ((a, narrow), Some(at)) = (self.grid(), self.pointer) else { return };
        let Some(c) = &mut self.carry else { return };
        let far = (at.0 - c.from.0).abs().max((at.1 - c.from.1).abs());
        let travel = if c.touch { FINGER_PX } else { MOUSE_PX };
        if !c.moved && (c.lifted || !c.touch) && far >= travel {
            (c.moved, c.lifted, self.armed) = (true, true, None);
        }
        if c.moved {
            let r = icons::cell(0, a, narrow);
            let center = (at.0 - c.off.0 + r.w / 2.0, at.1 - c.off.1 + r.h / 2.0);
            c.slot = icons::slot(self.icons.len() - c.icons.len(), a, narrow, center);
        }
    }

    /// Puts carried icons down: where they are headed if `keep` and they moved (keeping the
    /// order if it changed), else back; each slides there from where it showed.
    pub(crate) fn drop_icons(&mut self, keep: bool) {
        for (i, r) in self.carried() {
            if let Some(t) = self.motion.cells.get_mut(i) {
                *t = Tween::new(Vis::at(r));
            }
        }
        let Some(c) = self.carry.take() else { return };
        let order = icons::moved(self.icons.len(), &c.icons, c.slot);
        if keep && c.moved && order.iter().enumerate().any(|(k, &i)| k != i) {
            let (icons, cells) = (mem::take(&mut self.icons), mem::take(&mut self.motion.cells));
            for i in order {
                self.icons.push(icons[i].clone());
                self.motion.cells.extend(cells.get(i));
            }
            self.note_order(true);
        }
    }

    /// The selection box follows the pointer and selects the icons it touches.
    pub(crate) fn lasso_to(&mut self) {
        let (Some(from), Some(at)) = (self.lasso, self.pointer) else { return };
        let ((a, narrow), b) = (self.grid(), icons::boxed(from, at));
        let touches = |i: usize| {
            let r = icons::cell(i, a, narrow).inset(4.0).intersect(b);
            r.w > 0.0 && r.h > 0.0
        };
        self.selected.clear();
        for (i, e) in self.icons.iter().enumerate() {
            if touches(i) {
                self.selected.push(e.name.clone());
            }
        }
    }

    /// Escape puts carried icons back (and forgets the press), else ends the selection; Enter
    /// opens the selected apps. Whether `key` was theirs.
    pub(crate) fn home_key(&mut self, key: Key, m: Mods, out: &mut Response) -> bool {
        match key {
            Key::Escape if self.carry.is_some() || self.lasso.is_some() => {
                self.drop_icons(false);
                (self.lasso, self.armed) = (None, None);
                self.selected.clear();
            }
            Key::Escape if !self.selected.is_empty() => self.selected.clear(),
            Key::Enter if m == Mods::default() && !self.selected.is_empty() => {
                for name in mem::take(&mut self.selected) {
                    self.host.show(&name, out);
                }
            }
            _ => return false,
        }
        true
    }

    /// Every icon not carried, sliding to its cell, washed while hovered and ringed while
    /// selected; then the selection box.
    pub(crate) fn draw_icons(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let ((a, narrow), cells) = (self.grid(), self.cells());
        for (i, cell) in cells.into_iter().enumerate() {
            let (Some(cell), e) = (cell, &self.icons[i]) else { continue };
            let r = self.motion.cell(i, now).unwrap_or_else(|| icons::cell(cell, a, narrow));
            let hot = Some(Target::Icon(i));
            let hover = (self.hover == hot).then_some(self.armed == hot);
            let state = State { hover, selected: self.selected.contains(&e.name), lift: 0.0 };
            icons::draw(list, &mut self.host.text, theme, r, (e.icon, e.sigil, &e.label), state);
        }
        if let (Some(from), Some(at)) = (self.lasso, self.pointer) {
            icons::draw_box(list, &self.host.text, theme, from, at);
        }
    }

    /// The icons carried, over everything but menus: the one pressed where the pointer holds
    /// it, lifted, the others' tiles stacked behind it.
    pub(crate) fn draw_carried(&mut self, list: &mut DrawList, theme: &Theme) {
        for (k, (i, r)) in self.carried().into_iter().enumerate().rev() {
            let e = &self.icons[i];
            let label = if k == 0 { e.label.as_str() } else { "" };
            let state = State { lift: 1.0, ..State::default() };
            icons::draw(list, &mut self.host.text, theme, r, (e.icon, e.sigil, label), state);
        }
    }

    /// Where the lifted icons show: the one pressed under the pointer as it was pressed, then
    /// up to [`STACK`] others, each a step behind.
    fn carried(&self) -> Vec<(usize, RectF)> {
        let (Some(c), Some(at)) = (self.carry.as_ref().filter(|c| c.lifted), self.pointer) else {
            return Vec::new();
        };
        let cell = icons::cell(0, self.grid().0, self.grid().1);
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
