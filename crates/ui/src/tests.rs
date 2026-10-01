use super::*;
use gfx::{DrawList, Instance, RectF, Rgba};

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");

const WHITE: Rgba = Rgba::hex(0xffffff);
const SANS14: TextStyle = TextStyle::new(FontId::Sans, 14.0, WHITE);

fn ts() -> TextSystem {
    let mut t = TextSystem::new(SANS.to_vec()).unwrap();
    t.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    t.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    t
}

/// The instances of `kind` (4 for glyphs), in `color` if given.
fn find(list: &DrawList, kind: f32, color: Option<Rgba>) -> Vec<Instance> {
    let hit = |i: &&Instance| i.kind == kind && color.is_none_or(|c| i.color == c);
    list.instances().iter().filter(hit).copied().collect()
}

fn glyphs(list: &DrawList) -> Vec<Instance> {
    find(list, 4.0, None)
}

type Frame<R> = (R, DrawList, Vec<Hit>);

/// Builds one frame and checks the `Ui` left no clip open.
fn frame<R>(t: &mut TextSystem, r: RectF, st: UiState, f: impl FnOnce(&mut Ui) -> R) -> Frame<R> {
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let out = f(&mut Ui::new(&mut list, t, r, &mut hits, st));
    let none = RectF::new(-1e9, -1e9, 2e9, 2e9);
    assert_eq!(list.clip(), none, "clips left open");
    (out, list, hits)
}

fn state(hover: Option<u32>, pressed: Option<u32>, focused: bool) -> UiState {
    let mut s = UiState::default();
    (s.hover, s.pressed) = (hover.map(WidgetId), pressed.map(WidgetId));
    s.focused = focused;
    s
}

fn rest() -> UiState {
    UiState::default()
}

#[test]
fn widgets_stack_and_register_hits() {
    let mut t = ts();
    let rect = RectF::new(100.0, 50.0, 400.0, 600.0);
    let (rs, list, hits) = frame(&mut t, rect, rest(), |ui| {
        assert_eq!((ui.width(), ui.rect()), (376.0, rect));
        assert_eq!(ui.cursor(), (112.0, 62.0));
        let h = ui.heading("Settings");
        let l = ui.label("Body text");
        ui.space(10.0);
        let s = ui.separator();
        let b = ui.button(WidgetId(1), "Save");
        let f = ui.text_field(WidgetId(2), "", false, "Name");
        let k = ui.key_value("Version", "0.1.0");
        let m = ui.mono("let x = 1;");
        let sm = ui.small("fine print");
        let p = ui.button_primary(WidgetId(3), "Go");
        [h, l, s, b, f, k, m, sm, p]
    });
    let [h, l, s, b, f, k, m, sm, p] = rs;
    assert_eq!((h.x, h.y), (112.0, 62.0));
    assert!(h.w > 0.0 && h.h >= 20.0);
    assert_eq!(l.y, h.y + h.h + SPACING);
    assert_eq!((s.y, s.w, s.h), (l.y + l.h + SPACING + 10.0, 376.0, 1.0));
    assert_eq!((b.y, b.h), (s.y + 1.0 + SPACING, BUTTON_H));
    assert!(b.w > 28.0 && b.w < 100.0);
    assert_eq!((f.y, f.w, f.h), (b.y + BUTTON_H + SPACING, 376.0, FIELD_H));
    assert_eq!(k.y, f.y + FIELD_H + SPACING);
    assert!(m.y > k.y && sm.y > m.y && p.y > sm.y);
    let got: Vec<(u32, RectF, Sense)> = hits.iter().map(|h| (h.id.0, h.rect, h.sense)).collect();
    let (c, x) = (Sense::Click, Sense::Text);
    assert_eq!(got, [(1, b, c), (2, f, x), (3, p, c)]);
    let top = |x, y| hit_test(&hits, x, y).map(|h| h.id.0);
    assert_eq!(top(b.x + 1.0, b.y + 1.0), Some(1));
    assert_eq!(top(b.x - 1.0, b.y + 1.0), None);
    // Everything drew inside the rect's clip, and the placeholder is dim.
    assert!(list.instances().iter().all(|i| {
        let [x, y, w, h] = i.clip;
        x >= 100.0 && y >= 50.0 && x + w <= 500.0 && y + h <= 650.0
    }));
    assert!(find(&list, 4.0, Some(theme::TEXT_DIM)).len() >= 4);
}

