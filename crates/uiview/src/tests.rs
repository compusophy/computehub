use gfx::{DrawList, Instance, Kind};
use ui::{Code, Hit, Key, THEMES, UiState};
use uiwire::{Draw, Shape};

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
    // A char goes whole, however many bytes it takes.
    let mut b = Area::new("éaü");
    let keys = [Key::Backspace, Key::Home, Key::Delete].map(|k| b.key(k));
    assert_eq!((keys, b.text.as_str(), b.at), ([Some(true), Some(false), Some(true)], "a", 0));
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
    // In a Fill (Editor's text): down to the bottom at least; with more rows, as tall as they.
    let page = |texts: &mut Texts| {
        let fill = Node::Fill { id: 0, children: vec![area("")] };
        let d = draw(&[text(Style::Body, "bar"), fill], texts, &mut View::default(), None);
        let well = d.of(Kind::Fill).into_iter().find(|i| i.color == THEMES[0].surface_lo);
        well.expect("the well").rect
    };
    texts.areas[0].1.set("short");
    let r = page(&mut texts);
    assert!(r[3] > 144.0 && (r[1] + r[3] - (400.0 - PAD)).abs() < 1.0, "{r:?}");
    // A press under its text puts the caret at the end, as Down on the last row does.
    let ed = &mut texts.areas[0].1;
    ed.at = 0;
    ed.click(r[0] + 20.0, r[1] + r[3] - 9.0);
    page(&mut texts);
    assert_eq!(texts.areas[0].1.at, 5);
    texts.areas[0].1.set(&"line\n".repeat(40));
    assert!(page(&mut texts)[3] > 600.0);
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

/// How many of `d`'s `kind` instances cover `r` exactly in `color`.
fn at(d: &Drawn, kind: Kind, r: RectF, color: Rgba) -> usize {
    let on = |i: &&&Instance| i.rect == [r.x, r.y, r.w, r.h] && i.color == color;
    d.of(kind).iter().filter(on).count()
}

/// What a recording draw of `nodes` at 600 x 400 in theme `t` notes.
fn sem(nodes: &[Node], t: &Theme) -> gfx::Sem {
    sem_in(nodes, t, &mut Texts::default())
}

/// [`sem`], the text fields' text in `texts`.
fn sem_in(nodes: &[Node], t: &Theme, texts: &mut Texts) -> gfx::Sem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    let (mut list, mut hits) = (DrawList::recording(), Vec::new());
    let r = RectF::new(0.0, 0.0, 600.0, 400.0);
    let ui = Ui::new(&mut list, &mut ts, r, &mut hits, UiState::default(), t);
    super::draw(&mut { ui }, nodes, texts, &mut View::default());
    list.take_sem().unwrap()
}

#[test]
fn a_toggle_reads_as_a_switch_and_an_area_as_the_text_it_holds() {
    let nodes = [
        Node::Toggle { id: 3, on: true, label: "Include what is open".into() },
        Node::Area { id: 7, value: "the dock is small".into(), placeholder: "What?".into() },
    ];
    let mut texts = Texts::default();
    texts.adopt(&nodes, &[]);
    let marks = sem_in(&nodes, &THEMES[0], &mut texts).marks;
    let marks: Vec<_> = marks.into_iter().map(|m| (m.id, m.role, m.flags, m.value)).collect();
    let (switch, area) = ((3, ui::sem::SWITCH), (7, ui::sem::TEXTBOX));
    let want =
        [(switch.0, switch.1, ui::sem::CHECKED, ""), (area.0, area.1, 0, "the dock is small")];
    assert_eq!(marks, want.map(|m| (m.0, m.1, m.2, m.3.to_string())));
}

