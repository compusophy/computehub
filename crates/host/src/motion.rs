//! Motion: tweens along one ease-out curve, how a window shows while it moves
//! ([`Vis`]), draw-list replays scaled and faded, theme crossfades. A tween
//! waits for the next frame to arm it, so every animation's first frame moves.

use gfx::{DrawList, Icon, Kind, RectF, Rgba};
use ui::theme::{Glow, THEMES, Theme, mix};

use crate::paint::faded;

/// Values that move `t` (0 to 1) of the way to another.
pub trait Lerp: Copy + PartialEq {
    fn lerp(self, to: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, to: f32, t: f32) -> f32 {
        self * (1.0 - t) + to * t
    }
}

/// The ease-out curve: CSS `cubic-bezier(0.2, 0.8, 0.2, 1)`; 0 at `t <= 0` or NaN, 1 at `t >= 1`.
pub fn ease(t: f32) -> f32 {
    if t.is_nan() || t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let bezier = |u: f32, p1: f32, p2: f32| {
        let v = 1.0 - u;
        3.0 * v * u * (v * p1 + u * p2) + u * u * u
    };
    // x(u) rises from 0 to 1, so halving finds the u where it reaches t.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, 0.2, 0.2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, 0.8, 1.0)
}

/// A value moving to a target along [`ease`] over a duration.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tween<T> {
    from: T,
    to: T,
    /// When it started, on the page clock; `None` until armed.
    start: Option<f64>,
    dur: f32,
}

impl<T: Lerp> Tween<T> {
    pub fn new(v: T) -> Tween<T> {
        Tween { from: v, to: v, start: Some(0.0), dur: 0.0 }
    }

    /// How far along it is at `now`, 0 to 1 (0 while pending).
    fn progress(&self, now: f64) -> f32 {
        match self.start {
            _ if self.dur <= 0.0 => 1.0,
            None => 0.0,
            Some(s) => ((now - s) / f64::from(self.dur)).clamp(0.0, 1.0) as f32,
        }
    }

    pub fn value(&self, now: f64) -> T {
        match self.progress(now) {
            p if p >= 1.0 => self.to,
            p => self.from.lerp(self.to, ease(p)),
        }
    }

    pub fn target(&self) -> T {
        self.to
    }

    /// Heads for `to` from where it is at `now`, over `dur_ms`, from the next [`Tween::arm`].
    pub fn to(&mut self, to: T, now: f64, dur_ms: f32) {
        if to != self.to {
            (self.from, self.to, self.start, self.dur) = (self.value(now), to, None, dur_ms);
        }
    }

    /// Follows a target under the pointer: a running tween keeps its course
    /// but ends at `to`; one at rest jumps there.
    pub fn chase(&mut self, to: T, now: f64) {
        if !self.is_running(now) {
            self.from = to;
        }
        self.to = to;
    }

    pub fn arm(&mut self, now: f64) {
        self.start.get_or_insert(now);
    }

    /// Whether it is still moving at `now` (pending counts).
    pub fn is_running(&self, now: f64) -> bool {
        self.progress(now) < 1.0
    }
}

/// How a layer shows: drawn at `rect`, scaled by `s` about its center,
/// moved by `(dx, dy)`, at opacity `a`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vis {
    pub rect: RectF,
    pub s: f32,
    pub dx: f32,
    pub dy: f32,
    pub a: f32,
}

impl Vis {
    pub fn at(rect: RectF) -> Vis {
        Vis { rect, s: 1.0, dx: 0.0, dy: 0.0, a: 1.0 }
    }

    pub fn is_plain(&self) -> bool {
        (self.s, self.dx, self.dy, self.a) == (1.0, 0.0, 0.0, 1.0)
    }
}

/// A window at `base` shrunk into the dock tile `tile`, clear.
pub fn docked(base: RectF, tile: RectF) -> Vis {
    let dx = tile.x + tile.w / 2.0 - base.x - base.w / 2.0;
    let dy = tile.y + tile.h / 2.0 - base.y - base.h / 2.0;
    Vis { rect: base, s: tile.w / base.w.max(tile.w), dx, dy, a: 0.0 }
}

impl Lerp for Vis {
    /// Fading out lags the motion (`t` squared): what leaves stays seen.
    fn lerp(self, to: Vis, t: f32) -> Vis {
        let l = |a: f32, b: f32| a.lerp(b, t);
        let (r, q) = (self.rect, to.rect);
        let rect = RectF::new(l(r.x, q.x), l(r.y, q.y), l(r.w, q.w), l(r.h, q.h));
        let (s, dx, dy) = (l(self.s, to.s), l(self.dx, to.dx), l(self.dy, to.dy));
        Vis { rect, s, dx, dy, a: self.a.lerp(to.a, if to.a < self.a { t * t } else { t }) }
    }
}

