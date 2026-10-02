use gfx::{DrawList, Instance, Kind};
use ui::{Code, Hit, Key, THEMES, UiState};

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

/// What a frame drew: its instances, its hits, and the view after.
struct Drawn {
    list: DrawList,
    hits: Vec<Hit>,
}

impl Drawn {
    fn hit(&self, id: u32) -> Hit {
        *self.hits.iter().find(|h| h.id == WidgetId(id)).expect("a hit")
    }

    fn of(&self, kind: Kind) -> Vec<&Instance> {
        self.list.instances().iter().filter(|i| i.kind == kind as u8 as f32).collect()
    }

    /// Whether a fill of `color` covers `r` exactly.
    fn filled(&self, r: RectF, color: Rgba) -> bool {
        self.of(Kind::Fill).iter().any(|i| i.rect == [r.x, r.y, r.w, r.h] && i.color == color)
    }
}

/// `nodes` drawn at 600 x 400 (content at the origin) in Midnight, the window focused and the
/// pointer over `hover`.
fn draw(nodes: &[Node], texts: &mut Texts, view: &mut View, hover: Option<u32>) -> Drawn {
    draw_at(600.0, nodes, texts, view, hover)
}

/// The same, `w` wide.
fn draw_at(
    w: f32,
    nodes: &[Node],
    texts: &mut Texts,
    view: &mut View,
    hover: Option<u32>,
) -> Drawn {
    paint(w, 0.0, nodes, texts, view, hover).0
}

/// The same at page clock `now`, with the text system after (its atlas holds what was drawn).
fn paint(
    w: f32,
    now_ms: f64,
    nodes: &[Node],
    texts: &mut Texts,
    view: &mut View,
    hover: Option<u32>,
) -> (Drawn, TextSystem) {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(ui::FontId::Mono, MONO.to_vec()).unwrap();
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let state = UiState { focused: true, hover: hover.map(WidgetId), now_ms, ..UiState::default() };
    let rect = RectF::new(0.0, 0.0, w, 400.0);
    super::draw(
        &mut Ui::new(&mut list, &mut ts, rect, &mut hits, state, &THEMES[0]),
        nodes,
        texts,
        view,
    );
    (Drawn { list, hits }, ts)
}

/// Whether `d` has a scroll thumb: a 3 px fill in the faint ink.
fn thumb(d: &Drawn) -> Option<[f32; 4]> {
    let faint = |i: &&&Instance| i.color == THEMES[0].text_faint && i.rect[2] == 3.0;
    d.of(Kind::Fill).iter().find(faint).map(|i| i.rect)
}

/// A file's row, `id` and named `name`.
fn row(id: u32, name: &str) -> Node {
    let (text, detail) = (name.into(), String::new());
    Node::Entry { id, glyph: Glyph::File as u8, hue: 0x94a3b8, text, detail, more: false }
}

fn text(style: Style, s: &str) -> Node {
    Node::Text { id: 0, style, text: s.into() }
}

