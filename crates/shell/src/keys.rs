//! Keys: an open menu's, the home screen's (Enter opens the selected icons, Escape puts carried
//! ones back or ends the selection), the desktop's bindings (`mod` is Alt or Meta, without Ctrl),
//! and the rest for the focused app.

use ui::{AppEvent, Key, Mods};
use wm::{Cmd, Snap, State};

use crate::{Response, Shell};

/// The paste keys, left to the browser: the text comes as `Input::Text`.
fn is_paste(key: Key, m: Mods) -> bool {
    let ctrl = m.ctrl && !m.alt && !m.meta;
    let meta = m.meta && !m.ctrl && !m.alt && !m.shift;
    let shift = m.shift && !m.ctrl && !m.alt && !m.meta;
    (key == Key::Char('v') && (ctrl || meta)) || (key == Key::Insert && shift)
}

/// Reload and the developer tools, the browser's unless an app wants text.
fn is_browser(key: Key, m: Mods) -> bool {
    let ctrl = m.ctrl && !m.alt && !m.meta;
    match key {
        Key::F(5 | 12) => true,
        Key::Char('r') => ctrl,
        Key::Char('i') => ctrl && m.shift,
        _ => false,
    }
}

impl Shell {
    /// A key for an open menu, else for the home screen, else a binding, else for the focused
    /// app.
    pub(crate) fn key(&mut self, key: Key, m: Mods, out: &mut Response) {
        let chord = (m.alt || m.meta) && !m.ctrl;
        if let Some((menu, ..)) = &mut self.menu {
            match key {
                Key::Escape => self.menu = None,
                Key::Enter => {
                    let sel = menu.sel;
                    self.choose(sel, out);
                }
                _ => _ = menu.key(key),
            }
            out.consumed = true;
            return;
        }
        // Escape closes the open folder.
        if key == Key::Escape && self.grid.open.take().is_some() {
            (out.consumed, out.redraw) = (true, true);
            return;
        }
        // The overlay with the keys: Escape hides it, or stops its task (no other key does).
        let overlay = self.key_target() == Some(host::OVERLAY);
        if overlay && key == Key::Escape {
            match self.host.agent.working {
                true => self.host.halt(out),
                false => self.overlay = Default::default(),
            }
            out.consumed = true;
            return;
        }
        if !overlay && self.home_key(key, m, out) || chord && self.binding(key, m.shift, out) {
            out.consumed = true;
            return;
        }
        let Some(win) = self.key_target().filter(|_| !is_paste(key, m)) else {
            return;
        };
        let wants = self.host.win(win).is_some_and(|w| w.app.wants_text_input());
        self.host.deliver(win, AppEvent::Key { key, mods: m }, out);
        out.consumed = wants || !is_browser(key, m);
    }

    /// Carries out the binding of mod+`key`, if it is one.
    fn binding(&mut self, key: Key, shift: bool, out: &mut Response) -> bool {
        let focused = self.host.wm().focused();
        let maximized =
            focused.and_then(|w| self.placement(w)).is_some_and(|p| p.state == State::Maximized);
        let snap = |snap| focused.map(|win| Cmd::SnapTo { win, snap });
        let cmd = match (key, shift) {
            (Key::Space | Key::Char('a'), false) => {
                self.toggle_overlay(true);
                None
            }
            (Key::Enter, false) => {
                self.host.open("terminal", None, out);
                None
            }
            (Key::Char('d'), false) => {
                self.show_desktop();
                None
            }
            (Key::Char('q'), false) => focused.map(Cmd::Close),
            (Key::Up, false) => focused.map(Cmd::ToggleMaximize),
            (Key::Down, false) if maximized => focused.map(Cmd::Restore),
            (Key::Down, false) => focused.map(Cmd::Minimize),
            (Key::Left, false) => snap(Snap::Left),
            (Key::Right, false) => snap(Snap::Right),
            (Key::Char('`'), false) => Some(Cmd::FocusNext),
            (Key::Char('`'), true) => Some(Cmd::FocusPrev),
            _ => return false,
        };
        if let Some(cmd) = cmd {
            self.host.apply(cmd);
        }
        true
    }
}
