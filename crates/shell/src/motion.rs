//! Motion: where each window, dock tile and the snap preview are heading,
//! when tweens start, and whether anything still moves.

use gfx::RectF;
use host::motion::{Tween, Vis, docked};
use host::rectf;
use wm::WinId;

use crate::Shell;
use crate::desktop::{Grab, Target};

const OPEN_MS: f32 = 180.0;
const CLOSE_MS: f32 = 140.0;
/// Minimizing into the dock, and back.
const DOCK_MS: f32 = 220.0;
/// Maximizing, restoring, snapping.
const MOVE_MS: f32 = 200.0;
const LIFT_MS: f32 = 120.0;
const PREVIEW_MS: f32 = 160.0;
/// The scale windows open from and close to.
const OPEN_SCALE: f32 = 0.96;

/// The tweens of the desktop's moving parts.
pub(crate) struct Motion {
    /// Each window's [`Vis`], closing and minimized ones too.
    pub wins: Vec<(WinId, Tween<Vis>)>,
    /// Each dock tile's lift, 0 to 1, by app name.
    pub lifts: Vec<(String, Tween<f32>)>,
    /// The snap preview.
    pub preview: Tween<Vis>,
}

impl Default for Motion {
    fn default() -> Motion {
        let hidden = Vis { a: 0.0, ..Vis::at(RectF::default()) };
        Motion { wins: Vec::new(), lifts: Vec::new(), preview: Tween::new(hidden) }
    }
}

impl Motion {
    /// How lifted the dock tile of `name` is at `now`.
    pub(crate) fn lift(&self, name: &str, now: f64) -> f32 {
        self.lifts.iter().find(|l| l.0 == name).map_or(0.0, |l| l.1.value(now))
    }
}

impl Shell {
    /// Points every tween at where its part should be now: windows at their
    /// placements (opening from 96% and clear, closing back to it, flying
    /// into their dock tile while minimized), dock tiles lifted while
    /// hovered, the snap preview over the drop zone. While a window follows
    /// the pointer (`instant`), windows chase rather than restart. Closed
    /// windows whose animation ended are reaped.
    pub(crate) fn sync(&mut self) {
        let now = self.now();
        let layout = self.host.wm().layout();
        let wins: Vec<(WinId, Option<Vis>, bool, String)> = (self.host.wins().iter())
            .map(|w| {
                let shown = layout.iter().find(|p| p.win == w.id).map(|p| Vis::at(rectf(p.rect)));
                (w.id, shown, w.closing(), w.name.clone())
            })
            .collect();
        for (win, shown, closing, name) in wins {
            let Some(i) = self.motion.wins.iter().position(|e| e.0 == win) else {
                if let Some(v) = shown {
                    let mut t = Tween::new(Vis { s: OPEN_SCALE, a: 0.0, ..v });
                    t.to(v, now, OPEN_MS);
                    self.motion.wins.push((win, t));
                }
                continue;
            };
            let (cur, rest) = (self.motion.wins[i].1.value(now), self.motion.wins[i].1.target());
            let (to, ms) = match shown {
                _ if closing => (Vis { s: OPEN_SCALE, a: 0.0, ..rest }, CLOSE_MS),
                Some(v) if cur.a < 1.0 => (v, DOCK_MS),
                Some(v) => (v, MOVE_MS),
                None => {
                    let i = self.dock.iter().position(|d| d.0 == name);
                    (docked(rest.rect, i.map_or(self.dock_rect(), |i| self.dock_tile(i))), DOCK_MS)
                }
            };
            let t = &mut self.motion.wins[i].1;
            match () {
                _ if cur.a <= 0.0 && to.a <= 0.0 => *t = Tween::new(to),
                _ if self.instant && to.a > 0.0 => t.chase(to, now),
                _ => t.to(to, now, ms),
            }
            if closing && !t.is_running(now) {
                self.host.reap(win);
            }
        }
        let host = &self.host;
        self.motion.wins.retain(|e| host.win(e.0).is_some());
        self.sync_lifts(now);
        self.sync_preview(now);
    }

    fn sync_lifts(&mut self, now: f64) {
        let hovered = match self.hover {
            Some(Target::Dock(i)) => self.dock.get(i).map(|d| d.0.as_str()),
            _ => None,
        };
        let (dock, lifts) = (&self.dock, &mut self.motion.lifts);
        lifts.retain(|l| dock.iter().any(|d| d.0 == l.0));
        for d in dock {
            if !lifts.iter().any(|l| l.0 == d.0) {
                lifts.push((d.0.clone(), Tween::new(0.0)));
            }
        }
        for (name, t) in lifts.iter_mut() {
            t.to(f32::from(u8::from(hovered == Some(name.as_str()))), now, LIFT_MS);
        }
    }

    fn sync_preview(&mut self, now: f64) {
        let target = match self.grab {
            Some(Grab::Move { win, zone: Some(z), .. }) => {
                Some((win, rectf(z.rect(self.host.wm().area()))))
            }
            _ => None,
        };
        let m = &mut self.motion;
        let Some((win, rect)) = target else {
            let rest = m.preview.target();
            return m.preview.to(Vis { a: 0.0, ..rest }, now, PREVIEW_MS);
        };
        // A preview that was hidden grows out of the window.
        if m.preview.target().a <= 0.0 && m.preview.value(now).a <= 0.0 {
            let from = m.wins.iter().find(|w| w.0 == win).map_or(rect, |w| w.1.value(now).rect);
            m.preview = Tween::new(Vis { a: 0.0, ..Vis::at(from) });
        }
        m.preview.to(Vis::at(rect), now, PREVIEW_MS);
    }

    /// Starts every pending tween at `now`: called as a frame begins.
    pub(crate) fn arm(&mut self, now: f64) {
        let m = &mut self.motion;
        m.wins.iter_mut().for_each(|w| w.1.arm(now));
        m.lifts.iter_mut().for_each(|l| l.1.arm(now));
        m.preview.arm(now);
        self.launcher.t.arm(now);
        self.theme.arm(now);
    }

    /// Whether anything still moves (or waits for its first frame), so
    /// frames are wanted.
    pub fn animating(&self) -> bool {
        let now = self.now();
        let m = &self.motion;
        m.wins.iter().any(|w| w.1.is_running(now))
            || m.lifts.iter().any(|l| l.1.is_running(now))
            || m.preview.is_running(now)
            || self.launcher.t.is_running(now)
            || self.theme.is_running(now)
    }
}
