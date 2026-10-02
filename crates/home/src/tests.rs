use gfx::{DrawList, Kind, RectF, Rgba};
use ui::{AppIcon, Key, THEMES, TextSystem};

use super::dock::{Look, Spot, Strip, favorites, pin};
use super::icons;
use super::menu::{Item, Menu};
use super::touch::{Fling, Touch, decay};

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn text() -> TextSystem {
    TextSystem::new(SANS.to_vec()).unwrap()
}

/// Whether `list` fills (not borders) anything in `color`.
fn fills(list: &DrawList, color: Rgba) -> bool {
    let fill = Kind::Fill as u8 as f32;
    list.instances().iter().any(|i| i.kind == fill && i.color == color)
}

#[test]
fn fingers_scroll_past_eight_px_long_press_when_still_and_fling_on() {
    // Under 8 px nothing scrolls; past it scrolling starts where the finger is (no jump), and
    // the content follows the finger: up 10 px scrolls down 10.
    let mut t = Touch::new((100.0, 100.0), 0.0, true);
    assert_eq!((t.moved((100.0, 105.0), 10.0), t.scrolling, t.done), (None, false, false));
    assert_eq!((t.moved((103.0, 91.0), 20.0), t.scrolling, t.done), (None, true, true));
    assert_eq!(
        (t.moved((103.0, 81.0), 30.0), t.moved((103.0, 85.0), 40.0)),
        (Some(10.0), Some(-4.0))
    );
    // It lifts moving: a fling at its speed (each move weighs 0.8), unless it rested first.
    let f = t.lift(50.0).unwrap();
    assert!((f.v + 0.16).abs() < 1e-6 && f.t == 50.0, "{f:?}");
    assert_eq!(t.lift(141.0), None);
    assert!(!t.held(9999.0));
    // A finger on what does not scroll never does; held still (10 px of wander) for 500 ms, it
    // long-presses, once: on a page keeping up (frames 16 ms apart), at the first frame past.
    let mut t = Touch::new((0.0, 0.0), 1000.0, false);
    assert_eq!((t.moved((10.0, -10.0), 1200.0), t.scrolling), (None, false));
    assert!((1..32).all(|i| !t.held(1000.0 + 16.0 * f64::from(i))));
    assert!(t.held(1512.0) && t.done && !t.held(1600.0));
    // After a stall, 100 ms after the first frame past it: frames back to back after a busy
    // spell, before the lift queued behind it, never.
    let mut t = Touch::new((0.0, 0.0), 1000.0, false);
    assert!(!t.held(1016.0) && !t.held(1520.0) && !t.held(1522.0) && !t.held(1619.0));
    assert!(t.held(1620.0) && t.done);
    assert_eq!(t.lift(1700.0), None);
    let mut t = Touch::new((0.0, 0.0), 0.0, false);
    t.moved((0.0, 10.5), 10.0);
    assert!(t.done && !t.held(600.0));
    // A fling moves the integral of its decaying speed, and stops under 0.02 px/ms.
    let mut f = Fling { v: 1.0, t: 0.0 };
    let dy = f.step(16.0);
    assert!((dy - 325.0 * (1.0 - (-16.0f32 / 325.0).exp())).abs() < 0.01, "{dy}");
    assert!(f.moving() && (f.v - (-16.0f32 / 325.0).exp()).abs() < 1e-4);
    let (mut sum, mut frames) = (dy, 1);
    while f.moving() {
        sum += f.step(16.0 * (frames + 1) as f64);
        frames += 1;
    }
    assert!(frames == 80 && (sum - 318.0).abs() < 1.0, "{frames} {sum}");
    assert_eq!((f.step(5000.0) < 7.0, f.step(10.0), f.t), (true, 0.0, 5000.0));
    for x in [0.0, 0.05, 0.5, 1.0] {
        assert!((decay(x) - (-x).exp()).abs() <= 0.01 * (-x).exp(), "{x}");
    }
    assert_eq!(decay(-1.0), 1.0);
}

