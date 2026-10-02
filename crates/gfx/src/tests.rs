use super::*;

const WHITE: Rgba = Rgba::hex(0xffffff);
const OK: RectF = RectF::new(0.0, 0.0, 10.0, 10.0);

fn le(vs: &[f32]) -> Vec<u8> {
    vs.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn reds(list: &DrawList) -> Vec<u8> {
    list.instances().iter().map(|i| i.color.0).collect()
}

#[test]
fn colors_and_rects() {
    assert_eq!(Rgba::hex(0x12ab34), Rgba(0x12, 0xab, 0x34, 255));
    assert_eq!(Rgba::hex(0xff00_0000), Rgba(0, 0, 0, 255));
    assert_eq!(Rgba::hex(0x010203).with_alpha(7), Rgba(1, 2, 3, 7));
    let r = RectF::from_i32(10, 20, 30, 40);
    assert_eq!(r, RectF::new(10.0, 20.0, 30.0, 40.0));
    assert_eq!(r.inset(5.0), RectF::new(15.0, 25.0, 20.0, 30.0));
    assert_eq!(r.inset(-1.0), RectF::new(9.0, 19.0, 32.0, 42.0));
    assert_eq!(r.inset(18.0), RectF::new(28.0, 38.0, 0.0, 4.0));
    let pts = [(10.0, 20.0), (39.9, 59.9), (40.0, 30.0), (20.0, 60.0), (9.9, 30.0)];
    assert_eq!(pts.map(|(x, y)| r.contains(x, y)), [true, true, false, false, false]);
    assert!(!RectF::default().contains(0.0, 0.0));
    let b = RectF::new(5.0, -5.0, 10.0, 10.0);
    assert_eq!([OK.intersect(b), b.intersect(OK)], [RectF::new(5.0, 0.0, 5.0, 5.0); 2]);
    let far = RectF::new(20.0, 3.0, 5.0, 5.0);
    assert_eq!(OK.intersect(far), RectF::new(20.0, 3.0, 0.0, 5.0));
}

#[test]
fn encodes_the_documented_layout() {
    let mut list = DrawList::new();
    list.fill(RectF::new(1.0, 2.0, 3.0, 4.0), 0.5, Rgba(10, 20, 30, 40));
    list.push_clip(RectF::new(0.0, 0.0, 8.0, 8.0));
    list.glyph(RectF::new(-2.0, 0.0, 16.0, 8.0), RectF::new(1.0, 2.0, 3.0, 4.0), WHITE);
    list.pop_clip();
    list.gradient(RectF::new(0.0, 0.0, 4.0, 4.0), 1.0, Rgba(1, 2, 3, 4), Rgba(5, 6, 7, 0), -0.5);
    let mut out = vec![9; 300];
    list.encode_into(&mut out);
    // Each: rect, radius, kind (Fill, Glyph, Gradient), p0, p1; color; clip, uv; color2.
    let one = |a: &[f32], c: [u8; 4], b: &[f32], c2: [u8; 4]| [le(a), c.into(), le(b), c2.into()];
    let (none, glyph) = ([NO_CLIP, [0.0; 4]].concat(), [0.0, 0.0, 8.0, 8.0, 1.0, 2.0, 3.0, 4.0]);
    let want = [
        one(&[1.0, 2.0, 3.0, 4.0, 0.5, 0.0, 0.0, 0.0], [10, 20, 30, 40], &none, [0; 4]),
        one(&[-2.0, 0.0, 16.0, 8.0, 0.0, 4.0, 0.0, 0.0], [255; 4], &glyph, [0; 4]),
        one(&[0.0, 0.0, 4.0, 4.0, 1.0, 5.0, -0.5, 0.0], [1, 2, 3, 4], &none, [5, 6, 7, 0]),
    ];
    assert_eq!((out.len(), out), (3 * INSTANCE_BYTES, want.concat().concat()));
    let mut out = vec![1];
    DrawList::new().encode_into(&mut out);
    assert!(out.is_empty());
}

#[test]
fn kinds_and_params() {
    let mut list = DrawList::new();
    list.fill(OK, 2.0, WHITE);
    list.border(OK, 2.0, 1.5, WHITE);
    list.shadow(OK, 2.0, 12.0, WHITE);
    list.shadow(OK, 2.0, -3.0, WHITE);
    list.icon(OK, Icon::Square, -1.0, WHITE);
    list.glyph(OK, OK, WHITE);
    list.gradient(OK, 2.0, WHITE, Rgba(1, 2, 3, 4), -1.25); // an angle may be negative
    list.glow(OK, WHITE);
    list.grain(OK, 9, -3.5); // so may a seed
    list.shadow_offset(OK, 2.0, 8.0, 6.0, WHITE);
    list.shadow_offset(OK, 2.0, 8.0, -2.0, WHITE);
    let got: Vec<f32> = list.instances().iter().flat_map(|i| [i.kind, i.p0, i.p1]).collect();
    let want = [
        0.0, 0.0, 0.0, 1.0, 1.5, 0.0, 2.0, 12.0, 0.0, 2.0, 0.0, 0.0, 3.0, 4.0, 0.0, 4.0, 0.0, 0.0,
        5.0, -1.25, 0.0, 6.0, 0.0, 0.0, 7.0, -3.5, 0.0, 2.0, 8.0, 0.0, 2.0, 8.0, 0.0,
    ];
    assert_eq!(got, want);
    let items = list.instances();
    // Only a gradient has a second color; grain is white at the strength.
    let mut seconds = [CLEAR; 11];
    seconds[6] = Rgba(1, 2, 3, 4);
    assert_eq!(items.iter().map(|i| i.color2).collect::<Vec<_>>(), seconds);
    assert_eq!(items[8].color, Rgba(255, 255, 255, 9));
    // Offset shadows move down (or up), keeping their size and radius.
    assert_eq!((items[9].rect, items[10].rect), ([0.0, 6.0, 10.0, 10.0], [0.0, -2.0, 10.0, 10.0]));
    assert_eq!((items[9].radius, items[6].radius, items[7].radius), (2.0, 2.0, 0.0));
    let kinds = [Kind::Fill, Kind::Border, Kind::Shadow, Kind::Icon, Kind::Glyph, Kind::Gradient];
    let ids: Vec<u8> = kinds.iter().chain(&[Kind::Glow, Kind::Grain]).map(|&k| k as u8).collect();
    assert_eq!(ids, [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn degenerate_pushes_are_skipped() {
    let (r, nan, inf) = (RectF::new, f32::NAN, f32::INFINITY);
    let none = WHITE.with_alpha(0);
    let mut list = DrawList::new();
    let bad = [r(0.0, 0.0, 0.0, 10.0), r(0.0, 0.0, 10.0, -1.0), r(nan, 0.0, 10.0, 10.0)];
    let more = [r(0.0, inf, 10.0, 10.0), r(0.0, 0.0, nan, 10.0)];
    bad.into_iter().chain(more).for_each(|bad| list.fill(bad, 0.0, WHITE));
    list.fill(OK, inf, WHITE);
    list.fill(OK, 0.0, none);
    list.border(OK, 0.0, 0.0, WHITE);
    list.border(OK, 0.0, nan, WHITE);
    list.shadow(OK, 0.0, inf, WHITE);
    list.icon(OK, Icon::Dot, nan, WHITE);
    list.icon(r(0.0, 0.0, 10.0, 0.0), Icon::Dot, 1.0, WHITE);
    list.gradient(OK, 0.0, none, Rgba(9, 9, 9, 0), 0.0);
    let gradients = [(0.0, nan), (0.0, inf), (nan, 0.0)];
    gradients.into_iter().for_each(|(radius, a)| list.gradient(OK, radius, WHITE, WHITE, a));
    list.gradient(r(0.0, 0.0, -1.0, 10.0), 0.0, WHITE, WHITE, 0.0);
    let glows = [(OK, none), (r(0.0, 0.0, 10.0, 0.0), WHITE), (r(0.0, nan, 10.0, 10.0), WHITE)];
    glows.into_iter().for_each(|(rect, color)| list.glow(rect, color));
    let grains = [(OK, 0, 1.0), (OK, 8, nan), (OK, 8, -inf), (r(0.0, 0.0, 0.0, 0.0), 8, 1.0)];
    grains.into_iter().for_each(|(rect, strength, seed)| list.grain(rect, strength, seed));
    list.shadow_offset(OK, 0.0, 4.0, nan, WHITE);
    list.shadow_offset(OK, 0.0, 4.0, inf, WHITE);
    list.shadow_offset(OK, 0.0, nan, 2.0, WHITE);
    list.shadow_offset(OK, 0.0, 4.0, 2.0, none);
    list.shadow_offset(r(0.0, 0.0, 10.0, -3.0), 0.0, 4.0, 2.0, WHITE);
    list.shadow_offset(r(0.0, f32::MAX, 10.0, 10.0), 0.0, 4.0, f32::MAX, WHITE);
    assert!(list.is_empty());
    list.fill(OK, 0.0, WHITE.with_alpha(1));
    // One visible end is enough for a gradient.
    list.gradient(OK, 0.0, none, Rgba(0, 0, 0, 1), 0.0);
    list.gradient(OK, 0.0, Rgba(0, 0, 0, 1), none, 0.0);
    list.grain(OK, 1, 0.0);
    assert_eq!(list.len(), 4);
}

#[test]
fn glyphs_carry_their_uv() {
    let (r, nan, inf) = (RectF::new, f32::NAN, f32::INFINITY);
    let (dst, uv) = (r(3.0, 4.0, 7.0, 9.0), r(10.0, 20.0, 14.0, 18.0));
    let mut list = DrawList::new();
    list.glyph(dst, uv, WHITE);
    let bad = [r(10.0, 20.0, 0.0, 18.0), r(10.0, 20.0, 14.0, -1.0), r(nan, 20.0, 14.0, 18.0)];
    bad.into_iter().chain([r(10.0, inf, 14.0, 18.0)]).for_each(|uv| list.glyph(dst, uv, WHITE));
    list.glyph(r(3.0, 4.0, 0.0, 9.0), uv, WHITE);
    list.glyph(dst, uv, WHITE.with_alpha(0));
    assert_eq!(list.len(), 1);
    let g = list.instances()[0];
    assert_eq!((g.kind, g.rect, g.uv), (4.0, [3.0, 4.0, 7.0, 9.0], [10.0, 20.0, 14.0, 18.0]));
    assert_eq!((g.radius, g.p0, g.p1, g.clip), (0.0, 0.0, 0.0, NO_CLIP));
    list.fill(dst, 0.0, WHITE);
    assert_eq!(list.instances()[1].uv, [0.0; 4]);
}

#[test]
fn clips_nest_and_pop() {
    let [x, y, w, h] = NO_CLIP;
    let (none, big) = (RectF::new(x, y, w, h), RectF::new(0.0, 0.0, 200.0, 200.0));
    let mut list = DrawList::new();
    list.pop_clip(); // empty: a no-op
    assert_eq!(list.clip(), none);
    list.push_clip(RectF::new(10.0, 10.0, 100.0, 50.0));
    list.push_clip(RectF::new(50.0, 0.0, 100.0, 30.0));
    assert_eq!(list.clip(), RectF::new(50.0, 10.0, 60.0, 20.0));
    for _ in 0..3 {
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
    }
    assert_eq!(list.clip(), none);
    let clips: Vec<[f32; 4]> = list.instances().iter().map(|i| i.clip).collect();
    assert_eq!(clips, [[50.0, 10.0, 60.0, 20.0], [10.0, 10.0, 100.0, 50.0], NO_CLIP]);
    // Disjoint, inverted and non-finite clips hide everything until popped.
    list.push_clip(RectF::new(0.0, 0.0, 10.0, 10.0));
    list.push_clip(RectF::new(20.0, 0.0, 10.0, 10.0));
    list.shadow(big, 0.0, 1000.0, WHITE);
    list.pop_clip();
    let nan = f32::NAN;
    for bad in [(20.0, 0.0, 10.0), (0.0, 0.0, -5.0), (nan, 0.0, 10.0), (0.0, f32::INFINITY, 10.0)] {
        list.push_clip(RectF::new(bad.0, bad.1, bad.2, 10.0));
        list.fill(big, 0.0, WHITE);
        list.pop_clip();
    }
    assert_eq!((list.clip(), list.len()), (RectF::new(0.0, 0.0, 10.0, 10.0), 3));
    list.clear();
    assert_eq!(list.clip(), none);
}

#[test]
fn pushes_outside_the_clip_are_skipped() {
    let (r, uv) = (RectF::new, RectF::new(0.0, 0.0, 5.0, 5.0));
    let red = |r| Rgba(r, 0, 0, 255);
    let mut list = DrawList::new();
    list.push_clip(r(100.0, 100.0, 50.0, 50.0));
    // Antialiased kinds reach 1 px past their rect.
    list.fill(r(90.0, 100.0, 9.0, 10.0), 0.0, WHITE);
    list.fill(r(90.0, 100.0, 9.5, 10.0), 0.0, red(1));
    list.border(r(151.0, 100.0, 10.0, 10.0), 0.0, 1.0, WHITE);
    list.icon(r(100.0, 150.5, 10.0, 10.0), Icon::Dot, 1.0, red(2));
    // A shadow reaches its blur plus 1 px.
    list.shadow(r(100.0, 40.0, 10.0, 10.0), 0.0, 49.0, WHITE);
    list.shadow(r(100.0, 40.0, 10.0, 10.0), 0.0, 50.0, red(3));
    // A glyph or a glow reaches exactly its rect.
    list.glyph(r(150.0, 100.0, 5.0, 5.0), uv, WHITE);
    list.glyph(r(149.0, 145.0, 5.0, 5.0), uv, red(4));
    list.glow(r(90.0, 100.0, 10.0, 10.0), WHITE);
    list.glow(r(90.0, 100.0, 10.5, 10.0), red(5));
    // Gradients and grain antialias their edges.
    list.gradient(r(100.0, 151.0, 9.0, 9.0), 0.0, WHITE, WHITE, 0.0);
    list.gradient(r(100.0, 150.5, 9.0, 9.0), 0.0, red(6), WHITE, 0.0);
    list.grain(r(80.0, 80.0, 19.0, 19.0), 255, 0.0);
    list.grain(r(80.0, 80.0, 19.5, 19.5), 255, 0.0);
    // An offset shadow reaches from where it lands.
    list.shadow_offset(r(100.0, 151.0, 9.0, 9.0), 0.0, 0.0, 10.0, WHITE);
    list.shadow_offset(r(100.0, 89.0, 9.0, 9.0), 0.0, 0.0, 10.0, red(7));
    assert_eq!(reds(&list), [1, 2, 3, 4, 5, 6, 255, 7]);
}

#[test]
fn radii_clamp_and_order_is_push_order() {
    let mut list = DrawList::new();
    for (i, radius) in [100.0, -3.0, 3.0].into_iter().enumerate() {
        list.fill(RectF::new(0.0, 0.0, 20.0, 10.0), radius, Rgba(i as u8, 0, 0, 255));
    }
    list.border(RectF::new(0.0, 0.0, 4.0, 30.0), 9.0, 1.0, Rgba(3, 0, 0, 255));
    let radii: Vec<f32> = list.instances().iter().map(|i| i.radius).collect();
    assert_eq!((radii, reds(&list)), (vec![5.0, 0.0, 3.0, 2.0], vec![0, 1, 2, 3]));
    let copy = list.clone();
    list.clear();
    assert_eq!((list.len(), copy.len()), (0, 4));
}

#[test]
fn shaders_name_the_contract() {
    let (vs, fs) = (VERTEX_SHADER, FRAGMENT_SHADER);
    for src in [vs, fs] {
        assert!(src.starts_with("#version 300 es\nprecision highp float;"));
        assert!(src.contains("uniform float u_dpr;"));
    }
    let names = ["a_rect", "a_params", "a_color", "a_clip", "a_uv", "a_color2"];
    for (loc, name) in names.iter().enumerate() {
        assert!(vs.contains(&format!("layout(location = {loc}) in vec4 {name};")), "{name}");
    }
    // Glyphs and glows draw no antialiasing margin.
    let vert = ["uniform vec2 u_viewport;", "gl_VertexID", "v_color2 = a_color2;"];
    for name in vert.into_iter().chain(["kind == 4 || kind == 6 ? 0.0 : px"]) {
        assert!(vs.contains(name), "vertex shader lacks {name}");
    }
    let frag = "precision highp int;|out vec4 fragColor;|uniform sampler2D u_atlas;|\
        uniform vec2 u_atlas_size;|v_color2|floatBitsToUint(v_params.z)|gl_FragCoord";
    let kinds = (1..8).map(|k| format!("kind == {k}"));
    for name in frag.split('|').map(String::from).chain(kinds) {
        assert!(fs.contains(&name), "fragment shader lacks {name}");
    }
    // Every varying the vertex shader writes, the fragment shader reads.
    for line in vs.lines().filter(|l| l.contains("out ")) {
        let decl = line.replace("out ", "in ");
        assert!(fs.contains(&decl), "fragment shader lacks {decl}");
    }
}

#[test]
fn new_and_clear_are_zero_and_all_dirty() {
    let mut a = Atlas::new(8, 4);
    assert_eq!((a.size(), a.pixels(), a.generation()), ((8, 4), &[0; 32][..], 0));
    assert_eq!((a.take_dirty(), a.take_dirty()), (Some((0, 4)), None));
    assert_eq!((Atlas::new(0, 4).take_dirty(), Atlas::new(4, 0).take_dirty()), (None, None));
    let mut a = Atlas::new(8, 8);
    let (x, y) = a.alloc(2, 2).unwrap();
    a.write(x, y, 2, 2, &[7; 4]);
    assert_eq!(a.alloc(5, 5), None);
    a.take_dirty();
    a.clear();
    assert_eq!((a.take_dirty(), a.generation(), a.pixels()), (Some((0, 8)), 1, &[0; 64][..]));
    assert_eq!(a.alloc(5, 5), Some((1, 1)));
    a.clear();
    assert_eq!(a.generation(), 2);
}

#[test]
fn shelves_keep_a_one_pixel_gutter() {
    let mut a = Atlas::new(16, 16);
    let reqs = [(4, 3), (4, 3), (4, 2), (4, 3), (2, 1), (3, 1), (14, 5), (14, 4), (1, 1)];
    // A shorter rect shares a shelf; a full shelf or one 2x too tall does
    // not; no gutter row below (14, 5); with no row left for a new
    // shelf, a tall shelf takes a short rect.
    let more = [(6, 1), (15, 1), (20, 20)];
    let got: Vec<_> = reqs.into_iter().chain(more).map(|(w, h)| a.alloc(w, h)).collect();
    let s = Some;
    let want =
        [s((1, 1)), s((6, 1)), s((11, 1)), s((1, 5)), s((1, 9)), s((4, 9)), None, s((1, 11))];
    assert_eq!(got, [&want[..], &[s((8, 9)), s((6, 5)), None, None]].concat());
    let edge = [(u32::MAX, 1), (1, u32::MAX), (0, 7), (7, 0)].map(|(w, h)| a.alloc(w, h));
    assert_eq!(edge, [None, None, s((0, 0)), s((0, 0))]);
    assert_eq!(Atlas::new(0, 0).alloc(1, 1), None);
    assert_eq!((Atlas::new(3, 3).alloc(1, 1), Atlas::new(3, 3).alloc(2, 1)), (s((1, 1)), None));
}

#[test]
fn packing_never_overlaps() {
    let mut a = Atlas::new(64, 64);
    let mut taken = vec![0u8; 64 * 64];
    let mut n = 0;
    for i in 0..200u32 {
        let (w, h) = (1 + i * 7 % 9, 1 + i * 5 % 11);
        let Some((x, y)) = a.alloc(w, h) else { continue };
        n += 1;
        assert!(x >= 1 && y >= 1 && x + w < 64 && y + h < 64);
        // No earlier rect touches this one or its gutter ring.
        for yy in y - 1..=y + h {
            for xx in x - 1..=x + w {
                assert_eq!(taken[(yy * 64 + xx) as usize], 0, "overlap at {xx},{yy}");
            }
        }
        for yy in y..y + h {
            taken[(yy * 64 + x) as usize..(yy * 64 + x + w) as usize].fill(1);
        }
    }
    assert!(n > 40, "only {n} rects fit");
}

#[test]
fn writes_copy_rows_and_widen_the_dirty_band() {
    let mut a = Atlas::new(4, 6);
    a.take_dirty();
    assert!(a.write(1, 2, 2, 2, &[1, 2, 3, 4]) && a.write(0, 4, 1, 1, &[5]));
    let want = [[0; 4], [0; 4], [0, 1, 2, 0], [0, 3, 4, 0], [5, 0, 0, 0], [0; 4]];
    assert_eq!((a.take_dirty(), a.pixels()), (Some((2, 5)), &want.concat()[..]));
    assert!(a.write(3, 5, 1, 1, &[6]) && a.write(0, 0, 0, 3, &[]));
    assert_eq!(a.take_dirty(), Some((5, 6)));
    // Bad writes do nothing.
    let mut a = Atlas::new(4, 4);
    a.take_dirty();
    let bad = [(3, 0, 2, 1, 2), (0, 4, 1, 1, 1), (0, 0, 2, 2, 3), (0, 0, 2, 2, 5)];
    let more = [(u32::MAX, 0, 2, 1, 2), (0, 1, 1, u32::MAX, 1), (5, 0, 0, 0, 0)];
    for (x, y, w, h, n) in bad.into_iter().chain(more) {
        assert!(!a.write(x, y, w, h, &vec![1; n]), "{x} {y} {w} {h} {n}");
    }
    assert_eq!((a.take_dirty(), a.pixels()), (None, &[0; 16][..]));
}

#[test]
fn a_recording_list_keeps_the_text_that_shows_and_the_marks() {
    // An ordinary list records nothing.
    let mut list = DrawList::new();
    list.note_text(0.0, 10.0, 10.0, 20.0, "hi");
    list.mark(1, 2, 3, "v");
    assert!(list.sem().is_none());
    let mut list = DrawList::recording();
    list.push_clip(RectF::new(0.0, 0.0, 100.0, 50.0));
    // Runs on one baseline and size join: closer than 0.15 em as one word, else with a space.
    list.note_text(10.0, 20.0, 10.0, 6.0, "a");
    list.note_text(16.5, 20.0, 10.0, 6.0, "b");
    list.note_text(30.0, 20.0, 10.0, 6.0, "c");
    // An em on, another baseline, another size, blank or outside the clip: none joins.
    list.note_text(66.0, 20.0, 10.0, 6.0, "d");
    list.note_text(72.0, 21.0, 10.0, 6.0, "e");
    list.note_text(78.0, 21.0, 12.0, 6.0, "f");
    list.note_text(84.0, 21.0, 12.0, 6.0, "  ");
    list.note_text(0.0, 90.0, 10.0, 6.0, "gone");
    // A run is cut to the clip.
    list.note_text(95.0, 20.0, 10.0, 30.0, "edge");
    list.mark(7, 3, 2, "on");
    list.pop_clip();
    let sem = list.sem().unwrap();
    let runs: Vec<(&str, RectF)> = sem.runs.iter().map(|r| (&*r.text, r.rect)).collect();
    assert_eq!(
        runs,
        [
            ("ab c", RectF::new(10.0, 10.0, 26.0, 12.5)),
            ("d", RectF::new(66.0, 10.0, 6.0, 12.5)),
            ("e", RectF::new(72.0, 11.0, 6.0, 12.5)),
            ("f", RectF::new(78.0, 9.0, 6.0, 15.0)),
            ("edge", RectF::new(95.0, 10.0, 5.0, 12.5)),
        ]
    );
    assert_eq!(sem.marks, [Mark { id: 7, role: 3, flags: 2, value: "on".into() }]);
    // Nothing is drawn; clearing keeps it recording, taking leaves it an ordinary list.
    assert!(list.is_empty());
    list.clear();
    assert_eq!(list.sem(), Some(&Sem::default()));
    assert!(list.take_sem().is_some() && list.sem().is_none());
}