#[test]
fn glyphs_entries_toggles_and_chips_draw_in_the_theme() {
    let t = &THEMES[0];
    let entry = |id, glyph, more| {
        let (text, detail) = ("apps".into(), "1.5 KB".into());
        Node::Entry { id, glyph, hue: 0x60a5fa, text, detail, more }
    };
    let chips = vec![
        Node::Button { id: 1, variant: Variant::On, label: "Idea".into() },
        Node::Button { id: 2, variant: Variant::Chip, label: "Bug".into() },
        Node::Button { id: 3, variant: Variant::Quiet, label: "apps".into() },
        Node::Glyph { glyph: Glyph::Chevron as u8, size: 9 },
    ];
    let nodes = vec![
        Node::Glyph { glyph: Glyph::Mark as u8, size: 89 },
        Node::Row { id: 0, gap: 8, children: chips },
        Node::Col { id: 0, gap: 0, children: vec![entry(5, 6, true), entry(6, 200, false)] },
        Node::Toggle { id: 7, on: true, label: "Include what\u{2019}s open".into() },
        Node::Toggle { id: 8, on: false, label: "Off".into() },
        Node::Glyph { glyph: 200, size: 21 },
    ];
    let (mut texts, mut view) = (Texts::default(), View::default());
    let d = draw(&nodes, &mut texts, &mut view, Some(3));
    // The mark: one glyph 89 px square at the corner; an unknown glyph is empty space.
    let glyphs = d.of(Kind::Glyph);
    let mark = |g: &&&Instance| (86.0..92.0).contains(&g.rect[2]) && g.rect[0] < PAD + 3.0;
    assert_eq!(glyphs.iter().filter(mark).count(), 1);
    // The chosen chip on the accent, the other on the sunken pill; the quiet one washed under
    // the pointer. All small, in a row.
    let (on, chip, quiet) = (d.hit(1).rect, d.hit(2).rect, d.hit(3).rect);
    assert!(
        d.filled(on, t.accent) && d.filled(chip, t.surface_lo) && d.filled(quiet, t.wash(false))
    );
    assert!(on.h == CHIP_H && quiet.h == BUTTON_H && chip.x > on.x + on.w && quiet.x > chip.x);
    // On a phone's window chips and quiet buttons are touch targets.
    let phone = draw_at(360.0, &nodes, &mut Texts::default(), &mut View::default(), None);
    assert_eq!([1, 2, 3].map(|id| phone.hit(id).rect.h), [TOUCH; 3]);
    assert!(phone.hit(3).rect.w >= BUTTON_H);
    // Entries: 55 tall, abutting, a tile each (none for an unknown glyph), a chevron when `more`.
    let (a, b) = (d.hit(5).rect, d.hit(6).rect);
    assert!(a.h == ENTRY_H && b.y == a.y + a.h && d.hit(5).sense == Sense::Click, "{a:?} {b:?}");
    let tiles = d.list.instances().iter().filter(|i| i.rect[2] == 34.0 && i.rect[3] == 34.0);
    assert_eq!(tiles.filter(|i| i.kind == Kind::Gradient as u8 as f32 || i.kind == 0.0).count(), 1);
    // The same detail at the right of each; the folder's chevron besides.
    let right = |r: RectF| {
        let at = |g: &&&Instance| g.rect[1] > r.y && g.rect[1] < r.y + r.h && g.rect[0] > 300.0;
        glyphs.iter().filter(at).count()
    };
    assert_eq!(right(a), right(b) + 1);
    // Toggles: 44 tall, the switch's track the accent when on, sunken when off.
    let (on, off) = (d.hit(7).rect, d.hit(8).rect);
    assert!(on.h == TOGGLE_H && off.y == on.y + on.h + SPACING);
    let track = |r: RectF, c| {
        d.of(Kind::Fill).iter().any(|i| {
            i.rect[2] == 34.0
                && i.rect[3] == 21.0
                && i.rect[1] > r.y
                && i.rect[1] < r.y + r.h
                && i.color == c
        })
    };
    assert!(track(on, t.accent) && track(off, t.surface_lo) && !track(on, t.surface_lo));
}

#[test]
fn panes_take_their_width_chips_are_small_and_fills_match_a_taller_sibling() {
    // Studio wide, on code: a Fill beside a taller Pane still reaches the bottom.
    let (mut texts, mut view) = (Texts::default(), View::default());
    texts.codes.push((4, Code::new("a", 1)));
    let (spans, text) = (Vec::new(), "a".into());
    let code = Node::Code { id: 4, version: 1, line_numbers: true, text, spans };
    let chip = Node::Button { id: 7, variant: Variant::Chip, label: "a dice roller".into() };
    let pane = Node::Pane { id: 0, w: 200, children: vec![Node::Spacer { px: 300 }, chip] };
    let fill = Node::Fill { id: 0, children: vec![code] };
    let main = Node::Col { id: 0, gap: 8, children: vec![fill] };
    let row = Node::Row { id: 0, gap: 16, children: vec![main, pane] };
    let d = draw(&[row], &mut texts, &mut view, None);
    let (code, chip, content) = (d.hit(4), d.hit(7), 600.0 - 2.0 * PAD);
    assert!((code.rect.y + code.rect.h - (400.0 - PAD)).abs() < 1.0, "{code:?}");
    assert!((code.rect.w - (content - 216.0)).abs() < 1.0, "{code:?}");
    let at = (chip.rect.x, chip.rect.y, chip.rect.h);
    assert_eq!(at, (PAD + content - 200.0, PAD + 308.0, CHIP_H));
    assert!(chip.sense == Sense::Click && chip.rect.w < 120.0);
    // Room either side centers a Pane; one wider than its Row is cut to it.
    let input = Node::Input { id: 5, value: "".into(), placeholder: "name".into() };
    let pane = |w| Node::Pane { id: 0, w, children: vec![input.clone()] };
    let flex = || Node::Col { id: 0, gap: 0, children: vec![] };
    let row = Node::Row { id: 0, gap: 0, children: vec![flex(), pane(200), flex()] };
    let field = draw(&[row], &mut texts, &mut view, None).hits[0].rect;
    assert_eq!((field.x, field.w), (PAD + (content - 200.0) / 2.0, 200.0));
    assert_eq!(draw(&[pane(900)], &mut texts, &mut view, None).hits[0].rect.w, content);
}

