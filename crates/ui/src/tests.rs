use super::*;
use gfx::{DrawList, Instance, RectF, Rgba};
use theme::mix;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

const WHITE: Rgba = Rgba::hex(0xffffff);
const SANS14: TextStyle = TextStyle::new(FontId::Sans, 14.0, WHITE);
const MIDNIGHT: &Theme = &THEMES[0];
const REST: UiState = UiState { hover: None, pressed: None, focused: false, now_ms: 0.0 };

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

/// Builds one frame in `th` and checks the `Ui` left no clip open.
fn frame<R>(
    t: &mut TextSystem,
    th: &Theme,
    r: RectF,
    st: UiState,
    f: impl FnOnce(&mut Ui) -> R,
) -> (R, DrawList, Vec<Hit>) {
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let out = f(&mut Ui::new(&mut list, t, r, &mut hits, st, th));
    assert_eq!(list.clip(), RectF::new(-1e9, -1e9, 2e9, 2e9), "clips left open");
    (out, list, hits)
}

fn state(hover: Option<u32>, pressed: Option<u32>, focused: bool) -> UiState {
    let (hover, pressed) = (hover.map(WidgetId), pressed.map(WidgetId));
    UiState { hover, pressed, focused, now_ms: 0.0 }
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
fn widgets_stack_with_a_rhythm_and_register_hits() {
    let mut t = ts();
    let rect = RectF::new(100.0, 50.0, 400.0, 600.0);
    let (rs, list, hits) = frame(&mut t, MIDNIGHT, rect, REST, |ui| {
        assert_eq!((ui.width(), ui.rect(), ui.cursor()), (360.0, rect, (120.0, 70.0)));
        assert_eq!(ui.theme().name, "Midnight");
        let h = ui.heading("Settings");
        let l = ui.label("Body text");
        ui.space(10.0);
        let b = ui.button(WidgetId(1), "Save");
        let f = ui.text_field(WidgetId(2), "", false, "Name");
        let sm = ui.small("fine print");
        let p = ui.button_primary(WidgetId(3), "Go");
        let (sub, l2, h2) = (ui.subheading("Accent"), ui.label("Violet."), ui.heading("About"));
        ui.set_cursor(120.0, 600.0);
        let fresh = ui.heading("No margin");
        ui.advance_to(700.0);
        ui.advance_to(0.0);
        [h, l, b, f, sm, p, sub, l2, h2, fresh, ui.heading("After custom content")]
    });
    let [h, l, b, f, sm, p, sub, l2, h2, fresh, after] = rs;
    let below = |r: RectF| r.y + r.h;
    // No margin before the first item; SPACING after each, SPACING_MD before a
    // subheading, SPACING_LG before a heading; the larger gap wins.
    assert_eq!((h.x, h.y, h.h), (120.0, 70.0, t.line_height(MIDNIGHT.heading())));
    assert_eq!((l.y, b.y, b.h), (below(h) + SPACING, below(l) + SPACING + 10.0, BUTTON_H));
    assert!(b.w > 40.0 && b.w < 100.0 && b.w.fract() == 0.0);
    assert_eq!((f.y, f.w, f.h), (below(b) + SPACING, 360.0, FIELD_H));
    assert_eq!((sm.y, p.y), (below(f) + SPACING, below(sm) + SPACING));
    assert_eq!((sub.y, sub.h), (below(p) + SPACING_MD, t.line_height(MIDNIGHT.subheading())));
    assert_eq!((l2.y, h2.y), (below(sub) + SPACING, below(l2) + SPACING_LG));
    assert_eq!((fresh.y, after.y), (600.0, 700.0 + SPACING_LG));
    let got: Vec<(u32, RectF, Sense)> = hits.iter().map(|h| (h.id.0, h.rect, h.sense)).collect();
    assert_eq!(got, [(1, b, Sense::Click), (2, f, Sense::Text), (3, p, Sense::Click)]);
    let top = |x, y| hit_test(&hits, x, y).map(|h| h.id.0);
    assert_eq!((top(b.x + 1.0, b.y + 1.0), top(b.x - 1.0, b.y + 1.0)), (Some(1), None));
    // Everything drew inside the rect's clip, in the theme's colors.
    assert!(list.instances().iter().all(|i| {
        let [x, y, w, h] = i.clip;
        x >= 100.0 && y >= 50.0 && x + w <= 500.0 && y + h <= 650.0
    }));
    let th = MIDNIGHT;
    let ink = |c| find(&list, 4.0, Some(c)).len();
    assert_eq!((ink(th.text_faint), ink(th.text_dim), ink(th.accent_text)), (4, 9, 2));
    assert_eq!(find(&list, 0.0, Some(th.surface_lo)).len(), 1);
}

#[test]
fn rows_run_left_to_right() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 600.0, 400.0);
    let ((row, [a, b, c], after), _, hits) = frame(&mut t, MIDNIGHT, rect, REST, |ui| {
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
    assert_eq!((after.x, after.y, hits.len()), (PAD, PAD + b.h + SPACING, 2));
    // Nested rows, and custom content with the low-level calls.
    let ((inner, (w, del), below), list, hits) = frame(&mut t, MIDNIGHT, rect, REST, |ui| {
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
        ui.gradient(r, 4.0, WHITE, WHITE);
        ui.raised(r);
        assert!(ui.text(x, y + 20.0, "raw", SANS14) > 0.0);
        ui.list().glow(r, WHITE);
        // A sub-Ui over `r`, unpadded, leaves this one's cursor alone.
        let del = ui.within(r, |ui| (ui.width(), ui.button_danger(WidgetId(8), "Del")));
        assert_eq!(ui.cursor(), (x, y));
        ui.hit(WidgetId(7), r, Sense::Scroll);
        ui.advance_to(y + 50.0);
        (inner, del, ui.label("below"))
    });
    let y = PAD + BUTTON_H + SPACING;
    assert_eq!((inner.h, below.y, w, del.x, del.y), (BUTTON_H, y + 50.0, 50.0, PAD, y));
    assert_eq!(del.w, button_width(&mut t, MIDNIGHT, "Del"));
    assert_eq!((hits.len(), hits[3].sense), (4, Sense::Scroll));
    assert!(list.len() >= 10 && find(&list, 5.0, None).len() == 1);
    assert_eq!(find(&list, 0.0, Some(MIDNIGHT.surface_hi)).len(), 3);
}

#[test]
fn buttons_draw_their_states_and_hits_clip() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 100.0);
    let (hover, down, away) =
        (state(Some(9), None, false), state(Some(9), Some(9), false), state(None, Some(9), false));
    for th in &THEMES {
        let mut draw = |st: UiState, variant: u8| {
            frame(&mut t, th, rect, st, |ui| match variant {
                0 => ui.button(WidgetId(9), "Ok"),
                1 => ui.button_primary(WidgetId(9), "Ok"),
                _ => ui.button_danger(WidgetId(9), "Ok"),
            })
            .1
        };
        let fill = |list: &DrawList| list.instances()[0].color;
        let plain = [REST, hover, down, away].map(|st| fill(&draw(st, 0)));
        let hi = th.surface_hi;
        assert_eq!(plain, [hi, th.hover(hi), th.pressed(hi), hi], "{}", th.name);
        // Solid buttons (primary, danger) mix their color toward `accent_text`.
        let [primary, danger] = [1, 2].map(|v| [REST, hover, down].map(|st| fill(&draw(st, v))));
        for (fills, c) in [(primary, th.accent), (danger, th.danger)] {
            assert_eq!(fills, [0.0, 0.12, 0.24].map(|s| mix(c, th.accent_text, s)), "{}", th.name);
        }
        // Every state differs visibly from the one before it.
        let far = |x: Rgba, y: Rgba| {
            let d = |p: u8, q: u8| u32::from(p.abs_diff(q));
            d(x.0, y.0) + d(x.1, y.1) + d(x.2, y.2) >= 12
        };
        assert!(far(plain[0], plain[1]) && far(plain[1], plain[2]), "{}", th.name);
        assert!(far(primary[0], primary[1]) && far(primary[1], primary[2]), "{}", th.name);
        // A 1 px border on plain buttons only, a top sheen on both, and the
        // label in the right ink.
        let list = draw(REST, 0);
        assert_eq!(find(&list, 1.0, Some(th.border)).len(), 1);
        let sheen = find(&list, 1.0, Some(th.highlight));
        assert_eq!((sheen.len(), sheen[0].clip[3]), (1, 1.0));
        assert_eq!(find(&list, 4.0, Some(th.text)).len(), 2);
        let list = draw(REST, 1);
        assert!(find(&list, 1.0, Some(th.border)).is_empty());
        assert_eq!(find(&list, 4.0, Some(th.accent_text)).len(), 2);
    }
    // Labels sit with their capitals centered.
    let (b, list, _) = frame(&mut t, MIDNIGHT, rect, REST, |ui| ui.button(WidgetId(1), "HI"));
    let g = find(&list, 4.0, None);
    let cap_top = g.iter().map(|i| i.rect[1]).fold(f32::MAX, f32::min);
    let base = g.iter().map(|i| i.rect[1] + i.rect[3]).fold(0.0, f32::max);
    assert!(((cap_top - b.y) - (b.y + b.h - base)).abs() <= 1.0, "{cap_top} {base}");
    let (_, _, hits) = frame(&mut t, MIDNIGHT, rect, REST, |ui| {
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
    assert_eq!((hits.len(), hits[0].rect.y, hits[0].rect.h), (2, 20.0, 20.0));
    assert_eq!(hits[1].rect, RectF::new(250.0, 90.0, 50.0, 10.0));
}

#[test]
fn text_fields_scroll_and_show_a_caret() {
    let (mut t, th) = (ts(), MIDNIGHT);
    let rect = RectF::new(0.0, 0.0, 200.0, 100.0);
    let mut field = |st, value: &str, has| {
        frame(&mut t, MIDNIGHT, rect, st, |ui| ui.text_field(WidgetId(1), value, has, "Search"))
    };
    let focused = state(None, None, true);
    let caret = |list: &DrawList| find(list, 0.0, Some(th.accent)).first().copied();
    // Short, focused: text from the left, caret right after it, an accent
    // ring 1.5 px (2 device px at 1x) and a halo outside it.
    let (f, list, _) = field(focused, "hi", true);
    let c = caret(&list).expect("a caret");
    assert_eq!((c.rect[2], c.rect[3], list.instances()[0].color), (2.0, 17.0, th.surface_lo));
    let ring = find(&list, 1.0, Some(th.accent));
    assert_eq!((ring.len(), ring[0].p0, ring[0].rect), (1, 2.0, [f.x, f.y, f.w, f.h]));
    let halo = find(&list, 1.0, Some(th.accent.with_alpha(35)));
    assert_eq!((halo.len(), halo[0].rect[0], halo[0].p0), (1, f.x - 3.0, 3.0));
    assert_eq!(find(&list, 4.0, Some(th.text)).len(), 2);
    let text_end = c.rect[0];
    // A focused field in an unfocused window, or an unfocused field: no
    // caret, a 1 px border that doubles its alpha under the pointer.
    let (_, list, _) = field(REST, "hi", true);
    assert!(caret(&list).is_none() && find(&list, 1.0, Some(th.border))[0].p0 == 1.0);
    let (_, list, _) = field(state(Some(1), None, true), "", false);
    assert!(caret(&list).is_none());
    assert_eq!(find(&list, 1.0, Some(th.border.with_alpha(40))).len(), 1);
    assert_eq!(find(&list, 4.0, Some(th.text_faint)).len(), "Search".len());
    // Long: the end stays in view with the caret at the right edge, and the
    // start scrolls out of the clip.
    let long = "a fairly long value that cannot fit in the field";
    let (f, list, _) = field(focused, long, true);
    let [cx, _, cw, _] = caret(&list).unwrap().rect;
    assert!(cx + cw <= f.x + f.w - 12.0 + 0.5 && cx > f.x + f.w - 24.0);
    let g = find(&list, 4.0, None);
    assert!(g.len() < long.chars().filter(|c| *c != ' ').count());
    assert!(g.iter().all(|i| i.clip[0] == f.x + 12.0));
    assert!((text_end - (f.x + 12.0 + t.measure("hi", SANS14))).abs() <= 0.5);
}

#[test]
fn labels_wrap_to_the_content_width() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 240.0, 1000.0); // content 200 wide
    let text = "Wrapping keeps every line inside the window, between words.\nA new paragraph.";
    let (r, list, _) = frame(&mut t, MIDNIGHT, rect, REST, |ui| ui.label(text));
    let lines = t.wrap(text, SANS14, 200.0);
    assert!(lines.len() >= 4 && r.w <= 200.0 && r.h == lines.len() as f32 * 17.0);
    let g = find(&list, 4.0, None);
    assert!(g.iter().all(|i| i.rect[0] >= 20.0 && i.rect[0] + i.rect[2] <= 221.0));
    let mut rows: Vec<i32> = g.iter().map(|i| (i.rect[1] as i32 - 20) / 17).collect();
    rows.dedup();
    assert_eq!(rows.len(), lines.len());
    // Lines outside the clip are measured but not drawn.
    let short = RectF::new(0.0, 0.0, 240.0, 50.0);
    let (r2, list, _) = frame(&mut t, MIDNIGHT, short, REST, |ui| ui.label(text));
    let shown = find(&list, 4.0, None);
    assert!(r2 == r && !shown.is_empty() && shown.len() < g.len());
    assert!(shown.iter().all(|i| i.rect[1] < 50.0));
    // Any style wraps the same way.
    let (r3, list, _) = frame(&mut t, MIDNIGHT, rect, REST, |ui| ui.wrapped(text, SANS14));
    assert_eq!((r3, find(&list, 4.0, Some(WHITE)).len()), (r, g.len()));
}

