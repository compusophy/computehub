use super::*;
use gfx::{DrawList, Instance, RectF, Rgba};
use theme::mix;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

const WHITE: Rgba = Rgba::hex(0xffffff);
const SANS14: TextStyle = TextStyle::new(FontId::Sans, 14.0, WHITE);
const MIDNIGHT: &Theme = &THEMES[0];

fn ts() -> TextSystem {
    let mut t = TextSystem::new(SANS.to_vec()).unwrap();
    t.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    t.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    t
}

/// The instances of `kind` (0 fill, 1 border, 2 shadow, 4 glyph, 5
/// gradient, 6 glow, 7 grain), in `color` if given.
fn find(list: &DrawList, kind: f32, color: Option<Rgba>) -> Vec<Instance> {
    let hit = |i: &&Instance| i.kind == kind && color.is_none_or(|c| i.color == c);
    list.instances().iter().filter(hit).copied().collect()
}

fn glyphs(list: &DrawList) -> Vec<Instance> {
    find(list, 4.0, None)
}

type Frame<R> = (R, DrawList, Vec<Hit>);

/// Builds one frame in `th` and checks the `Ui` left no clip open.
fn frame_in<R>(
    t: &mut TextSystem,
    th: &Theme,
    r: RectF,
    st: UiState,
    f: impl FnOnce(&mut Ui) -> R,
) -> Frame<R> {
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let out = f(&mut Ui::new(&mut list, t, r, &mut hits, st, th));
    let none = RectF::new(-1e9, -1e9, 2e9, 2e9);
    assert_eq!(list.clip(), none, "clips left open");
    (out, list, hits)
}

fn frame<R>(t: &mut TextSystem, r: RectF, st: UiState, f: impl FnOnce(&mut Ui) -> R) -> Frame<R> {
    frame_in(t, MIDNIGHT, r, st, f)
}

fn state(hover: Option<u32>, pressed: Option<u32>, focused: bool) -> UiState {
    let mut s = UiState::default();
    (s.hover, s.pressed) = (hover.map(WidgetId), pressed.map(WidgetId));
    s.focused = focused;
    s
}

fn rest() -> UiState {
    UiState::default()
}

/// `top` composited over the opaque `under`.
fn over(top: Rgba, under: Rgba) -> Rgba {
    let a = f32::from(top.3) / 255.0;
    let ch = |t: u8, u: u8| (f32::from(t) * a + f32::from(u) * (1.0 - a)).round() as u8;
    Rgba(ch(top.0, under.0), ch(top.1, under.1), ch(top.2, under.2), 255)
}

/// The WCAG 2 contrast ratio of two opaque colors, 1 to 21.
fn contrast(a: Rgba, b: Rgba) -> f32 {
    let lin = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let lum = |c: Rgba| 0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2);
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn widgets_stack_and_register_hits() {
    let mut t = ts();
    let rect = RectF::new(100.0, 50.0, 400.0, 600.0);
    let (rs, list, hits) = frame(&mut t, rect, rest(), |ui| {
        assert_eq!((ui.width(), ui.rect()), (360.0, rect));
        assert_eq!(ui.cursor(), (120.0, 70.0));
        assert_eq!(ui.theme().name, "Midnight");
        let h = ui.heading("Settings");
        let l = ui.label("Body text");
        ui.space(10.0);
        let s = ui.separator();
        let b = ui.button(WidgetId(1), "Save");
        let f = ui.text_field(WidgetId(2), "", false, "Name");
        let k = ui.key_value("Version", "0.1.0");
        let m = ui.mono("let x = 1;");
        let sm = ui.small("fine print");
        let p = ui.button_primary(WidgetId(3), "Go");
        [h, l, s, b, f, k, m, sm, p]
    });
    let [h, l, s, b, f, k, m, sm, p] = rs;
    assert_eq!((h.x, h.y, h.h), (120.0, 70.0, t.line_height(MIDNIGHT.heading())));
    assert_eq!(l.y, h.y + h.h + SPACING);
    // The space already exceeds the separator's margin; 12 px follow it.
    assert_eq!((s.y, s.w, s.h), (l.y + l.h + SPACING + 10.0, 360.0, 1.0));
    assert_eq!((b.y, b.h), (s.y + 1.0 + SPACING_MD, BUTTON_H));
    assert!(b.w > 40.0 && b.w < 100.0 && b.w.fract() == 0.0);
    assert_eq!((f.y, f.w, f.h), (b.y + BUTTON_H + SPACING, 360.0, FIELD_H));
    assert_eq!(k.y, f.y + FIELD_H + SPACING);
    assert!(m.y > k.y && sm.y > m.y && p.y > sm.y);
    let got: Vec<(u32, RectF, Sense)> = hits.iter().map(|h| (h.id.0, h.rect, h.sense)).collect();
    let (c, x) = (Sense::Click, Sense::Text);
    assert_eq!(got, [(1, b, c), (2, f, x), (3, p, c)]);
    let top = |x, y| hit_test(&hits, x, y).map(|h| h.id.0);
    assert_eq!(top(b.x + 1.0, b.y + 1.0), Some(1));
    assert_eq!(top(b.x - 1.0, b.y + 1.0), None);
    // Everything drew inside the rect's clip, in the theme's colors.
    assert!(list.instances().iter().all(|i| {
        let [x, y, w, h] = i.clip;
        x >= 100.0 && y >= 50.0 && x + w <= 500.0 && y + h <= 650.0
    }));
    let th = MIDNIGHT;
    let ink = |c| find(&list, 4.0, Some(c)).len();
    assert_eq!(ink(th.text_faint), "Name".len());
    assert_eq!(ink(th.text_dim), "Version".len() + "fineprint".len());
    assert_eq!(ink(th.accent_text), "Go".len());
    assert_eq!(find(&list, 0.0, Some(th.border)).len(), 1);
    assert_eq!(find(&list, 0.0, Some(th.surface_lo)).len(), 1);
}