#[test]
fn the_strip_centers_the_ai_button_between_the_docks_wings() {
    let names = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert_eq!(
        (favorites(None), favorites(Some("terminal,,studio,terminal"))),
        (names(&[]), names(&["terminal", "studio"]))
    );
    let mut f = favorites(Some("a"));
    assert!(pin(&mut f, "b", true) && !pin(&mut f, "b", true));
    assert!(!pin(&mut f, "x,y", true) && !pin(&mut f, "", true));
    assert!(pin(&mut f, "a", false) && !pin(&mut f, "a", false) && f == names(&["b"]));
    assert_eq!(
        (super::joined(&names(&["a", "b,c", "d"])), super::names("d,,a,d")),
        ("a,d".into(), names(&["d", "a"]))
    );
    // Two favorites to the left of the button, one running app to its right.
    let s = Strip::new(2, 1, (1280.0, 800.0));
    assert_eq!((s.button, s.tile), (RectF::new(612.0, 727.0, 56.0, 56.0), 44.0));
    assert_eq!(
        s.wings,
        [RectF::new(487.0, 723.0, 112.0, 64.0), RectF::new(681.0, 723.0, 60.0, 64.0)]
    );
    assert_eq!(
        (&s.xs[..], s.tile(2)),
        (&[495.0, 547.0, 689.0][..], RectF::new(689.0, 731.0, 44.0, 44.0))
    );
    let at = |x, y| s.at(x, y);
    assert_eq!(
        [at(640.0, 750.0), at(500.0, 740.0), at(543.0, 740.0), at(489.0, 740.0), at(700.0, 700.0)],
        [Some(Spot::Button), Some(Spot::Tile(0)), Some(Spot::Tile(1)), Some(Spot::Wing), None]
    );
    // None at all: the button alone, where it always is; no wing to hit.
    let alone = Strip::new(0, 0, (1280.0, 800.0));
    assert_eq!((alone.button, alone.wings[0].w, alone.wings[1].w), (s.button, 0.0, 0.0));
    assert_eq!((alone.at(560.0, 750.0), alone.at(720.0, 750.0)), (None, None));
    // A phone shrinks the tiles until the fuller wing fits beside the button, to a finger's 32 px;
    // a fuller dock slides the button aside, and only one too long even so shrinks further:
    // every tile stays on the screen.
    let phone = Strip::new(1, 3, (390.0, 844.0));
    let right = phone.wings[1];
    assert_eq!((phone.tile, phone.button.x, right.x + right.w), (39.0, 167.0, 385.0));
    for (favs, others, tile, x) in [(4, 0, 32.0, 186.0), (8, 0, 29.0, 322.0), (0, 9, 25.0, 11.0)] {
        let s = Strip::new(favs, others, (390.0, 844.0));
        assert_eq!((s.tile, s.button.x, s.xs.len()), (tile, x, favs + others));
        assert!(s.xs.iter().all(|&x| x >= 13.0 && x + s.tile <= 377.0), "{favs} {others}");
    }
    assert_eq!(Strip::new(4, 4, (390.0, 844.0)).button.x, 167.0);
    // Drawn: the wings and tiles, the focused app's dot in the accent, the others' dim; the
    // button's glass, washed while held.
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[0]);
    let look = |running, focused| Look {
        icon: AppIcon::default(),
        sigil: None,
        lift: 0.0,
        running,
        focused,
    };
    s.draw(&mut list, &mut text, t, &[look(true, true), look(true, false), look(false, false)]);
    assert!(fills(&list, t.accent) && fills(&list, t.text_dim) && fills(&list, t.glass));
    s.draw_button(&mut list, &mut text, t, Some(true));
    assert!(fills(&list, t.wash(true)));
}

