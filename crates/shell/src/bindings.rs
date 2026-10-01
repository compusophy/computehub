//! Key and pointer bindings: what the wm does, and what goes to an app.

use ui::{AppEvent, Key, Mods, Sense};
use wm::{Cmd, Dir, Rect, WinId};

use crate::theme::WORKSPACES;
use crate::{Response, Shell, Target};

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
    /// A binding for the wm, else a key for the focused app.
    pub(crate) fn key(&mut self, key: Key, m: Mods, out: &mut Response) {
        if (m.alt || m.meta) && self.binding(key, m, out) {
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

    /// Applies a wm binding (Alt or Meta is held); returns whether the
    /// combination is one.
    fn binding(&mut self, key: Key, m: Mods, out: &mut Response) -> bool {
        let focused = self.host.wm().focused();
        let dir = match key {
            Key::Left | Key::Char('h') => Some(Dir::Left),
            Key::Down | Key::Char('j') => Some(Dir::Down),
            Key::Up | Key::Char('k') => Some(Dir::Up),
            Key::Right | Key::Char('l') => Some(Dir::Right),
            _ => None,
        };
        let ws = match key {
            Key::Char(c @ '1'..='9') => Some(c as usize - '1' as usize).filter(|&i| i < WORKSPACES),
            _ => None,
        };
        let px = crate::RESIZE_PX;
        let cmd = match (key, dir, ws, m.shift, m.ctrl) {
            (Key::Enter, _, _, floating, false) => {
                self.host.open("terminal", floating, out);
                None
            }
            (Key::Space, _, _, false, false) => {
                self.host.open("launcher", true, out);
                None
            }
            // The app goes when `settle` finds its window gone.
            (Key::Char('q'), _, _, false, false) => focused.map(Cmd::Close),
            (Key::Char('f'), _, _, false, false) => Some(Cmd::ToggleFloat),
            (Key::Char('o'), _, _, false, false) => Some(Cmd::ToggleOrientation),
            (_, Some(d), _, false, false) => Some(Cmd::FocusDir(d)),
            (_, Some(d), _, true, false) => Some(Cmd::MoveDir(d)),
            (_, Some(dir), _, false, true) => Some(Cmd::Resize { dir, px }),
            (_, _, Some(ws), false, false) => Some(Cmd::SwitchWorkspace(ws)),
            (_, _, Some(ws), true, false) => focused.map(|win| Cmd::MoveToWorkspace { win, ws }),
            _ => return false,
        };
        if let Some(cmd) = cmd {
            self.host.apply(cmd);
        }
        true
    }

    /// Button 0 down: focus a window, arm a button, start a drag, or press
    /// into an app's content.
    pub(crate) fn press(&mut self, out: &mut Response) {
        let (x, y) = self.pointer.unwrap_or_default();
        let hit = self.hit(x, y);
        self.armed = hit.filter(|h| h.is_button());
        (self.drag, self.app_press) = (None, None);
        let Some(win) = hit.and_then(Target::win) else {
            return;
        };
        self.host.apply(Cmd::Focus(win));
        // Focus events reach the apps before the press.
        self.settle(out);
        if let (Some(Target::Title(_)), Some(r)) = (hit, self.float_rect(win)) {
            self.drag = Some((win, x - r.x as f32, y - r.y as f32));
        }
        let content = self.content_of(win).filter(|c| c.contains(x, y));
        if let (Some(Target::Body(_)), Some(c)) = (hit, content) {
            let w = self.widget_at(win, x, y);
            self.app_press = w.map(|h| (win, h.id));
            let (x, y, id) = (x - c.x, y - c.y, w.map(|h| h.id));
            self.host.deliver(win, AppEvent::PointerDown { x, y, id }, out);
        }
    }

    /// Ends any drag and press; button 0 fires the armed button, or clicks
    /// the pressed widget, if still over it.
    pub(crate) fn release(&mut self, primary: bool, out: &mut Response) {
        let (x, y) = self.pointer.unwrap_or_default();
        let over = self.pointer.and_then(|_| self.hit(x, y));
        self.drag = None;
        let press = self.app_press.take();
        match self.armed.take().filter(|&a| primary && over == Some(a)) {
            Some(Target::Launcher) => self.host.open("launcher", true, out),
            Some(Target::Terminal) => self.host.open("terminal", false, out),
            Some(Target::Workspace(ws)) => self.host.apply(Cmd::SwitchWorkspace(ws)),
            Some(Target::Close(win)) => self.host.close(win, out),
            Some(Target::Float(win)) => {
                self.host.apply(Cmd::Focus(win));
                self.host.apply(Cmd::ToggleFloat);
            }
            _ => {}
        }
        if let (true, Some((win, id))) = (primary, press) {
            let hit = self.widget_at(win, x, y);
            let still = hit.is_some_and(|h| h.id == id && h.sense == Sense::Click);
            if still && over == Some(Target::Body(win)) {
                self.host.deliver(win, AppEvent::Click(id), out);
            }
        }
    }

    /// Moves the dragged window with the pointer, its top kept below the panel.
    /// The drag ends if the window is gone, tiled, or on another workspace.
    pub(crate) fn drag_to(&mut self) {
        let (x, y) = self.pointer.unwrap_or_default();
        let found = self.drag.and_then(|d| Some((d, self.float_rect(d.0)?)));
        self.drag = found.map(|f| f.0);
        if let Some(((win, dx, dy), r)) = found {
            let ny = ((y - dy).round() as i32).max(self.host.wm().area().y);
            let rect = Rect::new((x - dx).round() as i32, ny, r.w, r.h);
            self.host.apply(Cmd::SetFloatRect { win, rect });
        }
    }

    /// The wheel goes to the app of the window under the pointer, with the
    /// pointer relative to its content rect.
    pub(crate) fn wheel(&mut self, x: f32, y: f32, dy: f32, out: &mut Response) {
        let win = self.hit(x, y).and_then(Target::win);
        let c = win.and_then(|w| self.content_of(w));
        if let (Some(win), Some(c), true) = (win, c, dy.is_finite()) {
            let (x, y) = (x - c.x, y - c.y);
            self.host.deliver(win, AppEvent::Wheel { x, y, dy }, out);
            out.consumed = true;
        }
    }

    /// The rect of `win` if it floats on the active workspace.
    fn float_rect(&self, win: WinId) -> Option<Rect> {
        let layout = self.host.wm().layout();
        let p = layout.iter().find(|p| p.win == win && p.floating)?;
        Some(p.rect)
    }
}