#[test]
fn rows_run_left_to_right() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 600.0, 400.0);
    let ((row, [a, b, c], after), _, hits) = frame(&mut t, rect, rest(), |ui| {
        let mut r = [RectF::default(); 3];
        let row = ui.row(|ui| {
            r[0] = ui.button(WidgetId(1), "One");
            r[1] = ui.label("tall\ntext\nhere");
            ui.space(4.0);
            r[2] = ui.button(WidgetId(2), "Two");
        });
        (row, r, ui.label("after"))
    });
    assert_eq!((a.x, a.y), (12.0, 12.0));
    assert_eq!((b.x, b.y, b.h), (a.x + a.w + SPACING, 12.0, 3.0 * 17.0));
    assert_eq!((c.x, c.y), (b.x + b.w + SPACING + 4.0, 12.0));
    assert_eq!(row, RectF::new(12.0, 12.0, c.x + c.w - 12.0, b.h));
    assert_eq!((after.x, after.y), (12.0, 12.0 + b.h + SPACING));
    assert_eq!(hits.len(), 2);
    // Nested rows, and custom content with the low-level calls.
    let ((inner, below), list, hits) = frame(&mut t, rect, rest(), |ui| {
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
        assert!(ui.text(x, y + 20.0, "raw", SANS14) > 0.0);
        ui.hit(WidgetId(7), r, Sense::Scroll);
        ui.advance_to(y + 50.0);
        ui.advance_to(0.0);
        (inner, ui.label("below"))
    });
    assert_eq!(inner.h, BUTTON_H);
    assert_eq!(below.y, 12.0 + BUTTON_H + SPACING + 50.0);
    assert_eq!((hits.len(), hits[2].sense), (3, Sense::Scroll));
    assert!(list.len() >= 8);
}

#[test]
fn button_states_and_hit_clipping() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 100.0);
    let mut fill_of = |st: UiState, primary: bool| {
        let (_, list, _) = frame(&mut t, rect, st, |ui| match primary {
            true => ui.button_primary(WidgetId(9), "Ok"),
            false => ui.button(WidgetId(9), "Ok"),
        });
        list.instances()[0].color
    };
    let hover = state(Some(9), None, false);
    let down = state(Some(9), Some(9), false);
    let away = state(None, Some(9), false);
    assert_eq!(fill_of(rest(), false), theme::BUTTON);
    assert_eq!(fill_of(hover, false), theme::BUTTON_HOVER);
    assert_eq!(fill_of(down, false), theme::BUTTON_PRESSED);
    assert_eq!(fill_of(away, false), theme::BUTTON);
    assert_eq!(fill_of(rest(), true), theme::ACCENT);
    assert_eq!(fill_of(hover, true), theme::ACCENT_HOVER);
    assert_eq!(fill_of(down, true), theme::ACCENT_PRESSED);
    // Hits are clipped to what is visible; hidden ones are dropped.
    let (_, _, hits) = frame(&mut t, rect, rest(), |ui| {
        ui.push_clip(RectF::new(0.0, 0.0, 300.0, 40.0));
        ui.button(WidgetId(1), "Half"); // y 12..42: clipped at 40
        ui.button(WidgetId(2), "Gone"); // y 50..80: hidden
        ui.push_clip(RectF::new(0.0, 0.0, 10.0, 10.0));
        for _ in 0..3 {
            ui.pop_clip(); // one too many: the Ui's own clip stays
        }
        let r = RectF::new(250.0, 90.0, 99.0, 99.0);
        ui.hit(WidgetId(3), r, Sense::Click);
        ui.push_clip(r); // left open: dropping the Ui pops it
    });
    assert_eq!(hits.len(), 2);
    assert_eq!((hits[0].rect.y, hits[0].rect.h), (12.0, 28.0));
    assert_eq!(hits[1].rect, RectF::new(250.0, 90.0, 50.0, 10.0));
}

fn field(t: &mut TextSystem, st: UiState, value: &str, has: bool) -> Frame<RectF> {
    let rect = RectF::new(0.0, 0.0, 200.0, 100.0);
    frame(t, rect, st, |ui| ui.text_field(WidgetId(1), value, has, ""))
}

