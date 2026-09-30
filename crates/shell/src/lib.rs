//! The compusophyOS desktop shell: the panel, window chrome, and the pointer
//! and keyboard bindings.
//!
//! [`Shell`] owns a [`wm::Wm`], changes it only through [`wm::Wm::apply`],
//! and turns platform [`Input`] into [`wm::Cmd`]s and the layout into a
//! [`gfx::DrawList`]. Pure Rust: no browser.
//!
//! # Screen
//!
//! Logical pixels, origin top-left. A panel [`theme::PANEL_H`] tall spans the
//! top, drawn over everything: a launcher, a floating-window button and the
//! workspace indicators. The wm gets `Rect(0, PANEL_H, w, h - PANEL_H)`,
//! rounded and never negative. Non-finite sizes and positions count as 0.
//!
//! # Bindings
//!
//! Key-down events only. `mod` is Alt or Meta; keys without it belong to
//! apps. Directions are the arrows or H/J/K/L (left, down, up, right).
//!
//! | keys | action |
//! |---|---|
//! | mod+Enter, mod+Shift+Enter | open a tiled, a floating window |
//! | mod+Q | close the focused window |
//! | mod+F, mod+O | toggle floating, flip the split's orientation |
//! | mod+direction | focus that way |
//! | mod+Shift+direction | swap the focused tile that way |
//! | mod+Ctrl+direction | move that edge of the focused tile by 40 px |
//! | mod+1..4, mod+Shift+1..4 | switch to, move the focused window to, a workspace |
//!
//! Pointer button 0 focuses the window it presses, drags a floating window
//! by its titlebar (its top kept below the panel), and clicks buttons on
//! release over the button pressed. Other buttons do nothing, but any release
//! ends a drag or press: browsers report only a chord's last release.
//!
//! `consumed` is set for every recognized binding, even one the wm answers
//! with `Noop`, and every pointer move, press and release. `redraw` is set
//! when the wm state or the hovered or pressed button changed, and on resize.
//!
//! ```
//! use shell::{Input, Key, Mods, Shell};
//! let mut desk = Shell::new(1280.0, 800.0);
//! let mods = Mods { alt: true, ..Mods::default() };
//! let r = desk.input(Input::Key { key: Key::Enter, mods });
//! assert!(r.consumed && r.redraw && desk.wm().layout().len() == 4);
//! let mut list = gfx::DrawList::new();
//! desk.draw(&mut list);
//! ```

#![forbid(unsafe_code)]

pub mod theme;

use gfx::{DrawList, Icon, RectF, Rgba};
use theme::*;
use wm::{Cmd, Dir, Gaps, Placement, Rect, WinId, Wm};

// Panel buttons and workspace slots: their side, and the space before the
// first and between each. Titlebar buttons: their side, the close button's
// distance from the right edge, and the space between them.
const PANEL_BTN: f32 = 28.0;
const PANEL_SPACING: f32 = 4.0;
const TITLE_BTN: f32 = 22.0;
const TITLE_MARGIN: f32 = 6.0;
const TITLE_SPACING: f32 = 4.0;
/// Windows narrower than this get no titlebar buttons.
const CHROME_MIN_W: f32 = 2.0 * (TITLE_BTN + TITLE_MARGIN) + TITLE_SPACING;
/// Windows shorter than this get no titlebar, content or buttons.
const CHROME_MIN_H: f32 = TITLEBAR_H + RADIUS;
/// Inset of the content placeholder below the titlebar.
const CONTENT_INSET: f32 = 8.0;
/// Pixels one resize binding moves an edge.
const RESIZE_PX: i32 = 40;

/// Modifier keys held during a key press. `alt` is also Option; `meta` is
/// Command or the Windows key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// The keys the shell can bind; every other key is [`Key::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Left,
    Right,
    Up,
    Down,
    H,
    J,
    K,
    L,
    Q,
    F,
    O,
    /// A digit key, `0..=9`.
    Digit(u8),
    Other,
}

/// One platform event. Positions and sizes are logical pixels; pointer
/// button 0 is the primary button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// A key went down.
    Key {
        key: Key,
        mods: Mods,
    },
    PointerMove {
        x: f32,
        y: f32,
    },
    PointerDown {
        x: f32,
        y: f32,
        button: u8,
    },
    PointerUp {
        x: f32,
        y: f32,
        button: u8,
    },
    /// The pointer left the canvas.
    PointerLeave,
    /// The canvas has a new size.
    Resize {
        w: f32,
        h: f32,
    },
}

/// What the platform should do after an [`Input`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// The screen changed: draw a new frame.
    pub redraw: bool,
    /// The shell used the event: the platform should `preventDefault` it.
    pub consumed: bool,
}

