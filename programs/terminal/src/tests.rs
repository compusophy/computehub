use super::*;
use gfx::{DrawList, Instance, Kind, RectF, Rgba};
use ui::{FontId, THEMES, TextSystem, Theme};

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
/// The content size of an 80 x 24 terminal.
const W80: u16 = 668;
const H24: u16 = 436;

/// A terminal sized 80 x 24, its first requests (the console) taken.
fn sized() -> Terminal {
    let mut t = Terminal::default();
    t.event(&Event::Resize { w: W80, h: H24 });
    assert_eq!(t.frame().requests, [Request::Tty { cols: 80, rows: 24 }]);
    t
}

/// The bytes the requests of the next frame type into the console.
fn typed(t: &mut Terminal) -> Vec<u8> {
    let reqs = t.frame().requests;
    reqs.into_iter()
        .flat_map(|r| if let Request::Input { data } = r { data } else { vec![] })
        .collect()
}

fn key(key: Key, mods: u8, ch: char) -> Event {
    Event::Key { id: 0, key, mods, ch }
}

fn text(s: &str) -> Event {
    Event::Text { text: s.into() }
}

/// Row `r` of the screen as text, trailing blanks trimmed.
fn row(t: &Terminal, r: u16) -> String {
    let cells = t.term.row(r).iter().filter(|c| c.width != 0);
    cells.map(|c| c.ch).collect::<String>().trim_end().into()
}

#[test]
fn the_first_size_asks_for_a_console_and_a_new_one_resizes_it() {
    let mut t = sized();
    // The same size asks nothing; a new one, a console that size.
    assert!(t.event(&Event::Resize { w: W80, h: H24 }) && t.frame().requests.is_empty());
    t.event(&Event::Resize { w: W80 - 80, h: H24 });
    assert_eq!(t.frame().requests, [Request::Tty { cols: 70, rows: 24 }]);
    // The screen as the window shows it: the grid, the cursor at home, the title.
    let f = t.frame();
    let Node::Screen { cols, rows, cursor, cells, .. } = &f.nodes[0] else { panic!("a screen") };
    assert_eq!(
        (*cols, *rows, *cursor, cells.len(), f.title.as_str()),
        (70, 24, Some((0, 0)), 70 * 24 * 13, "Terminal")
    );
    t.event(&Event::Output { data: b"\x1b]2;notes\x07".to_vec() });
    assert_eq!(t.frame().title, "Terminal \u{2014} notes");
    // The largest screens give way in rows; the least is one cell.
    assert_eq!([uiwire::screen(0, 0), uiwire::screen(9000, 9000)], [(1, 1), (1000, 65)]);
}

#[test]
fn keys_and_pastes_go_to_the_console_as_xterm_sends_them() {
    let mut t = sized();
    t.event(&key(Key::Up, 0, '\0'));
    t.event(&key(Key::Char, mods::CTRL, 'c'));
    assert!(!t.event(&key(Key::Char, 0, 'x')), "comes as text");
    t.event(&text("é"));
    assert!(!t.event(&text("\n")), "Enter came as a key already");
    t.event(&key(Key::Enter, 0, '\0'));
    t.event(&text("a\nb"));
    t.event(&key(Key::Home, mods::SHIFT, '\0'));
    t.event(&key(Key::F, 0, '\u{5}'));
    assert_eq!(typed(&mut t), "\x1b[A\x03é\ra\rb\x1b[1;2H\x1b[15~".as_bytes());
    // Application cursor keys, and a bracketed paste, as the program asked.
    t.event(&Event::Output { data: b"\x1b[?1h\x1b[?2004h".to_vec() });
    t.event(&key(Key::Left, 0, '\0'));
    t.event(&text("p\x1bq"));
    assert_eq!(typed(&mut t), b"\x1bOD\x1b[200~pq\x1b[201~");
}

#[test]
fn the_terminal_shows_its_programs_answers_their_queries_and_does_their_asks() {
    let mut t = sized();
    assert!(!t.event(&Event::Output { data: b"x\r\ny".to_vec() }) || row(&t, 1) == "y");
    assert_eq!((row(&t, 0), row(&t, 1), t.term.cursor()), ("x".into(), "y".into(), (1, 1)));
    // A query's answer goes back to the console.
    t.event(&Event::Output { data: b"\x1b[6n".to_vec() });
    assert_eq!(typed(&mut t), b"\x1b[2;2R");
    // Asks: an app opens, a known theme applies (Mono, Mono Dark's old name, is none).
    let ask = |verb: &str, arg: &str| ["\x1b]1729;", verb, ";", arg, "\x07"].concat();
    let asks = [ask("open", "editor:/tmp/a"), ask("theme", "Mono Light"), ask("theme", "Mono")];
    t.event(&Event::Output { data: asks.concat().into_bytes() });
    let theme = Request::Pref { key: "theme".into(), value: "Mono Light".into() };
    assert_eq!(t.frame().requests, [Request::Open { name: "editor:/tmp/a".into() }, theme]);
    // The wheel scrolls back by whole rows; a key snaps back.
    (0..40)
        .for_each(|i| _ = t.event(&Event::Output { data: format!("line {i}\r\n").into_bytes() }));
    let sb = t.term.scrollback_len();
    assert!(sb > 10 && t.event(&Event::Wheel { dy: -17 * 3 }) && t.scroll == 3, "{sb}");
    assert!(!t.event(&Event::Wheel { dy: 8 }), "half a row");
    assert!(t.event(&Event::Wheel { dy: 9 }) && t.scroll == 2);
    assert!(t.event(&Event::Wheel { dy: i32::MIN }) && t.scroll == sb);
    let f = t.frame();
    assert!(matches!(f.nodes[0], Node::Screen { cursor: None, .. }), "no cursor scrolled back");
    assert!(t.event(&text("x")) && t.scroll == 0, "text snaps back");
    t.event(&Event::Output { data: b"\x1b[?1049h".to_vec() });
    assert!(!t.event(&Event::Wheel { dy: -34 }), "no scrollback on the alternate screen");
}