#[test]
fn centers_put_each_child_and_each_line_in_the_middle() {
    let nodes = vec![Node::Center {
        id: 0,
        gap: 5,
        children: vec![
            Node::Glyph { glyph: Glyph::Mark as u8, size: 89 },
            text(Style::Title, "compusophy"),
            Node::Button { id: 4, variant: Variant::Normal, label: "Go".into() },
        ],
    }];
    let d = draw(&nodes, &mut Texts::default(), &mut View::default(), None);
    let mid = |x: f32, w: f32| (x + w / 2.0 - 300.0).abs() <= 1.0;
    let mark = d.of(Kind::Glyph).into_iter().find(|g| g.rect[2] > 80.0).expect("the mark").rect;
    assert!(mid(mark[0], mark[2]), "{mark:?}");
    // The title's glyphs span the middle evenly.
    let ink: Vec<_> =
        d.of(Kind::Glyph).into_iter().filter(|g| g.rect[1] > mark[1] + 80.0).collect();
    let (left, right) = (
        ink.iter().map(|g| g.rect[0]).fold(f32::MAX, f32::min),
        ink.iter().map(|g| g.rect[0] + g.rect[2]).fold(0.0, f32::max),
    );
    assert!(((left + right) / 2.0 - 300.0).abs() < 4.0, "{left} {right}");
    let go = d.hit(4).rect;
    assert!(mid(go.x, go.w) && go.y > mark[1] + 89.0 + 5.0, "{go:?}");
}

#[test]
fn areas_edit_wrap_grow_and_keep_the_caret_in_view() {
    let mut a = Area::new("ab");
    assert_eq!((a.at, a.version), (2, 0));
    assert!(a.insert("c\u{7}\nd") && (a.text.as_str(), a.at, a.version) == ("abc\nd", 5, 1));
    assert!(!a.insert("\u{7}") && a.version == 1);
    // Moves change no text; edits do, a version each.
    for (key, at, changed) in [
        (Key::Home, 4, false),
        (Key::Left, 3, false),
        (Key::Backspace, 2, true),
        (Key::Delete, 2, true),
        (Key::End, 3, false),
    ] {
        assert_eq!((a.key(key), a.at), (Some(changed), at), "{key:?}");
    }
    assert_eq!((a.text.as_str(), a.version), ("abd", 3));
    assert_eq!(a.key(Key::Enter), Some(true));
    assert!(a.key(Key::Tab).is_none() && a.key(Key::Escape).is_none() && a.text == "abd\n");
    a.set("other");
    assert_eq!((a.text.as_str(), a.at), ("other", 5));
    // In a frame: a text hit, at least 144 tall, growing with its rows.
    let area = |value: &str| Node::Area { id: 9, value: value.into(), placeholder: "Say".into() };
    let mut texts = Texts { areas: vec![(9, Area::new(""))], focus: 9, ..Texts::default() };
    assert!(texts.has(9) && !texts.has(8));
    let mut view = View::default();
    let d = draw(&[area("")], &mut texts, &mut view, None);
    let box1 = d.hit(9);
    assert!(box1.sense == Sense::Text && box1.rect.h == 144.0);
    let caret = |d: &Drawn| {
        d.of(Kind::Fill)
            .into_iter()
            .find(|i| i.color == THEMES[0].accent && i.rect[2] <= 2.0)
            .map(|i| i.rect)
    };
    assert!(caret(&d).is_some(), "focused: a caret");
    // Up and Down go by rows, resolved where text is measured: the next draw; a press puts the
    // caret where it lands.
    let ed = &mut texts.areas[0].1;
    ed.insert("one\ntwo\nthree");
    ed.key(Key::Up);
    draw(&[area("")], &mut texts, &mut view, None);
    assert_eq!(texts.areas[0].1.at, 7, "the end of the row above");
    let r = box1.rect;
    texts.areas[0].1.click(r.x + 13.0, r.y + 13.0 + 1.0);
    draw(&[area("")], &mut texts, &mut view, None);
    assert_eq!(texts.areas[0].1.at, 0);
    // Many rows grow the box past the window; typing at the end scrolls it into view.
    let ed = &mut texts.areas[0].1;
    ed.key(Key::Down);
    ed.set(&"line\n".repeat(40));
    ed.insert("x");
    let d = draw(&[text(Style::Body, "intro"), area("")], &mut texts, &mut view, None);
    let well = d.of(Kind::Fill).into_iter().find(|i| i.color == THEMES[0].surface_lo);
    assert!(well.is_some_and(|w| w.rect[3] > 600.0) && view.scroll > 300.0, "{}", view.scroll);
    let c = caret(&d).expect("the caret");
    assert!(c[1] > 0.0 && c[1] + c[3] < 400.0, "in view: {c:?}");
    // Not focused (or no such area in the text): the frame's value, no caret.
    texts.focus = 0;
    let d = draw(
        &[area("seen"), Node::Area { id: 0, value: "plain".into(), placeholder: "".into() }],
        &mut texts,
        &mut view,
        None,
    );
    assert!(caret(&d).is_none() && d.hits.len() == 1);
}