#[test]
fn grids_draw_theme_squares_and_points_find_them() {
    let t = &THEMES[0];
    let (mut texts, mut view) = (Texts::default(), View::default());
    let words = ["", "X", "", "", ""].map(String::from).to_vec();
    let grid = Node::Grid { id: 7, cols: 2, cells: vec![0, 1, 8, 2, 0], texts: words };
    let d = draw(std::slice::from_ref(&grid), &mut texts, &mut view, None);
    // Two columns across the width: 96 px squares (the most), centered, 3 rows, a pixel apart.
    let x = PAD + (600.0 - 2.0 * PAD - 192.0) / 2.0;
    let rect = RectF::new(x, PAD, 192.0, 288.0);
    assert_eq!(view.grids, [Board { id: 7, nth: 0, rect, cols: 2, n: 5, fine: false }]);
    let square = |c: f32, r: f32| RectF::new(x + 96.0 * c, PAD + 96.0 * r, 95.0, 95.0);
    assert!(d.filled(square(0.0, 0.0), t.surface_lo) && d.filled(square(1.0, 0.0), t.ansi[1]));
    assert!(d.filled(square(0.0, 1.0), t.ansi[8]) && d.filled(square(1.0, 1.0), t.ansi[2]));
    assert_eq!((d.hit(7).rect, d.hit(7).sense), (rect, Sense::Pad));
    // The empty ones outlined, as a text field is, so the board shows on any surface.
    let edge = |c, r| at(&d, Kind::Border, square(c, r), t.border);
    assert_eq!([edge(0.0, 0.0), edge(0.0, 2.0), edge(1.0, 0.0)], [1, 1, 0]);
    // A point finds its square; past the last square, or outside: none.
    let b = view.grids[0];
    assert_eq!(b.at(x + 100.0, PAD + 100.0), Some((1, 1)));
    assert!([(x + 100.0, PAD + 200.0), (x - 1.0, PAD)].iter().all(|p| b.at(p.0, p.1).is_none()));
    // For the AI: a mark of its size, the rows a line each, then each text by its square.
    let marked = |g: &Node| sem(std::slice::from_ref(g), t).marks[0].value.clone();
    assert_eq!(marked(&grid), "2 columns, 3 rows\n01\n82\n0\n1: X");
    // Boards alike but for where their texts are read apart.
    let board = |at: [usize; 3]| {
        let mut texts = vec![String::new(); 9];
        (texts[at[0]], texts[at[1]], texts[at[2]]) = ("X".into(), "O".into(), "X".into());
        Node::Grid { id: 1, cols: 3, cells: vec![0; 9], texts }
    };
    let (a, b) = (marked(&board([0, 4, 8])), marked(&board([1, 3, 7])));
    assert!(a.ends_with("\n000\n0: X\n4: O\n8: X") && b.ends_with("\n1: X\n3: O\n7: X"), "{a}");
    // A hundred columns are as wide as the width holds (here 5 px squares) with no gap, the
    // whole outlined; with no id, nothing to tap.
    let wide = Node::Grid { id: 0, cols: 100, cells: vec![1; 100], texts: Vec::new() };
    let d = draw(&[wide], &mut texts, &mut view, None);
    assert!(view.grids.is_empty() && d.hits.is_empty());
    let five = |f: &&&Instance| f.rect[2] == 5.0 && f.color == t.ansi[1];
    assert_eq!(d.of(Kind::Fill).iter().filter(five).count(), 100);
    assert_eq!(at(&d, Kind::Border, RectF::new(PAD + 30.0, PAD, 500.0, 5.0), t.border), 1);
    // On a phone's window, 3 device px a px, every column stays in the content.
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_dpr(3.0);
    let (mut list, mut hits, phone) =
        (DrawList::new(), Vec::new(), RectF::new(0.0, 0.0, 375.0, 400.0));
    let ui = Ui::new(&mut list, &mut ts, phone, &mut hits, UiState::default(), t);
    let tap = Node::Grid { id: 2, cols: 100, cells: vec![1; 400], texts: Vec::new() };
    super::draw(&mut { ui }, &[tap], &mut texts, &mut view);
    let r = view.grids[0].rect;
    assert!(r.x >= PAD && r.x + r.w <= 375.0 - PAD && r.w > 330.0, "{r:?}");
    // A tall one alone takes the view's height: 20 rows in 360 px are 18 px squares (17 and a
    // pixel apart).
    let tall = Node::Grid { id: 0, cols: 10, cells: vec![1; 200], texts: Vec::new() };
    let d = draw(std::slice::from_ref(&tall), &mut texts, &mut view, None);
    let eighteen = |f: &&&Instance| f.rect[2] == 17.0 && f.color == t.ansi[1];
    assert_eq!(d.of(Kind::Fill).iter().filter(eighteen).count(), 200);
    // Among a game's labels and buttons it takes what they leave: all of it shows.
    let button = |id| Node::Button { id, variant: Variant::Normal, label: "Left".into() };
    let row = |id| Node::Row { id: 0, gap: 8, children: vec![button(id), button(id + 1)] };
    let score = || text(Style::Body, "Score 0");
    let game = [score(), score(), tall, row(1), row(3), button(5)];
    let d = draw(&game, &mut texts, &mut view, None);
    assert!(view.heights.0 <= 400.0 && d.hit(5).rect.y + BUTTON_H <= 400.0 - PAD, "{view:?}");
    assert_eq!(d.of(Kind::Fill).iter().filter(|f| f.color == t.ansi[1]).count(), 200);
}

