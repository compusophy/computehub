//! Motion: values that tween along the desktop's one ease-out curve, how a
//! window is shown while it moves ([`Vis`]), replaying a draw list scaled,
//! moved and faded ([`replay`]), and crossfading themes ([`blend`],
//! [`Themes`]).
//!
//! Tweens start lazily: a new target waits ([`Tween::is_pending`]) until
//! the next frame arms it ([`Tween::arm`]), so the first frame of every
//! animation shows its first step, however long after the input it comes.

use gfx::{DrawList, Icon, Kind, RectF, Rgba};
use ui::theme::{Glow, THEMES, Theme, mix};

/// Values that interpolate.
pub trait Lerp: Copy + PartialEq {
    /// `self` moved `t` (0 to 1) of the way to `to`.
    fn lerp(self, to: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, to: f32, t: f32) -> f32 {
        self * (1.0 - t) + to * t
    }
}

impl Lerp for RectF {
    fn lerp(self, to: RectF, t: f32) -> RectF {
        let l = |a: f32, b: f32| a.lerp(b, t);
        RectF::new(l(self.x, to.x), l(self.y, to.y), l(self.w, to.w), l(self.h, to.h))
    }
}

/// The ease-out curve every animation follows: CSS
/// `cubic-bezier(0.2, 0.8, 0.2, 1)`, from 0 at `t <= 0` (and NaN) to 1 at
/// `t >= 1`.
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween<T> {
    from: T,
    to: T,
    /// When it started, on the page clock; `None` until armed.
    start: Option<f64>,
    dur: f32,
}

impl<T: Lerp> Tween<T> {
    /// At rest at `v`.
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

    /// The value at `now`.
    pub fn value(&self, now: f64) -> T {
        match self.progress(now) {
            p if p >= 1.0 => self.to,
            p => self.from.lerp(self.to, ease(p)),
        }
    }

    /// Where it is going.
    pub fn target(&self) -> T {
        self.to
    }

    /// Heads for `to` from wherever it is at `now`, over `dur_ms`, starting
    /// at the next [`Tween::arm`]. Nothing changes if `to` is already the
    /// target.
    pub fn to(&mut self, to: T, now: f64, dur_ms: f32) {
        if to != self.to {
            (self.from, self.to, self.start, self.dur) = (self.value(now), to, None, dur_ms);
        }
    }

    /// Follows a target that moves under the pointer: a running tween keeps
    /// its course and timing but ends at `to`; one at rest jumps there.
    pub fn chase(&mut self, to: T, now: f64) {
        if !self.is_running(now) {
            self.from = to;
        }
        self.to = to;
    }

    /// Starts a pending tween at `now`.
    pub fn arm(&mut self, now: f64) {
        self.start.get_or_insert(now);
    }

    /// Whether it waits for [`Tween::arm`].
    pub fn is_pending(&self) -> bool {
        self.start.is_none() && self.dur > 0.0
    }

    /// Whether it is still moving at `now` (pending counts).
    pub fn is_running(&self, now: f64) -> bool {
        self.progress(now) < 1.0
    }
}

/// How a window (or a ghost of one) is shown: drawn at `rect`, then scaled
/// by `s` about the rect's center, moved by `(dx, dy)` and drawn at opacity
/// `a`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vis {
    /// Where it is laid out.
    pub rect: RectF,
    /// Its scale about the rect's center.
    pub s: f32,
    /// How far it is moved right.
    pub dx: f32,
    /// How far it is moved down.
    pub dy: f32,
    /// Its opacity, 0 to 1.
    pub a: f32,
}

impl Vis {
    /// Plainly at `rect`: no scale, move or fade.
    pub fn at(rect: RectF) -> Vis {
        Vis { rect, s: 1.0, dx: 0.0, dy: 0.0, a: 1.0 }
    }

    /// Whether it draws plainly at its rect.
    pub fn is_plain(&self) -> bool {
        (self.s, self.dx, self.dy, self.a) == (1.0, 0.0, 0.0, 1.0)
    }

    /// The transform that shows what was drawn at `rect` this way.
    pub fn xform(&self) -> Xform {
        let (ox, oy) = (self.rect.x + self.rect.w / 2.0, self.rect.y + self.rect.h / 2.0);
        Xform { ox, oy, s: self.s, dx: self.dx, dy: self.dy, alpha: self.a }
    }
}

/// How a window laid out at `base` hides in a dock tile `tile`: shrunk
/// into it, clear.
pub fn docked(base: RectF, tile: RectF) -> Vis {
    let (dx, dy) = (
        tile.x + tile.w / 2.0 - base.x - base.w / 2.0,
        tile.y + tile.h / 2.0 - base.y - base.h / 2.0,
    );
    Vis { rect: base, s: tile.w / base.w.max(tile.w), dx, dy, a: 0.0 }
}

impl Lerp for Vis {
    /// Fading out lags the motion (`t` squared), so what leaves stays seen
    /// while it moves.
    fn lerp(self, to: Vis, t: f32) -> Vis {
        Vis {
            rect: self.rect.lerp(to.rect, t),
            s: self.s.lerp(to.s, t),
            dx: self.dx.lerp(to.dx, t),
            dy: self.dy.lerp(to.dy, t),
            a: self.a.lerp(to.a, if to.a < self.a { t * t } else { t }),
        }
    }
}