#[test]
fn vertical_rhythm() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 400.0, 800.0);
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| {
        let title = ui.title("Settings");
        let head = ui.heading("Appearance");
        let body = ui.label("Pick a theme.");
        let sub = ui.subheading("Accent");
        let body2 = ui.label("Violet.");
        let sep = ui.separator();
        let head2 = ui.heading("About");
        ui.set_cursor(20.0, 600.0);
        let fresh = ui.heading("No margin");
        ui.advance_to(700.0);
        let after = ui.heading("After custom content");
        [title, head, body, sub, body2, sep, head2, fresh, after]
    });
    let [title, head, body, sub, body2, sep, head2, fresh, after] = r;
    let below = |r: RectF| r.y + r.h;
    assert_eq!(title.y, PAD);
    assert_eq!(head.y, below(title) + SPACING_LG);
    assert_eq!(body.y, below(head) + SPACING);
    assert_eq!(sub.y, below(body) + SPACING_MD);
    assert_eq!(body2.y, below(sub) + SPACING);
    assert_eq!(sep.y, below(body2) + SPACING_MD);
    assert_eq!(head2.y, below(sep) + SPACING_LG);
    assert_eq!((fresh.y, after.y), (600.0, 700.0 + SPACING_LG));
    let th = MIDNIGHT;
    assert_eq!(title.h, t.line_height(th.title()));
    assert_eq!(sub.h, t.line_height(th.subheading()));
    assert!(title.h > head.h && head.h > sub.h);
    assert!(glyphs(&list).iter().all(|g| g.color == th.text));
}

