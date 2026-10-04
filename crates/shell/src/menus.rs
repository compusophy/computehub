//! Context menus: which one a secondary press opens, what their items do (the dock's favorites
//! they change: `home::dock::Dock::keep`; the person's own apps they delete, asked again first).

use gfx::DrawList;
use home::menu::{Item, Menu};
use ui::Theme;
use vfs::Vfs;
use wm::{Cmd, State, WinId};

use crate::desktop::Target;
use crate::{Response, Shell};

/// What a menu item does to the menu's app: open a new window of it, show it (its window, else a
/// new one), add it to the dock (or remove it), close its windows, delete it (asking again
/// first; then its windows close, the dock lets it go and its file goes); or show the
/// Assistant, show a built-in app, open a terminal, command the menu's window (minimize, toggle
/// maximize, close), or sign out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Act {
    Open,
    Show,
    Keep(bool),
    Close,
    Delete(bool),
    Ask,
    Go(&'static str),
    Terminal,
    Wm(usize),
    SignOut,
}

/// The desktop's menu.
const DESKTOP: [Item<Act>; 8] = [
    ("Open Terminal", "Alt+\u{23ce}", Some(Act::Terminal)),
    ("Ask the Assistant", "", Some(Act::Ask)),
    ("", "", None),
    ("Settings", "", Some(Act::Go("settings"))),
    ("Send feedback", "", Some(Act::Go("feedback"))),
    ("About compusophy", "", Some(Act::Go("about"))),
    ("", "", None),
    ("Sign out", "", Some(Act::SignOut)),
];

/// An open menu, the app it is about (`""` if none) and its window, if a window's.
pub(crate) type Open = (Menu<Act>, String, Option<WinId>);

impl Shell {
    /// A secondary press at `(x, y)` (a finger's if `touch`): the menu of what is there,
    /// replacing any other; nothing inside an open menu.
    pub(crate) fn secondary(&mut self, (x, y): (f32, f32), touch: bool, out: &mut Response) {
        if self.menu.as_ref().is_some_and(|m| m.0.rect.contains(x, y)) {
            return;
        }
        self.menu = self.hit(x, y).and_then(|hit| self.menu_for(hit, (x, y), touch));
        out.redraw |= self.menu.is_some();
    }

    /// The menu for `hit` at `at` (the desktop's, the Assistant's tile's, an app's for an icon or
    /// a dock tile, or a window's), with its app and window. The Assistant is no dock's: its
    /// icon's menu only opens it. Only the person's own apps (`~/apps/*.app`) can be deleted.
    fn menu_for(&mut self, hit: Target, at: (f32, f32), touch: bool) -> Option<Open> {
        let menu = |s: &mut Shell, items: &[Item<Act>]| {
            Menu::new(items, at, s.size, touch, &mut s.host.text)
        };
        let (name, running) = match hit {
            Target::Desktop => return Some((menu(self, &DESKTOP), String::new(), None)),
            Target::Assistant => {
                let ask = [("Ask the Assistant", "Alt+Space", Some(Act::Ask))];
                return Some((menu(self, &ask), String::new(), None));
            }
            Target::Title(w) | Target::Ctl(w, _) => {
                self.host.apply(Cmd::Focus(w));
                let max = self.placement(w).is_some_and(|p| p.state == State::Maximized);
                let [min, close] = [("Minimize", "Alt+\u{2193}", 0), ("Close", "Alt+Q", 2)]
                    .map(|(label, hint, i)| (label, hint, Some(Act::Wm(i))));
                let label = if max { "Restore" } else { "Maximize" };
                let items: &[_] = match self.host.narrow() {
                    true => &[min, close],
                    false => &[min, (label, "Alt+\u{2191}", Some(Act::Wm(1))), close],
                };
                return Some((menu(self, items), String::new(), Some(w)));
            }
            Target::Icon(i) => (self.grid.icons.get(i)?.name.clone(), false),
            Target::Dock(i) => self.tiles.get(i).map(|d| (d.0.clone(), !d.2.is_empty()))?,
            _ => return None,
        };
        let kept = self.dock.favs.contains(&name);
        let mine = name.strip_prefix(Vfs::HOME).is_some_and(|p| p.starts_with("/apps/"));
        let mut items = [
            match running {
                true => ("New window", "", Some(Act::Open)),
                false => ("Open", "", Some(Act::Show)),
            },
            match kept {
                true => ("Remove from dock", "", Some(Act::Keep(false))),
                false => ("Add to dock", "", Some(Act::Keep(true))),
            },
            ("Close", "", Some(Act::Close)),
            ("", "", None),
            ("Delete", "", Some(Act::Delete(false))),
        ];
        // Not running: no Close, Delete (if any) right after the line.
        if !running {
            items.swap(2, 4);
            items.swap(2, 3);
        }
        let n = 2 + usize::from(running) + 2 * usize::from(mine);
        let n = if name == host::ASSISTANT { 1 } else { n };
        Some((menu(self, &items[..n]), name, None))
    }

    /// Does what item `i` of the open menu says, closing it.
    pub(crate) fn choose(&mut self, i: Option<usize>, out: &mut Response) {
        let Some((menu, name, win)) = self.menu.take() else { return };
        let Some(act) = i.and_then(|i| menu.act(i)) else { return };
        match act {
            Act::Open => self.host.open(&name, None, out),
            Act::Show => self.host.show(&name, out),
            Act::Keep(keep) => self.dock.keep(&name, keep, &mut self.pending),
            // Asked again where it was: a press anywhere else keeps it.
            Act::Delete(false) => {
                let again = [("Delete for good", "No undo", Some(Act::Delete(true)))];
                let (at, touch) = ((menu.rect.x, menu.rect.y), menu.row > 40.0);
                let menu = Menu::new(&again, at, self.size, touch, &mut self.host.text);
                (self.menu, out.redraw) = (Some((menu, name, None)), true);
            }
            Act::Close | Act::Delete(true) => {
                for w in self.host.windows_of(&name) {
                    self.host.apply(Cmd::Close(w));
                }
                if matches!(act, Act::Delete(true)) {
                    self.dock.keep(&name, false, &mut self.pending);
                    let _ = self.host.vfs.remove(&name, false);
                }
            }
            Act::Ask => self.host.show("assistant", out),
            Act::Go(app) => self.host.show(app, out),
            Act::Terminal => self.host.open("terminal", None, out),
            Act::SignOut => out.effects.push(crate::Effect::SignOut),
            Act::Wm(i) => {
                if let Some(w) = win {
                    self.host.apply([Cmd::Minimize, Cmd::ToggleMaximize, Cmd::Close][i](w));
                }
            }
        }
    }

    /// The hovered action becomes the selection.
    pub(crate) fn point_menu(&mut self) {
        let hit = self.pointer.and_then(|(x, y)| self.hit(x, y));
        if let (Some(menu), Some(Target::Menu(i))) = (&mut self.menu, hit) {
            menu.0.sel = Some(i);
        }
    }

    /// The open menu; its selection washed more while pressed.
    pub(crate) fn draw_menu(&mut self, list: &mut DrawList, theme: &Theme) {
        let held =
            self.armed.is_some_and(|a| matches!(a, Target::Menu(_)) && self.hover == Some(a));
        if let Some(menu) = &self.menu {
            menu.0.draw(list, &mut self.host.text, theme, held);
        }
    }
}
