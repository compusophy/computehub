use gfx::{DrawList, Instance, Kind};
use ui::{Hit, Key, THEMES, UiState};

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
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(ui::FontId::Mono, MONO.to_vec()).unwrap();
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let state = UiState { focused: true, hover: hover.map(WidgetId), ..UiState::default() };
    let rect = RectF::new(0.0, 0.0, w, 400.0);
    super::draw(
        &mut Ui::new(&mut list, &mut ts, rect, &mut hits, state, &THEMES[0]),
        nodes,
        texts,
        view,
    );
    Drawn { list, hits }
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