#[test]
fn a_press_on_a_control_keeps_the_keyboard_and_a_waiting_edit_keeps_its_text() {
    // Typing in an Area, a chip or a switch pressed: the keyboard stays, so what is typed next
    // lands; an Input takes it; empty space takes it away.
    let area = Area::new("");
    let mut t = Texts {
        areas: vec![(9, area)],
        inputs: vec![(5, "".into(), 0)],
        focus: 0,
        ..Texts::default()
    };
    assert!(t.press(9, 0.0, 0.0) && t.focus == 9);
    assert!(t.press(2, 0.0, 0.0) && t.focus == 9, "a chip keeps it");
    assert!(t.press(5, 0.0, 0.0) && t.focus == 5);
    assert!(t.press(0, 0.0, 0.0) && t.focus == 0);
    assert!(!t.press(0, 0.0, 0.0) && !t.press(2, 0.0, 0.0) && t.focus == 0);
    // A frame: the text of the one with the keyboard, and of one whose Change still waits,
    // stays the host's; the others take the frame's.
    t.areas[0].1.insert("typed");
    t.inputs[0].1 = "draft".into();
    let nodes = [
        Node::Area { id: 9, value: "old".into(), placeholder: "".into() },
        Node::Input { id: 5, value: "old".into(), placeholder: "".into() },
    ];
    t.adopt(&nodes, &[9]);
    assert_eq!((t.areas[0].1.text.as_str(), t.inputs[0].1.as_str()), ("typed", "old"));
    t.adopt(&nodes, &[]);
    assert_eq!(t.areas[0].1.text, "old");
    (t.focus, t.inputs[0].1) = (5, "mine".into());
    t.adopt(&nodes[1..], &[]);
    assert_eq!((t.inputs[0].1.as_str(), t.focus, t.areas.len()), ("mine", 5, 0));
    t.adopt(&[], &[]);
    assert_eq!((t.focus, t.inputs.len()), (0, 0), "gone, and the keyboard with it");
}

#[test]
fn a_button_pressed_while_typing_may_clear_or_rewrite_the_text() {
    // `input name; button "Clear" { name = ""; }`: "Ada" typed, each edit echoed.
    let input = |value: &str| [Node::Input { id: 5, value: value.into(), placeholder: "".into() }];
    let mut t = Texts::default();
    t.adopt(&input("x"), &[]);
    assert!(t.press(5, 0.0, 0.0) && t.focus == 5);
    (t.inputs[0].1, t.inputs[0].2) = ("Ada".into(), 3);
    // A frame the program sent before the edits reached it, or that echoes them: the person's.
    t.adopt(&input("x"), &[]);
    t.adopt(&input("Ad"), &[]);
    t.adopt(&input("Ada"), &[]);
    assert_eq!(t.inputs[0].1, "Ada");
    // Clear pressed, the keyboard still in the Input: a value the program changes is its own.
    assert!(t.press(2, 0.0, 0.0) && t.focus == 5);
    t.adopt(&input("Ada"), &[]);
    assert_eq!(t.inputs[0].1, "Ada", "unchanged: still the person's");
    t.adopt(&input(""), &[]);
    assert_eq!((t.inputs[0].1.as_str(), t.focus), ("", 5));
    t.adopt(&input("ADA"), &[]);
    assert_eq!(t.inputs[0].1, "ADA");
    // Typed again, the text is the person's; and one whose Change waits is always.
    (t.inputs[0].1, t.inputs[0].2) = ("B".into(), 4);
    t.adopt(&input("Ada"), &[]);
    assert_eq!(t.inputs[0].1, "B");
    assert!(t.press(2, 0.0, 0.0));
    t.adopt(&input("Z"), &[5]);
    assert_eq!(t.inputs[0].1, "B");
    // An Area too.
    let area = |value: &str| [Node::Area { id: 9, value: value.into(), placeholder: "".into() }];
    t.adopt(&area("old"), &[]);
    assert!(t.press(9, 0.0, 0.0) && t.areas[0].1.insert(" typed"));
    t.adopt(&area("old typed"), &[]);
    assert!(t.press(2, 0.0, 0.0));
    t.adopt(&area(""), &[]);
    assert_eq!(t.areas[0].1.text, "");
}