#[test]
fn grid_texts_fit_their_squares_and_its_colors_read_apart() {
    // Four digits in 96 px squares, and the card's words: each text inside its own square.
    let tiles = ["1024", "2048", "banana", "cherry"].map(String::from).to_vec();
    let grid = Node::Grid { id: 1, cols: 4, cells: vec![3, 4, 0, 0], texts: tiles };
    let (mut texts, mut view) = (Texts::default(), View::default());
    let d = draw(&[grid], &mut texts, &mut view, None);
    let r = view.grids[0].rect;
    for i in 0..4 {
        let (left, right) = (r.x + 96.0 * i as f32, r.x + 96.0 * (i + 1) as f32 - 1.0);
        let ink = |g: &&Instance| (left..right).contains(&(g.rect[0] + g.rect[2] / 2.0));
        let glyphs: Vec<_> = d.of(Kind::Glyph).into_iter().filter(ink).collect();
        assert!(glyphs.len() >= 4, "{i}");
        assert!(glyphs.iter().all(|g| g.rect[0] >= left && g.rect[0] + g.rect[2] <= right), "{i}");
    }
    // Silver (7) and gray (8) read apart in every theme, and their texts on them.
    let lum = |c: Rgba| {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
    };
    let contrast = |a: Rgba, b: Rgba| {
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    for t in &THEMES {
        let ((silver, on_silver), (gray, on_gray)) = (canvas::square(t, 7), canvas::square(t, 8));
        assert!(contrast(silver, gray) >= 2.0, "{} {}", t.name, contrast(silver, gray));
        assert!(contrast(on_silver, silver) >= 4.5 && contrast(on_gray, gray) >= 3.0, "{}", t.name);
    }
}

#[test]
fn play_ticks_while_shown_once_answered_and_taps_new_squares() {
    let mut p = Play::default();
    assert_eq!(p.due_in(0.0), None, "no timer");
    assert!(p.ask(&Request::Timer { ms: 100 }) && p.ask(&Request::Keys { on: true }));
    assert!(!p.ask(&Request::Close) && p.keys);
    // The first frame starts the count; due after 100 ms, then not until answered (or 1 s).
    let tick = |p: &mut Play, now: f64| p.tick(now, now - 10.0);
    assert_eq!(p.due_in(1000.0), Some(0), "a frame to start the count");
    assert_eq!([tick(&mut p, 1000.0), tick(&mut p, 1050.0)], [None, None]);
    assert_eq!((p.due_in(1050.0), p.due_in(1099.5)), (Some(50), Some(1)));
    assert_eq!(tick(&mut p, 1116.0), Some(Event::Tick { ms: 116 }));
    assert_eq!((tick(&mut p, 1300.0), p.due_in(1300.0)), (None, Some(816)));
    p.answered();
    assert_eq!(p.due_in(1300.0), Some(0));
    assert_eq!(tick(&mut p, 1300.0), Some(Event::Tick { ms: 184 }));
    assert_eq!(tick(&mut p, 2400.0), Some(Event::Tick { ms: 1100 }));
    // With frames only when due, a slow timer's window last drew one wait before: it ticks.
    let mut slow = Play::default();
    slow.ask(&Request::Timer { ms: 5000 });
    assert_eq!(slow.tick(100.0, 90.0), None);
    assert_eq!(slow.tick(5100.0, 100.0), Some(Event::Tick { ms: 5000 }));
    // Hidden (not drawn for a while), none; shown again, the count starts over; 0 stops.
    assert_eq!((p.tick(5000.0, 2400.0), tick(&mut p, 5016.0)), (None, None));
    p.answered();
    assert_eq!(tick(&mut p, 5117.0), Some(Event::Tick { ms: 101 }));
    assert!(p.ask(&Request::Timer { ms: 0 }) && tick(&mut p, 9000.0).is_none());
    // A press taps its square; a drag taps each new square of the grid pressed, none other.
    let board = |id, nth, rect| Board { id, nth, rect, cols: 2, n: 4, fine: false };
    let (tap, none) = (|cells: &[u32]| (7, cells.to_vec()), (0, vec![]));
    let mut view =
        View { grids: vec![board(7, 0, RectF::new(10.0, 10.0, 64.0, 64.0))], ..View::default() };
    assert!(!p.pressed());
    assert_eq!(p.tap(&view, Some(7), 50.0, 50.0), tap(&[3]));
    assert!(p.pressed(), "its Click is none");
    assert_eq!(p.tap(&view, None, 51.0, 51.0), tap(&[]));
    assert_eq!(p.tap(&view, None, 11.0, 11.0), tap(&[0]));
    assert_eq!(p.tap(&view, Some(3), 11.0, 11.0), none);
    assert!(p.tap(&view, None, 50.0, 11.0) == none && !p.pressed());
    // Moved farther than a square between samples: each square on the way, once.
    view.grids = vec![Board { cols: 4, n: 16, ..board(7, 0, RectF::new(0.0, 0.0, 64.0, 64.0)) }];
    assert_eq!(p.tap(&view, Some(7), 1.0, 1.0), tap(&[0]));
    assert_eq!(p.tap(&view, None, 63.0, 63.0), tap(&[5, 10, 15]));
    assert_eq!(p.tap(&view, None, 1.0, 40.0), tap(&[14, 9, 8]));
    // Off the squares and back: from where it came back, alone.
    assert_eq!(p.tap(&view, None, 80.0, 1.0), tap(&[]));
    assert_eq!(p.tap(&view, None, 63.0, 1.0), tap(&[3]));
    // What was sent is answered in order, after the frames of the Tick and the Change out; till
    // then, and till that answer draws, a drag waits, then taps the squares passed meanwhile.
    let mut q = Play::default();
    q.ask(&Request::Timer { ms: 100 });
    assert_eq!((q.tick(0.0, 0.0), q.tick(100.0, 90.0)), (None, Some(Event::Tick { ms: 100 })));
    assert_eq!(q.tap(&view, Some(7), 1.0, 1.0), tap(&[0]));
    q.sent(true);
    for _ in 0..3 {
        assert!(q.busy() && q.tap(&view, None, 40.0, 1.0) == none);
        q.answered();
    }
    assert!(!q.busy() && q.tap(&view, None, 63.0, 1.0) == none && !q.held());
    q.drew();
    // Where it went is held till then, for the window to tap at its next frame.
    assert!(q.held() && q.replay(&view) == tap(&[1, 2, 3]));
    assert!(!q.held() && q.replay(&view) == none);
    // Its three taps are answered by three frames, the last after the two before it.
    (0..3).for_each(|_| q.sent(false));
    for _ in 0..3 {
        assert!(q.busy());
        q.answered();
    }
    assert!(!q.busy());
    // The grid pressed has another id now (a button shown above it): its taps take that.
    view.grids =
        vec![board(9, 0, RectF::new(10.0, 50.0, 64.0, 64.0)), board(7, 1, view.grids[0].rect)];
    assert_eq!(p.tap(&view, None, 11.0, 51.0), (9, vec![0]));
    view.grids.clear();
    assert_eq!(p.tap(&view, Some(7), 50.0, 50.0), none);
}

/// `nodes` drawn `w` x `h` at `dpr` device px a px in theme `t`, and what a recording draw of
/// them notes: what it drew, the view after, and the notes.
fn boards(size: (f32, f32), dpr: f32, t: &Theme, nodes: &[Node]) -> (Drawn, View, gfx::Sem) {
    let mut out = Vec::new();
    for mut list in [DrawList::new(), DrawList::recording()] {
        let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
        ts.set_dpr(dpr);
        let (mut hits, mut view) = (Vec::new(), View::default());
        let r = RectF::new(0.0, 0.0, size.0, size.1);
        let ui = Ui::new(&mut list, &mut ts, r, &mut hits, UiState::default(), t);
        super::draw(&mut { ui }, nodes, &mut Texts::default(), &mut view);
        out.push((Drawn { list, hits }, view));
    }
    let (mut noted, drawn) = (out.pop().unwrap().0, out.pop().unwrap());
    (drawn.0, drawn.1, noted.list.take_sem().unwrap())
}

/// A draw of `shape` in `color` at `at`, with `text`.
fn shape(shape: Shape, color: u8, at: [i16; 5], text: &str) -> Draw {
    Draw { shape, color, at, text: text.into() }
}

#[test]
fn canvases_fit_a_phone_and_land_on_device_pixels() {
    let t = &THEMES[0];
    let draws = vec![
        shape(Shape::Rect, 1, [100, 0, 100, 300, 0], ""),
        shape(Shape::Line, 8, [100, 8, 100, 291, 3], ""),
        shape(Shape::Line, 2, [22, 22, 78, 78, 8], ""),
    ];
    let tic = Node::Canvas { id: 9, w: 300, h: 300, draws };
    let new = Node::Button { id: 1, variant: Variant::Normal, label: "New game".into() };
    let game = [text(Style::Body, "X to play"), tic, new];
    for dpr in [1.0, 2.0, 3.5] {
        // A phone's maximized window: 369 px across inside, the board as wide, a tic-tac-toe
        // square 123 px; a pad.
        let (d, view, _) = boards((409.0, 552.0), dpr, t, &game);
        let b = view.grids[0];
        assert_eq!((b.id, b.nth, b.cols, b.n, b.fine), (9, 0, 300, 90_000, true));
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(b.rect.x, PAD) && near(b.rect.w, 369.0) && near(b.rect.h, 369.0), "{b:?}");
        assert_eq!(d.hit(9).sense, Sense::Pad);
        // The well, the rect and the line along an axis: each edge on a device pixel.
        let on = |v: f32| (v * dpr - (v * dpr).round()).abs() < 1e-3;
        let fill = |c: Rgba| *d.of(Kind::Fill).into_iter().find(|f| f.color == c).unwrap();
        for c in [t.surface_lo, t.ansi[1], t.ansi[8]] {
            assert!(fill(c).rect.iter().all(|v| on(*v)), "{dpr} {:?}", fill(c));
        }
        let (rect, line) = (fill(t.ansi[1]).rect, fill(t.ansi[8]));
        assert!((rect[0] - PAD - 123.0).abs() < 1.0 && (rect[2] - 123.0).abs() < 1.0, "{rect:?}");
        assert_eq!(line.radius, line.rect[2] / 2.0, "round ends");
        // The diagonal is a line, falling left to right.
        assert_eq!(d.of(Kind::Line).iter().map(|l| l.p1).collect::<Vec<_>>(), [0.0]);
    }
    // Short of room (here 600 x 400), it takes the height left, a third at least, its units
    // square, centered across.
    let tall = Node::Canvas { id: 0, w: 100, h: 200, draws: Vec::new() };
    let (d, view, _) = boards((600.0, 400.0), 1.0, t, &[tall]);
    assert!(view.grids.is_empty() && d.hits.is_empty(), "no id: nothing to tap");
    assert_eq!(d.of(Kind::Fill)[0].rect, [PAD + 190.0, PAD, 180.0, 360.0]);
}