#[test]
fn text_fields_scroll_and_show_a_caret() {
    let mut t = ts();
    let focused = state(None, None, true);
    let caret = |list: &DrawList| find(list, 0.0, Some(theme::ACCENT)).first().copied();
    // Short, focused: text from the left, caret right after it, accent ring.
    let (f, list, _) = field(&mut t, focused, "hi", true);
    let c = caret(&list).expect("a caret");
    assert!((c.rect[0] - (f.x + 10.0 + t.measure("hi", SANS14))).abs() <= 0.5);
    assert_eq!(find(&list, 1.0, Some(theme::ACCENT)).len(), 1);
    // A focused field in an unfocused window, or an unfocused field: none.
    assert!(caret(&field(&mut t, rest(), "hi", true).1).is_none());
    assert!(caret(&field(&mut t, focused, "hi", false).1).is_none());
    // Long: the end stays in view with the caret at the right edge, and the
    // start scrolls out of the clip.
    let long = "a fairly long value that cannot fit in the field";
    let (f, list, _) = field(&mut t, focused, long, true);
    let [cx, _, cw, _] = caret(&list).unwrap().rect;
    assert!(cx + cw <= f.x + f.w - 10.0 + 0.5 && cx > f.x + f.w - 20.0);
    let g = glyphs(&list);
    assert!(g.len() < long.chars().filter(|c| *c != ' ').count());
    assert!(g.iter().all(|i| i.clip[0] == f.x + 10.0));
}

#[test]
fn labels_wrap_to_the_content_width() {
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 224.0, 1000.0); // content 200 wide
    let text = "Wrapping keeps every line inside the window, between words.\nA new paragraph.";
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.label(text));
    let lines = t.wrap(text, SANS14, 200.0);
    assert!(lines.len() >= 4 && r.w <= 200.0);
    assert_eq!(r.h, lines.len() as f32 * 17.0);
    let g = glyphs(&list);
    for [x, _, w, _] in g.iter().map(|i| i.rect) {
        assert!(x >= 12.0 && x + w <= 213.0);
    }
    let mut rows: Vec<i32> = g.iter().map(|i| (i.rect[1] as i32 - 12) / 17).collect();
    rows.dedup();
    assert_eq!(rows.len(), lines.len());
    // Lines outside the clip are measured but not drawn.
    let short = RectF::new(0.0, 0.0, 224.0, 40.0);
    let (r2, list, _) = frame(&mut t, short, rest(), |ui| ui.label(text));
    let shown = glyphs(&list);
    assert!(r2 == r && !shown.is_empty() && shown.len() < g.len());
    assert!(shown.iter().all(|i| i.rect[1] < 40.0));
}

#[test]
fn pairing_fragments() {
    let tok = "0123456789abcdefABCDEF0123456789abcdef0123456789abcdef0123456789";
    let p = parse_pairing(&format!("node=8123&token={tok}")).unwrap();
    assert_eq!((p.port, &p.token[..4]), (8123, &[1, 0x23, 0x45, 0x67][..]));
    assert_eq!(p.token_hex(), tok.to_ascii_lowercase());
    assert!(!format!("{p:?}").contains("0123"));
    let port = |s: String| parse_pairing(&s).map(|p| p.port);
    assert_eq!(port(format!("#token={tok}&x=1&node=1")), Some(1));
    assert_eq!(port(format!("a&node=65535&&token={tok}&b=")), Some(65535));
    assert_eq!(parse_token(&tok[1..]), None);
    assert_eq!(parse_token(&format!("{tok}0")), None);
    assert_eq!(parse_token(&format!("{}g", &tok[1..])), None);
    assert_eq!(parse_token(&"é".repeat(32)), None);
    for n in ["0", "65536", "+80", "-1", "", "123456", " 80", "80&node=81"] {
        assert_eq!(port(format!("node={n}&token={tok}")), None, "{n}");
    }
    for s in ["", "node=8123", "node=80&token", "node=80&token=00"] {
        assert_eq!(parse_pairing(s), None, "{s}");
    }
    assert_eq!(port(format!("token={tok}")), None);
    assert_eq!(port(format!("node=80&token={tok}&token={tok}")), None);
}