#[test]
fn scrolls_keep_the_bar_above_them_still_and_start_at_the_top_when_new() {
    let up = Node::Button { id: 1, variant: Variant::Quiet, label: "\u{2191}".into() };
    let nodes = |id, n: u32| {
        let rows = (0..n).map(|i| row(100 + i, "notes.txt")).collect();
        let list = Node::Col { id: 0, gap: 0, children: rows };
        vec![up.clone(), Node::Separator, Node::Scroll { id, children: vec![list] }]
    };
    let (mut texts, mut view) = (Texts::default(), View::default());
    let d = draw(&nodes(7, 200), &mut texts, &mut view, None);
    // The list takes what the bar leaves, to the bottom; the window has nothing to scroll.
    let s = view.scrolls[0];
    assert_eq!((s.id, s.y, s.content), (7, 0.0, 200.0 * ENTRY_H));
    assert!((s.rect.y + s.rect.h - (400.0 - PAD)).abs() < 1.0 && view.heights.0 <= 400.5);
    assert!(d.hits.iter().filter(|h| h.id.0 >= 100).count() <= 6);
    let (bar, mark) = (d.hit(1).rect, thumb(&d).expect("a thumb"));
    assert!(mark[0] == 600.0 - 6.0 && mark[1] == s.rect.y && mark[3] < s.rect.h, "{mark:?}");
    // The wheel over the list scrolls it; the bar stays, the thumb goes down.
    let (x, y) = (s.rect.x + 10.0, s.rect.y + 10.0);
    assert!(wheel(&mut texts, &mut view, x, y, 1e9));
    assert!(
        !wheel(&mut texts, &mut view, x, y, 5.0) && !wheel(&mut texts, &mut view, x, y, f32::NAN)
    );
    let d = draw(&nodes(7, 200), &mut texts, &mut view, None);
    let s = view.scrolls[0];
    assert!(s.y == s.content - s.rect.h && d.hit(1).rect == bar);
    assert!(d.hits.iter().any(|h| h.id.0 == 299) && !d.hits.iter().any(|h| h.id.0 == 100));
    let low = thumb(&d).expect("a thumb");
    assert!((low[1] + low[3] - (s.rect.y + s.rect.h)).abs() < 1.0, "{low:?}");
    // Over the bar the window would scroll, but it fits.
    assert!(!wheel(&mut texts, &mut view, bar.x + 2.0, bar.y + 2.0, 50.0));
    // Another list (a new id) starts at its top; one that fits has no thumb.
    let d = draw(&nodes(8, 200), &mut texts, &mut view, None);
    assert!(view.scrolls[0].y == 0.0 && d.hits.iter().any(|h| h.id.0 == 100));
    let d = draw(&nodes(8, 3), &mut texts, &mut view, None);
    assert!(thumb(&d).is_none() && !wheel(&mut texts, &mut view, x, y, 50.0));
}