/// Draws `src` into `dst` as `v` shows what was drawn at `v.rect`.
pub fn replay(dst: &mut DrawList, src: &DrawList, v: Vis) {
    const ICONS: [Icon; 6] =
        [Icon::Plus, Icon::Cross, Icon::Minus, Icon::Dot, Icon::Square, Icon::Grid];
    use Kind::{Border, Fill, Glow, Glyph, Gradient, Grain, Shadow};
    const KINDS: [Kind; 8] = [Fill, Border, Shadow, Kind::Icon, Glyph, Gradient, Glow, Grain];
    let (ox, oy, s) = (v.rect.x + v.rect.w / 2.0, v.rect.y + v.rect.h / 2.0, v.s);
    let rect = |[x, y, w, h]: [f32; 4]| {
        RectF::new(ox + (x - ox) * s + v.dx, oy + (y - oy) * s + v.dy, w * s, h * s)
    };
    for i in src.instances() {
        let Some(&kind) = KINDS.get(i.kind as usize) else { continue };
        let (r, c, radius) = (rect(i.rect), faded(i.color, v.a), i.radius * s);
        dst.push_clip(rect(i.clip));
        match kind {
            Fill => dst.fill(r, radius, c),
            Border => dst.border(r, radius, i.p0 * s, c),
            Shadow => dst.shadow(r, radius, i.p0 * s, c),
            Kind::Icon => dst.icon(r, ICONS[(i.p0 as usize).min(5)], i.p1 * s, c),
            Glyph => dst.glyph(r, RectF::new(i.uv[0], i.uv[1], i.uv[2], i.uv[3]), c),
            Gradient => dst.gradient(r, radius, c, faded(i.color2, v.a), i.p0),
            Glow => dst.glow(r, c),
            Grain => dst.grain(r, c.3, i.p0),
        }
        dst.pop_clip();
    }
}

/// `a` crossfaded `t` of the way to `b`: colors mixed, lights moved (or, on
/// one side only, faded in place); the name and darkness are `b`'s.
#[rustfmt::skip]
pub fn blend(a: &Theme, b: &Theme, t: f32) -> Theme {
    let m = |x: Rgba, y: Rgba| mix(x, y, t);
    let glow = |g: Glow, h: Glow| {
        let (g, h) = match (g.color.3, h.color.3) {
            (0, _) => (Glow { color: h.color.with_alpha(0), ..h }, h),
            (_, 0) => (g, Glow { color: g.color.with_alpha(0), ..g }),
            _ => (g, h),
        };
        let l = |x: f32, y: f32| x.lerp(y, t);
        let color = m(g.color, h.color);
        Glow { cx: l(g.cx, h.cx), cy: l(g.cy, h.cy), rx: l(g.rx, h.rx), ry: l(g.ry, h.ry), color }
    };
    Theme {
        name: b.name, dark: b.dark, base: m(a.base, b.base),
        glows: [0, 1, 2, 3].map(|i| glow(a.glows[i], b.glows[i])),
        grain: f32::from(a.grain).lerp(f32::from(b.grain), t).round() as u8,
        surface: m(a.surface, b.surface), surface_hi: m(a.surface_hi, b.surface_hi),
        surface_lo: m(a.surface_lo, b.surface_lo), glass: m(a.glass, b.glass),
        border: m(a.border, b.border), highlight: m(a.highlight, b.highlight),
        text: m(a.text, b.text), text_dim: m(a.text_dim, b.text_dim),
        text_faint: m(a.text_faint, b.text_faint), accent: m(a.accent, b.accent),
        accent_text: m(a.accent_text, b.accent_text), danger: m(a.danger, b.danger),
        shadow: m(a.shadow, b.shadow), selection: m(a.selection, b.selection),
        ansi: std::array::from_fn(|i| m(a.ansi[i], b.ansi[i])),
    }
}

/// The desktop's theme, and the crossfade into it from the last.
#[derive(Clone, Debug)]
pub struct Themes {
    current: &'static Theme,
    fade: Option<(Theme, Tween<f32>)>,
}

impl Themes {
    pub fn new(name: &str) -> Themes {
        Themes { current: ui::theme(name), fade: None }
    }

    pub fn current(&self) -> &'static Theme {
        self.current
    }

    /// Switches to another theme named `name` (any ASCII case), fading from
    /// what shows at `now` over `ms`; whether it did.
    pub fn set(&mut self, name: &str, now: f64, ms: f32) -> bool {
        let next = ui::theme(name);
        if !next.name.eq_ignore_ascii_case(name) || next.name == self.current.name {
            return false;
        }
        let mut t = Tween::new(0.0);
        t.to(1.0, now, ms);
        (self.fade, self.current) = (Some((self.at(now), t)), next);
        true
    }

    pub fn next(&self) -> &'static str {
        let i = THEMES.iter().position(|t| t.name == self.current.name).unwrap_or(0);
        THEMES[(i + 1) % THEMES.len()].name
    }

    /// What shows at `now`.
    pub fn at(&self, now: f64) -> Theme {
        match &self.fade {
            Some((from, t)) if t.is_running(now) => blend(from, self.current, t.value(now)),
            _ => *self.current,
        }
    }

    pub fn arm(&mut self, now: f64) {
        if let Some(f) = &mut self.fade {
            f.1.arm(now);
        }
    }

    pub fn is_running(&self, now: f64) -> bool {
        self.fade.as_ref().is_some_and(|f| f.1.is_running(now))
    }
}
