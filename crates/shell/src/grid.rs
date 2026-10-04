//! The home screen's grid (see `home::grid`) wired to the pointer, the keys, the files and the
//! frame.

use gfx::DrawList;
use home::grid::{APPS, Press};
use ui::{Key, Mods, Theme};

use crate::desktop::Target;
use crate::{Response, Shell};

impl Shell {
    /// Lists the apps again if the files or the folders changed; the overlay learns them (those
    /// in folders too).
    pub(crate) fn list_icons(&mut self) {
        let (generation, host) = (self.host.vfs.generation(), &mut self.host);
        if self.grid.list(generation, || host.home(&APPS), &mut self.pending) {
            let all = self.grid.icons.iter().chain(self.grid.inside.iter().flatten());
            let apps = all.map(|e| &e.name).filter(|n| *n != host::ASSISTANT);
            let apps = apps.filter(|n| home::folders::index(n).is_none());
            self.host.agent.apps = apps.cloned().collect();
        }
    }

    /// Opens the app `name`, or the folder it names.
    pub(crate) fn launch(&mut self, name: &str, out: &mut Response) {
        match home::folders::index(name) {
            Some(k) => (self.grid.open, out.redraw) = (Some(k), true),
            None => self.host.show(name, out),
        }
    }

    /// A button-0 press on `hit` (see `home::grid::Grid::press`); a mouse's on a kept dock tile
    /// may carry it.
    pub(crate) fn press_home(&mut self, hit: Option<Target>, touch: bool) {
        let at = self.pointer.unwrap_or_default();
        self.dock.carry = match hit {
            Some(Target::Dock(i)) if !touch => self.dock.pick(i, at, false),
            _ => None,
        };
        let on = match hit {
            Some(Target::Icon(i)) => Press::Icon(i),
            Some(Target::Desktop) => Press::Desktop,
            Some(Target::Menu(_) | Target::MenuPanel) => Press::Menu,
            _ => Press::Other,
        };
        self.grid.press(on, at, touch);
    }

    /// Puts carried icons down (`keep`: where they are headed): over the bottom row, the dock
    /// keeps their apps where its gap shows, and they go back to their places.
    pub(crate) fn drop_icons(&mut self, keep: bool) {
        if keep {
            self.dock.take_in(&mut self.pending);
        }
        self.grid.drop(keep, self.pointer, &mut self.pending);
    }

    /// Escape and Enter for the icons (forgetting a press they end), Escape for a dock tile
    /// carried; whether `key` was theirs.
    pub(crate) fn home_key(&mut self, key: Key, m: Mods, out: &mut Response) -> bool {
        if key == Key::Escape && self.dock.carry.is_some() {
            self.dock.drop(false, &mut self.pending);
            self.armed = None;
            return true;
        }
        let busy = self.grid.carry.is_some() || self.grid.lasso.is_some();
        let Some(open) = self.grid.key(key, m, self.pointer) else { return false };
        if busy {
            self.armed = None;
        }
        for name in open {
            self.launch(&name, out);
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

    /// The open folder, over everything but menus; the app under the pointer washed (more while
    /// pressed).
    pub(crate) fn draw_folder(&mut self, list: &mut DrawList, theme: &Theme) {
        let hover = match self.hover {
            Some(Target::Inside(i)) => Some((i, self.armed == self.hover)),
            _ => None,
        };
        self.grid.draw_open(list, &mut self.host.text, theme, self.size, hover);
    }

    /// The icons or dock tile carried, over everything but menus.
    pub(crate) fn draw_carried(&mut self, list: &mut DrawList, theme: &Theme) {
        self.grid.draw_carried(list, &mut self.host.text, theme, self.pointer);
        self.draw_carried_tile(list, theme);
    }
}
