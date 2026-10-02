use gfx::{DrawList, Kind, RectF, Rgba};
use host::{Effect, Entry};
use ui::{AppIcon, Key, Mods, THEMES, TextSystem};

use super::dock::{Look, Spot, Strip, favorites, pin};
use super::grid::{APPS, Grid, Press};
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
fn the_strip_centers_the_ai_button_and_the_dock_above_it_only_while_it_holds_any() {
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
    // Nothing kept, nothing running: the button alone, 64 px at the bottom center, 13 px above
    // the screen's bottom; no dock at all, nothing to hit but the button.
    let alone = Strip::new(0, 0, (1280.0, 800.0));
    assert_eq!(
        (alone.button, alone.shelf.w, alone.top()),
        (RectF::new(608.0, 723.0, 64.0, 64.0), 0.0, 723.0)
    );
    assert_eq!(
        [alone.at(640.0, 755.0), alone.at(640.0, 700.0), alone.at(560.0, 755.0)],
        [Some(Spot::Button), None, None]
    );
    let (mut list, mut text, t) = (DrawList::new(), text(), &THEMES[0]);
    alone.draw(&mut list, &mut text, t, &[]);
    assert!(list.instances().is_empty());
    // Two favorites, then one running app: one shelf centered above the button (8 px between),
    // a hairline between the groups; a tile takes half the space beside it.
    let s = Strip::new(2, 1, (1280.0, 800.0));
    assert_eq!((s.button, s.tile, s.top()), (alone.button, 44.0, 651.0));
    assert_eq!((s.shelf, s.sep), (RectF::new(554.0, 651.0, 173.0, 64.0), Some(666.0)));
    assert_eq!(
        (&s.xs[..], s.tile(2)),
        (&[562.0, 614.0, 675.0][..], RectF::new(675.0, 659.0, 44.0, 44.0))
    );
    let at = |x, y| s.at(x, y);
    assert_eq!(
        [at(640.0, 755.0), at(570.0, 670.0), at(612.0, 670.0), at(556.0, 670.0), at(666.0, 670.0)],
        [
            Some(Spot::Button),
            Some(Spot::Tile(0)),
            Some(Spot::Tile(1)),
            Some(Spot::Shelf),
            Some(Spot::Shelf)
        ]
    );
    assert_eq!((at(700.0, 645.0), at(640.0, 719.0), s.tile(3)), (None, None, s.button));
    // A phone: the button centered and 64 px still; tiles shrink evenly to fit 8 apps or more
    // between 13 px margins, never off the screen.
    let phone = Strip::new(1, 3, (390.0, 844.0));
    assert_eq!((phone.button, phone.tile), (RectF::new(163.0, 767.0, 64.0, 64.0), 44.0));
    assert_eq!(phone.shelf, RectF::new(83.0, 695.0, 225.0, 64.0));
    for (favs, others, tile) in
        [(0, 8, 36.0), (4, 4, 35.0), (0, 9, 31.0), (6, 6, 20.0), (20, 0, 9.0)]
    {
        let s = Strip::new(favs, others, (390.0, 844.0));
        assert_eq!((s.tile, s.xs.len(), s.button.x), (tile, favs + others, 163.0));
        let (l, r) = (s.shelf.x, s.shelf.x + s.shelf.w);
        assert!(l >= 13.0 && r <= 377.0 && (l + r - 390.0).abs() <= 1.0, "{favs} {others}");
        assert!(s.xs.iter().all(|&x| x >= l && x + s.tile <= r), "{favs} {others}");
    }
    // The work area leaves the button's row, and the dock's while it shows.
    assert_eq!((super::dock::clear(false), super::dock::clear(true)), (85.0, 157.0));
    // Drawn: the shelf and tiles, the hairline, the focused app's dot in the accent, the others'
    // dim; the button's glass, washed while held.
    let look = |running, focused| Look {
        icon: AppIcon::default(),
        sigil: None,
        lift: 0.0,
        running,
        focused,
    };
    s.draw(&mut list, &mut text, t, &[look(true, true), look(true, false), look(false, false)]);
    assert!(fills(&list, t.accent) && fills(&list, t.text_dim) && fills(&list, t.glass));
    assert!(fills(&list, t.border));
    s.draw_button(&mut list, &mut text, t, Some(true));
    assert!(fills(&list, t.wash(true)));
}

