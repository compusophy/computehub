use gfx::{DrawList, Kind, RectF, Rgba};
use host::{Effect, Entry};
use ui::icon::Mark;
use ui::{AppIcon, Key, Mods, THEMES, TextSystem};

use super::dock::{Dock, Look, Spot, Strip, favorites, pin, shift};
use super::grid::{APPS, Grid, Press};
use super::icons;
use super::menu::{Item, Menu};
use super::place::{self, Dims, Place};
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
fn the_row_holds_the_dock_at_the_left_and_the_assistant_alone_at_the_right() {
    // One tile high: 8 px above the tiles, 10 below (the dots'), which the work area leaves.
    assert_eq!((super::CLEAR, super::dock::ROW), (62.0, 62.0));
    // Nothing kept, nothing running: the Assistant alone, 44 px, 10 px from the screen's
    // bottom-right corner, and out to it; nothing else in the row to hit or draw.
    let alone = Strip::new(0, 0, (1280.0, 800.0));
    assert_eq!((alone.top, alone.assistant), (738.0, RectF::new(1226.0, 746.0, 44.0, 44.0)));
    let hits = [(1279.0, 799.0), (1222.0, 740.0), (1221.0, 760.0), (1250.0, 737.0)];
    let want = [Some(Spot::Assistant), Some(Spot::Assistant), None, None];
    assert_eq!(hits.map(|(x, y)| alone.at(x, y)), want);
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[0]);
    alone.draw(&mut list, &mut text, t, &[]);
    assert!(list.instances().is_empty());
    // Two kept, then one running: left-aligned from 10 px, 8 px apart, a hairline between the
    // groups; a tile takes half the space beside it, the hairline's gap none; past the last,
    // the Assistant's rect.
    let s = Strip::new(2, 1, (1280.0, 800.0));
    assert_eq!(
        (s.tile, s.y, &s.xs[..], s.sep),
        (44.0, 746.0, &[10.0, 62.0, 123.0][..], Some(114.0))
    );
    let hits = [(6.0, 790.0), (57.0, 760.0), (58.0, 760.0), (114.0, 760.0), (5.0, 760.0)];
    let want = [Some(Spot::Tile(0)), Some(Spot::Tile(0)), Some(Spot::Tile(1)), None, None];
    assert_eq!(hits.map(|(x, y)| s.at(x, y)), want);
    assert_eq!((s.tile(2), s.tile(3)), (RectF::new(123.0, 746.0, 44.0, 44.0), s.assistant));
    // A tile carried with its middle at x lands in the nearest of the kept ones' slots.
    assert_eq!([0.0, 32.0, 90.0, 500.0, f32::NAN].map(|x| s.slot(x, 2)), [0, 0, 1, 1, 0]);
    // A phone: the Assistant in its corner still; the dock's tiles shrink evenly to fit beside
    // it, never under it or off the screen.
    for (kept, others, tile) in
        [(0, 6, 44.0), (7, 0, 40.0), (0, 8, 34.0), (4, 4, 33.0), (20, 0, 8.0)]
    {
        let p = Strip::new(kept, others, (411.0, 794.0));
        assert_eq!((p.assistant.x, p.tile, p.xs.len()), (357.0, tile, kept + others));
        let right = p.xs.last().map_or(0.0, |x| x + p.tile);
        assert!(p.xs[0] == 10.0 && right <= 341.0, "{kept} {others}");
    }
    // Drawn: the hairline, each tile at its own rect over its dot (the focus's in the accent, a
    // running app's dim, none for one that does not run); a tile carried, shadowed.
    let look = |i, dot, focused| Look {
        icon: AppIcon::default(),
        mark: None,
        r: s.tile(i),
        lift: 0.0,
        dot,
        focused,
    };
    s.draw(
        &mut list,
        &mut text,
        t,
        &[look(0, 1.0, true), look(1, 1.0, false), look(2, 0.0, false)],
    );
    assert!(fills(&list, t.accent) && fills(&list, t.text_dim) && fills(&list, t.border));
    let n = list.instances().len();
    look(3, 0.0, false).draw_carried(&mut list, &mut text, t);
    assert!(list.instances().len() > n);
}

