//! The home screen's grid (see `home::grid`) wired to the pointer, the keys, the files and the
//! frame.

use gfx::DrawList;
use home::grid::{APPS, Press};
use ui::{Key, Mods, Theme};

use crate::desktop::Target;
use crate::{Response, Shell};

impl Shell {
    /// Lists the apps again if the files changed; the overlay learns them.
    pub(crate) fn list_icons(&mut self) {
        let (generation, host) = (self.host.vfs.generation(), &mut self.host);
        if self.grid.list(generation, || host.home(&APPS), &mut self.pending) {
            let apps = self.grid.icons.iter().map(|e| &e.name).filter(|n| *n != host::ASSISTANT);
            self.host.agent.apps = apps.cloned().collect();
        }
    }

    /// A button-0 press on `hit` (see `home::grid::Grid::press`).
    pub(crate) fn press_home(&mut self, hit: Option<Target>, touch: bool) {
        let on = match hit {
            Some(Target::Icon(i)) => Press::Icon(i),
            Some(Target::Desktop) => Press::Desktop,
            Some(Target::Menu(_) | Target::MenuPanel) => Press::Menu,
            _ => Press::Other,
        };
        self.grid.press(on, self.pointer.unwrap_or_default(), touch);
    }

    /// Puts carried icons down (`keep`: where they are headed).
    pub(crate) fn drop_icons(&mut self, keep: bool) {
        self.grid.drop(keep, self.pointer, &mut self.pending);
    }

    /// Escape and Enter for the icons (forgetting a press they end); whether `key` was theirs.
    pub(crate) fn home_key(&mut self, key: Key, m: Mods, out: &mut Response) -> bool {
        let busy = self.grid.carry.is_some() || self.grid.lasso.is_some();
        let Some(open) = self.grid.key(key, m, self.pointer) else { return false };
        if busy {
            self.armed = None;
        }
        for name in open {
            self.host.show(&name, out);
        }
        true
    }

    /// The icons behind the windows, the hovered one washed (more while pressed).
    pub(crate) fn draw_icons(&mut self, list: &mut DrawList, theme: &Theme, now: f64) {
        let hover = match self.hover {
            Some(Target::Icon(i)) => Some((i, self.armed == self.hover)),
            _ => None,
        };
        self.grid.draw(list, &mut self.host.text, theme, now, hover, self.pointer);
    }

    /// The icons carried, over everything but menus.
    pub(crate) fn draw_carried(&mut self, list: &mut DrawList, theme: &Theme) {
        self.grid.draw_carried(list, &mut self.host.text, theme, self.pointer);
    }
}