#[test]
fn cards_hold_their_content() {
    let (mut t, th) = (ts(), MIDNIGHT);
    let rect = RectF::new(0.0, 0.0, 400.0, 600.0);
    let mut runs = 0;
    let ((card, [label, button], after), list, hits) =
        frame(&mut t, MIDNIGHT, rect, state(Some(4), None, false), |ui| {
            ui.label("before");
            let mut inner = [RectF::default(); 2];
            let card = ui.card(|ui| {
                runs += 1;
                inner = [ui.label("Inside"), ui.button(WidgetId(4), "Apply")];
            });
            (card, inner, ui.label("after"))
        });
    assert_eq!(runs, 2);
    assert_eq!((card.x, card.y, card.w), (PAD, PAD + 17.0 + SPACING, 360.0));
    assert_eq!((label.x, label.y), (card.x + CARD_PAD, card.y + CARD_PAD));
    assert_eq!(button.y, label.y + 17.0 + SPACING);
    assert_eq!(card.h, CARD_PAD + 17.0 + SPACING + BUTTON_H + CARD_PAD);
    assert_eq!(after.y, card.y + card.h + SPACING);
    // One hit (the measuring run's are dropped), the button hovered, and
    // the card's fill under its content.
    assert_eq!((hits.len(), hits[0].id, hits[0].rect), (1, WidgetId(4), button));
    let at = |c: Rgba| list.instances().iter().position(|i| i.kind == 0.0 && i.color == c);
    let fill = at(th.surface_hi).unwrap();
    let card_fill = &list.instances()[fill];
    assert_eq!((card_fill.rect, card_fill.radius), ([card.x, card.y, card.w, card.h], RADIUS_LG));
    assert!(at(th.hover(th.surface_hi)).unwrap() > fill);
    let inside = |i: &Instance| i.kind == 4.0 && i.rect[1] >= card.y;
    assert!(list.instances().iter().position(inside).unwrap() > fill);
    assert_eq!(find(&list, 1.0, Some(th.border)).len(), 2);
    // An empty card is just its padding.
    let (empty, _, _) = frame(&mut t, MIDNIGHT, rect, REST, |ui| ui.card(|_| {}));
    assert_eq!(empty.h, 2.0 * CARD_PAD);
}