#[test]
fn a_board_keeps_its_size_and_place_as_widgets_come_and_go() {
    let (mut texts, mut view) = (Texts::default(), View::default());
    let start = Node::Button { id: 1, variant: Variant::Normal, label: "Start".into() };
    let board = Node::Canvas { id: 9, w: 160, h: 120, draws: Vec::new() };
    let ready = [text(Style::Body, "Score 0"), board, start];
    let mut rect = |nodes: &[Node], w: f32| {
        draw_at(w, nodes, &mut texts, &mut view, None);
        view.grids[0].rect
    };
    // Start hidden as it plays, and back: the board stays, the room under it empty.
    let shown = rect(&ready, 600.0);
    assert_eq!([rect(&ready[..2], 600.0), rect(&ready, 600.0)], [shown, shown]);
    // Another width starts over: alone, it takes the room; Start back, it gives it, once.
    let (alone, back) = (rect(&ready[..2], 640.0), rect(&ready, 640.0));
    assert!(alone.h > back.h && back.h == shown.h, "{alone:?} {back:?}");
    assert_eq!(rect(&ready[..2], 640.0), back);
    // The score above it gone as it plays: the board stays put, the room above it empty.
    assert_eq!(rect(&ready[1..], 640.0), back);
}

#[test]
fn canvas_taps_points_and_its_mark_lists_its_shapes() {
    // A press taps the unit under it (y * w + x); a drag each unit on its way, once.
    let rect = RectF::new(0.0, 0.0, 100.0, 50.0);
    let b = Board { id: 3, nth: 0, rect, cols: 10, n: 50, fine: true };
    let (mut view, mut p) = (View { grids: vec![b], ..View::default() }, Play::default());
    assert_eq!(p.tap(&view, Some(3), 25.0, 15.0), (3, vec![12]));
    assert_eq!(p.tap(&view, None, 95.0, 45.0), (3, vec![13, 24, 25, 36, 37, 48, 49]));
    assert_eq!(p.tap(&view, None, 96.0, 46.0), (3, vec![]));
    // Units of a px: one every 4 px or so on the way, 64 at most for one sample (256 px).
    view.grids[0] = Board { cols: 400, n: 400, rect: RectF::new(0.0, 0.0, 400.0, 1.0), ..b };
    assert_eq!(p.tap(&view, Some(3), 0.5, 0.5), (3, vec![0]));
    let way = [4, 7, 11, 15, 18, 22, 25, 29, 33, 36, 40];
    assert_eq!(p.tap(&view, None, 40.5, 0.5), (3, way.to_vec()));
    let far = p.tap(&view, None, 399.5, 0.5).1;
    assert!(far.len() == 64 && far[63] == 399 && far.windows(2).all(|w| w[0] < w[1]), "{far:?}");
    let back = p.tap(&view, None, 147.5, 0.5).1;
    assert!(back.len() == 64 && back.windows(2).all(|w| w[0] - w[1] <= 4), "{back:?}");
    // For the AI: its size in units, then each shape, its text, numbers and color.
    let draws = vec![
        shape(Shape::Line, 9, [10, 5, 30, 5, 2], ""),
        shape(Shape::Ring, 4, [50, 25, 10, 2, 0], ""),
        shape(Shape::Text, 11, [1, 2, 3, 0, 0], "hi"),
        shape(Shape::Sprite, 0, [0, -1, 2, 0, 0], "1"),
    ];
    let one = Node::Canvas { id: 5, w: 100, h: 50, draws };
    let rects = (0..70).map(|i| shape(Shape::Rect, 1, [i, 0, 1, 1, 0], "")).collect();
    let many = Node::Canvas { id: 6, w: 70, h: 1, draws: rects };
    let (_, view, sem) = boards((600.0, 400.0), 1.0, &THEMES[0], &[one, many]);
    let want = "100 x 50 units\nline 10 5 30 5 2 9\nring 50 25 10 2 4\ntext \"hi\" 1 2 3 11\n\
        sprite 0 -1 2";
    assert_eq!((sem.marks[0].role, &*sem.marks[0].value), (ui::sem::CANVAS, want));
    let lines: Vec<&str> = sem.marks[1].value.lines().collect();
    assert_eq!((lines.len(), lines[64], lines[65]), (66, "rect 63 0 1 1 1", "and 6 more"));
    let ids: Vec<_> = view.grids.iter().map(|b| (b.id, b.nth)).collect();
    assert_eq!(ids, [(5, 0), (6, 1)]);
}