#[test]
fn the_overlay_sits_above_the_assistant_a_card_a_sheet_or_a_pill() {
    // Wide: a card 560 px wide at most, 60% of the screen tall at most, 8 px above the
    // Assistant, its right edge the Assistant's; working, a pill 52 px tall, 420 px wide at most.
    let s = Strip::new(2, 1, (1280.0, 800.0));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, false), RectF::new(710.0, 258.0, 560.0, 480.0));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, true), RectF::new(850.0, 686.0, 420.0, 52.0));
    // A short screen keeps it under the bar; a narrow one a sheet (or pill) out to the row's
    // edges: its first tile's and the Assistant's.
    assert_eq!(Strip::new(0, 0, (1280.0, 250.0)).overlay((1280.0, 250.0), 44.0, false).y, 52.0);
    let phone = Strip::new(1, 3, (390.0, 844.0));
    let edges = (phone.xs[0], phone.assistant.x + phone.assistant.w);
    assert_eq!(phone.overlay((390.0, 844.0), 44.0, false), RectF::new(10.0, 360.0, 370.0, 422.0));
    assert_eq!(phone.overlay((390.0, 844.0), 44.0, true), RectF::new(10.0, 730.0, 370.0, 52.0));
    assert_eq!(edges, (10.0, 380.0));
    assert_eq!(Strip::new(0, 0, (0.0, 0.0)).overlay((0.0, 0.0), 44.0, false).w, 0.0);
}

#[test]
fn the_dock_is_the_persons_own_its_kept_tiles_carried_to_a_new_order() {
    let names = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    // None kept at first; a stored list each once, never the Assistant (its corner is its own).
    assert_eq!(
        (favorites(None), favorites(Some("terminal,,assistant,studio,terminal"))),
        (names(&[]), names(&["terminal", "studio"]))
    );
    let mut f = favorites(Some("a"));
    assert!(pin(&mut f, "b", true) && !pin(&mut f, "b", true) && !pin(&mut f, "assistant", true));
    assert!(!pin(&mut f, "x,y", true) && !pin(&mut f, "", true));
    assert!(pin(&mut f, "a", false) && !pin(&mut f, "a", false) && f == names(&["b"]));
    assert_eq!(
        (super::joined(&names(&["a", "b,c", "d"])), super::names("d,,a,d,assistant")),
        ("a,d".into(), names(&["d", "a", "assistant"]))
    );
    let mut v = [0, 1, 2, 3];
    for (from, to, want) in [(0, 2, [1, 2, 0, 3]), (2, 0, [0, 1, 2, 3]), (3, 9, [0, 1, 2, 3])] {
        shift(&mut v, from, to);
        assert_eq!(v, want);
    }
    // Kept a, x (which the registry does not know: it does not show) and b; then one running.
    // Only a kept tile carries: a mouse's once it travels 4 px, to the slot under it.
    let (mut d, mut fx, size) = (Dock::new(Some("a,x,b")), Vec::new(), (1280.0, 800.0));
    d.layout(vec![0, 2], 3, size, (&[], 0.0));
    let at = |i: usize| (32.0 + 52.0 * i as f32, 768.0);
    assert!(d.pick(2, at(2), false).is_none());
    d.carry = d.pick(0, at(0), false);
    assert!(!d.carry_to(Some((at(0).0 + 3.0, 768.0))) && d.carried(Some(at(1))).is_none());
    assert!(d.carry_to(Some(at(1))) && d.moving() == Some((0, 1)));
    assert_eq!(d.carried(Some(at(1))), Some((1, d.strip.tile(1))));
    // Put down there: the favorites shift (x with them), and are kept.
    d.drop(true, &mut fx);
    assert_eq!(d.favs, names(&["x", "b", "a"]));
    assert_eq!(fx, [Effect::Pref { key: "dock".into(), value: "x,b,a".into() }]);
    // Put back (Escape, the pointer gone), or down where it was: nothing changes.
    d.layout(vec![1, 2], 3, size, (&[], 0.0));
    for (keep, to) in [(false, 0), (true, 1)] {
        d.carry = d.pick(1, at(1), false);
        d.carry_to(Some(at(to)));
        d.drop(keep, &mut fx);
        assert!(d.carry.is_none() && fx.len() == 1 && d.favs == names(&["x", "b", "a"]));
    }
    // A finger held on one picks it up at once: lifted unmoved it holds its menu's place; moved
    // 8 px from there it drags. A tile gone from the kept puts it down.
    d.carry = d.pick(1, at(1), true);
    assert_eq!(d.carry.as_ref().and_then(|c| c.held()), Some(at(1)));
    assert!(!d.carry_to(Some((at(1).0 - 7.0, 768.0))) && d.carry_to(Some(at(0))));
    d.layout(vec![1], 1, size, (&[], 0.0));
    assert!(d.carry.is_none());
    // Kept or not by the menus, the preference following.
    d.keep("c", true, &mut fx);
    d.keep("x", false, &mut fx);
    d.keep("assistant", true, &mut fx);
    let pref = |v: &str| Effect::Pref { key: "dock".into(), value: v.into() };
    assert_eq!(fx[1..], [pref("x,b,a,c"), pref("b,a,c")]);
}