#[test]
fn tiles_draw_icon_glyph_and_label() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 400.0, 400.0);
    let hue = Rgba::hex(0x22d3ee);
    for th in &THEMES {
        let (r, list, hits) = frame(&mut t, th, rect, state(Some(3), None, false), |ui| {
            ui.tile(WidgetId(3), "Terminal", ">_", hue)
        });
        assert_eq!(r, RectF::new(PAD, PAD, TILE_W, TILE_H));
        assert_eq!((hits.len(), hits[0].rect, hits[0].sense), (1, r, Sense::Click));
        assert_eq!(list.instances()[0].color, th.wash(false));
        // The icon: one rounded gradient, lighter at the top.
        let icon = [r.x + 18.0, r.y + SPACING, TILE_ICON, TILE_ICON];
        let (grad, g) = (find(&list, 5.0, None), find(&list, 5.0, None)[0]);
        assert_eq!((grad.len(), g.rect, g.radius, g.p0), (1, icon, RADIUS_LG, 0.0));
        let light = |c: Rgba| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
        assert!(light(g.color) > light(hue) && light(hue) > light(g.color2));
        // The glyph in white Mono over its shadow, the label in text below.
        let white = find(&list, 4.0, Some(WHITE));
        assert_eq!(white.len(), 2);
        assert!(white.iter().all(|g| g.rect[1] > icon[1] && g.rect[1] < icon[1] + 44.0));
        let label = find(&list, 4.0, Some(th.text));
        assert_eq!(label.len(), "Terminal".len());
        assert!(label.iter().all(|g| g.rect[1] > icon[1] + 44.0 && g.rect[1] < r.y + r.h));
    }
    // A long label is cut to fit; an empty glyph shows the first letter.
    let (r, list, _) = frame(&mut t, MIDNIGHT, rect, REST, |ui| {
        ui.tile(WidgetId(1), "settings and more", "", Rgba::hex(0x6d5cff))
    });
    let label = find(&list, 4.0, Some(MIDNIGHT.text));
    assert!(label.len() < "settingsandmore".len());
    assert!(label.iter().all(|g| g.rect[0] >= r.x + 4.0 && g.rect[0] + g.rect[2] <= r.x + 76.0));
    assert_eq!(find(&list, 4.0, Some(WHITE)).len(), 1);
    assert!(find(&list, 0.0, Some(MIDNIGHT.wash(false))).is_empty());
}