#[test]
fn trees_draw_with_the_toolkit_and_fills_take_the_rest() {
    let (mut t, mut view) = (Texts::default(), View::default());
    let button = |id, variant| Node::Button { id, variant, label: "Go".into() };
    let row =
        vec![button(1, Variant::Primary), button(2, Variant::Danger), text(Style::Small, "a")];
    let row = Node::Row { id: 0, gap: 8, children: row };
    let code =
        Node::Code { id: 4, version: 1, line_numbers: true, text: "a\nb".into(), spans: vec![] };
    let input = |v: &str| Node::Input { id: 5, value: v.into(), placeholder: "name".into() };
    let card = Node::Card { id: 0, children: vec![input("v"), Node::Separator] };
    let item = Node::Item { id: 9, text: "E1 2:3 bad".into(), detail: "x".into(), selected: true };
    let fill = Node::Fill { id: 0, children: vec![code] };
    let nodes = [row, fill, card, Node::Spacer { px: 4 }, item];
    t.adopt(&nodes, &[]);
    let d = draw(&nodes, &mut t, &mut view, None);
    let (go, danger, field, code, row) = (d.hit(1), d.hit(2), d.hit(5), d.hit(4), d.hit(9));
    let senses = (go.sense, field.sense, code.sense, row.sense);
    assert_eq!(senses, (Sense::Click, Sense::Text, Sense::Text, Sense::Click));
    assert_eq!((go.rect.x, go.rect.y, go.rect.h), (PAD, PAD, BUTTON_H));
    assert!(danger.rect.x > go.rect.x + go.rect.w && danger.rect.y == go.rect.y);
    // The Fill's Code takes what the others leave: the item ends at the bottom.
    assert_eq!(row.rect.y + row.rect.h, 400.0 - PAD);
    assert!(code.rect.y > PAD + BUTTON_H && code.rect.h > 100.0);
    assert_eq!(field.rect.x, PAD + CARD_PAD);
    // Too tall to fit: the wheel scrolls the window.
    let tall = [Node::Spacer { px: 900 }, input("")];
    t.adopt(&tall, &[]);
    assert!(draw(&tall, &mut t, &mut view, None).hits.is_empty());
    assert!(wheel(&mut t, &mut view, 10.0, 10.0, 2000.0));
    assert_eq!(draw(&tall, &mut t, &mut view, None).hits[0].rect.y, 400.0 - PAD - FIELD_H);
    assert!(!wheel(&mut t, &mut view, 10.0, 10.0, 5.0));
    // Following (the Assistant), a view at the bottom stays there as the content grows.
    view.follow = true;
    let taller = [Node::Spacer { px: 1500 }, input("")];
    assert_eq!(draw(&taller, &mut t, &mut view, None).hits[0].rect.y, 400.0 - PAD - FIELD_H);
}