#[test]
fn icons_carried_onto_the_row_open_a_gap_where_the_dock_keeps_their_apps() {
    let names = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    // Kept a, x (unknown: it does not show) and b, then one running. Files, a and the Assistant
    // carried over b: one gap opens there (never for the Assistant, nor for one kept), b and the
    // running one moving over; the gap is no tile's.
    let (mut d, mut fx, size) = (Dock::new(Some("a,x,b")), Vec::new(), (1280.0, 800.0));
    d.layout(vec![0, 2], 3, size, (&names(&["files", "a", "assistant"]), 84.0));
    assert_eq!((&d.incoming[..], d.strip.gap), (&names(&["files"])[..], (1, 1)));
    let xs = [0, 1, 2].map(|i| d.strip.tile(i).x);
    assert_eq!((xs, d.strip.sep), ([10.0, 114.0, 175.0], Some(166.0)));
    assert_eq!([84.0, 130.0].map(|x| d.strip.at(x, 760.0)), [None, Some(Spot::Tile(1))]);
    // Dropped: kept there, before b (x stays where it was).
    d.take_in(&mut fx);
    assert_eq!(fx, [Effect::Pref { key: "dock".into(), value: "a,x,files,b".into() }]);
    // Past the kept tiles (over a running one, or the Assistant) the gap follows them, as wide
    // as the apps carried in; with none, no gap, and nothing is kept.
    d.layout(vec![0, 3], 3, size, (&names(&["c", "d"]), 1250.0));
    assert_eq!(d.strip.gap, (2, 2));
    d.take_in(&mut fx);
    assert_eq!(d.favs, names(&["a", "x", "files", "b", "c", "d"]));
    d.layout(vec![0, 3], 3, size, (&[], 84.0));
    d.take_in(&mut fx);
    assert!(d.strip.gap == (0, 0) && d.incoming.is_empty() && fx.len() == 2);
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
fn the_bar_holds_the_mark_feedback_settings_and_the_clock() {
    use super::bar::{self, Button};
    // The mark at the left, Feedback and Settings at the right, each 44 px; each shows its app.
    let at = |w, x| bar::at(w, x, 22.0);
    let wide = [at(1280.0, 27.0), at(1280.0, 1209.0), at(1280.0, 1253.0), at(1280.0, 640.0)];
    let (m, f, s) = (Some(Button::Mark), Some(Button::Feedback), Some(Button::Settings));
    assert_eq!((wide, at(390.0, 368.0), bar::at(390.0, 368.0, 50.0)), ([m, f, s, None], s, None));
    let apps = [Button::Mark, Button::Feedback, Button::Settings].map(Button::app);
    assert_eq!(apps, ["welcome", "feedback", "settings"]);
    // Drawn: the hovered button washed; the date beside the time where it fits, the time alone
    // on a phone.
    let (mut text, t) = (text(), &THEMES[0]);
    let time = host::LocalTime { year: 2026, month: 10, day: 1, weekday: 3, hour: 9, minute: 5 };
    let mut drawn = |w: f32| {
        let mut l = DrawList::new();
        bar::draw(&mut l, &mut text, t, w, (Some((Button::Mark, true)), Some(time)));
        let glyph = Kind::Glyph as u8 as f32;
        (fills(&l, t.wash(true)), l.instances().iter().filter(|i| i.kind == glyph).count())
    };
    let ((washed, wide), (_, phone)) = (drawn(1280.0), drawn(390.0));
    assert!(washed && wide >= phone + 7, "{wide} {phone}");
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
    // An icon is hit in the cell it shows in.
    let spots = [0, 7, 59];
    let hits = [(110.0, 160.0), (5.0, 50.0), (60.0, 110.0), (900.0, 600.0)];
    assert_eq!(
        hits.map(|(x, y)| icons::at(&spots, a, false, x, y)),
        [Some(1), None, Some(0), Some(2)]
    );
    let phone = RectF::new(0.0, 44.0, 375.0, 631.0);
    assert_eq!(at(icons::cell(5, phone, true)), (100.0, 153.0, 87.0, 96.0));
    // Too short a work area for one icon a column: one row.
    assert_eq!(
        at(icons::cell(3, RectF::new(0.0, 0.0, 400.0, 50.0), false)),
        (280.0, 13.0, 89.0, 96.0)
    );
    // 14 columns of 6 wide, 4 of 6 on the phone. A drop goes to the cell under it, any of
    // them, empty or not; outside the grid, the nearest.
    let (wide, narrow) = (icons::dims(a, false), icons::dims(phone, true));
    assert_eq!((wide, narrow), (Dims::new(14, 6, false), Dims::new(4, 6, true)));
    let slots = [(60.0, 110.0), (150.0, 120.0), (60.0, 290.0), (900.0, 700.0), (-5.0, 9e9)];
    assert_eq!(slots.map(|p| icons::slot(a, false, p)), [0, 6, 2, 59, 5]);
    assert_eq!(icons::slot(phone, true, (330.0, 110.0)), 3);
    assert_eq!(icons::slot(phone, true, (330.0, 600.0)), 23);
}

#[test]
fn places_read_the_old_order_and_keep_each_layouts_cells() {
    fn names(p: &[Place]) -> Vec<&str> {
        p.iter().map(|p| p.name.as_str()).collect()
    }
    // The order as kept before: names alone, each once, no cells (packed in that order).
    let old = place::parse("terminal,,studio,terminal,/apps/a:b.app");
    assert_eq!(names(&old), ["terminal", "studio", "/apps/a:b.app"]);
    assert!(old.iter().all(|p| p.cells == [None, None]));
    // Now: `@2`, then `name:wide:narrow`, a cell `col.row` or none; a name may hold colons.
    let stored = "@2,studio:0.0:3.1,/apps/x:y.app::0.0,files:2.5:,bad:x.1:9";
    let p = place::parse(stored);
    assert_eq!(names(&p), ["studio", "/apps/x:y.app", "files", "bad"]);
    let cells = p.iter().map(|p| p.cells).collect::<Vec<_>>();
    let want =
        [[Some((0, 0)), Some((3, 1))], [None, Some((0, 0))], [Some((2, 5)), None], [None; 2]];
    assert_eq!(cells, want);
    assert_eq!(place::format(&p), "@2,studio:0.0:3.1,/apps/x:y.app::0.0,files:2.5:,bad::");
    assert_eq!((place::parse("@2"), place::parse("")), (vec![], vec![]));
    assert_eq!(names(&place::parse("@2,files")), ["files"]);
    let odd = place::parse("@2,a:+1.0:65536.0,b:12.65535:.3");
    assert_eq!(
        odd.iter().map(|p| p.cells).collect::<Vec<_>>(),
        [[None; 2], [Some((12, 65535)), None]]
    );
    // The apps as kept first (unknown ones dropped), new ones after with no cells.
    let apps = ["studio", "assistant", "terminal", "files", "clock.app"].map(String::from);
    let (got, kept, new) = place::arrange(&old, apps.to_vec(), |n| n.as_str());
    assert_eq!((got[..3].join(","), new), ("terminal,studio,assistant".to_string(), true));
    assert_eq!((names(&kept)[2], kept[2].cells), ("assistant", [None; 2]));
    let (_, _, new) = place::arrange(
        &place::parse("studio,clock.app,files,assistant,terminal"),
        apps.to_vec(),
        |n| n.as_str(),
    );
    assert!(!new);
}

#[test]
fn icons_stay_in_their_cells_and_the_rest_fill_the_first_free_ones() {
    let (wide, phone) = (Dims::new(3, 2, false), Dims::new(4, 2, true));
    // Reading order: down the columns wide, across the rows on a phone; past the cells shown,
    // more columns or rows.
    assert_eq!(
        [wide.pos((1, 0)), wide.pos((0, 1)), wide.pos((3, 0)), wide.pos((0, 2))],
        [Some(2), Some(1), None, None]
    );
    assert_eq!([phone.pos((1, 0)), phone.pos((0, 1)), phone.pos((4, 0))], [Some(1), Some(4), None]);
    assert_eq!([wide.cell(7), phone.cell(9)], [(3, 1), (1, 2)]);
    assert_eq!([wide.nearest(9, 9), phone.nearest(9, 0)], [5, 3]);
    // Each where its cell is in this layout (the first to claim one), the rest in order in the
    // first free cells: a gap stays a gap.
    let at = |name: &str, w: Option<(u16, u16)>, n: Option<(u16, u16)>| Place {
        name: name.into(),
        cells: [w, n],
    };
    let p = [
        at("a", Some((2, 1)), None),
        at("b", None, Some((3, 1))),
        at("c", Some((2, 1)), None),
        at("d", Some((0, 1)), Some((0, 0))),
    ];
    assert_eq!(
        (place::resolve(&p, wide), place::resolve(&p, phone)),
        (vec![5, 0, 2, 1], vec![1, 7, 2, 0])
    );
    // A cell off a smaller screen is kept for a larger one; meanwhile its icon takes a free one,
    // past those shown if they are full.
    let small = Dims::new(1, 2, false);
    assert_eq!(place::resolve(&p, small), [0, 2, 3, 1]);
    // Kept: where they show, in this layout only; but a cell off this screen stays (as when a
    // phone's keyboard shortens it), unless all are kept as they show (a drop: what the person
    // sees is what stays).
    let mut q = p.to_vec();
    place::keep(&mut q, &[0, 2, 3, 1], small, false);
    let cells = |q: &[Place]| q.iter().map(|p| p.cells[0]).collect::<Vec<_>>();
    assert_eq!(cells(&q), [Some((2, 1)), Some((1, 0)), Some((2, 1)), Some((0, 1))]);
    assert_eq!(q[1].cells[1], Some((3, 1)));
    place::keep(&mut q, &[0, 2, 3, 1], small, true);
    assert_eq!(cells(&q), [Some((0, 0)), Some((1, 0)), Some((1, 1)), Some((0, 1))]);
    // A layout never arranged keeps none but on a drop: it packs as the screen's size changes.
    let mut q = [Place::new("a"), Place::new("b")];
    place::keep(&mut q, &[0, 1], small, false);
    assert_eq!(q.map(|p| p.cells), [[None; 2]; 2]);
}

#[test]
fn drops_land_in_any_cell_those_in_the_way_moving_along_to_the_first_gap() {
    let d = Dims::new(4, 3, true);
    let plan =
        |spots: &[usize], carried: &[usize], to| place::plan(spots, carried, carried[0], to, d);
    // a b c d / e _ _ _ / _ _ _ _: a dropped on an empty cell goes there; the rest stay, and its
    // own cell stays empty.
    let row = [0, 1, 2, 3, 4];
    assert_eq!(plan(&row, &[0], 10), [10, 1, 2, 3, 4]);
    // Dropped on c: c, then d and e, move a cell on, as far as the first gap.
    assert_eq!(plan(&row, &[0], 2), [2, 1, 3, 4, 5]);
    // e dropped on b: b, c and d move on into the cell e left; a is not in the way.
    assert_eq!(plan(&row, &[4], 1), [0, 2, 3, 4, 1]);
    // Dropped where it was, nothing moves.
    assert_eq!(plan(&row, &[2], 2), row);
    // A grid full from there to its end: those in the way move back instead.
    let full = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    assert_eq!(plan(&full, &[0], 9), [9, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11]);
    // Full both ways (more icons than cells, the one past the end carried in): on past its end.
    let all: Vec<usize> = (0..13).collect();
    assert_eq!(plan(&all, &[12], 5), [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 5]);
    // Several carried keep their places around the one pressed: a and b (a diagonal) dropped
    // with a a cell on, b on c's cell: c and d move on.
    let spots = [0, 5, 6, 7];
    assert_eq!(plan(&spots, &[0, 1], 1), [1, 6, 7, 8]);
    // Kept inside the grid: dropped at the right edge, the group moves left to fit; past the
    // top-left corner, b pressed, it stays whole where it was.
    assert_eq!(plan(&spots, &[0, 1], 3), [2, 7, 6, 8]);
    assert_eq!(place::plan(&spots, &[0, 1], 1, 4, d), spots);
    // A group larger than the grid (one past its last row) lands as a run, in order.
    assert_eq!(place::plan(&[0, 12], &[0, 1], 0, 8, d), [8, 9]);
    // Headed home (one past the grid's end, carried below it), a lead not among the carried, or
    // one past the icons: nothing moves.
    assert_eq!(place::plan(&[0, 1, 13], &[2], 2, 13, d), [0, 1, 13]);
    assert_eq!(place::plan(&row, &[1], 0, 9, d), row);
    assert_eq!(place::plan(&row, &[9], 9, 9, d), row);
}

#[test]
fn icons_draw_selected_lifted_and_as_marks() {
    // Drawn: selected, the accent's ring and wash; carried, larger and shadowed; a `.app` file's
    // tile shows its mark (its sigil, or the icon it made), not its glyph.
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
    let sigil = Some(&Mark::Sigil(7));
    icons::draw(&mut lifted, &mut text, t, r, (AppIcon::default(), sigil, "Clock"), state);
    let (tile, big) = (kind(&list, Kind::Gradient)[0].0, kind(&lifted, Kind::Gradient)[0].0);
    assert!(big > tile && kind(&lifted, Kind::Shadow).len() > kind(&list, Kind::Shadow).len());
    let glyph = |l: &DrawList| kind(l, Kind::Glyph).into_iter().find(|g| g.0 > 20.0).map(|g| g.1);
    assert_ne!(glyph(&lifted), glyph(&list));
    let made = Mark::Made(ui::icon::Made::parse(b"ring 12 12 8").unwrap());
    let mut own = DrawList::new();
    icons::draw(&mut own, &mut text, t, r, (AppIcon::default(), Some(&made), "Clock"), state);
    assert_eq!(kind(&own, Kind::Gradient), kind(&lifted, Kind::Gradient));
    assert!(glyph(&own).is_some() && glyph(&own) != glyph(&lifted) && glyph(&own) != glyph(&list));
    icons::draw_box(&mut list, &text, t, (300.0, 200.0), (100.0, 400.0));
    assert_eq!(list.instances().last().map(|i| i.rect), Some([100.0, 200.0, 200.0, 200.0]));
}

/// A 1280 x 800 desktop's grid area.
const AREA: RectF = RectF { x: 0.0, y: 44.0, w: 1280.0, h: 694.0 };

/// The home screen of `names`, as the host lists them.
fn entries(names: &[&str]) -> Vec<Entry> {
    let entry = |n: &&str| {
        let (name, label, icon) = (n.to_string(), host::app_label(n), AppIcon::default());
        Entry { name, label, icon, mark: None }
    };
    names.iter().map(entry).collect()
}

/// The built-in apps' grid on a 1280 x 800 desktop in the `stored` order, listed and at rest.
fn grid(stored: Option<&str>) -> Grid {
    let (mut g, mut fx) = (Grid::new(stored), Vec::new());
    (g.area, g.narrow) = (AREA, false);
    assert!(g.list(1, || entries(&APPS), &mut fx) && fx.is_empty());
    assert!(!g.list(1, || unreachable!(), &mut fx));
    g.sync(0.0, true);
    g
}

/// The middle of the icon in cell `i`.
fn mid(i: usize) -> (f32, f32) {
    let r = icons::cell(i, AREA, false);
    (r.x + r.w / 2.0, r.y + 30.0)
}

fn labels(g: &Grid) -> Vec<&str> {
    g.icons.iter().map(|e| &*e.label).collect()
}

/// The preference that keeps `value`.
fn pref(value: &str) -> Effect {
    Effect::Pref { key: "home.order".into(), value: value.into() }
}

/// Where the icon of `name` shows.
fn spot(g: &Grid, name: &str) -> Option<usize> {
    g.icons.iter().position(|e| e.name == name).map(|i| g.spots[i])
}

#[test]
fn icons_carried_past_their_travel_land_in_any_cell_and_stay() {
    let (mut g, mut fx) = (grid(None), Vec::new());
    // A mouse carries an icon once it travels 4 px: it follows the pointer; over another, that
    // one and those after it up to the first empty cell slide a cell on to make room.
    g.press(Press::Icon(0), mid(0), false);
    assert!(!g.carry_to(Some((mid(0).0 + 3.0, mid(0).1))));
    assert!(g.carry.as_ref().is_some_and(|c| !c.lifted));
    assert!(g.carry_to(Some(mid(3))) && g.carry.as_ref().map(|c| c.slot) == Some(3));
    let cells = [None, Some(1), Some(2), Some(4), Some(5)];
    assert_eq!((g.at(mid(3).0, mid(3).1), &g.cells()[..5]), (None, &cells[..]));
    g.sync(10.0, false);
    assert!(g.moving(10.0));
    // Dropped: there it stays, its own cell stays empty, and every cell is kept (this layout's;
    // the phone's has none yet), the icons in their order.
    g.drop(true, Some(mid(3)), &mut fx);
    assert_eq!((labels(&g)[0], &g.spots), ("Studio", &vec![3, 1, 2, 4, 5, 6, 7, 8, 9]));
    let kept = "@2,studio:0.3:,assistant:0.1:,terminal:0.2:,files:0.4:,editor:0.5:,".to_string()
        + "settings:1.0:,feedback:1.1:,about:1.2:,welcome:1.3:";
    assert_eq!(fx, [pref(&kept)]);
    // Dropped on an empty cell far off, it goes there and nothing else moves; reloaded, every
    // icon is where it was.
    let far = mid(40);
    g.press(Press::Icon(2), mid(2), false);
    g.carry_to(Some(far));
    g.drop(true, Some(far), &mut fx);
    assert_eq!(g.spots, [3, 1, 40, 4, 5, 6, 7, 8, 9]);
    let Some(Effect::Pref { value, .. }) = fx.last() else { panic!("kept") };
    let h = grid(Some(value.as_str()));
    assert_eq!((labels(&h), &h.spots), (labels(&g), &g.spots));
    // Escape, or the pointer leaving (a drop that keeps nothing), puts it back: it slides back
    // from where it showed, and nothing changes.
    for escape in [true, false] {
        g.press(Press::Icon(0), mid(3), false);
        g.carry_to(Some(mid(6)));
        match escape {
            true => assert_eq!(g.key(Key::Escape, Mods::default(), Some(mid(6))), Some(vec![])),
            false => g.drop(false, Some(mid(6)), &mut fx),
        }
        let r = g.cells[0].value(2000.0).rect;
        assert!(g.carry.is_none() && (r.x + r.w / 2.0, r.y + 30.0) == mid(6) && fx.len() == 2);
    }
    // Moved, but landing where it was: no change, nothing kept.
    let near = (mid(3).0 + 9.0, mid(3).1);
    g.press(Press::Icon(0), mid(3), false);
    g.carry_to(Some(near));
    g.drop(true, Some(near), &mut fx);
    assert!(fx.len() == 2 && g.spots[0] == 3);
    // Carried below the grid (over the bottom row), it heads back to its cell, its app offered
    // to the dock; dropped there, nothing moves.
    let low = (300.0, 760.0);
    g.press(Press::Icon(0), mid(3), false);
    assert!(g.carry_to(Some(low)) && g.below(Some(mid(3))).is_empty());
    let got = (g.carry.as_ref().map(|c| c.slot), g.below(Some(low)));
    assert_eq!(got, (Some(3), vec!["studio".to_string()]));
    g.drop(true, Some(low), &mut fx);
    assert!(fx.len() == 2 && spot(&g, "studio") == Some(3) && g.below(Some(low)).is_empty());
    // The order as kept before (names alone): unknown names dropped, new apps last, packed;
    // nothing kept yet. An app new since (a `.app` saved) takes the first free cell, and the
    // order is kept (a layout never arranged keeps no cells: it packs as the screen changes).
    let mut g = grid(Some("welcome,gone,terminal"));
    #[rustfmt::skip]
    let want = ["Welcome", "Terminal", "Studio", "Assistant", "Files", "Editor", "Settings",
        "Feedback", "About"];
    assert_eq!((labels(&g), &g.spots), (want.to_vec(), &(0..9).collect::<Vec<_>>()));
    let all: Vec<&str> = APPS.iter().copied().chain(["/apps/clock.app"]).collect();
    assert!(g.list(2, || entries(&all), &mut fx) && (labels(&g)[9], g.spots[9]) == ("Clock", 9));
    let kept = "@2,welcome::,terminal::,studio::,assistant::,files::,editor::,settings::,";
    let kept = [kept, "feedback::,about::,/apps/clock.app::"].concat();
    assert_eq!(fx.last(), Some(&pref(&kept)));
}

#[test]
fn a_phone_keeps_icons_where_dropped_and_a_wide_screen_its_own_arrangement() {
    // A 411 x 794 phone: rows of four, six of them. The order as kept before, packed.
    let phone = RectF::new(0.0, 44.0, 411.0, 688.0);
    let mid = |g: &Grid, i: usize| {
        let r = icons::cell(i, g.area, g.narrow);
        (r.x + r.w / 2.0, r.y + 30.0)
    };
    let (mut g, mut fx) = (Grid::new(Some("files,studio")), Vec::new());
    (g.area, g.narrow) = (phone, true);
    g.list(1, || entries(&APPS), &mut fx);
    g.sync(0.0, true);
    assert_eq!(
        (&labels(&g)[..3], &g.spots),
        (&["Files", "Studio", "Assistant"][..], &(0..9).collect::<Vec<_>>())
    );
    // A finger holds Studio, drags it to the last cell, lifts: it stays there, no other moves.
    g.carry = g.pick(1, mid(&g, 1), true);
    g.carry_to(Some(mid(&g, 23)));
    g.drop(true, Some(mid(&g, 23)), &mut fx);
    assert_eq!((spot(&g, "studio"), &g.spots), (Some(23), &vec![0, 23, 2, 3, 4, 5, 6, 7, 8]));
    let kept = "@2,files::0.0,studio::3.5,assistant::2.0,terminal::3.0,editor::0.1,".to_string()
        + "settings::1.1,feedback::2.1,about::3.1,welcome::0.2";
    let kept = kept.as_str();
    assert_eq!(fx, [pref(kept)]);
    // Reloaded, every icon is where it was. A new app takes the first free cell, Studio's old.
    let mut h = Grid::new(Some(kept));
    (h.area, h.narrow) = (phone, true);
    let all: Vec<&str> = APPS.iter().copied().chain(["/apps/clock.app"]).collect();
    h.list(1, || entries(&APPS), &mut fx);
    assert_eq!((labels(&h), &h.spots), (labels(&g), &g.spots));
    h.list(2, || entries(&all), &mut fx);
    assert!(spot(&h, "/apps/clock.app") == Some(1) && spot(&h, "studio") == Some(23));
    // The keyboard shortens the screen to two rows: Studio, off it, shows in the first free
    // cell meanwhile; back, it is home again.
    h.area.h = 250.0;
    h.sync(0.0, true);
    assert_eq!(spot(&h, "studio"), Some(8));
    h.area.h = 688.0;
    h.sync(0.0, true);
    assert_eq!(spot(&h, "studio"), Some(23));
    // A wide screen keeps its own arrangement: never arranged there, the icons packed in their
    // order (as kept before, the new one last). Arranged there (Files carried to the next
    // column), the phone's stays as it was, and both are kept.
    (h.area, h.narrow) = (AREA, false);
    h.sync(0.0, true);
    assert_eq!((h.spots.clone(), labels(&h)[9]), ((0..10).collect::<Vec<_>>(), "Clock"));
    h.press(Press::Icon(0), mid(&h, 0), false);
    h.carry_to(Some(mid(&h, 6)));
    h.drop(true, Some(mid(&h, 6)), &mut fx);
    assert_eq!(h.spots, [6, 1, 2, 3, 4, 5, 7, 8, 9, 10]);
    let Some(Effect::Pref { value, .. }) = fx.last() else { panic!("kept") };
    let ends = value.ends_with(",welcome:1.3:0.2,/apps/clock.app:1.4:1.0");
    assert!(value.starts_with("@2,files:1.0:0.0,studio:0.1:3.5,") && ends, "{value}");
    (h.area, h.narrow) = (phone, true);
    h.sync(0.0, true);
    let names = ["files", "/apps/clock.app", "assistant", "studio"];
    assert_eq!(names.map(|n| spot(&h, n)), [Some(0), Some(1), Some(2), Some(23)]);
}

#[test]
fn a_box_selects_icons_which_open_and_move_together_and_a_finger_picks_one_up() {
    let (mut g, mut fx, none) = (grid(None), Vec::new(), Mods::default());
    let boxed = |g: &mut Grid, to: (f32, f32)| {
        g.press(Press::Desktop, (5.0, 50.0), false);
        g.lasso_to(Some(to));
        g.lasso = None;
    };
    // From the bare desktop a mouse draws a box; the icons it touches are selected.
    boxed(&mut g, (95.0, 260.0));
    assert_eq!(g.selected, ["studio", "assistant", "terminal"]);
    // A press in a menu keeps it; one on the bare desktop ends it, as Escape does.
    g.press(Press::Menu, (5.0, 50.0), false);
    assert_eq!(g.selected.len(), 3);
    g.press(Press::Desktop, (600.0, 640.0), false);
    assert!(g.selected.is_empty());
    boxed(&mut g, (95.0, 260.0));
    assert!(g.key(Key::Escape, none, None) == Some(vec![]) && g.selected.is_empty());
    // Enter opens what is selected (its names), and the selection ends; other keys aren't theirs.
    boxed(&mut g, (95.0, 160.0));
    assert_eq!(g.key(Key::Char('x'), none, None), None);
    let open = g.key(Key::Enter, none, None);
    assert!(open == Some(vec!["studio".into(), "assistant".into()]) && g.selected.is_empty());
    // Dragging a selected icon carries the selection: they land together, as they were around
    // the one pressed (kept inside the grid: one above the bottom cell), those in the way moving
    // on; the cells they left stay empty.
    boxed(&mut g, (95.0, 160.0));
    g.press(Press::Icon(1), mid(1), false);
    assert_eq!(g.carry.as_ref().map(|c| c.icons.clone()), Some(vec![0, 1]));
    g.carry_to(Some(mid(5)));
    g.drop(true, Some(mid(5)), &mut fx);
    assert_eq!(g.spots, [4, 5, 2, 3, 6, 7, 8, 9, 10]);
    // Carried, the one pressed shows where the pointer holds it, two of the rest behind it.
    let mut g = grid(None);
    g.selected = ["studio", "assistant", "terminal", "files"].map(String::from).into();
    g.press(Press::Icon(2), mid(2), false);
    g.carry_to(Some((300.0, 300.0)));
    let (shown, r) = (g.carried(Some((300.0, 300.0))), icons::cell(2, AREA, false));
    assert_eq!(shown.iter().map(|s| s.0).collect::<Vec<_>>(), [2, 0, 1]);
    let (lead, behind) = (shown[0].1, shown[1].1);
    assert_eq!((lead.x, behind.x, behind.y), (300.0 - mid(2).0 + r.x, lead.x + 5.0, lead.y - 5.0));
    // A finger draws no box. Held, it picks an icon up at once where it is (it drifted 9 px
    // while it waited), lifted and unmoved; moved 8 px from there it drags.
    g.press(Press::Desktop, (600.0, 640.0), true);
    assert!(g.lasso.is_none() && g.carry.is_none());
    g.carry = g.pick(0, (mid(0).0 + 9.0, mid(0).1), true);
    assert!(g.carry.as_ref().is_some_and(|c| c.lifted && !c.moved && c.touch));
    assert!(!g.carry_to(Some((mid(0).0 + 16.0, mid(0).1))));
    assert!(g.carry_to(Some((mid(0).0 + 17.0, mid(0).1))));
}