#[test]
fn the_overlay_sits_above_the_strip_a_card_a_sheet_or_a_pill() {
    // Wide: a card 560 px wide at most, 60% of the screen tall at most, centered above the
    // button (or the dock, while it shows); working, a pill 52 px tall, 420 px wide at most.
    let (s, alone) = (Strip::new(2, 1, (1280.0, 800.0)), Strip::new(0, 0, (1280.0, 800.0)));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, false), RectF::new(360.0, 163.0, 560.0, 480.0));
    assert_eq!(s.overlay((1280.0, 800.0), 44.0, true), RectF::new(430.0, 591.0, 420.0, 52.0));
    assert_eq!(alone.overlay((1280.0, 800.0), 44.0, false), RectF::new(360.0, 235.0, 560.0, 480.0));
    assert_eq!(alone.overlay((1280.0, 800.0), 44.0, true), RectF::new(430.0, 663.0, 420.0, 52.0));
    // A short screen keeps it under the bar; a narrow one inside its gutters.
    assert_eq!(Strip::new(2, 1, (1280.0, 300.0)).overlay((1280.0, 300.0), 44.0, false).y, 52.0);
    let phone = Strip::new(1, 3, (390.0, 844.0));
    let sheet = phone.overlay((390.0, 844.0), 44.0, false);
    assert_eq!(sheet, RectF::new(16.0, 265.0, 358.0, 422.0));
    assert_eq!(phone.overlay((390.0, 844.0), 44.0, true), RectF::new(16.0, 635.0, 358.0, 52.0));
    let bare = Strip::new(0, 0, (390.0, 844.0)).overlay((390.0, 844.0), 44.0, false);
    assert_eq!(bare, RectF::new(16.0, 337.0, 358.0, 422.0));
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

/// A 1280 x 800 desktop's grid area.
const AREA: RectF = RectF { x: 0.0, y: 44.0, w: 1280.0, h: 671.0 };

/// The home screen of `names`, as the host lists them.
fn entries(names: &[&str]) -> Vec<Entry> {
    let entry = |n: &&str| {
        let (name, label, icon) = (n.to_string(), host::app_label(n), AppIcon::default());
        Entry { name, label, icon, sigil: None }
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

#[test]
fn icons_carried_past_their_travel_land_in_the_persons_order() {
    let (mut g, mut fx) = (grid(None), Vec::new());
    // A mouse carries an icon once it travels 4 px: it follows the pointer, the others close up
    // around its slot and slide.
    g.press(Press::Icon(0), mid(0), false);
    assert!(!g.carry_to(Some((mid(0).0 + 3.0, mid(0).1))));
    assert!(g.carry.as_ref().is_some_and(|c| !c.lifted));
    assert!(g.carry_to(Some(mid(3))) && g.carry.as_ref().map(|c| c.slot) == Some(3));
    let cells = [None, Some(0), Some(1), Some(2), Some(4)];
    assert_eq!((g.at(mid(3).0, mid(3).1), &g.cells()[..5]), (None, &cells[..]));
    g.sync(10.0, false);
    assert!(g.moving(10.0));
    // Dropped: there it stays, and the order is kept.
    g.drop(true, Some(mid(3)), &mut fx);
    assert_eq!(labels(&g)[..4], ["Assistant", "Terminal", "Files", "Studio"]);
    let order = "assistant,terminal,files,studio,activity,settings,feedback,about,welcome";
    assert_eq!(fx, [Effect::Pref { key: "home.order".into(), value: order.into() }]);
    // Escape, or the pointer leaving (a drop that keeps nothing), puts it back: it slides back
    // from where it showed, and nothing changes.
    for escape in [true, false] {
        g.press(Press::Icon(0), mid(0), false);
        g.carry_to(Some(mid(6)));
        match escape {
            true => assert_eq!(g.key(Key::Escape, Mods::default(), Some(mid(6))), Some(vec![])),
            false => g.drop(false, Some(mid(6)), &mut fx),
        }
        let r = g.cells[0].value(2000.0).rect;
        assert!(g.carry.is_none() && (r.x + r.w / 2.0, r.y + 30.0) == mid(6) && fx.len() == 1);
    }
    // Moved, but landing where it was: no change, nothing kept.
    let near = (mid(1).0 + 9.0, mid(1).1);
    g.press(Press::Icon(1), mid(1), false);
    g.carry_to(Some(near));
    g.drop(true, Some(near), &mut fx);
    assert!(fx.len() == 1 && labels(&g)[1] == "Terminal");
    // The order comes from the page: unknown names dropped, new apps last, nothing kept yet. An
    // app new since (a `.app` saved) shows up last, and the order is kept.
    let mut g = grid(Some("welcome,gone,terminal"));
    let want = ["Welcome", "Terminal", "Studio", "Assistant", "Files", "Activity", "Settings"];
    assert_eq!(labels(&g), [&want[..], &["Feedback", "About"]].concat());
    let all: Vec<&str> = APPS.iter().copied().chain(["/apps/clock.app"]).collect();
    assert!(g.list(2, || entries(&all), &mut fx) && labels(&g)[9] == "Clock");
    let order =
        "welcome,terminal,studio,assistant,files,activity,settings,feedback,about,/apps/clock.app";
    assert_eq!(fx.last(), Some(&Effect::Pref { key: "home.order".into(), value: order.into() }));
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
    // Dragging a selected icon carries the selection: they land together, in their order.
    boxed(&mut g, (95.0, 160.0));
    g.press(Press::Icon(1), mid(1), false);
    assert_eq!(g.carry.as_ref().map(|c| c.icons.clone()), Some(vec![0, 1]));
    g.carry_to(Some(mid(5)));
    g.drop(true, Some(mid(5)), &mut fx);
    let order = ["Terminal", "Files", "Activity", "Settings", "Feedback", "Studio", "Assistant"];
    assert_eq!(labels(&g), [&order[..], &["About", "Welcome"]].concat());
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
