use super::*;
use gfx::{DrawList, Instance, Rgba};
use std::sync::atomic::{AtomicUsize, Ordering};

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const SYM_B: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-b.ttf");

const WHITE: Rgba = Rgba::hex(0xffffff);
const SANS14: TextStyle = TextStyle::new(FontId::Sans, 14.0, WHITE);
const BOLD20: TextStyle = TextStyle::new(FontId::SansBold, 20.0, WHITE);
const MONO13: TextStyle = TextStyle::new(FontId::Mono, 13.0, WHITE);

fn ts() -> TextSystem {
    let mut t = TextSystem::new(SANS.to_vec()).unwrap();
    t.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    t.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    t
}

fn sans(size: f32) -> TextStyle {
    TextStyle::new(FontId::Sans, size, WHITE)
}

fn glyphs(list: &DrawList) -> Vec<Instance> {
    list.instances().iter().filter(|i| i.kind == 4.0).copied().collect()
}

fn on_grid(v: f32, dpr: f32) -> bool {
    (v * dpr - (v * dpr).round()).abs() < 1e-3
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn metrics_and_glyphs_land_on_device_pixels() {
    let mut t = ts();
    // Inter: (1984 + 494) / 2048 * 14 = 16.94; Mono: 1.32 * 13 = 17.16.
    assert_eq!((t.line_height(SANS14), t.line_height(MONO13)), (17.0, 17.0));
    assert_eq!((t.ascent(MONO13), t.descent(MONO13)), (13.0, 4.0));
    for dpr in [1.25, 1.5, 2.0, 3.0] {
        t.set_dpr(dpr);
        for style in [SANS14, MONO13, BOLD20] {
            for v in [t.line_height(style), t.ascent(style), t.descent(style)] {
                assert!(on_grid(v, dpr), "{v} at {dpr}");
            }
        }
        assert!(on_grid(t.cell_width(13.0), dpr) && on_grid(t.snap(0.37), dpr));
    }
    t.set_dpr(1.0);
    assert_eq!([13.0, 12.0, 0.0].map(|s| t.cell_width(s)), [8.0, 7.0, 0.0]); // 7.8 and 7.2
    for (dpr, want) in [(f32::NAN, 1.0), (-1.0, 1.0), (0.0, 1.0), (100.0, 8.0)] {
        t.set_dpr(dpr);
        assert_eq!(t.dpr(), want);
    }
    assert_eq!((t.line_height(sans(0.0)), t.line_height(sans(0.01))), (0.0, 1.0 / 8.0));
    for dpr in [1.0, 1.5, 2.0, 2.625] {
        let (mut t, mut list) = (ts(), DrawList::new());
        t.set_dpr(dpr);
        t.draw_text(&mut list, 10.3, 20.7, "Snap gj", SANS14);
        t.draw_text(&mut list, 3.1, 40.45, "mono{}", MONO13);
        let g = glyphs(&list);
        assert_eq!(g.len(), 12);
        for i in &g {
            let ([x, y, w, h], [u, v, uw, vh]) = (i.rect, i.uv);
            assert!(on_grid(x, dpr) && on_grid(y, dpr), "{:?} at {dpr}", i.rect);
            // Sampled 1:1: the rect is the uv size in device pixels.
            assert!((w * dpr - uw).abs() < 1e-3 && (h * dpr - vh).abs() < 1e-3);
            assert!(u >= 1.0 && v >= 1.0 && u + uw < 1024.0 && v + vh < 1024.0);
        }
    }
    // Twice the dpr is a separate cache entry; the same glyph and size, one slot.
    let (mut t, mut list) = (ts(), DrawList::new());
    t.draw_text(&mut list, 0.0, 20.0, "O", SANS14);
    t.set_dpr(2.0);
    t.draw_text(&mut list, 0.0, 20.0, "OO", SANS14);
    let g = glyphs(&list);
    assert!((g[1].uv[2] - 2.0 * g[0].uv[2]).abs() <= 2.0 && g[0].uv != g[1].uv);
    assert!(g[2].uv == g[1].uv);
    assert_eq!(t.atlas_mut().size(), (ATLAS_SIZE, ATLAS_SIZE));
}

#[test]
fn atlas_cells_and_fallbacks() {
    let (mut t, mut list, mut resets) = (ts(), DrawList::new(), 0);
    // Big glyphs at many sizes: far more than 1024 x 1024 holds.
    for size in (100..=400).step_by(5) {
        let style = TextStyle::new(FontId::SansBold, size as f32, WHITE);
        t.draw_text(&mut list, 0.0, 400.0, "MWQ@", style);
        if t.take_atlas_reset() {
            resets += 1;
            assert!(!t.take_atlas_reset());
        }
    }
    assert!(resets >= 1 && resets == t.atlas_mut().generation(), "{resets} resets");
    // After a reset, new glyphs get fresh, in-bounds slots.
    list.clear();
    t.draw_text(&mut list, 0.0, 40.0, "fresh", SANS14);
    assert!(glyphs(&list).iter().all(|g| g.uv[0] + g.uv[2] < 1024.0 && g.uv[1] + g.uv[3] < 1024.0));
    // A glyph wider than the atlas (U+23E5 is 1.26 em) is skipped without a
    // reset, and so is any glyph over 1000 device px per em.
    t.add_fallback(SYM_B.to_vec()).unwrap();
    list.clear();
    t.draw_text(&mut list, 0.0, 900.0, "\u{23e5}", sans(900.0));
    t.set_dpr(2.0);
    t.draw_text(&mut list, 0.0, 900.0, "i", sans(501.0));
    assert!(list.is_empty() && !t.take_atlas_reset());
    t.draw_text(&mut list, 0.0, 900.0, "\u{23e5}i", sans(400.0));
    assert_eq!(list.len(), 2);
    let (mut t, mut list) = (ts(), DrawList::new());
    // No face has U+273B yet: a hollow box in a cell, .notdef in a line.
    t.draw_cell_char(&mut list, 0.0, 13.0, 8.0, '✻', 13.0, WHITE);
    assert!(list.instances()[0].kind == 1.0 && t.measure("✻", MONO13) > 0.0);
    t.add_fallback(SYM_A.to_vec()).unwrap();
    t.add_fallback(SYM_B.to_vec()).unwrap();
    // Symbols A has U+273B (750 units, wider than a cell), only Symbols B U+23BF;
    // the built-ins stand in for each other: Inter lacks box drawing, Mono a check.
    assert!(near(t.measure("✻", MONO13), 750.0 * 13.0 / 1000.0));
    assert!(near(t.measure("✻", SANS14), 750.0 * 14.0 / 1000.0));
    assert!(near(t.measure("⎿", MONO13), 699.0 * 13.0 / 1000.0));
    assert!(near(t.measure("─", SANS14), 600.0 * 14.0 / 1000.0));
    assert!(near(t.measure("✓", MONO13), 1811.0 * 13.0 / 2048.0));
    for c in ['✻', '⎿', '⠋', '⣿'] {
        list.clear();
        t.draw_cell_char(&mut list, 16.0, 13.0, 8.0, c, 13.0, WHITE);
        let g = glyphs(&list);
        // Scaled to the cell: its width, within the ink's overhang.
        let [x, _, w, _] = g[0].rect;
        assert!(g.len() == 1 && w <= 9.0 && x >= 15.0 && x + w <= 26.0, "{c} {g:?}");
    }
    // Braille frames scale together: a full cell and a top row share a top edge.
    list.clear();
    t.draw_cell_char(&mut list, 0.0, 13.0, 8.0, '⣿', 13.0, WHITE);
    t.draw_cell_char(&mut list, 8.0, 13.0, 8.0, '⠉', 13.0, WHITE);
    let g = glyphs(&list);
    assert!((g[0].rect[1] - g[1].rect[1]).abs() <= 1.0, "{g:?}");
    // Bad fonts are refused, naming the slot; fallbacks are capped.
    let e = TextSystem::new(vec![1, 2, 3]).err().unwrap();
    let e2 = t.set_font(FontId::SansBold, vec![1]).unwrap_err();
    assert!(e.starts_with("sans:") && e2.starts_with("sans bold:"));
    assert!(t.has_font(FontId::SansBold) && t.add_fallback(vec![0; 10]).is_err());
    while t.fallback_count() < MAX_FALLBACKS {
        t.add_fallback(SYM_B.to_vec()).unwrap();
    }
    assert!(t.add_fallback(SYM_B.to_vec()).is_err());
    list.clear();
    let (size, cw, row, lh) = (13.0, t.cell_width(13.0), 40.0, t.line_height(MONO13));
    let base = row + t.ascent(MONO13);
    for (i, c) in "─│█╭".chars().enumerate() {
        t.draw_cell_char(&mut list, i as f32 * cw, base, cw, c, size, WHITE);
    }
    let g = glyphs(&list);
    assert_eq!(g.len(), 4);
    // ─ and █ reach both cell edges; │ and █ reach both row edges.
    for (x0, [x, _, w, _]) in [(0.0, g[0].rect), (2.0 * cw, g[2].rect)] {
        assert!(x <= x0 && x + w >= x0 + cw, "{x} {w}");
    }
    for [_, y, _, h] in [g[1].rect, g[2].rect] {
        assert!(y <= row && y + h >= row + lh, "{y} {h}");
    }
    // The block's texels are opaque right at the cell's edges.
    let ([ux, uy, uw, uh], dx) = (g[2].uv, g[2].rect[0]);
    let px = t.atlas_mut().pixels();
    let at = |x: f32| px[(uy + (uh / 2.0).floor()) as usize * 1024 + x as usize];
    let first = ux + (2.0 * cw - dx).round();
    let last = ux + (3.0 * cw - dx).round() - 1.0;
    assert!(first >= ux && last < ux + uw);
    assert_eq!((at(first), at(last)), (255, 255));
    // Plain Mono glyphs keep their size and sit centered.
    list.clear();
    t.draw_cell_char(&mut list, 80.0, base, cw, 'A', size, WHITE);
    t.draw_text(&mut list, 80.0, base, "A", MONO13);
    let g = glyphs(&list);
    assert!(g[0].uv[2..] == g[1].uv[2..] && (g[0].rect[0] - g[1].rect[0]).abs() <= 1.0);
    // Spaces, control chars and bad sizes draw nothing.
    list.clear();
    for c in [' ', '\n', '\u{1b}'] {
        t.draw_cell_char(&mut list, 0.0, base, cw, c, size, WHITE);
    }
    t.draw_cell_char(&mut list, 0.0, base, cw, 'A', f32::NAN, WHITE);
    t.draw_cell_char(&mut list, 0.0, base, 0.0, 'A', size, WHITE);
    assert!(list.is_empty());
}

static SQUARES: AtomicUsize = AtomicUsize::new(0);

/// The middle 80% of the box, counting its rasterizations.
fn square(o: &mut Vec<Vec<Point>>) {
    SQUARES.fetch_add(1, Ordering::Relaxed);
    let p = |x, y| Point { x, y, on: true };
    o.push(vec![p(100.0, 100.0), p(900.0, 100.0), p(900.0, 900.0), p(100.0, 900.0)]);
}

static SEEDS: AtomicUsize = AtomicUsize::new(0);

/// A square `seed` px wide from the box's corner, counting its rasterizations.
fn seeded(seed: u32, o: &mut Vec<Vec<Point>>) {
    SEEDS.fetch_add(1, Ordering::Relaxed);
    let (p, s) = (|x, y| Point { x, y, on: true }, seed as f32);
    o.push(vec![p(0.0, 0.0), p(s, 0.0), p(s, s), p(0.0, s)]);
}

#[test]
fn vectors_are_cached_and_drawn_on_device_pixels() {
    let (mut t, mut list) = (ts(), DrawList::new());
    t.set_dpr(2.0);
    // A 25 x 25 square centered in r: 50 device px from (36, 40), so the shape is 40 px from (41, 45).
    let r = RectF::new(10.3, 20.1, 40.0, 25.0);
    t.draw_vector(&mut list, r, 7, square, WHITE);
    t.atlas_mut().take_dirty();
    t.draw_vector(&mut list, r, 7, square, WHITE);
    let g = glyphs(&list);
    assert!(
        g[0] == g[1]
            && SQUARES.load(Ordering::Relaxed) == 1
            && t.atlas_mut().take_dirty().is_none()
    );
    let ([x, y, w, h], [u, v, ..]) = (g[0].rect, g[0].uv);
    assert!(
        on_grid(x, 2.0)
            && on_grid(y, 2.0)
            && (x * 2.0 - 41.0).abs() <= 1.0
            && (y * 2.0 - 45.0).abs() <= 1.0
    );
    assert!((w * 2.0 - 40.0).abs() <= 1.0 && (h * 2.0 - 40.0).abs() <= 1.0, "{:?}", g[0].rect);
    let px = t.atlas_mut().pixels();
    assert_eq!(px[(v + 20.0) as usize * 1024 + (u + 20.0) as usize], 255);
    // Too small or too big draws nothing; two shapes too big to share the atlas clear it.
    t.draw_vector(&mut list, RectF::new(0.0, 0.0, 0.2, 9.0), 7, square, WHITE);
    t.draw_vector(&mut list, RectF::new(0.0, 0.0, 600.0, 600.0), 7, square, WHITE);
    assert!(list.len() == 2 && !t.take_atlas_reset());
    t.draw_vector(&mut list, RectF::new(0.0, 0.0, 450.0, 450.0), 8, square, WHITE);
    t.draw_vector(&mut list, RectF::new(0.0, 0.0, 450.0, 450.0), 9, square, WHITE);
    assert!(t.take_atlas_reset() && SQUARES.load(Ordering::Relaxed) == 3);
    // The cleared atlas lost the small one: it comes back in a fresh slot.
    t.draw_vector(&mut list, r, 7, square, WHITE);
    assert_eq!((list.len(), SQUARES.load(Ordering::Relaxed)), (5, 4));
    // Seeded shapes are cached by seed, apart from the ids: seed 7 is not shape 7.
    list.clear();
    for seed in [7, 7, 500, 7] {
        t.draw_seeded(&mut list, r, seed, seeded, WHITE);
    }
    let w: Vec<f32> = glyphs(&list).iter().map(|g| g.rect[2] * 2.0).collect();
    assert_eq!((w, SEEDS.load(Ordering::Relaxed)), (vec![1.0, 1.0, 25.0, 1.0], 2));
}

#[test]
fn empty_slots_until_the_deferred_fonts_arrive() {
    let mut t = TextSystem::new(SANS.to_vec()).unwrap();
    assert!(t.has_font(FontId::Sans) && !t.has_font(FontId::SansBold) && !t.has_font(FontId::Mono));
    // SansBold is Sans: same metrics, same glyphs.
    let (mut a, mut b) = (DrawList::new(), DrawList::new());
    let bold = t.draw_text(&mut a, 0.0, 30.0, "Hello", BOLD20);
    assert_eq!(bold, t.draw_text(&mut b, 0.0, 30.0, "Hello", sans(20.0)));
    assert_eq!((glyphs(&a).len(), a.instances()), (5, b.instances()));
    assert_eq!(t.line_height(BOLD20), t.line_height(sans(20.0)));
    // Mono draws nothing but keeps JetBrains Mono's grid: 0.6 em a char.
    let w = t.measure("abc\t", MONO13);
    let cells = [10.0, 13.0, 12.0].map(|s| t.cell_width(s));
    assert!(near(w, 7.0 * 0.6 * 13.0) && cells == [6.0, 8.0, 7.0]);
    let v = |t: &TextSystem| [t.line_height(MONO13), t.ascent(MONO13), t.descent(MONO13)];
    let (vm, list) = (v(&t), &mut DrawList::new());
    assert_eq!(t.draw_text(list, 0.0, 13.0, "abc\t", MONO13), w);
    for c in ['A', '─', '✻'] {
        t.draw_cell_char(list, 0.0, 13.0, 8.0, c, 13.0, WHITE);
    }
    assert!(list.is_empty() && t.measure("─", SANS14) == t.measure("\u{ffff}", SANS14));
    // Filling the slots changes glyphs but not the grid.
    t.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    t.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    assert!(t.has_font(FontId::SansBold) && t.has_font(FontId::Mono));
    assert_eq!((t.measure("abc\t", MONO13), v(&t)), (w, vm));
    assert_eq!([10.0, 13.0, 12.0].map(|s| t.cell_width(s)), cells);
    assert!(near(t.measure("─", SANS14), 0.6 * 14.0) && t.measure("Hello", BOLD20) > bold);
    t.draw_cell_char(list, 0.0, 13.0, 8.0, 'A', 13.0, WHITE);
    assert_eq!(glyphs(list).len(), 1);
    // Replacing a slot drops its cached glyphs: SemiBold as Sans has the
    // same glyph ids as Regular, yet draws fresh, as SansBold does.
    let old = t.draw_text(&mut a, 0.0, 30.0, "O", sans(20.0));
    t.set_font(FontId::Sans, BOLD.to_vec()).unwrap();
    assert!(t.draw_text(&mut a, 0.0, 30.0, "O", sans(20.0)) > old);
    t.draw_text(&mut a, 0.0, 30.0, "O", BOLD20);
    let g = glyphs(&a);
    let [r, s, bo] = [&g[g.len() - 3], &g[g.len() - 2], &g[g.len() - 1]];
    assert!(r.uv != s.uv && s.rect[2..] == bo.rect[2..]);
}

#[test]
fn measures_draws_and_wraps() {
    let mut t = ts();
    // Inter 'A' advances 1413 of 2048 units; JetBrains Mono is 600 of 1000.
    assert!(near(t.measure("A", SANS14), 1413.0 * 14.0 / 2048.0));
    assert!(near(t.measure("iiii", MONO13), 4.0 * 7.8));
    assert_eq!(t.measure("iiii", MONO13), t.measure("WWWW", MONO13));
    assert!(t.measure("WWW", SANS14) > t.measure("iii", SANS14));
    assert_eq!((t.measure("", SANS14), t.measure("abc", sans(f32::NAN))), (0.0, 0.0));
    assert_eq!(t.measure("a\tb", MONO13), t.measure("a    b", MONO13));
    assert_eq!(t.measure("a\u{7}b", MONO13), t.measure("ab", MONO13));
    let mut list = DrawList::new();
    let adv = t.draw_text(&mut list, 10.0, 30.0, "Hello, world", SANS14);
    assert_eq!(adv, t.measure("Hello, world", SANS14));
    assert_eq!(t.draw_text(&mut list, 0.0, 0.0, "abc", sans(0.0)), 0.0);
    // All but the space, left to right, on the baseline: 'H' is cap height tall.
    let g = glyphs(&list);
    assert_eq!((g.len(), list.len()), (11, 11));
    assert!(g.windows(2).all(|w| w[0].rect[0] < w[1].rect[0]));
    let h = g[0].rect;
    assert!((h[1] + h[3] - 30.0).abs() <= 1.0 && h[1] < 21.0, "{h:?}");
    assert!(!t.take_atlas_reset() && t.atlas_mut().take_dirty().is_some());
    let w = t.measure("hello world", SANS14);
    assert_eq!(t.wrap("hello world", SANS14, w), ["hello world"]);
    assert_eq!(t.wrap("hello world", SANS14, w - 1.0), ["hello", "world"]);
    assert_eq!(t.wrap("hello   world", SANS14, w - 1.0), ["hello", "world"]);
    assert_eq!(t.wrap("a\nb\r\n\nc", SANS14, 500.0), ["a", "b", "", "c"]);
    assert_eq!(t.wrap("", SANS14, 500.0), [""]);
    assert_eq!(t.wrap("  indented", MONO13, 500.0), ["  indented"]);
    // Mono: 7.8 px a char, so 5 chars fit in 40 px.
    let words = ["abcde", "fghij", "kl"];
    assert_eq!(t.wrap("abcdefghijkl", MONO13, 40.0), words);
    assert_eq!(t.wrap("ab abcdefghijkl", MONO13, 40.0)[1..], words);
    assert_eq!(t.wrap("abc", MONO13, 0.0), ["a", "b", "c"]);
    assert_eq!(t.wrap("é ü", MONO13, 500.0), ["é ü"]);
    // Leading spaces before a long word leave no empty first line.
    assert_eq!(t.wrap("  verylongword", MONO13, 47.0), ["verylo", "ngword"]);
    // `+`, `/` and `-` between letters or digits are break opportunities, kept at
    // the end of the line; not between two digits, nor after a leading or doubled one.
    assert_eq!(t.wrap("Alt+Shift+Enter", MONO13, 94.0), ["Alt+Shift+", "Enter"]);
    assert_eq!(t.wrap("Alt+Shift+Enter", MONO13, 63.0), ["Alt+", "Shift+", "Enter"]);
    assert_eq!(t.wrap("Alt+Shift+1-4", MONO13, 94.0), ["Alt+Shift+", "1-4"]);
    assert_eq!(t.wrap("usr/local-bin", MONO13, 79.0), ["usr/local-", "bin"]);
    assert_eq!(t.wrap("ab --cdefgh", MONO13, 63.0), ["ab", "--cdefgh"]);
    let w = t.measure("Shift+", SANS14);
    assert_eq!(t.wrap("Alt+Shift+Enter", SANS14, w), ["Alt+", "Shift+", "Enter"]);
    let long = "The quick brown fox jumps over the lazy dog. ".repeat(8);
    let lines = t.wrap(&long, SANS14, 180.0);
    assert!(lines.len() > 5);
    for l in &lines {
        assert!(t.measure(l, SANS14) <= 180.0 && !l.starts_with(' ') && !l.ends_with(' '), "{l:?}");
    }
    let rejoined: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
    assert_eq!(long.split_whitespace().collect::<Vec<_>>(), rejoined);
}

#[test]
fn a_recording_list_notes_text_and_never_touches_the_atlas() {
    let mut t = ts();
    t.atlas_mut().take_dirty();
    let mut list = DrawList::recording();
    // A line advances as drawn; cells a char at a time, spaces skipped, read as words (a wider
    // gap, an em or more, starts another run).
    let adv = t.draw_text(&mut list, 10.0, 30.0, "Send feedback", SANS14);
    assert_eq!(adv, t.measure("Send feedback", SANS14));
    let cell = t.cell_width(13.0);
    for (i, c) in "ls -la   x".chars().enumerate() {
        t.draw_cell_char(&mut list, 5.0 + i as f32 * cell, 60.0, cell, c, 13.0, WHITE);
    }
    t.draw_vector(&mut list, RectF::new(0.0, 0.0, 20.0, 20.0), 1, square, WHITE);
    let sem = list.sem().unwrap();
    let runs: Vec<&str> = sem.runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(runs, ["Send feedback", "ls -la", "x"]);
    assert_eq!(sem.runs[0].rect, RectF::new(10.0, 16.0, adv, 17.5));
    assert!(list.is_empty() && t.atlas_mut().take_dirty().is_none());
}

#[test]
fn editor_edits_across_lines() {
    let mut e = Editor::new("row {\n  label 1;\n}");
    assert_eq!((e.len(), e.is_empty(), Editor::default().is_empty()), (18, false, true));
    // Each edit, then caret and text: Enter keeps the indent left of the caret,
    // Backspace and Delete join lines at their ends, tabs become two spaces, \r and controls go.
    type Edit = (fn(&mut Editor), (usize, usize), &'static str);
    let edits: [Edit; 15] = [
        (|e| e.set_caret(1, 99), (1, 10), "row {\n  label 1;\n}"),
        (|e| e.newline(), (2, 2), "row {\n  label 1;\n  \n}"),
        (|e| e.insert("label 2;"), (2, 10), "row {\n  label 1;\n  label 2;\n}"),
        (|e| e.set_caret(2, 1), (2, 1), "row {\n  label 1;\n  label 2;\n}"),
        (|e| e.newline(), (3, 1), "row {\n  label 1;\n \n  label 2;\n}"),
        (|e| e.delete(true), (3, 0), "row {\n  label 1;\n \n label 2;\n}"),
        (|e| e.delete(true), (2, 1), "row {\n  label 1;\n  label 2;\n}"),
        (|e| e.set_caret(1, usize::MAX), (1, 10), "row {\n  label 1;\n  label 2;\n}"),
        (|e| e.delete(false), (1, 10), "row {\n  label 1;  label 2;\n}"),
        (|e| e.set_caret(0, 0), (0, 0), "row {\n  label 1;  label 2;\n}"),
        (|e| e.delete(true), (0, 0), "row {\n  label 1;  label 2;\n}"),
        (|e| e.insert("a\tb\r\nc\u{7}"), (1, 1), "a  b\ncrow {\n  label 1;  label 2;\n}"),
        (|e| e.insert("éü"), (1, 3), "a  b\ncéürow {\n  label 1;  label 2;\n}"),
        (|e| e.step(true), (1, 2), "a  b\ncéürow {\n  label 1;  label 2;\n}"),
        (|e| e.delete(false), (1, 2), "a  b\ncérow {\n  label 1;  label 2;\n}"),
    ];
    for (edit, caret, text) in edits {
        edit(&mut e);
        assert_eq!((e.caret(), e.text()), (caret, text.to_string()));
    }
    assert_eq!(e.line_count(), 4);
}