#[test]
fn windows_that_overflow_show_a_thumb_and_rows_out_of_view_cost_nothing() {
    let (mut texts, mut view) = (Texts::default(), View::default());
    let tall = [Node::Spacer { px: 900 }, text(Style::Body, "end")];
    let d = draw(&tall, &mut texts, &mut view, None);
    let t = thumb(&d).expect("a thumb");
    assert_eq!((t[0], t[1]), (594.0, 0.0));
    assert!((t[3] - 400.0 * 400.0 / view.heights.0).abs() < 1.0, "as long as the share shown");
    assert!(wheel(&mut texts, &mut view, 5.0, 5.0, 1e9));
    let t = thumb(&draw(&tall, &mut texts, &mut view, None)).expect("a thumb");
    assert!((t[1] + t[3] - 400.0).abs() < 1.0, "at the bottom: {t:?}");
    assert!(thumb(&draw(&tall[1..], &mut texts, &mut view, None)).is_none(), "it fits");
    // A thousand rows, the last one named in letters no other row has: what is out of view is
    // never drawn, so its glyphs never reach the atlas.
    let rows = |last: &str| {
        let mut rows: Vec<Node> = (0..999).map(|i| row(i + 1, "aaa")).collect();
        rows.push(row(1000, last));
        vec![Node::Col { id: 0, gap: 0, children: rows }]
    };
    let atlas = |nodes: &[Node]| {
        let (d, mut ts) =
            paint(600.0, 0.0, nodes, &mut Texts::default(), &mut View::default(), None);
        (d.hits.len(), ts.atlas_mut().pixels().to_vec())
    };
    let (seen, plain) = atlas(&rows("aaa"));
    assert!(seen <= 7, "{seen}");
    assert!(atlas(&rows("WXYZ")).1 == plain, "the far row's glyphs were drawn");
}

#[test]
fn strips_stay_on_one_line_and_slide_left_to_keep_their_end_in_view() {
    let quiet = |id, label: &str| Node::Button { id, variant: Variant::Quiet, label: label.into() };
    let mut crumbs: Vec<Node> = (0..8).map(|i| quiet(10 + i, "a-long-folder")).collect();
    crumbs.push(text(Style::Body, "the-last-folder-of-all"));
    let strut = Node::Pane { id: 0, w: 0, children: vec![Node::Spacer { px: 44 }] };
    let bar = vec![Node::Row {
        id: 0,
        gap: 0,
        children: vec![
            quiet(1, "\u{2191}"),
            Node::Strip { id: 0, gap: 0, children: crumbs },
            strut,
        ],
    }];
    let (mut texts, mut view) = (Texts::default(), View::default());
    let d = draw_at(360.0, &bar, &mut texts, &mut view, None);
    // Up keeps its place; the first crumbs slid out of view (no hits), the rest on Up's line.
    let up = d.hit(1).rect;
    assert!(up.x == PAD && up.h == TOUCH && !d.hits.iter().any(|h| h.id.0 == 10));
    assert!(d.hits.iter().all(|h| h.rect.y == up.y && h.rect.h == TOUCH), "{:?}", d.hits);
    // The last crumb, unwrapped, ends at the bar's right edge; every glyph on the one line.
    let glyphs = d.of(Kind::Glyph);
    let right = glyphs.iter().map(|g| g.rect[0] + g.rect[2]).fold(0.0, f32::max);
    assert!(right <= 360.0 - PAD && right > 360.0 - PAD - 6.0, "{right}");
    assert!(glyphs.iter().all(|g| g.rect[1] >= up.y && g.rect[1] + g.rect[3] <= up.y + up.h));
    // With room, nothing slides: the first crumb right after Up, centered on the bar too.
    let d = draw_at(3000.0, &bar, &mut texts, &mut view, None);
    let (up, first) = (d.hit(1).rect, d.hit(10).rect);
    assert!(first.x == up.x + up.w && first.y == up.y, "{first:?}");
    assert_eq!(up.y, PAD + (44.0 - BUTTON_H) / 2.0, "on the strut's line");
}

#[test]
fn a_revealed_mark_comes_in_ring_by_ring_once_by_its_windows_clock() {
    let mark = |reveal| vec![Node::Glyph { glyph: Glyph::Mark as u8 | reveal, size: 144 }];
    let mut view = View::default();
    assert!(!view.animating(0.0), "not before it is drawn");
    let at = |now, nodes: &[Node], view: &mut View| {
        paint(600.0, now, nodes, &mut Texts::default(), view, None).0
    };
    let early = at(100.0, &mark(REVEAL), &mut view);
    assert_eq!(view.reveal, Some(100.0));
    assert!(view.animating(400.0) && view.animating(717.0));
    assert!(!view.animating(718.0) && !view.animating(f64::NAN));
    let mid = at(400.0, &mark(REVEAL), &mut view);
    let done = at(718.0, &mark(REVEAL), &mut view);
    // 300 ms in: the center dot and the five inner rings, 132 dots; nothing at the start; then
    // the mark's one glyph. Later frames keep the first clock.
    let fills = |d: &Drawn| d.of(Kind::Fill).len();
    let whole = |d: &Drawn| d.of(Kind::Glyph).iter().filter(|g| g.rect[2] > 130.0).count();
    assert_eq!((fills(&early), fills(&mid)), (fills(&done), fills(&done) + 132));
    assert_eq!([whole(&early), whole(&mid), whole(&done)], [0, 0, 1]);
    assert_eq!(view.reveal, Some(100.0));
    // Not revealed, or not the mark: whole at once, and no frames asked for.
    for nodes in [mark(0), vec![Node::Glyph { glyph: Glyph::Cog as u8 | REVEAL, size: 144 }]] {
        let mut plain = View::default();
        let d = at(100.0, &nodes, &mut plain);
        assert!(plain.reveal.is_none() && whole(&d) == 1 && !plain.animating(100.0));
    }
}