/// What lies under a point: the bare panel, a panel button (launcher or
/// workspace indicator), a titlebar button, a titlebar, or a window body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Panel,
    Launch { floating: bool },
    Workspace(usize),
    Close(WinId),
    Float(WinId),
    Title(WinId),
    Body(WinId),
}

impl Hit {
    fn is_button(self) -> bool {
        !matches!(self, Hit::Panel | Hit::Title(_) | Hit::Body(_))
    }
}

/// The desktop: a window manager plus the panel, chrome and bindings around
/// it.
#[derive(Clone, Debug)]
pub struct Shell {
    wm: Wm,
    /// Screen width and height.
    size: (f32, f32),
    /// Where the pointer is; `None` once it leaves.
    pointer: Option<(f32, f32)>,
    /// The button under the pointer, and the one pressed but not released.
    hover: Option<Hit>,
    armed: Option<Hit>,
    /// The floating window being dragged, and the grab point's offset from
    /// its top-left corner.
    drag: Option<(WinId, f32, f32)>,
}

impl Shell {
    /// A desktop of `w` x `h` logical pixels with three tiled windows open
    /// on workspace 0.
    pub fn new(w: f32, h: f32) -> Shell {
        let size = (coord(w).max(0.0), coord(h).max(0.0));
        let mut gaps = Gaps::default();
        [gaps.outer, gaps.inner] = [GAP; 2];
        let mut shell = Shell {
            wm: Wm::new(wm_area(size), gaps, WORKSPACES),
            size,
            pointer: None,
            hover: None,
            armed: None,
            drag: None,
        };
        (0..3).for_each(|_| shell.apply(Cmd::Open { floating: false }));
        shell
    }

    /// The window manager, read-only: change it through [`Shell::input`].
    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// The frame's clear color, [`theme::BG`].
    pub fn clear_color(&self) -> Rgba {
        BG
    }

    /// Handles one event; see the crate docs for the bindings and rules.
    pub fn input(&mut self, input: Input) -> Response {
        let visuals = |s: &Shell| (s.wm.state_hash(), s.hover, s.armed);
        let before = visuals(self);
        self.pointer = match input {
            Input::PointerMove { x, y }
            | Input::PointerDown { x, y, .. }
            | Input::PointerUp { x, y, .. } => Some((coord(x), coord(y))),
            Input::PointerLeave => None,
            _ => self.pointer,
        };
        let consumed = match input {
            Input::Key { key, mods } => self.key(key, mods),
            Input::PointerMove { .. } => self.drag_to(),
            Input::PointerDown { button: 0, .. } => self.press(),
            Input::PointerUp { button, .. } => self.release(button == 0),
            Input::PointerDown { .. } => true,
            Input::PointerLeave => false,
            Input::Resize { w, h } => {
                self.size = (coord(w).max(0.0), coord(h).max(0.0));
                self.apply(Cmd::SetArea(wm_area(self.size)));
                false
            }
        };
        self.hover = match (self.drag, self.pointer) {
            (None, Some((x, y))) => self.hit(x, y).filter(|h| h.is_button()),
            _ => None,
        };
        let redraw = matches!(input, Input::Resize { .. }) || visuals(self) != before;
        Response { redraw, consumed }
    }

    /// Clears `list`, then draws every window of the active workspace
    /// bottom to top and the panel over them. The desktop background is
    /// [`Shell::clear_color`], not an instance.
    pub fn draw(&self, list: &mut DrawList) {
        list.clear();
        for p in self.wm.layout() {
            self.draw_window(list, &p);
        }
        self.draw_panel(list);
    }

    /// Errors leave the wm untouched, and a stale id is not worth reporting.
    fn apply(&mut self, cmd: Cmd) {
        let _ = self.wm.apply(cmd);
    }

    /// Applies a key binding; returns whether the combination is one.
    fn key(&mut self, key: Key, m: Mods) -> bool {
        let focused = self.wm.focused();
        let dir = match key {
            Key::Left | Key::H => Some(Dir::Left),
            Key::Down | Key::J => Some(Dir::Down),
            Key::Up | Key::K => Some(Dir::Up),
            Key::Right | Key::L => Some(Dir::Right),
            _ => None,
        };
        let ws = match key {
            Key::Digit(n) if (1..=WORKSPACES).contains(&usize::from(n)) => Some(usize::from(n) - 1),
            _ => None,
        };
        let cmd = match (key, dir, ws, m.shift, m.ctrl) {
            _ if !(m.alt || m.meta) => return false,
            (Key::Enter, _, _, floating, false) => Some(Cmd::Open { floating }),
            (Key::Q, _, _, false, false) => focused.map(Cmd::Close),
            (Key::F, _, _, false, false) => Some(Cmd::ToggleFloat),
            (Key::O, _, _, false, false) => Some(Cmd::ToggleOrientation),
            (_, Some(d), _, false, false) => Some(Cmd::FocusDir(d)),
            (_, Some(d), _, true, false) => Some(Cmd::MoveDir(d)),
            (_, Some(dir), _, false, true) => Some(Cmd::Resize { dir, px: RESIZE_PX }),
            (_, _, Some(ws), false, false) => Some(Cmd::SwitchWorkspace(ws)),
            (_, _, Some(ws), true, false) => focused.map(|win| Cmd::MoveToWorkspace { win, ws }),
            _ => return false,
        };
        if let Some(cmd) = cmd {
            self.apply(cmd);
        }
        true
    }