#[test]
fn pages_are_a_column_or_tabs_and_cards_ring_what_is_chosen() {
    let t = &THEMES[0];
    let nodes = vec![
        Node::Pages { id: 1, on: 1, labels: "Appearance\nAI\nPrivacy".into() },
        Node::Themes { id: 10 },
        Node::Choice { id: 20, on: true, text: "GLM 5.3\nbest answers".into() },
        Node::Switch { id: 30, on: false, label: "Living grain".into() },
        Node::Button { id: 31, variant: Variant::Link, label: "Send feedback".into() },
    ];
    let (mut texts, mut view) = (Texts::default(), View::default());
    // Wide, a column with the rest right of it; narrow, tabs side by side with the rest under.
    let d = draw_at(720.0, &nodes, &mut texts, &mut view, None);
    let (a, b) = (d.hit(1).rect, d.hit(2).rect);
    assert!(a.x == b.x && b.y > a.y && d.hit(10).rect.x > a.x + a.w, "{a:?} {b:?}");
    // The current theme (the first card's, Midnight's) and the chosen model wear the accent's
    // ring 3 px out; the switch and the link fill the width.
    let ring = |d: &Drawn| -> Vec<[f32; 4]> {
        let ring = |b: &&&Instance| b.color == t.accent && b.p0 == 2.0;
        d.of(Kind::Border).iter().filter(ring).map(|b| b.rect).collect()
    };
    let out = |r: RectF| [r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0];
    assert_eq!(ring(&d), [out(d.hit(10).rect), out(d.hit(20).rect)]);
    assert!([30, 31].iter().all(|&id| d.hit(id).rect.w == d.hit(20).rect.w));
    // Narrow, each tab its label's width and an even share of the rest, edge to edge.
    let d = draw_at(360.0, &nodes, &mut texts, &mut view, None);
    let (a, b, c) = (d.hit(1).rect, d.hit(2).rect, d.hit(3).rect);
    assert!(a.y == c.y && a.w > c.w && c.w > b.w && d.hit(10).rect.y > a.y + a.h, "{a:?} {c:?}");
    assert!((a.x + a.w - b.x).abs() < 0.5 && (b.x + b.w - c.x).abs() < 0.5);
    // Three themes across, then fewer as the window narrows.
    let mut rows = |w| {
        let d = draw_at(w, &nodes[1..2], &mut texts, &mut view, None);
        let mut ys: Vec<u32> = (10..13).map(|id| d.hit(id).rect.y as u32).collect();
        (ys.sort(), ys.dedup(), ys.len()).2
    };
    assert_eq!([rows(600.0), rows(400.0), rows(200.0)], [1, 2, 3]);
    // As the AI reads them: the page's tab selected, the theme and model chosen, the switch off,
    // a link (what lies below the window unmarked: without the themes, all of it shows).
    let marks = |nodes: &[Node]| sem(nodes, t).marks.into_iter().map(|m| (m.id, m.role, m.flags));
    let marks: Vec<_> = marks(&nodes).chain(marks(&[&nodes[..1], &nodes[2..]].concat())).collect();
    let want = [(1, 2, 0), (2, 2, 1), (10, 4, 1), (11, 4, 0), (20, 4, 1), (30, 3, 0), (31, 9, 0)];
    assert!(want.iter().all(|m| marks.contains(m)), "{marks:?}");
    // The faces, in even rows (two of five at 560 px), each 55 px; the one on ringed in the
    // accent and chosen.
    let faces = [Node::Faces { id: 50, on: 3 }];
    let d = draw(&faces, &mut texts, &mut view, None);
    let (first, sixth) = (d.hit(50).rect, d.hit(55).rect);
    assert!(first.w == 55.0 && sixth.x == first.x && sixth.y == first.y + 55.0 + 13.0);
    let on = d.hit(53).rect;
    let lit = |b: &&&Instance| b.color == t.accent && b.rect == [on.x, on.y, on.w, on.h];
    assert_eq!(d.of(Kind::Border).iter().filter(lit).count(), 1);
    let marks: Vec<_> = sem(&faces, t).marks.into_iter().map(|m| (m.id, m.flags)).collect();
    assert!(marks.len() == 10 && marks.contains(&(53, 1)) && marks.contains(&(50, 0)));
}