#[test]
fn app_rows_wear_their_sigil_rows_say_two_lines_and_display_is_large() {
    let t = &THEMES[0];
    let seed = 0x1234_5678;
    let entry = |glyph, text: &str| {
        let (text, detail) = (text.into(), String::new());
        Node::Entry { id: 1, glyph, hue: seed, text, detail, more: true }
    };
    let plates = |d: &Drawn, color| {
        let tile = |i: &&Instance| i.rect[2] == 34.0 && i.rect[3] == 34.0 && i.color == color;
        d.list.instances().iter().filter(tile).count()
    };
    // A .app's tile: the plate of its seed's tint, as the home screen's, and the sigil on it.
    let sigil_top = t.icon_colors(ui::theme::app_tint(seed))[0];
    let window_top = t.icon_colors(Rgba::hex(seed))[0];
    let (mut texts, mut view) = (Texts::default(), View::default());
    let d = draw(&[entry(SIGIL, "clock.app")], &mut texts, &mut view, None);
    assert_eq!((plates(&d, sigil_top), plates(&d, window_top)), (1, 0));
    let w = draw(&[entry(Glyph::Window as u8, "clock.app")], &mut texts, &mut view, None);
    assert_eq!(plates(&w, window_top), 1);
    let ink = |glyph| {
        let (t, v) = (&mut Texts::default(), &mut View::default());
        let (_, mut ts) = paint(600.0, 0.0, &[entry(glyph, "")], t, v, None);
        ts.atlas_mut().pixels().to_vec()
    };
    assert!(ink(SIGIL) != ink(Glyph::Window as u8), "a sigil, not a window");
    // Two lines: the name in the body ink, what it is for small and dim under it.
    let d = draw(&[entry(Glyph::Studio as u8, "Studio\nbuild apps")], &mut texts, &mut view, None);
    let band = |color| {
        let text = |g: &&Instance| g.color == color && g.rect[0] < 500.0; // not the chevron
        let ink: Vec<_> = d.of(Kind::Glyph).into_iter().filter(text).collect();
        let top = ink.iter().map(|g| g.rect[1]).fold(f32::MAX, f32::min);
        (ink.len(), top)
    };
    let ((names, name_y), (lines, line_y)) = (band(t.text), band(t.text_dim));
    assert!(names >= 6 && lines >= 9 && line_y > name_y + 10.0, "{name_y} {line_y}");
    let r = d.hit(1).rect;
    assert!(name_y > r.y + 5.0 && line_y < r.y + r.h - 10.0);
    // Display: a first screen's name, larger and bolder than a Title.
    let tallest = |style| {
        let d =
            draw(&[text(style, "compusophy")], &mut Texts::default(), &mut View::default(), None);
        d.of(Kind::Glyph).iter().map(|g| g.rect[3]).fold(0.0, f32::max)
    };
    assert!(tallest(Style::Display) > tallest(Style::Title) * 1.3);
}

