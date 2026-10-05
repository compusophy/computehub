//! The top bar: compusophy's mark at the left (Show desktop: every window minimized, and back
//! again; Welcome is in the System folder), the date and time in the middle (the time alone on a
//! phone, or where the date does not fit), Feedback (a bug) and Settings at the right; a tooltip
//! names each button.

use gfx::{DrawList, RectF};
use host::LocalTime;
use host::paint::{cap_baseline, px};
use ui::icon::Glyph;
use ui::{FontId, TextStyle, TextSystem, Theme};

/// The bar's height.
pub const H: f32 = 44.0;
/// A button's side (its hit box), its distance from the screen's edge, the mark's and the other
/// glyphs' sides, the hover wash's diameter, the clock's size.
const BTN: f32 = 44.0;
const EDGE: f32 = 5.0;
const MARK: f32 = 26.0;
const GLYPH: f32 = 20.0;
const WASH: f32 = 34.0;
const CLOCK_SIZE: f32 = 13.0;

/// The bar's buttons: the mark (Show desktop), Feedback and Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Mark,
    Feedback,
    Settings,
}

/// The button under the pointer, and whether it is held.
pub type Hover = Option<(Button, bool)>;

impl Button {
    /// The app it shows: Feedback or Settings; none for the mark, which shows the desktop.
    pub fn app(self) -> Option<&'static str> {
        match self {
            Button::Mark => None,
            Button::Feedback => Some("feedback"),
            Button::Settings => Some("settings"),
        }
    }

    /// What its tooltip calls it.
    pub fn label(self) -> &'static str {
        match self {
            Button::Mark => "Show desktop",
            Button::Feedback => "Send feedback",
            Button::Settings => "Settings",
        }
    }
}

/// Where the buttons sit on a screen `w` wide.
pub fn buttons(w: f32) -> [(RectF, Button); 3] {
    let at = |x: f32| RectF::new(x, 0.0, BTN, BTN);
    [
        (at(EDGE), Button::Mark),
        (at(w - EDGE - 2.0 * BTN), Button::Feedback),
        (at(w - EDGE - BTN), Button::Settings),
    ]
}

/// The button at `(x, y)` on a screen `w` wide, if any.
pub fn at(w: f32, x: f32, y: f32) -> Option<Button> {
    buttons(w).into_iter().find(|b| b.0.contains(x, y)).map(|b| b.1)
}

/// Where the mark sits: centered in its button, 26 px square (the welcome flies there).
pub fn mark_rect() -> RectF {
    let r = buttons(0.0)[0].0;
    r.inset((r.w - MARK) / 2.0)
}

/// The bar on a screen `w` wide: its glass, the buttons (a round wash while hovered,
/// `Some((button, held))`), and the clock once it ticked.
pub fn draw(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    w: f32,
    (hover, clock): (Hover, Option<LocalTime>),
) {
    let line = px(text, 1.0);
    list.fill(RectF::new(0.0, 0.0, w, H), 0.0, theme.glass);
    list.fill(RectF::new(0.0, H - line, w, line), 0.0, theme.border);
    for b in buttons(w) {
        button(list, text, theme, b, hover);
    }
    if let Some(time) = clock {
        draw_clock(list, text, theme, w, time);
    }
}

/// Button `b` at `r`, its glyph, washed while `hover` holds it. An open folder draws the mark so
/// again above its dim, since it answers there.
pub fn button(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    (r, b): (RectF, Button),
    hover: Hover,
) {
    let square = |side: f32| r.inset((r.w - side) / 2.0);
    if let Some((_, held)) = hover.filter(|h| h.0 == b) {
        list.fill(square(WASH), WASH / 2.0, theme.wash(held));
    }
    let (side, glyph) = match b {
        Button::Mark => (MARK, Glyph::Mark),
        Button::Feedback => (GLYPH, Glyph::Bug),
        Button::Settings => (GLYPH, Glyph::Cog),
    };
    ui::icon::draw(list, text, square(side), glyph, theme.text);
}

/// The date (dim) and time centered in the bar of a screen `w` wide: the time alone on a phone,
/// or where the date does not fit. The welcome draws it in the same place.
pub fn draw_clock(
    list: &mut DrawList,
    text: &mut TextSystem,
    theme: &Theme,
    w: f32,
    time: LocalTime,
) {
    let (date, clock) = (time.date(), time.clock());
    let style = TextStyle::new(FontId::Sans, CLOCK_SIZE, theme.text_dim);
    let (dw, sw, cw) =
        (text.measure(&date, style), text.measure("  ", style), text.measure(&clock, style));
    let base = cap_baseline(text, 0.0, H, CLOCK_SIZE);
    let x = text.snap((w - dw - sw - cw) / 2.0);
    let x = if !crate::narrow(w) && dw + sw + cw <= w - 2.0 * (EDGE + 2.0 * BTN + 13.0) {
        text.draw_text(list, x, base, &date, style);
        x + dw + sw
    } else {
        text.snap((w - cw) / 2.0)
    };
    text.draw_text(list, x, base, &clock, style.with_color(theme.text));
}