#[test]
fn the_overlay_sits_above_the_ai_button_a_card_a_sheet_or_a_pill() {
    // Wide: a card 560 px wide at most, 60% of the screen tall at most, centered on the button;
    // working, a pill 52 px tall, 420 px wide at most.
    let s = Strip::new(2, 1, (1280.0, 800.0));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, false), RectF::new(360.0, 239.0, 560.0, 480.0));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, true), RectF::new(430.0, 667.0, 420.0, 52.0));
    // A short screen keeps it under the bar; a narrow one inside its gutters.
    assert_eq!(Strip::new(2, 1, (1280.0, 300.0)).overlay((1280.0, 300.0), 44.0, false).y, 52.0);
    let phone = Strip::new(1, 3, (390.0, 844.0));
    let sheet = phone.overlay((390.0, 844.0), 44.0, false);
    assert_eq!(sheet, RectF::new(16.0, 341.0, 358.0, 422.0));
    assert_eq!(phone.overlay((390.0, 844.0), 44.0, true), RectF::new(16.0, 711.0, 358.0, 52.0));
    // A button slid aside: the sheet stays on the screen.
    let aside = Strip::new(0, 9, (390.0, 844.0));
    let r = aside.overlay((390.0, 844.0), 44.0, true);
    assert!(r.x >= 16.0 && r.x + r.w <= 374.0 && r.y + r.h < aside.button.y);
}

#[test]
fn menus_open_on_screen_and_follow_the_keys() {
    let mut text = text();
    let items: Vec<Item<u8>> =
        vec![("Open", "", Some(1)), ("", "", None), ("Close", "Alt+Q", Some(2))];
    let mut m = Menu::new(&items, (100.0, 100.0), (1280.0, 800.0), false, &mut text);
    assert_eq!(m.rect, RectF::new(100.0, 100.0, 180.0, 91.0));
    assert_eq!([m.item(0).h, m.item(1).y, m.item(2).y], [34.0, 139.0, 152.0]);
    let at = [m.at(110.0, 110.0), m.at(110.0, 140.0), m.at(110.0, 160.0), m.at(10.0, 10.0)];
    assert_eq!(at, [Some(Some(0)), Some(None), Some(Some(2)), None]);
    // The arrows skip separators and wrap; other keys are not theirs.
    let mut sels = Vec::new();
    for k in [Key::Down, Key::Down, Key::Down, Key::Up] {
        assert!(m.key(k));
        sels.push(m.sel);
    }
    assert_eq!(sels, [Some(0), Some(2), Some(0), Some(2)]);
    assert!(!m.key(Key::Left) && m.sel == Some(2));
    assert_eq!([m.act(0), m.act(1), m.act(2), m.act(9)], [Some(1), None, Some(2), None]);
    // Near the corner it opens up and left; for touch the items are 44 high; never off screen.
    let m = Menu::new(&items, (1270.0, 790.0), (1280.0, 800.0), true, &mut text);
    assert_eq!(m.rect, RectF::new(1090.0, 679.0, 180.0, 111.0));
    let m = Menu::new(&items, (-50.0, 30.0), (100.0, 100.0), false, &mut text);
    assert_eq!((m.rect.x, m.rect.y), (8.0, 8.0));
    // A long label widens it; drawn, the selection is washed.
    let wide: Vec<Item<u8>> = vec![("A label much longer than the least width", "Alt+X", Some(1))];
    let mut m = Menu::new(&wide, (0.0, 0.0), (1280.0, 800.0), false, &mut text);
    assert!(m.rect.w > 300.0);
    m.sel = Some(0);
    let (mut list, t) = (DrawList::new(), &THEMES[2]);
    m.draw(&mut list, &mut text, t, false);
    assert!(fills(&list, t.wash(false)));
}