#[test]
fn rows_run_left_to_right() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 600.0, 400.0);
    let ((row, [a, b, c], after), _, hits) = frame(&mut t, rect, rest(), |ui| {
        let mut r = [RectF::default(); 3];
        let row = ui.row(|ui| {
            r[0] = ui.button(WidgetId(1), "One");
            r[1] = ui.label("tall\ntext\nhere");
            ui.space(4.0);
            r[2] = ui.button(WidgetId(2), "Two");
        });
        (row, r, ui.label("after"))
    });
    assert_eq!((a.x, a.y), (PAD, PAD));
    assert_eq!((b.x, b.y, b.h), (a.x + a.w + SPACING, PAD, 3.0 * 17.0));
    assert_eq!((c.x, c.y), (b.x + b.w + SPACING + 4.0, PAD));
    assert_eq!(row, RectF::new(PAD, PAD, c.x + c.w - PAD, b.h));
    assert_eq!((after.x, after.y), (PAD, PAD + b.h + SPACING));
    assert_eq!(hits.len(), 2);
    // Nested rows, and custom content with the low-level calls.
    let ((inner, below), list, hits) = frame(&mut t, rect, rest(), |ui| {
        let mut inner = RectF::default();
        ui.row(|ui| {
            ui.label("x");
            inner = ui.row(|ui| {
                ui.button(WidgetId(5), "A");
                ui.button(WidgetId(6), "B");
            });
        });
        let (x, y) = ui.cursor();
        let r = RectF::new(x, y, 50.0, 50.0);
        ui.fill(r, 4.0, WHITE);
        ui.border(r, 4.0, 1.0, WHITE);
        assert!(ui.text(x, y + 20.0, "raw", SANS14) > 0.0);
        ui.hit(WidgetId(7), r, Sense::Scroll);
        ui.advance_to(y + 50.0);
        ui.advance_to(0.0);
        (inner, ui.label("below"))
    });
    assert_eq!(inner.h, BUTTON_H);
    assert_eq!(below.y, PAD + BUTTON_H + SPACING + 50.0);
    assert_eq!((hits.len(), hits[2].sense), (3, Sense::Scroll));
    assert!(list.len() >= 8);
}

#[test]
fn flows_wrap_tiles_into_a_grid() {
    let mut t = ts();
    // Content 360 wide: four 80 px tiles and three gaps fit on a line.
    let rect = RectF::new(0.0, 0.0, 400.0, 600.0);
    let ((grid, tiles, after), _, hits) = frame(&mut t, rect, rest(), |ui| {
        let mut tiles = Vec::new();
        let grid = ui.flow(|ui| {
            for i in 0..6 {
                tiles.push(ui.tile(WidgetId(i), "App", "", Rgba::hex(0x6d5cff)));
            }
        });
        (grid, tiles, ui.label("after"))
    });
    let at: Vec<(f32, f32)> = tiles.iter().map(|r| (r.x, r.y)).collect();
    let (x, y) = (|i: f32| PAD + i * (TILE_W + SPACING), PAD + TILE_H + SPACING);
    let want = [(x(0.0), PAD), (x(1.0), PAD), (x(2.0), PAD), (x(3.0), PAD), (PAD, y), (x(1.0), y)];
    assert_eq!(at, want);
    assert_eq!(grid, RectF::new(PAD, PAD, 4.0 * TILE_W + 3.0 * SPACING, 2.0 * TILE_H + SPACING));
    assert_eq!(after.y, PAD + grid.h + SPACING);
    assert_eq!(hits.len(), 6);
    assert!(tiles.iter().all(|r| (r.w, r.h) == (TILE_W, TILE_H)));
}

#[test]
fn buttons_draw_their_states_in_every_theme() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 100.0);
    let hover = state(Some(9), None, false);
    let down = state(Some(9), Some(9), false);
    let away = state(None, Some(9), false);
    for th in &THEMES {
        let mut draw = |st: UiState, primary: bool| {
            frame_in(&mut t, th, rect, st, |ui| match primary {
                true => ui.button_primary(WidgetId(9), "Ok"),
                false => ui.button(WidgetId(9), "Ok"),
            })
            .1
        };
        let fill = |list: &DrawList| list.instances()[0].color;
        let plain = [rest(), hover, down, away].map(|st| fill(&draw(st, false)));
        let hi = th.surface_hi;
        assert_eq!(plain, [hi, th.hover(hi), th.pressed(hi), hi], "{}", th.name);
        let primary = [rest(), hover, down].map(|st| fill(&draw(st, true)));
        let a = |s| mix(th.accent, th.accent_text, s);
        assert_eq!(primary, [th.accent, a(0.12), a(0.24)], "{}", th.name);
        // Every state differs visibly from the one before it.
        let far = |x: Rgba, y: Rgba| {
            let d = |p: u8, q: u8| u32::from(p.abs_diff(q));
            d(x.0, y.0) + d(x.1, y.1) + d(x.2, y.2) >= 12
        };
        assert!(far(plain[0], plain[1]) && far(plain[1], plain[2]), "{}", th.name);
        assert!(far(primary[0], primary[1]) && far(primary[1], primary[2]), "{}", th.name);
        // A 1 px border on plain buttons only, a top sheen on both, and the
        // label in the right ink.
        let list = draw(rest(), false);
        assert_eq!(find(&list, 1.0, Some(th.border)).len(), 1);
        let sheen = find(&list, 1.0, Some(th.highlight));
        assert_eq!((sheen.len(), sheen[0].clip[3]), (1, 1.0));
        assert_eq!(find(&list, 4.0, Some(th.text)).len(), 2);
        let list = draw(rest(), true);
        assert!(find(&list, 1.0, Some(th.border)).is_empty());
        assert_eq!(find(&list, 4.0, Some(th.accent_text)).len(), 2);
    }
    // Labels sit with their capitals centered.
    let (b, list, _) = frame(&mut t, rect, rest(), |ui| ui.button(WidgetId(1), "HI"));
    let g = glyphs(&list);
    let cap_top = g.iter().map(|i| i.rect[1]).fold(f32::MAX, f32::min);
    let base = g.iter().map(|i| i.rect[1] + i.rect[3]).fold(0.0, f32::max);
    assert!(((cap_top - b.y) - (b.y + b.h - base)).abs() <= 1.0, "{cap_top} {base}");
}