#[test]
fn grids_draw_theme_squares_and_points_find_them() {
    let t = &THEMES[0];
    let (mut texts, mut view) = (Texts::default(), View::default());
    let words = ["", "X", "", "", ""].map(String::from).to_vec();
    let grid = Node::Grid { id: 7, cols: 2, cells: vec![0, 1, 8, 2, 0], texts: words };
    let d = draw(std::slice::from_ref(&grid), &mut texts, &mut view, None);
    // Two columns across the width: 32 px squares (the most), centered, 3 rows, a pixel apart.
    let x = PAD + (600.0 - 2.0 * PAD - 64.0) / 2.0;
    assert_eq!(view.grids, [(7, RectF::new(x, PAD, 64.0, 96.0), 2, 5)]);
    let square = |c: f32, r: f32| RectF::new(x + 32.0 * c, PAD + 32.0 * r, 31.0, 31.0);
    assert!(d.filled(square(0.0, 0.0), t.surface_lo) && d.filled(square(1.0, 0.0), t.ansi[1]));
    assert!(d.filled(square(0.0, 1.0), t.ansi[8]) && d.filled(square(1.0, 1.0), t.ansi[2]));
    assert_eq!((d.hit(7).rect, d.hit(7).sense), (RectF::new(x, PAD, 64.0, 96.0), Sense::Click));
    // A point finds its square; past the last square, outside, or another id: none.
    assert_eq!(cell(&view, 7, x + 40.0, PAD + 40.0), Some(3));
    let none = [(7, x + 40.0, PAD + 70.0), (7, x - 1.0, PAD), (8, x, PAD)];
    assert!(none.iter().all(|&(id, px, py)| cell(&view, id, px, py).is_none()));
    // For the AI: a mark with the rows, a line each, and the texts as runs.
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    let (mut list, mut hits) = (DrawList::recording(), Vec::new());
    let r = RectF::new(0.0, 0.0, 600.0, 400.0);
    let ui = Ui::new(&mut list, &mut ts, r, &mut hits, UiState::default(), t);
    super::draw(&mut { ui }, &[grid], &mut texts, &mut view);
    let sem = list.take_sem().unwrap();
    let mark = (sem.marks[0].role, sem.marks[0].value.as_str());
    assert_eq!(mark, (ui::sem::GRID, "2 columns\n01\n82\n0"));
    assert_eq!(sem.runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["X"]);
    // A hundred columns are 6 px squares (the least) with no gap; with no id, nothing to tap.
    let wide = Node::Grid { id: 0, cols: 100, cells: vec![1; 100], texts: Vec::new() };
    let d = draw(&[wide], &mut texts, &mut view, None);
    assert!(view.grids.is_empty() && d.hits.is_empty());
    let six = |f: &&&Instance| f.rect[2] == 6.0 && f.color == t.ansi[1];
    assert_eq!(d.of(Kind::Fill).iter().filter(six).count(), 100);
}

#[test]
fn play_ticks_while_shown_once_answered_and_taps_new_squares() {
    let mut p = Play::default();
    assert!(p.ask(&Request::Timer { ms: 100 }) && p.ask(&Request::Keys { on: true }));
    assert!(!p.ask(&Request::Close) && p.keys);
    // The first frame starts the count; due after 100 ms, then not until answered (or 1 s).
    let tick = |p: &mut Play, now: f64| p.tick(now, now - 10.0);
    assert_eq!([tick(&mut p, 1000.0), tick(&mut p, 1050.0)], [None, None]);
    assert_eq!(tick(&mut p, 1116.0), Some(Event::Tick { ms: 116 }));
    assert_eq!(tick(&mut p, 1300.0), None);
    p.answered();
    assert_eq!(tick(&mut p, 1300.0), Some(Event::Tick { ms: 184 }));
    assert_eq!(tick(&mut p, 2400.0), Some(Event::Tick { ms: 1100 }));
    // Hidden (not drawn for a while), none; shown again, the count starts over; 0 stops.
    assert_eq!((p.tick(5000.0, 2400.0), tick(&mut p, 5016.0)), (None, None));
    p.answered();
    assert_eq!(tick(&mut p, 5117.0), Some(Event::Tick { ms: 101 }));
    assert!(p.ask(&Request::Timer { ms: 0 }) && tick(&mut p, 9000.0).is_none());
    // A press taps its square; a drag taps each new square of the grid pressed, none other.
    let grids = vec![(7, RectF::new(10.0, 10.0, 64.0, 64.0), 2, 4)];
    let mut view = View { grids, ..View::default() };
    assert_eq!(p.tap(&view, Some(7), 50.0, 50.0), Some(Event::Tap { id: 7, cell: 3 }));
    assert_eq!(p.tap(&view, None, 51.0, 51.0), None);
    assert_eq!(p.tap(&view, None, 11.0, 11.0), Some(Event::Tap { id: 7, cell: 0 }));
    assert_eq!((p.tap(&view, Some(3), 11.0, 11.0), p.tap(&view, None, 50.0, 11.0)), (None, None));
    view.grids.clear();
    assert_eq!(p.tap(&view, Some(7), 50.0, 50.0), None);
}