#[test]
fn icons_fill_columns_wide_and_rows_of_four_narrow() {
    let a = RectF::new(0.0, 44.0, 1280.0, 671.0);
    let cells = [0, 5, 6, 7].map(|i| icons::cell(i, a, false));
    let at = |c: RectF| (c.x, c.y, c.w, c.h);
    assert_eq!(
        cells.map(at),
        [
            (13.0, 57.0, 89.0, 96.0),
            (13.0, 537.0, 89.0, 96.0),
            (102.0, 57.0, 89.0, 96.0),
            (102.0, 153.0, 89.0, 96.0)
        ]
    );
    assert_eq!(
        (icons::at(8, a, false, 110.0, 160.0), icons::at(8, a, false, 5.0, 50.0)),
        (Some(7), None)
    );
    let phone = RectF::new(0.0, 44.0, 375.0, 631.0);
    assert_eq!(at(icons::cell(5, phone, true)), (100.0, 153.0, 87.0, 96.0));
    // Too short a work area for one icon a column: one row.
    assert_eq!(
        at(icons::cell(3, RectF::new(0.0, 0.0, 400.0, 50.0), false)),
        (280.0, 13.0, 89.0, 96.0)
    );
    // A drop goes to the nearest cell of one more than the others; past the end, the end.
    let slots = [(60.0, 110.0), (150.0, 120.0), (60.0, 290.0), (900.0, 700.0)];
    assert_eq!(slots.map(|p| icons::slot(8, a, false, p)), [0, 6, 2, 8]);
    assert_eq!(icons::slot(3, phone, true, (330.0, 110.0)), 3);
}

#[test]
fn the_persons_order_survives_new_apps_and_moves() {
    let names = |s: &str| super::names(s);
    let apps = || names("studio,assistant,terminal,files,clock.app");
    // The stored order first (unknown names dropped), new apps after, in their default order.
    let arranged = |order: &str| {
        let (got, new) = icons::arrange(&names(order), apps(), |n| n.as_str());
        (super::joined(&got), new)
    };
    assert_eq!(
        arranged("terminal,gone,studio"),
        ("terminal,studio,assistant,files,clock.app".into(), true)
    );
    assert!(!arranged("files,assistant,clock.app,studio,terminal").1);
    // Carried icons land together at the slot among the rest, in their own order.
    let moved = |carried: &[usize], slot| {
        let order = icons::moved(5, carried, slot);
        order.iter().map(|&i| ["a", "b", "c", "d", "e"][i]).collect::<Vec<_>>().join(",")
    };
    assert_eq!(
        [moved(&[3], 0), moved(&[0], 4), moved(&[1, 4], 1), moved(&[1], 9)],
        ["d,a,b,c,e", "b,c,d,e,a", "a,b,e,c,d", "a,c,d,e,b"]
    );
    // Drawn: selected, the accent's ring and wash; carried, larger and shadowed; a `.app` file's
    // tile shows its sigil, not its glyph.
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[1]);
    let r = icons::cell(0, RectF::new(0.0, 44.0, 1280.0, 671.0), false);
    let state = icons::State { selected: true, ..Default::default() };
    icons::draw(&mut list, &mut text, t, r, (AppIcon::default(), None, "Clock"), state);
    assert!(fills(&list, t.accent.with_alpha(31)));
    let kind = |l: &DrawList, k: Kind| {
        l.instances()
            .iter()
            .filter(|i| i.kind == k as u8 as f32)
            .map(|i| (i.rect[2], i.uv))
            .collect::<Vec<_>>()
    };
    let mut lifted = DrawList::new();
    let state = icons::State { lift: 1.0, ..Default::default() };
    icons::draw(&mut lifted, &mut text, t, r, (AppIcon::default(), Some(7), "Clock"), state);
    let (tile, big) = (kind(&list, Kind::Gradient)[0].0, kind(&lifted, Kind::Gradient)[0].0);
    assert!(big > tile && kind(&lifted, Kind::Shadow).len() > kind(&list, Kind::Shadow).len());
    let glyph = |l: &DrawList| kind(l, Kind::Glyph).into_iter().find(|g| g.0 > 20.0).map(|g| g.1);
    assert_ne!(glyph(&lifted), glyph(&list));
    icons::draw_box(&mut list, &text, t, (300.0, 200.0), (100.0, 400.0));
    assert_eq!(list.instances().last().map(|i| i.rect), Some([100.0, 200.0, 200.0, 200.0]));
    assert_eq!(super::CLEAR, 85.0);
}