#[test]
fn hits_are_clipped() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 100.0);
    let (_, _, hits) = frame(&mut t, rect, rest(), |ui| {
        ui.push_clip(RectF::new(0.0, 0.0, 300.0, 40.0));
        ui.button(WidgetId(1), "Half"); // y 20..52: clipped at 40
        ui.button(WidgetId(2), "Gone"); // y 60..92: hidden
        ui.push_clip(RectF::new(0.0, 0.0, 10.0, 10.0));
        for _ in 0..3 {
            ui.pop_clip(); // one too many: the Ui's own clip stays
        }
        let r = RectF::new(250.0, 90.0, 99.0, 99.0);
        ui.hit(WidgetId(3), r, Sense::Click);
        ui.push_clip(r); // left open: dropping the Ui pops it
    });
    assert_eq!(hits.len(), 2);
    assert_eq!((hits[0].rect.y, hits[0].rect.h), (20.0, 20.0));
    assert_eq!(hits[1].rect, RectF::new(250.0, 90.0, 50.0, 10.0));
}

fn field(t: &mut TextSystem, st: UiState, value: &str, has: bool) -> Frame<RectF> {
    let rect = RectF::new(0.0, 0.0, 200.0, 100.0);
    frame(t, rect, st, |ui| ui.text_field(WidgetId(1), value, has, "Search"))
}

#[test]
fn text_fields_scroll_and_show_a_caret() {
    let mut t = ts();
    let th = MIDNIGHT;
    let focused = state(None, None, true);
    let caret = |list: &DrawList| find(list, 0.0, Some(th.accent)).first().copied();
    // Short, focused: text from the left, caret right after it, an accent
    // ring 1.5 px (2 device px at 1x) and a halo outside it.
    let (f, list, _) = field(&mut t, focused, "hi", true);
    let c = caret(&list).expect("a caret");
    assert!((c.rect[0] - (f.x + 12.0 + t.measure("hi", SANS14))).abs() <= 0.5);
    assert_eq!((c.rect[2], c.rect[3]), (2.0, 17.0));
    let ring = find(&list, 1.0, Some(th.accent));
    assert_eq!((ring.len(), ring[0].p0, ring[0].rect), (1, 2.0, [f.x, f.y, f.w, f.h]));
    let halo = find(&list, 1.0, Some(th.accent.with_alpha(35)));
    assert_eq!((halo.len(), halo[0].rect[0], halo[0].p0), (1, f.x - 3.0, 3.0));
    assert_eq!(list.instances()[0].color, th.surface_lo);
    assert_eq!(find(&list, 4.0, Some(th.text)).len(), 2);
    // A focused field in an unfocused window, or an unfocused field: no
    // caret, a 1 px border that doubles its alpha under the pointer.
    let (_, list, _) = field(&mut t, rest(), "hi", true);
    assert!(caret(&list).is_none());
    assert_eq!(find(&list, 1.0, Some(th.border))[0].p0, 1.0);
    let (_, list, _) = field(&mut t, state(Some(1), None, true), "", false);
    assert!(caret(&list).is_none());
    assert_eq!(find(&list, 1.0, Some(th.border.with_alpha(40))).len(), 1);
    assert_eq!(find(&list, 4.0, Some(th.text_faint)).len(), "Search".len());
    // Long: the end stays in view with the caret at the right edge, and the
    // start scrolls out of the clip.
    let long = "a fairly long value that cannot fit in the field";
    let (f, list, _) = field(&mut t, focused, long, true);
    let [cx, _, cw, _] = caret(&list).unwrap().rect;
    assert!(cx + cw <= f.x + f.w - 12.0 + 0.5 && cx > f.x + f.w - 24.0);
    let g = glyphs(&list);
    assert!(g.len() < long.chars().filter(|c| *c != ' ').count());
    assert!(g.iter().all(|i| i.clip[0] == f.x + 12.0));
}