    /// Button 0 down: focus a window, arm a button, or start a drag.
    fn press(&mut self) -> bool {
        let (x, y) = self.pointer.unwrap_or_default();
        let hit = self.hit(x, y);
        self.armed = hit.filter(|h| h.is_button());
        self.drag = None;
        if let Some(Hit::Close(win) | Hit::Float(win) | Hit::Title(win) | Hit::Body(win)) = hit {
            self.apply(Cmd::Focus(win));
            if let (Some(Hit::Title(_)), Some(r)) = (hit, self.float_rect(win)) {
                self.drag = Some((win, x - r.x as f32, y - r.y as f32));
            }
        }
        true
    }

    /// Ends any drag and disarms; button 0 fires the armed button if over it.
    fn release(&mut self, primary: bool) -> bool {
        let over = self.pointer.and_then(|(x, y)| self.hit(x, y));
        self.drag = None;
        match self.armed.take().filter(|&a| primary && over == Some(a)) {
            Some(Hit::Launch { floating }) => self.apply(Cmd::Open { floating }),
            Some(Hit::Workspace(ws)) => self.apply(Cmd::SwitchWorkspace(ws)),
            Some(Hit::Close(win)) => self.apply(Cmd::Close(win)),
            Some(Hit::Float(win)) => {
                self.apply(Cmd::Focus(win));
                self.apply(Cmd::ToggleFloat);
            }
            _ => {}
        }
        true
    }

    /// Moves the dragged window with the pointer, its top kept below the panel.
    /// The drag ends if the window is gone, tiled, or on another workspace.
    fn drag_to(&mut self) -> bool {
        let (x, y) = self.pointer.unwrap_or_default();
        let found = self.drag.and_then(|d| Some((d, self.float_rect(d.0)?)));
        self.drag = found.map(|f| f.0);
        if let Some(((win, dx, dy), r)) = found {
            let ny = ((y - dy).round() as i32).max(self.wm.area().y);
            let rect = Rect::new((x - dx).round() as i32, ny, r.w, r.h);
            self.apply(Cmd::SetFloatRect { win, rect });
        }
        true
    }

    /// The rect of `win` if it floats on the active workspace.
    fn float_rect(&self, win: WinId) -> Option<Rect> {
        let layout = self.wm.layout();
        let p = layout.iter().find(|p| p.win == win && p.floating)?;
        Some(p.rect)
    }