#[test]
fn palette_and_keys() {
    let x = theme::xterm_color;
    assert_eq!((x(1), x(15)), (theme::ANSI[1], theme::ANSI[15]));
    let cube = [(16, 0x000000), (21, 0x0000ff), (196, 0xff0000)];
    let more = [(208, 0xff8700), (110, 0x87afd7), (231, 0xffffff)];
    let grays = [(232, 0x080808), (244, 0x808080), (255, 0xeeeeee)];
    for (i, rgb) in cube.into_iter().chain(more).chain(grays) {
        assert_eq!(x(i), Rgba::hex(rgb), "{i}");
    }
    // Every ANSI color but black stands out from the window.
    let luma = |c: Rgba| 2 * u32::from(c.0) + 7 * u32::from(c.1) + u32::from(c.2);
    for c in &theme::ANSI[1..] {
        assert!(luma(*c) > luma(theme::WINDOW) + 600, "{c:?}");
    }
    assert_ne!(theme::app_tint(1), theme::app_tint(2));
    let k = Key::from_code;
    let char_of = |code| if let Key::Char(c) = k(code) { c } else { '?' };
    let chars = ["KeyA", "KeyZ", "Digit0", "Numpad9"].map(char_of);
    assert_eq!(chars, ['a', 'z', '0', '9']);
    assert_eq!((k("Enter"), k("NumpadEnter")), (Key::Enter, Key::Enter));
    assert_eq!((k("ArrowUp"), k("PageDown")), (Key::Up, Key::PageDown));
    assert_eq!((k("F1"), k("F24")), (Key::F(1), Key::F(24)));
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
        cx.open_floating("about");
        true
    }
}

#[test]
fn apps_and_their_context() {
    let mut app: Box<dyn App> = Box::new(Echo(0));
    assert!(!app.wants_text_input() && app.preferred_size().is_none());
    let mut t = ts();
    let rect = RectF::new(0.0, 0.0, 300.0, 200.0);
    let (_, _, hits) = frame(&mut t, rect, rest(), |ui| app.draw(ui));
    let (mut fs, mut next) = (vfs::Vfs::new(), u32::MAX);
    let mut cx = Cx::new(&mut fs, 5.0, None, &mut next);
    assert!(!app.event(AppEvent::Focus(true), &mut cx));
    assert!(app.event(AppEvent::Click(hits[0].id), &mut cx));
    cx.close_self();
    assert_eq!(cx.connect(9), SocketId(u32::MAX));
    assert_eq!(cx.connect(9), SocketId(0));
    cx.close_socket(SocketId(0));
    cx.load_fallback_fonts();
    cx.open("files");
    let want = "[Open { name: \"about\", floating: true }, CloseSelf, \
        Connect { socket: SocketId(4294967295), port: 9 }, \
        Connect { socket: SocketId(0), port: 9 }, CloseSocket(SocketId(0)), \
        LoadFallbackFonts, Open { name: \"files\", floating: false }]";
    assert_eq!(format!("{:?}", cx.take_requests()), want);
    assert!(cx.take_requests().is_empty());
    assert_eq!((cx.now_ms, cx.pairing), (5.0, None));
    assert_eq!((fs.read("/tmp/clicks"), next), (Ok(&[1][..]), 1));
    assert_eq!(app.title(), "Echo 1");
}

#[test]
fn key_columns_break_keys_between_words() {
    let mut t = ts();
    let dim = Some(theme::TEXT_DIM);
    // Content 256 wide: a 102 px key column, too narrow for the whole key.
    let rect = RectF::new(0.0, 0.0, 280.0, 400.0);
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.key_value("Alt+Shift+Enter", "Run"));
    assert_eq!(r.h, 2.0 * 17.0);
    let rows: Vec<i32> =
        find(&list, 4.0, dim).iter().map(|i| (i.rect[1] as i32 - 12) / 17).collect();
    assert_eq!(rows.iter().filter(|&&y| y == 1).count(), "Enter".len());
    // A key whose one word is wider than 40% widens its column instead.
    let word = t.measure("platform", SANS14);
    let rect = RectF::new(0.0, 0.0, 160.0, 400.0);
    let (r, list, _) = frame(&mut t, rect, rest(), |ui| ui.key_value("platform", "x"));
    assert_eq!(r.h, 17.0);
    let value = find(&list, 4.0, Some(theme::TEXT));
    assert!(value[0].rect[0] >= 12.0 + word + SPACING, "{value:?}");
}