#[test]
fn labels_wrap_to_the_content_width() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 240.0, 1000.0); // content 200 wide
    let text = "Wrapping keeps every line inside the window, between words.\nA new paragraph.";
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.label(text));
    let lines = t.wrap(text, SANS14, 200.0);
    assert!(lines.len() >= 4 && r.w <= 200.0);
    assert_eq!(r.h, lines.len() as f32 * 17.0);
    let g = glyphs(&list);
    for [x, _, w, _] in g.iter().map(|i| i.rect) {
        assert!(x >= 20.0 && x + w <= 221.0);
    }
    let mut rows: Vec<i32> = g.iter().map(|i| (i.rect[1] as i32 - 20) / 17).collect();
    rows.dedup();
    assert_eq!(rows.len(), lines.len());
    // Lines outside the clip are measured but not drawn.
    let short = RectF::new(0.0, 0.0, 240.0, 50.0);
    let (r2, list, _) = frame(&mut t, short, rest(), |ui| ui.label(text));
    let shown = glyphs(&list);
    assert!(r2 == r && !shown.is_empty() && shown.len() < g.len());
    assert!(shown.iter().all(|i| i.rect[1] < 50.0));
    // Any style wraps the same way.
    let (r3, list, _) = frame(&mut t, rect, rest(), |ui| ui.wrapped(text, SANS14));
    assert_eq!((r3, find(&list, 4.0, Some(WHITE)).len()), (r, g.len()));
}

#[test]
fn cards_hold_their_content() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 400.0, 600.0);
    let mut runs = 0;
    let ((card, inner, after), list, hits) =
        frame(&mut t, rect, state(Some(4), None, false), |ui| {
            ui.label("before");
            let mut inner = [RectF::default(); 2];
            let card = ui.card(|ui| {
                runs += 1;
                inner = [ui.label("Inside"), ui.button(WidgetId(4), "Apply")];
            });
            (card, inner, ui.label("after"))
        });
    let th = MIDNIGHT;
    assert_eq!(runs, 2);
    let [label, button] = inner;
    assert_eq!((card.x, card.y, card.w), (PAD, PAD + 17.0 + SPACING, 360.0));
    assert_eq!((label.x, label.y), (card.x + CARD_PAD, card.y + CARD_PAD));
    assert_eq!(button.y, label.y + 17.0 + SPACING);
    assert_eq!(card.h, CARD_PAD + 17.0 + SPACING + BUTTON_H + CARD_PAD);
    assert_eq!(after.y, card.y + card.h + SPACING);
    // One hit (the measuring run's are dropped), the button hovered, and
    // the card's fill under its content.
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].id, hits[0].rect), (WidgetId(4), button));
    let at = |c: Rgba| list.instances().iter().position(|i| i.kind == 0.0 && i.color == c);
    let fill = at(th.surface_hi).unwrap();
    let card_fill = &list.instances()[fill];
    assert_eq!((card_fill.rect, card_fill.radius), ([card.x, card.y, card.w, card.h], RADIUS_LG));
    assert!(at(th.hover(th.surface_hi)).unwrap() > fill);
    let inside = |i: &Instance| i.kind == 4.0 && i.rect[1] >= card.y;
    assert!(list.instances().iter().position(inside).unwrap() > fill);
    assert_eq!(find(&list, 1.0, Some(th.border)).len(), 2);
    // An empty card is just its padding.
    let (empty, _, _) = frame(&mut t, rect, rest(), |ui| ui.card(|_| {}));
    assert_eq!(empty.h, 2.0 * CARD_PAD);
}

