//! Motion: where each window, dock tile and the snap preview are heading (the home screen's
//! icons: `home::grid`), when tweens start, and whether anything still moves (or must be
//! watched).

use host::grab::Grab;
use host::motion::{Tween, Vis, docked};
use host::rectf;
use wm::WinId;

use crate::Shell;
use crate::desktop::Target;

/// Durations (`MOVE_MS`: maximize, restore, snap); the scale windows open from.
const OPEN_MS: f32 = 180.0;
const CLOSE_MS: f32 = 140.0;
const DOCK_MS: f32 = 220.0;
const MOVE_MS: f32 = 200.0;
const LIFT_MS: f32 = 120.0;
const SLIDE_MS: f32 = 180.0;
const PREVIEW_MS: f32 = 160.0;
const OPEN_SCALE: f32 = 0.96;

/// The tweens of every window (closing and minimized too), each dock tile's lift (0 to 1) and
/// place (by app name), the Assistant's lift and the top bar's tooltip, and the snap preview.
#[derive(Default)]
pub(crate) struct Motion {
    pub wins: Vec<(WinId, Tween<Vis>)>,
    pub tiles: Vec<(String, Tween<f32>, Tween<f32>)>,
    pub ai: Tween<f32>,
    pub bar: Tween<f32>,
    pub preview: Tween<Vis>,
}

impl Motion {
    /// How lifted the dock tile of `name` is at `now`, and where its left edge shows.
    pub(crate) fn tile(&self, name: &str, now: f64) -> Option<(f32, f32)> {
        let t = self.tiles.iter().find(|t| t.0 == name)?;
        Some((t.1.value(now), t.2.value(now)))
    }
}

impl Shell {
    /// Points every tween where its part should be (`instant`: at once), and
    /// reaps closed windows that faded out.
    pub(crate) fn sync(&mut self) {
        let (now, layout) = (self.host.now_ms, self.host.wm().layout());
        let wins: Vec<(WinId, bool)> =
            self.host.wins.iter().map(|w| (w.id, !self.host.live(w.id))).collect();
        for (win, closing) in wins {
            let shown = layout.iter().find(|p| p.win == win).map(|p| Vis::at(rectf(p.rect)));
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
                    let i = self.tiles.iter().position(|d| d.2.contains(&win));
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
        self.sync_tiles(now);
        self.sync_preview(now);
        self.grid.sync(now, self.instant);
    }

    /// Each dock tile's lift, the Assistant's and the top bar's tooltip head up while hovered,
    /// else down; each tile slides to its place (a new one shows there; on a new screen size, all
    /// at once).
    fn sync_tiles(&mut self, now: f64) {
        let hovered = match self.hover {
            Some(Target::Dock(i)) => self.tiles.get(i).map(|d| d.0.as_str()),
            _ => None,
        };
        let old = std::mem::take(&mut self.motion.tiles);
        for (i, (name, ..)) in self.tiles.iter().enumerate() {
            let to = self.dock_tile(i).x;
            let was = old.iter().find(|t| &t.0 == name);
            let (mut lift, mut at) = was.map_or((Tween::new(0.0), Tween::new(to)), |t| (t.1, t.2));
            lift.to(f32::from(u8::from(hovered == Some(name.as_str()))), now, LIFT_MS);
            match self.instant {
                true => at = Tween::new(to),
                false => at.to(to, now, SLIDE_MS),
            }
            self.motion.tiles.push((name.clone(), lift, at));
        }
        let ai = f32::from(u8::from(self.hover == Some(Target::Assistant)));
        self.motion.ai.to(ai, now, LIFT_MS);
        let bar = f32::from(u8::from(matches!(self.hover, Some(Target::Bar(_)))));
        self.motion.bar.to(bar, now, LIFT_MS);
    }

    fn sync_preview(&mut self, now: f64) {
        let (area, m) = (self.host.wm().area(), &mut self.motion);
        let Some(Grab::Move { win, zone: Some(z), .. }) = self.grab else {
            let rest = m.preview.target();
            return m.preview.to(Vis { a: 0.0, ..rest }, now, PREVIEW_MS);
        };
        let rect = rectf(z.rect(area));
        // A preview that was hidden grows out of the window.
        if m.preview.target().a <= 0.0 && m.preview.value(now).a <= 0.0 {
            let from = m.wins.iter().find(|w| w.0 == win).map_or(rect, |w| w.1.value(now).rect);
            m.preview = Tween::new(Vis { a: 0.0, ..Vis::at(from) });
        }
        m.preview.to(Vis::at(rect), now, PREVIEW_MS);
    }

    /// Starts every pending tween as a frame begins.
    pub(crate) fn arm(&mut self, now: f64) {
        let m = &mut self.motion;
        m.wins.iter_mut().for_each(|w| w.1.arm(now));
        for t in &mut m.tiles {
            t.1.arm(now);
            t.2.arm(now);
        }
        self.grid.arm(now);
        m.ai.arm(now);
        m.bar.arm(now);
        m.preview.arm(now);
        self.host.theme.arm(now);
    }

    /// Whether frames must come: something moves, flings, a finger waits to long-press, or a
    /// shown app animates.
    pub(crate) fn animating(&self) -> bool {
        self.frame_in() == Some(0)
    }

    /// When the next frame is wanted, in ms: at once (0) while frames must come; else when the
    /// living grain's next pattern or a shown app's timer wants one ([`ui::App::frame_in`]);
    /// `None`, not till an event.
    pub fn frame_in(&self) -> Option<u32> {
        let (now, m) = (self.host.now_ms, &self.motion);
        if m.wins.iter().any(|w| w.1.is_running(now))
            || m.tiles.iter().any(|t| t.1.is_running(now) || t.2.is_running(now))
            || self.grid.moving(now)
            || m.ai.is_running(now)
            || m.bar.is_running(now)
            || m.preview.is_running(now)
            || self.host.theme.is_running(now)
            || self.touch.is_some_and(|t| !t.0.done)
            || self.fling.is_some()
            || self.agent_moves(now)
        {
            return Some(0);
        }
        let mut next = self.grain_in();
        for p in self.host.wm().layout() {
            if let Some(ms) = self.host.win(p.win).and_then(|w| w.app.frame_in(now)) {
                next = Some(next.map_or(ms, |n| n.min(ms)));
            }
        }
        next
    }
}