#[test]
fn charts_skip_what_they_do_not_know_meters_fill_and_columns_head_the_cells() {
    let (t, unknown) = (&THEMES[0], uiwire::UNKNOWN);
    let nodes = vec![
        Node::Chart { id: 0, hue: 11, h: 64, values: vec![unknown, unknown, 0, 500, 1000] },
        Node::Meter { id: 0, hue: 4, value: 250 },
        Node::Columns { id: 20, on: 2, labels: "Name\tCPU\tMemory".into() },
    ];
    let d = draw(&nodes, &mut Texts::default(), &mut View::default(), None);
    // Two segments: none to or from a moment unknown; the newest a dot in the accent.
    assert_eq!(d.of(Kind::Line).len(), 2);
    assert!(d.of(Kind::Fill).iter().any(|i| i.color == t.accent && i.rect[2] == 6.0));
    let bar = d.of(Kind::Fill).into_iter().find(|i| i.color == t.ansi[4]).expect("the fill");
    assert_eq!(bar.rect[2], 140.0, "a quarter of 560");
    // Each label a click, the columns 80 px wide from the right, a chevron's room kept.
    let (cpu, mem) = (d.hit(21).rect, d.hit(22).rect);
    assert_eq!((cpu.w, mem.w, mem.x + mem.w, mem.x - cpu.x), (80.0, 80.0, 551.0, 80.0));
    assert_eq!((d.hit(20).rect.x, d.hit(20).rect.x + d.hit(20).rect.w), (20.0, cpu.x));
}