/// A uniform scale by `s` about `(ox, oy)`, then a move by `(dx, dy)`, at
/// opacity `alpha` (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub ox: f32,
    pub oy: f32,
    pub s: f32,
    pub dx: f32,
    pub dy: f32,
    pub alpha: f32,
}

impl Xform {
    fn rect(&self, [x, y, w, h]: [f32; 4]) -> RectF {
        let (x, y) = (self.ox + (x - self.ox) * self.s, self.oy + (y - self.oy) * self.s);
        RectF::new(x + self.dx, y + self.dy, w * self.s, h * self.s)
    }

    fn fade(&self, c: Rgba) -> Rgba {
        c.with_alpha((f32::from(c.3) * self.alpha.clamp(0.0, 1.0)).round() as u8)
    }
}

/// Draws every instance of `src` into `dst` through `t`: rects, clips,
/// radii, strokes and blurs scaled and moved, every alpha faded. Each keeps
/// its own clip, within `dst`'s current one.
pub fn replay(dst: &mut DrawList, src: &DrawList, t: Xform) {
    const ICONS: [Icon; 6] =
        [Icon::Plus, Icon::Cross, Icon::Minus, Icon::Dot, Icon::Square, Icon::Grid];
    const KINDS: [Kind; 8] = [
        Kind::Fill,
        Kind::Border,
        Kind::Shadow,
        Kind::Icon,
        Kind::Glyph,
        Kind::Gradient,
        Kind::Glow,
        Kind::Grain,
    ];
    for i in src.instances() {
        let Some(&kind) = KINDS.get(i.kind as usize) else {
            continue;
        };
        let (r, c, radius) = (t.rect(i.rect), t.fade(i.color), i.radius * t.s);
        dst.push_clip(t.rect(i.clip));
        match kind {
            Kind::Fill => dst.fill(r, radius, c),
            Kind::Border => dst.border(r, radius, i.p0 * t.s, c),
            Kind::Shadow => dst.shadow(r, radius, i.p0 * t.s, c),
            Kind::Icon => dst.icon(r, ICONS[(i.p0 as usize).min(5)], i.p1 * t.s, c),
            Kind::Glyph => dst.glyph(r, RectF::new(i.uv[0], i.uv[1], i.uv[2], i.uv[3]), c),
            Kind::Gradient => dst.gradient(r, radius, c, t.fade(i.color2), i.p0),
            Kind::Glow => dst.glow(r, c),
            Kind::Grain => dst.grain(r, c.3, i.p0),
        }
        dst.pop_clip();
    }
}

/// `a` crossfaded `t` (0 to 1) of the way to `b`: every color mixed, every
/// light moved (a light only one side has keeps its place and fades); the
/// name and darkness are `b`'s.
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
        name: b.name,
        dark: b.dark,
        base: m(a.base, b.base),
        glows: [0, 1, 2, 3].map(|i| glow(a.glows[i], b.glows[i])),
        grain: f32::from(a.grain).lerp(f32::from(b.grain), t).round() as u8,
        surface: m(a.surface, b.surface),
        surface_hi: m(a.surface_hi, b.surface_hi),
        surface_lo: m(a.surface_lo, b.surface_lo),
        glass: m(a.glass, b.glass),
        border: m(a.border, b.border),
        highlight: m(a.highlight, b.highlight),
        text: m(a.text, b.text),
        text_dim: m(a.text_dim, b.text_dim),
        text_faint: m(a.text_faint, b.text_faint),
        accent: m(a.accent, b.accent),
        accent_text: m(a.accent_text, b.accent_text),
        danger: m(a.danger, b.danger),
        shadow: m(a.shadow, b.shadow),
        selection: m(a.selection, b.selection),
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
    /// In the theme named `name` (the first of [`THEMES`] if none is).
    pub fn new(name: &str) -> Themes {
        Themes { current: ui::theme(name), fade: None }
    }

    /// The theme switched to last.
    pub fn current(&self) -> &'static Theme {
        self.current
    }

    /// Switches to the theme named `name` (ignoring ASCII case), fading
    /// from what shows at `now` over `ms`; whether it switched (not to an
    /// unknown name, nor to the current theme).
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

    /// The name of the theme after the current one in [`THEMES`], the
    /// first after the last.
    pub fn next(&self) -> &'static str {
        let i = THEMES.iter().position(|t| t.name == self.current.name).unwrap_or(0);
        THEMES[(i + 1) % THEMES.len()].name
    }

    /// What shows at `now`: the current theme, or a crossfade into it.
    pub fn at(&self, now: f64) -> Theme {
        match &self.fade {
            Some((from, t)) if t.is_running(now) => blend(from, self.current, t.value(now)),
            _ => *self.current,
        }
    }

    /// Starts a pending crossfade at `now`.
    pub fn arm(&mut self, now: f64) {
        if let Some(f) = &mut self.fade {
            f.1.arm(now);
        }
    }

    /// Whether a crossfade runs at `now`.
    pub fn is_running(&self, now: f64) -> bool {
        self.fade.as_ref().is_some_and(|f| f.1.is_running(now))
    }
}