#[test]
fn everything_lands_on_device_pixels() {
    let mut t = ts();
    t.set_dpr(1.5);
    let rect = RectF::new(10.3, 20.7, 401.0, 900.0);
    let (rects, list, _) = frame(&mut t, MIDNIGHT, rect, state(None, None, true), |ui| {
        let mut out = vec![ui.heading("Title"), ui.label("text")];
        out.push(ui.button(WidgetId(1), "Odd width"));
        out.push(ui.text_field(WidgetId(2), "value", true, ""));
        out.push(ui.card(|ui| _ = ui.small("in a card")));
        out.push(ui.row(|ui| {
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
fn themes_are_complete_readable_and_glow() {
    assert_eq!(THEMES.map(|t| t.name), ["Midnight", "Dawn", "Mono"]);
    assert_eq!(THEMES.map(|t| t.dark), [true, false, true]);
    for (name, want) in [("Midnight", 0), ("dawn", 1), ("MONO", 2), ("", 0), ("Neon", 0)] {
        assert_eq!(theme(name), &THEMES[want], "{name}");
    }
    assert_eq!((THEMES[0].grain, THEMES[1].grain), (9, 6));
    assert_eq!((THEMES[0].base, THEMES[1].accent), (Rgba::hex(0x07080c), Rgba::hex(0x5b5bd6)));
    assert_eq!(THEMES[0].glows[0].color, Rgba::hex(0x6d5cff).with_alpha(90));
    assert_eq!((THEMES[1].glows[2].cy, THEMES[1].glows[2].rx), (0.98, 0.60));
    assert!(THEMES[2].glows.iter().all(|g| g.color.3 == 0) && THEMES[2].grain == 5);
    for th in &THEMES {
        // The surface over the base, and over the base lit by each glow at
        // its peak: every text tone stays readable on all of them.
        let lit = th.glows.iter().map(|g| over(g.color, th.base));
        for under in std::iter::once(th.base).chain(lit) {
            let (s, n) = (over(th.surface, under), th.name);
            assert!(contrast(th.text, s) >= 7.0, "{n} text {}", contrast(th.text, s));
            assert!(contrast(th.text_dim, s) >= 3.5, "{n} dim {}", contrast(th.text_dim, s));
            for (i, &c) in th.ansi.iter().enumerate() {
                let need = if i % 8 == 0 { [1.25, 3.0][i / 8] } else { 4.5 };
                assert!(contrast(c, s) >= need, "{n} ansi {i} {}", contrast(c, s));
            }
        }
        let on_accent = contrast(th.accent_text, th.accent);
        assert!(on_accent >= 4.5, "{} accent {on_accent}", th.name);
        assert!(contrast(th.text, th.surface_hi) >= 7.0 && contrast(th.text, th.surface_lo) >= 7.0);
        assert!(th.surface.3 > 0 && th.glass.3 < th.surface.3 && th.shadow.3 > 0);
    }
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
fn icon_tiles_are_readable_and_crisp() {
    // Every theme's glyph ink reads on both ends of its tile (3:1) for any hue.
    let hues = [0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xffffff, 0x000000];
    let more = [0x808080, 0x2dd4bf, 0x8b7bff, 0xf59e0b, 0x64748b].map(Rgba::hex);
    let all: Vec<Rgba> =
        hues.map(Rgba::hex).into_iter().chain(more).chain((0..12).map(theme::app_tint)).collect();
    for th in &THEMES {
        for &hue in &all {
            let [top, bottom, ink] = th.icon_colors(hue);
            let (a, b) = (contrast(ink, top), contrast(ink, bottom));
            assert!(a >= 3.0 && b >= 3.0, "{} {hue:?}: {a} {b}", th.name);
        }
    }
    // Mono is monochrome and flat; Midnight casts a shadow and fades its tiles.
    let (mono, hue, mut t) = (&THEMES[2], Rgba::hex(0x2dd4bf), ts());
    assert_eq!(mono.icon_colors(hue), [mono.surface_hi, mono.surface_hi, mono.text]);
    t.set_dpr(1.5);
    let tile = |t: &mut TextSystem, th: &Theme| {
        let mut list = DrawList::new();
        icon::tile(&mut list, t, RectF::new(10.2, 20.1, 50.0, 44.0), icon::Glyph::Cog, hue, th);
        list
    };
    let kinds = |l: &DrawList| l.instances().iter().map(|i| i.kind).collect::<Vec<_>>();
    let (list, lit) = (tile(&mut t, mono), tile(&mut t, MIDNIGHT));
    assert_eq!((kinds(&list), kinds(&lit)), (vec![0.0, 1.0, 4.0], vec![2.0, 5.0, 1.0, 4.0]));
    // The tile: a 44 px square on device pixels, centered; the glyph inside, in ink.
    let (i, d) = (list.instances(), |v: f32| (v * 1.5 - (v * 1.5).round()).abs() < 1e-3);
    let ([x, y, w, h], [gx, gy, gw, gh]) = (i[0].rect, i[2].rect);
    assert!(d(x) && d(y) && w == 44.0 && h == 44.0 && (x - 13.2).abs() <= 0.5, "{:?}", i[0].rect);
    assert!(gx > x && gy > y && gx + gw < x + w && gy + gh < y + h && i[2].color == mono.text);
    // A second tile of the glyph rasterizes nothing new.
    t.atlas_mut().take_dirty();
    assert!(
        tile(&mut t, mono).instances()[2].uv == i[2].uv && t.atlas_mut().take_dirty().is_none()
    );
}

#[test]
fn palettes_and_keys() {
    let th = &THEMES[1];
    assert_eq!((th.xterm(1), th.xterm(15), th.xterm(4)), (th.ansi[1], th.ansi[15], th.ansi[4]));
    let cube = [(16, 0x000000), (21, 0x0000ff), (196, 0xff0000)];
    let more = [(208, 0xff8700), (110, 0x87afd7), (231, 0xffffff)];
    let grays = [(232, 0x080808), (244, 0x808080), (255, 0xeeeeee)];
    for (i, rgb) in cube.into_iter().chain(more).chain(grays) {
        assert_eq!((th.xterm(i), THEMES[0].xterm(i)), (Rgba::hex(rgb), Rgba::hex(rgb)), "{i}");
    }
    assert_ne!(theme::app_tint(1), theme::app_tint(2));
    let (a, b) = (Rgba(0, 100, 200, 255), Rgba(255, 0, 100, 55));
    assert_eq!((mix(a, b, 0.0), mix(a, b, 1.0), mix(a, b, 2.0)), (a, b, b));
    assert_eq!(mix(a, b, 0.5), Rgba(128, 50, 150, 155));
    let wash = [MIDNIGHT.wash(false), MIDNIGHT.wash(true)];
    assert_eq!(wash, [MIDNIGHT.text.with_alpha(16), MIDNIGHT.text.with_alpha(26)]);
    let k = Key::from_code;
    let char_of = |code| if let Key::Char(c) = k(code) { c } else { '?' };
    assert_eq!(["KeyA", "KeyZ", "Digit0", "Numpad9"].map(char_of), ['a', 'z', '0', '9']);
    assert_eq!((k("Enter"), k("NumpadEnter"), k("ArrowUp")), (Key::Enter, Key::Enter, Key::Up));
    assert_eq!((k("PageDown"), k("F1"), k("F24")), (Key::PageDown, Key::F(1), Key::F(24)));
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
        cx.open("about");
        true
    }
}

#[test]
fn apps_and_their_context() {
    let mut app: Box<dyn App> = Box::new(Echo(0));
    assert!(!app.wants_text_input() && app.preferred_size().is_none());
    assert_eq!(app.icon(), AppIcon { glyph: "", hue: Rgba::hex(0x64748b) });
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 200.0);
    let (_, _, hits) = frame(&mut t, MIDNIGHT, rect, REST, |ui| app.draw(ui));
    let mut fs = vfs::Vfs::new();
    let mut kernel = kernel::Kernel::new();
    let mut cx = Cx::new(&mut fs, &mut kernel, 5.0);
    assert!(!app.event(AppEvent::Focus(true), &mut cx));
    assert!(app.event(AppEvent::Click(hits[0].id), &mut cx));
    cx.close_self();
    cx.load_fallback_fonts();
    cx.set_theme("Mono");
    // Preferences: the AI model's shows in the status at once, others only leave.
    cx.pref(AI_MODEL, "m");
    cx.pref("dock", "left");
    let want = "[Open { name: \"about\", floating: false }, CloseSelf, LoadFallbackFonts, \
        SetTheme(\"Mono\"), Pref { key: \"ai.model\", value: \"m\" }, \
        Pref { key: \"dock\", value: \"left\" }]";
    assert_eq!(format!("{:?}", cx.take_requests()), want);
    assert!(cx.take_requests().is_empty() && cx.now_ms == 5.0);
    assert_eq!(cx.ai, AiStatus { model: "m".into() });
    assert_eq!(fs.read("/tmp/clicks"), Ok(&[1][..]));
    assert_eq!(app.title(), "Echo 1");
}

#[test]
fn code_edits_scrolls_and_colors_its_spans() {
    let mut t = ts();
    let mut c = Code::new("let a = \"s\";\n  x", 1);
    c.set_spans(&[(0, 3, 1), (8, 3, 2), (15, 1, 7)]);
    let rect = RectF::new(10.0, 10.0, 300.0, 120.0);
    let (_, list, hits) = frame(&mut t, MIDNIGHT, rect, state(None, None, true), |ui| {
        c.draw(ui, WidgetId(4), rect, true, true);
    });
    assert_eq!(hits, [Hit { id: WidgetId(4), rect, sense: Sense::Text }]);
    let th = MIDNIGHT;
    let ink = |c| find(&list, 4.0, Some(c)).len();
    // `let` in the accent, `"s"` green, the gutter's 1 and 2, an underline.
    assert_eq!((ink(th.accent), ink(th.ansi[2]), ink(th.text_dim) + ink(th.text_faint)), (3, 3, 2));
    assert_eq!(find(&list, 0.0, Some(th.danger)).len(), 1);
    // Edits move the version on; caret keys do not; others are not taken.
    let v = c.version;
    c.ed.set_caret(1, 3);
    let keys = [Key::Enter, Key::Left, Key::F(2)].map(|k| (c.key(k), c.version - v));
    assert_eq!(keys, [(Some(true), 1), (Some(false), 1), (None, 1)]);
    assert!(c.insert("\ty") && !c.insert("\r"));
    assert_eq!((c.ed.text(), c.version), ("let a = \"s\";\n  x\n   y ".to_string(), v + 2));
    // Lines whose text is unchanged keep their colors, at the same place
    // from the top or from the bottom; the edited line has none.
    let kept = [(0, "let a = \"s\";"), (1, "  x")].map(|(i, l)| c.runs(i, 3, l));
    assert_eq!(kept, [&[(0, 3, 1), (8, 11, 2)][..], &[(2, 3, 7)]]);
    let mut d = Code::new("a\nb", 1);
    d.set_spans(&[(0, 1, 1), (2, 1, 3)]);
    d.insert("\n");
    let kept = [(0, ""), (1, "a"), (2, "b")].map(|(i, l)| d.runs(i, 3, l));
    assert_eq!(kept, [&[][..], &[(0, 1, 1)], &[(0, 1, 3)]]);
    // Past CODE_MAX nothing is added, but text can still go.
    c.set_text(&"x".repeat(CODE_MAX + 2), 9);
    assert!(!c.insert("y"));
    c.ed.set_caret(0, 1);
    assert_eq!((c.key(Key::Backspace), c.ed.len(), c.version), (Some(true), CODE_MAX + 1, 10));
    // Arrows wrap at line ends; Up and Down keep the column they aim for and
    // stop at the first line's start and the last line's end.
    use Key::*;
    let mut c = Code::new("abcdef\nab\nabcdef", 0);
    c.ed.set_caret(0, 6);
    let keys = [Right, Left, Left, Down, Down, Down, Right, Home, Left, Up, Up, Left, End];
    let carets = keys.map(|k| c.key(k).map(|_| c.ed.caret()));
    let lines = [1, 0, 0, 1, 2, 2, 2, 2, 1, 0, 0, 0, 0];
    let cols = [0, 6, 5, 2, 5, 6, 6, 0, 2, 2, 0, 0, 6];
    let want: Vec<_> = lines.into_iter().zip(cols).map(Some).collect();
    assert_eq!((carets.to_vec(), c.version), (want, 0));
    // A press puts the caret at the nearest char boundary on the grid of 8 x
    // 17 px cells from (16, 16), clamped into the text.
    let mut c = Code::new("state x = 1;\nlabel x;", 0);
    frame(&mut t, MIDNIGHT, rect, REST, |ui| c.draw(ui, WidgetId(4), rect, false, false));
    let at = [(3.4, 1.5), (3.6, 0.3), (99.0, 9.0), (-1.0, -1.0), (f32::NAN, 1.2)].map(|(x, y)| {
        c.click(16.0 + 8.0 * x, 16.0 + 17.0 * y);
        c.ed.caret()
    });
    assert_eq!(at, [(1, 3), (0, 4), (1, 8), (0, 0), (1, 0)]);
    // The wheel scrolls whole rows, and a press lands on the rows in view.
    let mut c = Code::new(&"line\n".repeat(50), 0);
    frame(&mut t, MIDNIGHT, rect, REST, |ui| c.draw(ui, WidgetId(4), rect, false, false));
    assert!(c.contains(20.0, 20.0) && !c.contains(5.0, 5.0));
    assert!(c.wheel(1000.0) && !c.wheel(f32::NAN));
    assert!(c.top > 40 && c.ed.caret().0 >= c.top);
    c.click(10.0 + 6.0, 10.0 + 6.0);
    assert_eq!(c.ed.caret(), (c.top, 0));
}
