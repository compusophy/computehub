use gfx::{DrawList, Kind, RectF, Rgba};
use host::search::{Entry, Search};
use ui::{AppIcon, Key, THEMES, TextSystem};

use super::dock::{Look, Shelf, favorites, pin};
use super::menu::{Item, Menu};
use super::panel::Panel;
use super::touch::{Fling, Touch, decay};
use super::{field, icons};

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
    // long-presses, once.
    let mut t = Touch::new((0.0, 0.0), 1000.0, false);
    assert_eq!((t.moved((10.0, -10.0), 1200.0), t.scrolling), (None, false));
    assert!(!t.held(1499.0) && t.held(1500.0) && !t.held(1600.0));
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
fn the_dock_keeps_favorites_and_lays_out_groups() {
    let names = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert_eq!(favorites(None), names(&["studio", "assistant", "terminal", "files", "settings"]));
    assert_eq!(favorites(Some("terminal,,studio,terminal")), names(&["terminal", "studio"]));
    assert_eq!(favorites(Some("")), names(&[]));
    let mut f = favorites(Some("a"));
    assert!(pin(&mut f, "b", true) && !pin(&mut f, "b", true));
    assert!(!pin(&mut f, "x,y", true) && !pin(&mut f, "", true));
    assert!(pin(&mut f, "a", false) && !pin(&mut f, "a", false) && f == names(&["b"]));
    // Five favorites, a hairline, one other app, a hairline, the Apps button.
    let s = Shelf::new(5, 1, 1280.0, 600.0);
    assert_eq!((s.rect, s.tile), (RectF::new(441.0, 600.0, 398.0, 64.0), 44.0));
    assert_eq!(
        (&s.xs[..], &s.lines[..]),
        (&[449.0, 501.0, 553.0, 605.0, 657.0, 722.0, 787.0][..], &[711.5, 776.5][..])
    );
    assert_eq!(s.tile(6), RectF::new(787.0, 608.0, 44.0, 44.0));
    let at = |x| s.at(x, 620.0);
    assert_eq!(
        [at(496.0), at(497.0), at(711.0), at(800.0)],
        [Some(Some(0)), Some(Some(1)), Some(None), Some(Some(6))]
    );
    assert_eq!((s.at(440.0, 620.0), s.at(500.0, 599.0)), (None, None));
    // No other apps: one hairline; none at all: the Apps button alone.
    assert_eq!(Shelf::new(2, 0, 1280.0, 0.0).lines.len(), 1);
    assert_eq!(Shelf::new(0, 0, 1280.0, 0.0).lines.len(), 0);
    assert_eq!(Shelf::new(0, 2, 1280.0, 0.0).lines.len(), 1);
    // A phone shrinks the tiles (not below 28) to fit.
    let phone = Shelf::new(5, 2, 375.0, 0.0);
    assert_eq!((phone.tile, phone.rect.w, phone.rect.h), (32.0, 354.0, 52.0));
    assert_eq!(Shelf::new(20, 0, 375.0, 0.0).tile, 28.0);
    // Drawn: the dot of the focused app in the accent, the others' dim; the Apps button washed.
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[0]);
    let look = |running, focused| Look { icon: AppIcon::default(), lift: 0.0, running, focused };
    s.draw(&mut list, &mut text, t, &[look(true, true), look(true, false)], Some(true));
    assert!(fills(&list, t.accent) && fills(&list, t.text_dim) && fills(&list, t.wash(true)));
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
    let a = RectF::new(0.0, 44.0, 1280.0, 619.0);
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
    // Drawn: the wash while hovered, the label in up to two lines.
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[1]);
    let label = "A rather long application name";
    icons::draw(&mut list, &mut text, t, cells[0], (AppIcon::default(), label), Some(false));
    assert!(fills(&list, t.wash(false)));
    let glyphs =
        |l: &DrawList| l.instances().iter().filter(|i| i.kind == Kind::Glyph as u8 as f32).count();
    let rows = |l: &DrawList| {
        let mut ys: Vec<i32> = l
            .instances()
            .iter()
            .filter(|i| i.kind == Kind::Glyph as u8 as f32)
            .map(|i| i.rect[1] as i32 / 10)
            .collect();
        ys.dedup();
        ys.len()
    };
    assert!(glyphs(&list) > 10 && rows(&list) >= 2);
}

#[test]
fn the_bar_and_the_panel_sit_at_the_bottom() {
    assert_eq!(super::CLEAR, 137.0);
    let f = field::rect((1280.0, 800.0));
    assert_eq!((f, field::rect((375.0, 812.0)).x), (RectF::new(360.0, 743.0, 560.0, 44.0), 16.0));
    // The Ask row, two rows of tiles, three files: as tall as that, above the bar.
    let p = Panel::new(1280.0, f, 44.0, true, 5, 3);
    assert_eq!((p.rect, p.cols()), (RectF::new(340.0, 336.0, 600.0, 399.0), 4));
    assert_eq!(
        (p.ask_row(), p.tile(0), p.tile(4).y),
        (RectF::new(353.0, 349.0, 574.0, 44.0), RectF::new(385.0, 401.0, 80.0, 84.0), 493.0)
    );
    assert_eq!((p.row(0), p.fit()), (RectF::new(353.0, 590.0, 574.0, 44.0), 3));
    let at = |x, y| p.at(x, y, 0, 3);
    assert_eq!(
        [at(400.0, 360.0), at(400.0, 420.0), at(400.0, 600.0), at(345.0, 600.0), at(10.0, 10.0)],
        [Some(Some(0)), Some(Some(1)), Some(Some(6)), Some(None), None]
    );
    assert_eq!(p.at(400.0, 640.0, 2, 4), Some(Some(9)));
    // Files only; the Ask row only; a phone's full-width sheet; no room.
    let q = Panel::new(1280.0, f, 44.0, false, 0, 2);
    assert_eq!((q.rect.y, q.row(0).y), (621.0, 634.0));
    assert_eq!(Panel::new(1280.0, f, 44.0, true, 0, 0).rect.h, 70.0);
    let n = Panel::new(375.0, field::rect((375.0, 812.0)), 44.0, false, 8, 40);
    assert_eq!((n.rect.x, n.rect.w, n.rect.y, n.cols()), (0.0, 375.0, 52.0, 4));
    assert_eq!(Panel::new(1280.0, RectF::new(0.0, 50.0, 10.0, 44.0), 44.0, true, 3, 3).rect.h, 0.0);
    // Drawn: the bar with its caret in the accent while focused; the selection's ring.
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[0]);
    field::draw(&mut list, &mut text, t, f, "hi", true, None);
    assert!(fills(&list, t.accent));
    let entry =
        |n: &str| Entry { name: n.into(), label: n.into(), icon: AppIcon::default(), place: None };
    let mut s = Search::new(vec![entry("studio"), entry("terminal")]);
    s.type_text("t");
    p.draw(&mut list, &mut text, t, &s, Some((0, true)));
    assert!(fills(&list, t.wash(true)) && s.sel == 1);
}