#[test]
fn tiles_draw_icon_glyph_and_label() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 400.0, 400.0);
    let hue = Rgba::hex(0x22d3ee);
    for th in &THEMES {
        let (r, list, hits) = frame_in(&mut t, th, rect, state(Some(3), None, false), |ui| {
            ui.tile(WidgetId(3), "Terminal", ">_", hue)
        });
        assert_eq!(r, RectF::new(PAD, PAD, TILE_W, TILE_H));
        assert_eq!((hits.len(), hits[0].rect, hits[0].sense), (1, r, Sense::Click));
        assert_eq!(list.instances()[0].color, th.wash(false));
        // The icon: one rounded gradient, lighter at the top.
        let icon = [r.x + 18.0, r.y + SPACING, TILE_ICON, TILE_ICON];
        let grad = find(&list, 5.0, None);
        assert_eq!(
            (grad.len(), grad[0].rect, grad[0].radius, grad[0].p0),
            (1, icon, RADIUS_LG, 0.0)
        );
        let light = |c: Rgba| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        assert!(light(grad[0].color) > light(hue) && light(hue) > light(grad[0].color2));
        // The glyph in white Mono over its shadow, the label in text below.
        let white = find(&list, 4.0, Some(Rgba::hex(0xffffff)));
        assert_eq!(white.len(), 2);
        assert!(white.iter().all(|g| g.rect[1] > icon[1] && g.rect[1] < icon[1] + 44.0));
        let label = find(&list, 4.0, Some(th.text));
        assert_eq!(label.len(), "Terminal".len());
        assert!(label.iter().all(|g| g.rect[1] > icon[1] + 44.0 && g.rect[1] < r.y + r.h));
    }
    // A long label is cut to fit; an empty glyph shows the first letter.
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| {
        ui.tile(WidgetId(1), "settings and more", "", Rgba::hex(0x6d5cff))
    });
    let label = find(&list, 4.0, Some(MIDNIGHT.text));
    assert!(label.len() < "settingsandmore".len());
    assert!(label.iter().all(|g| g.rect[0] >= r.x + 4.0 && g.rect[0] + g.rect[2] <= r.x + 76.0));
    assert_eq!(find(&list, 4.0, Some(Rgba::hex(0xffffff))).len(), 1);
    assert!(find(&list, 0.0, Some(MIDNIGHT.wash(false))).is_empty());
}

#[test]
fn everything_lands_on_device_pixels() {
    let mut t = ts();
    t.set_dpr(1.5);
    let rect = RectF::new(10.3, 20.7, 401.0, 900.0);
    let (rects, list, _) = frame(&mut t, rect, state(None, None, true), |ui| {
        let mut out = vec![ui.title("Title"), ui.label("text"), ui.separator()];
        out.push(ui.button(WidgetId(1), "Odd width"));
        out.push(ui.text_field(WidgetId(2), "value", true, ""));
        out.push(ui.card(|ui| {
            ui.small("in a card");
        }));
        out.push(ui.flow(|ui| {
            ui.tile(WidgetId(3), "A", "A", Rgba::hex(0x6d5cff));
            ui.tile(WidgetId(4), "B", "B", Rgba::hex(0x6d5cff));
        }));
        out
    });
    let on_grid = |v: f32| ((v * 1.5).round() - v * 1.5).abs() < 1e-3;
    for r in &rects {
        assert!([r.x, r.y, r.x + r.w, r.y + r.h].into_iter().all(on_grid), "{r:?}");
    }
    for i in list.instances().iter().filter(|i| i.kind == 1.0) {
        assert!(on_grid(i.p0), "stroke {}", i.p0);
    }
}