    /// What is under `(x, y)`: the panel first, then windows top to bottom.
    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if (0.0..PANEL_H).contains(&y) {
            let targets = self.panel_targets();
            let on = targets.iter().find(|t| t.0.contains(x, y));
            return Some(on.map_or(Hit::Panel, |t| t.1));
        }
        let layout = self.wm.layout();
        let p = layout.iter().rev().find(|p| rectf(p.rect).contains(x, y))?;
        let r = rectf(p.rect);
        let mut buttons = title_buttons(r, p.win).into_iter().flatten();
        Some(match buttons.find(|b| b.0.contains(x, y)) {
            Some((_, hit)) => hit,
            None if y < r.y + TITLEBAR_H => Hit::Title(p.win),
            None => Hit::Body(p.win),
        })
    }

    /// The panel's buttons: launcher and floating launcher at the left, then
    /// one slot per workspace, centered.
    fn panel_targets(&self) -> Vec<(RectF, Hit)> {
        let step = PANEL_BTN + PANEL_SPACING;
        let slot = |x| RectF::new(x, (PANEL_H - PANEL_BTN) / 2.0, PANEL_BTN, PANEL_BTN);
        let mut out = vec![
            (slot(PANEL_SPACING), Hit::Launch { floating: false }),
            (slot(PANEL_SPACING + step), Hit::Launch { floating: true }),
        ];
        let n = self.wm.workspace_count();
        let x0 = ((self.size.0 - (n as f32 * step - PANEL_SPACING)) / 2.0).round();
        out.extend((0..n).map(|i| (slot(x0 + i as f32 * step), Hit::Workspace(i))));
        out
    }

    /// Shadow, body, titlebar, content placeholder, border, then buttons.
    fn draw_window(&self, list: &mut DrawList, p: &Placement) {
        let r = rectf(p.rect);
        let (blur, alpha, drop) = if p.floating {
            (SHADOW_BLUR * 1.6, SHADOW.3.saturating_add(50), 6.0)
        } else {
            (SHADOW_BLUR, SHADOW.3, 2.0)
        };
        let shadow = RectF { y: r.y + drop, ..r };
        list.shadow(shadow, RADIUS, blur, SHADOW.with_alpha(alpha));
        list.fill(r, RADIUS, WINDOW);
        if r.h >= CHROME_MIN_H {
            let bar = [TITLEBAR, TITLEBAR_FOCUSED][usize::from(p.focused)];
            list.fill(RectF { h: TITLEBAR_H, ..r }, RADIUS, bar);
            // Square the strip's lower corners where it meets the body.
            let half = TITLEBAR_H / 2.0;
            list.fill(RectF::new(r.x, r.y + half, r.w, half), 0.0, bar);
            let below = RectF::new(r.x, r.y + TITLEBAR_H, r.w, r.h - TITLEBAR_H);
            let tint = app_tint(p.win.0).with_alpha(44);
            list.fill(below.inset(CONTENT_INSET), RADIUS - 4.0, tint);
        }
        let width = if p.focused { 2.0 } else { 1.5 };
        let color = if p.focused { BORDER_FOCUSED } else { BORDER };
        list.border(r, RADIUS, width, color);
        let ink = if p.focused { ICON } else { ICON_DIM };
        for (b, hit) in title_buttons(r, p.win).into_iter().flatten() {
            self.highlight(list, b, hit, TITLE_BTN / 2.0);
            let icon = match hit {
                Hit::Close(_) => Icon::Cross,
                _ => Icon::Square,
            };
            list.icon(b.inset(4.0), icon, 1.5, ink);
        }
    }

    /// The bar, its buttons, and the workspace indicators: the active one an
    /// accent pill, the rest dots, brighter where windows are open.
    fn draw_panel(&self, list: &mut DrawList) {
        let w = self.size.0;
        list.fill(RectF::new(0.0, 0.0, w, PANEL_H), 0.0, PANEL);
        list.fill(RectF::new(0.0, PANEL_H - 1.0, w, 1.0), 0.0, BORDER);
        let active = self.wm.active_workspace();
        for (r, hit) in self.panel_targets() {
            self.highlight(list, r, hit, 8.0);
            match hit {
                Hit::Launch { floating } => {
                    let icon = if floating { Icon::Square } else { Icon::Grid };
                    list.icon(r.inset(6.0), icon, 1.5, ICON);
                }
                Hit::Workspace(i) => {
                    let empty = self.wm.layout_of(i).is_empty();
                    let dim = ICON_DIM.with_alpha(if empty { 150 } else { 255 });
                    let dot_w = if i == active { 20.0 } else { 8.0 };
                    let color = if i == active { ACCENT } else { dim };
                    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    let dot = RectF::new(cx - dot_w / 2.0, cy - 4.0, dot_w, 8.0);
                    list.fill(dot, 4.0, color);
                }
                _ => {}
            }
        }
    }

    /// The hover wash behind a button, doubled while it is pressed.
    fn highlight(&self, list: &mut DrawList, r: RectF, hit: Hit, radius: f32) {
        if self.hover == Some(hit) {
            let pressed = u8::from(self.armed == Some(hit));
            let a = HOVER.3.saturating_mul(1 + pressed);
            list.fill(r, radius, HOVER.with_alpha(a));
        }
    }
}

/// A finite coordinate within the wm's range; NaN and infinities become 0.
fn coord(v: f32) -> f32 {
    let max = wm::MAX_COORD as f32;
    let v = if v.is_finite() { v } else { 0.0 };
    v.clamp(-max, max)
}

/// The wm's area for a screen of `(w, h)`: everything below the panel.
fn wm_area((w, h): (f32, f32)) -> Rect {
    let h = (h - PANEL_H).max(0.0);
    Rect::new(0, PANEL_H as i32, w.round() as i32, h.round() as i32)
}

fn rectf(r: Rect) -> RectF {
    RectF::from_i32(r.x, r.y, r.w, r.h)
}

/// A window's float and close buttons, or `None` if it is too small.
fn title_buttons(r: RectF, win: WinId) -> Option<[(RectF, Hit); 2]> {
    if r.w < CHROME_MIN_W || r.h < CHROME_MIN_H {
        return None;
    }
    let y = r.y + (TITLEBAR_H - TITLE_BTN) / 2.0;
    let x = r.x + r.w - TITLE_MARGIN - TITLE_BTN;
    let close = RectF::new(x, y, TITLE_BTN, TITLE_BTN);
    let float = RectF::new(x - TITLE_SPACING - TITLE_BTN, y, TITLE_BTN, TITLE_BTN);
    Some([(float, Hit::Float(win)), (close, Hit::Close(win))])
}

#[cfg(test)]
mod tests;
