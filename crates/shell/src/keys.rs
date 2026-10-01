//! Keys: the desktop's bindings, the launcher's keys, and the rest for the
//! focused app.

use ui::{AppEvent, Key, Mods};
use wm::{Cmd, Snap, State};

use crate::{Response, Shell};

/// Ctrl+V, Ctrl+Shift+V, Meta+V and Shift+Insert: the platform leaves them
/// to the browser, which pastes; the text comes as [`crate::Input::Text`].
fn is_paste(key: Key, m: Mods) -> bool {
    let ctrl = m.ctrl && !m.alt && !m.meta;
    let meta = m.meta && !m.ctrl && !m.alt && !m.shift;
    let shift = m.shift && !m.ctrl && !m.alt && !m.meta;
    (key == Key::Char('v') && (ctrl || meta)) || (key == Key::Insert && shift)
}

/// F5, F12, Ctrl+R, Ctrl+Shift+R and Ctrl+Shift+I: reload and the developer
/// tools, left to the browser unless the app wants text input.
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
    /// A binding, else a key for the launcher while it shows, else for the
    /// focused app.
    pub(crate) fn key(&mut self, key: Key, m: Mods, out: &mut Response) {
        let chord = (m.alt || m.meta) && !m.ctrl;
        if self.launcher.open {
            match key {
                Key::Space if chord && !m.shift => self.hide_launcher(),
                Key::Enter => self.launch(self.launcher.search.sel, out),
                Key::Escape => self.hide_launcher(),
                _ => _ = self.launcher.search.key(key, self.panel().cols()),
            }
            out.consumed = !is_paste(key, m);
            return;
        }
        if chord && self.binding(key, m.shift, out) {
            out.consumed = true;
            return;
        }
        let Some(win) = self.host.focused_app().filter(|_| !is_paste(key, m)) else {
            return;
        };
        let wants = self.host.win(win).is_some_and(|w| w.app.wants_text_input());
        self.host.deliver(win, AppEvent::Key { key, mods: m }, out);
        out.consumed = wants || !is_browser(key, m);
    }

    /// Carries out a binding (Alt or Meta is held, Ctrl is not); returns
    /// whether the combination is one.
    fn binding(&mut self, key: Key, shift: bool, out: &mut Response) -> bool {
        let focused = self.host.wm().focused();
        let maximized =
            focused.and_then(|w| self.placement(w)).is_some_and(|p| p.state == State::Maximized);
        let snap = |snap| focused.map(|win| Cmd::SnapTo { win, snap });
        let cmd = match (key, shift) {
            (Key::Space, false) => {
                self.toggle_launcher();
                None
            }
            (Key::Enter, false) => {
                self.host.open("terminal", None, out);
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