#[test]
fn themes_are_complete_and_readable() {
    assert_eq!(THEMES.map(|t| t.name), ["Midnight", "Dawn", "Mono"]);
    assert_eq!(THEMES.map(|t| t.dark), [true, false, true]);
    for (name, want) in [("Midnight", 0), ("dawn", 1), ("MONO", 2), ("", 0), ("Neon", 0)] {
        assert_eq!(theme(name), &THEMES[want], "{name}");
    }
    assert_eq!((Theme::default(), THEMES[0].grain, THEMES[1].grain), (THEMES[0], 9, 6));
    assert_eq!((THEMES[0].base, THEMES[1].accent), (Rgba::hex(0x07080c), Rgba::hex(0x5b5bd6)));
    assert_eq!(THEMES[0].glows[0].color, Rgba::hex(0x6d5cff).with_alpha(90));
    assert_eq!((THEMES[1].glows[2].cy, THEMES[1].glows[2].rx), (0.98, 0.60));
    assert!(THEMES[2].glows.iter().all(|g| g.color.3 == 0) && THEMES[2].grain == 5);
    for th in &THEMES {
        // The surface over the base, and over the base lit by each glow at
        // its peak: every text tone stays readable on all of them.
        let lit = th.glows.iter().map(|g| over(g.color, th.base));
        for under in std::iter::once(th.base).chain(lit) {
            let s = over(th.surface, under);
            let n = th.name;
            assert!(contrast(th.text, s) >= 7.0, "{n} text {}", contrast(th.text, s));
            assert!(contrast(th.text_dim, s) >= 3.5, "{n} dim {}", contrast(th.text_dim, s));
            for (i, &c) in th.ansi.iter().enumerate() {
                let need = match i {
                    0 => 1.25,
                    8 => 3.0,
                    _ => 4.5,
                };
                assert!(contrast(c, s) >= need, "{n} ansi {i} {}", contrast(c, s));
            }
        }
        let on_accent = contrast(th.accent_text, th.accent);
        assert!(on_accent >= 4.5, "{} accent {on_accent}", th.name);
        assert!(contrast(th.text, th.surface_hi) >= 7.0 && contrast(th.text, th.surface_lo) >= 7.0);
        assert!(th.surface.3 > 0 && th.glass.3 < th.surface.3 && th.shadow.3 > 0);
    }
}

#[test]
fn backdrops_glow() {
    let mut list = DrawList::new();
    let screen = RectF::new(0.0, 0.0, 1000.0, 500.0);
    THEMES[0].draw_backdrop(&mut list, screen);
    let i = list.instances();
    let kinds: Vec<f32> = i.iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [0.0, 6.0, 6.0, 6.0, 7.0]);
    assert_eq!((i[0].rect, i[0].color), ([0.0, 0.0, 1000.0, 500.0], THEMES[0].base));
    // Violet at (0.18, 0.22) with radii (0.55, 0.50): a 1100 x 500 px box.
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    assert!(near(i[1].rect, [180.0 - 550.0, 110.0 - 250.0, 1100.0, 500.0]), "{:?}", i[1].rect);
    assert_eq!(i[1].color, THEMES[0].glows[0].color);
    assert_eq!((i[4].rect, i[4].color.3), ([0.0, 0.0, 1000.0, 500.0], 9));
    list.clear();
    THEMES[2].draw_backdrop(&mut list, screen);
    let kinds: Vec<f32> = list.instances().iter().map(|i| i.kind).collect();
    assert_eq!(kinds, [0.0, 7.0]);
}

#[test]
fn palettes_and_keys() {
    let th = &THEMES[1];
    let x = |i| theme::xterm_color(i, th);
    assert_eq!((x(1), x(15), th.xterm(4)), (th.ansi[1], th.ansi[15], th.ansi[4]));
    let cube = [(16, 0x000000), (21, 0x0000ff), (196, 0xff0000)];
    let more = [(208, 0xff8700), (110, 0x87afd7), (231, 0xffffff)];
    let grays = [(232, 0x080808), (244, 0x808080), (255, 0xeeeeee)];
    for (i, rgb) in cube.into_iter().chain(more).chain(grays) {
        assert_eq!(x(i), Rgba::hex(rgb), "{i}");
        assert_eq!(theme::xterm_color(i, &THEMES[0]), Rgba::hex(rgb), "{i}");
    }
    assert_ne!(theme::app_tint(1), theme::app_tint(2));
    let (a, b) = (Rgba(0, 100, 200, 255), Rgba(255, 0, 100, 55));
    assert_eq!((mix(a, b, 0.0), mix(a, b, 1.0), mix(a, b, 2.0)), (a, b, b));
    assert_eq!(mix(a, b, 0.5), Rgba(128, 50, 150, 155));
    let wash = [MIDNIGHT.wash(false), MIDNIGHT.wash(true)];
    assert_eq!(wash, [MIDNIGHT.text.with_alpha(16), MIDNIGHT.text.with_alpha(26)]);
    let k = Key::from_code;
    let char_of = |code| if let Key::Char(c) = k(code) { c } else { '?' };
    let chars = ["KeyA", "KeyZ", "Digit0", "Numpad9"].map(char_of);
    assert_eq!(chars, ['a', 'z', '0', '9']);
    assert_eq!((k("Enter"), k("NumpadEnter")), (Key::Enter, Key::Enter));
    assert_eq!((k("ArrowUp"), k("PageDown")), (Key::Up, Key::PageDown));
    assert_eq!((k("F1"), k("F24")), (Key::F(1), Key::F(24)));
    assert_eq!((k("Space"), k("Insert")), (Key::Space, Key::Insert));
    for code in "F25 F0 F01 F+1 F Fn Keya KeyAB NumpadAdd Minus ".split(' ') {
        assert_eq!(k(code), Key::Other, "{code}");
    }
}