#[test]
fn the_shells_clean_end_closes_the_window_and_a_failure_says_so() {
    let mut t = sized();
    t.event(&Event::Ended { status: 0 });
    assert_eq!(t.frame().requests, [Request::Close]);
    let mut t = sized();
    t.event(&Event::Output { data: b"half".to_vec() });
    t.event(&Event::Ended { status: 3 });
    assert_eq!([row(&t, 0), row(&t, 1)], ["half", "[sh stopped with status 3]"]);
    // Nothing more goes to a console that ended, nor is asked of it.
    t.event(&text("x"));
    t.event(&Event::Resize { w: W80 - 8, h: H24 });
    assert!(t.frame().requests.is_empty());
}

/// A text system once the Mono font arrived (with a symbols fallback).
fn text_system() -> TextSystem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    ts.add_fallback(SYM_A.to_vec()).unwrap();
    ts
}

/// The terminal's screen drawn in `r` as the desktop draws it, in `theme`.
fn draw(t: &mut Terminal, ts: &mut TextSystem, r: RectF, theme: &Theme, focused: bool) -> DrawList {
    let mut list = DrawList::new();
    let f = t.frame();
    let Node::Screen { cols, rows, cursor, cells, .. } = &f.nodes[0] else { panic!("a screen") };
    ui::screen::draw(&mut list, ts, theme, r, (*cols, *rows, *cursor, cells), focused);
    list
}

fn of(list: &DrawList, kind: Kind) -> impl Iterator<Item = &Instance> {
    list.instances().iter().filter(move |i| i.kind == kind as u8 as f32)
}

#[test]
fn the_desktop_draws_the_screen_in_every_theme() {
    let mut t = Terminal::default();
    t.event(&Event::Resize { w: W80 + 160, h: H24 });
    let styled = "\x1b[1;31mred\x1b[m \x1b[7minv\x1b[m \x1b[4;9mls\x1b[m \x1b[2mdim\x1b[m \
                  \x1b[8mhid\x1b[m ✻⏺╭─╮日\r\n\x1b[48;2;10;20;30m  \x1b[m";
    t.event(&Event::Output { data: styled.as_bytes().to_vec() });
    let rect = RectF::new(10.0, 40.0, f32::from(W80) + 160.0, f32::from(H24));
    let mut ts = text_system();
    // Before Mono arrives: the same fills, but no glyphs.
    let mut boot = TextSystem::new(SANS.to_vec()).unwrap();
    let early = draw(&mut t, &mut boot, rect, &THEMES[0], true);
    assert_eq!((t.term.cols(), t.term.rows()), (100, 24));
    for theme in &THEMES {
        let list = draw(&mut t, &mut ts, rect, theme, true);
        // Every visible char but the hidden ones and the one in no font (a box).
        let ink: Vec<Rgba> = of(&list, Kind::Glyph).map(|i| i.color).collect();
        assert_eq!((ink.len(), of(&list, Kind::Border).count()), (16, 1));
        // Bold makes red bright; dim is dim; inverse puts the opaque surface in front.
        let (red, dim, opaque) = (theme.ansi[9], theme.text.with_alpha(153), theme.surface);
        let n = |c: Rgba| ink.iter().filter(|&&i| i == c).count();
        assert_eq!([red, dim, opaque.with_alpha(255)].map(n), [3; 3]);
        let fills: Vec<_> = of(&list, Kind::Fill).map(|i| (i.rect, i.color)).collect();
        let has = |r: [f32; 4], color| fills.contains(&(r, color));
        assert!(has([56.0, 54.0, 24.0, 17.0], theme.text), "inverse: text behind");
        assert!(has([24.0, 71.0, 16.0, 17.0], Rgba(10, 20, 30, 255)));
        assert!(has([40.0, 71.0, 8.0, 17.0], theme.accent), "a steady block cursor");
        assert_eq!(fills.iter().filter(|f| f.0[3] == 1.0).count(), 4, "lines");
        if theme == &THEMES[0] {
            assert!(early.instances().iter().eq(of(&list, Kind::Fill)), "the same fills");
        }
        let list = draw(&mut t, &mut ts, rect, theme, false);
        let ring = |i: &&Instance| i.kind == 1.0 && i.color == theme.accent;
        assert_eq!(list.instances().iter().filter(ring).count(), 1, "unfocused: an outline");
    }
    // On a char the block cursor shows it in the accent's own ink.
    let mut t = sized();
    t.event(&Event::Output { data: b"ab\x1b[D".to_vec() });
    let list = draw(&mut t, &mut ts, rect, &THEMES[0], true);
    let ink = of(&list, Kind::Glyph).filter(|i| i.color == THEMES[0].accent_text).count();
    assert_eq!(ink, 1);
}