struct Echo(u8);

impl App for Echo {
    fn title(&self) -> String {
        format!("Echo {}", self.0)
    }
    fn draw(&mut self, ui: &mut Ui<'_>) {
        ui.button(WidgetId(1), "Click");
    }
    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        if ev != AppEvent::Click(WidgetId(1)) {
            return false;
        }
        self.0 += 1;
        cx.vfs.write("/tmp/clicks", &[self.0]).unwrap();
        cx.open_floating("about");
        true
    }
}

struct Term;

impl App for Term {
    fn title(&self) -> String {
        "Terminal".into()
    }
    fn draw(&mut self, _: &mut Ui<'_>) {}
    fn event(&mut self, _: AppEvent, _: &mut Cx<'_>) -> bool {
        false
    }
    fn icon(&self) -> AppIcon {
        AppIcon { glyph: ">_", hue: Rgba::hex(0x22d3ee) }
    }
}

#[test]
fn apps_and_their_context() {
    let mut app: Box<dyn App> = Box::new(Echo(0));
    assert!(!app.wants_text_input() && app.preferred_size().is_none());
    assert_eq!(app.icon(), AppIcon { glyph: "", hue: Rgba::hex(0x64748b) });
    assert_eq!(Term.icon().glyph, ">_");
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 200.0);
    let (_, _, hits) = frame(&mut t, rect, rest(), |ui| app.draw(ui));
    let mut fs = vfs::Vfs::new();
    let mut cx = Cx::new(&mut fs, 5.0);
    assert!(!app.event(AppEvent::Focus(true), &mut cx));
    assert!(app.event(AppEvent::Click(hits[0].id), &mut cx));
    cx.close_self();
    cx.load_fallback_fonts();
    cx.open("files");
    cx.set_theme("Mono");
    let want = "[Open { name: \"about\", floating: true }, CloseSelf, \
        LoadFallbackFonts, Open { name: \"files\", floating: false }, SetTheme(\"Mono\")]";
    assert_eq!(format!("{:?}", cx.take_requests()), want);
    assert!(cx.take_requests().is_empty());
    assert_eq!(cx.now_ms, 5.0);
    assert_eq!(fs.read("/tmp/clicks"), Ok(&[1][..]));
    assert_eq!(app.title(), "Echo 1");
}

#[test]
fn key_columns_break_keys_between_words() {
    let mut t = ts();
    let dim = Some(MIDNIGHT.text_dim);
    // Content 256 wide: a 102 px key column, too narrow for the whole key.
    let rect = RectF::new(0.0, 0.0, 296.0, 400.0);
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.key_value("Alt+Shift+Enter", "Run"));
    assert_eq!(r.h, 2.0 * 17.0);
    let rows: Vec<i32> =
        find(&list, 4.0, dim).iter().map(|i| (i.rect[1] as i32 - 20) / 17).collect();
    assert_eq!(rows.iter().filter(|&&y| y == 1).count(), "Enter".len());
    // A key whose one word is wider than 40% widens its column instead.
    let word = t.measure("platform", SANS14);
    let rect = RectF::new(0.0, 0.0, 176.0, 400.0);
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.key_value("platform", "x"));
    assert_eq!(r.h, 17.0);
    let value = find(&list, 4.0, Some(MIDNIGHT.text));
    assert!(value[0].rect[0] >= 20.0 + word + SPACING, "{value:?}");
}
