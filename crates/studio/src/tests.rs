use super::*;
use applang::{Node, codes};
use gfx::{DrawList, Instance, RectF};
use ui::{App, AppEvent, Cx, FontId, Hit, Key, Mods, Request, Sense, TextSystem, Ui, UiState};
use ui::{BUTTON_H, PAD, SPACING, THEMES, Theme, WidgetId};

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../assets/fonts/deferred/Inter-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/deferred/JetBrainsMono-Regular.ttf");
const INPUT: u32 = 1 << 31;
const RUN: AppEvent = AppEvent::Click(WidgetId(1));
/// Where test frames put the `Ui` rect: away from the corner, as a window
/// below the panel is, so content-relative and rect coordinates differ.
const ORIGIN: (f32, f32) = (37.0, 81.0);

fn fs() -> Vfs {
    let mut fs = Vfs::new();
    install_samples(&mut fs);
    fs
}

/// Sends `ev` with a fresh context; returns the redraw flag and requests.
fn send(app: &mut dyn App, fs: &mut Vfs, ev: AppEvent) -> (bool, Vec<Request>) {
    let mut kernel = ui::kernel::Kernel::new();
    let mut cx = Cx::new(fs, &mut kernel, 0.0);
    let redraw = app.event(ev, &mut cx);
    (redraw, cx.take_requests())
}

fn key(key: Key) -> AppEvent {
    AppEvent::Key { key, mods: Mods::default() }
}

fn ctrl(key: Key) -> AppEvent {
    AppEvent::Key { key, mods: Mods { ctrl: true, ..Mods::default() } }
}

fn text(t: &str) -> AppEvent {
    AppEvent::Text(t.to_string())
}

fn click(id: u32) -> AppEvent {
    AppEvent::Click(WidgetId(id))
}

fn wheel(dy: f32) -> AppEvent {
    AppEvent::Wheel { x: 0.0, y: 0.0, dy }
}

fn opened(name: &str) -> Vec<Request> {
    vec![Request::Open { name: name.to_string(), floating: false }]
}

/// A press at `(dx, dy)` into hit `h`, relative to the content's corner as
/// the shell sends it (hits are in the `Ui` rect's space).
fn press(h: Hit, dx: f32, dy: f32) -> AppEvent {
    let (x, y) = (h.rect.x + dx - ORIGIN.0, h.rect.y + dy - ORIGIN.1);
    AppEvent::PointerDown { x, y, id: Some(h.id) }
}

fn text_system() -> TextSystem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    ts
}

/// One focused frame of `app` in `theme`, `w` x 480 at [`ORIGIN`].
fn themed(app: &mut dyn App, w: f32, theme: &Theme) -> (DrawList, Vec<Hit>) {
    let (mut ts, mut list, mut hits) = (text_system(), DrawList::new(), Vec::new());
    let state = UiState { focused: true, ..UiState::default() };
    let rect = RectF::new(ORIGIN.0, ORIGIN.1, w, 480.0);
    app.draw(&mut Ui::new(&mut list, &mut ts, rect, &mut hits, state, theme));
    assert!(list.len() > 5);
    (list, hits)
}

/// The hits of one 640 px wide frame.
fn frame(app: &mut dyn App) -> Vec<Hit> {
    themed(app, 640.0, &THEMES[0]).1
}

fn hit(hits: &[Hit], id: u32) -> Hit {
    *hits.iter().find(|h| h.id == WidgetId(id)).expect("no such hit")
}

fn labels(nodes: &[Node]) -> Vec<String> {
    let each = |n: &Node| match n {
        Node::Label { text } => vec![text.clone()],
        Node::Row { children } | Node::Col { children } => labels(children),
        _ => Vec::new(),
    };
    nodes.iter().flat_map(each).collect()
}

fn host(fs: &Vfs, path: &str) -> AppHost {
    let mut h = AppHost::new(path);
    h.load(fs);
    h
}

/// The line of the problem that keeps `h` from running.
fn broken(h: &AppHost) -> String {
    let crate::host::Run::Broken(p) = &h.run else { panic!("it runs") };
    p.line()
}

fn studio(fs: &Vfs, path: &str) -> Studio {
    let mut s = Studio::new(path);
    s.load(fs);
    s
}

#[test]
fn samples_compile_and_install_once() {
    for (path, src) in SAMPLES.into_iter().chain([("new", NEW_APP)]) {
        assert!(applang::compile(src).is_ok(), "{path}");
    }
    let mut fs = Vfs::new();
    fs.write(DEFAULT_FILE, b"label 1;").unwrap();
    install_samples(&mut fs);
    assert_eq!(fs.read(DEFAULT_FILE), Ok(&b"label 1;"[..]));
    assert_eq!(fs.read("/apps/clicker.app"), Ok(CLICKER.as_bytes()));
    assert!(fs.is_file("/apps/greeter.app"));
}

#[test]
fn editor_edits_across_lines() {
    let mut e = Editor::new("row {\n  label 1;\n}");
    e.set_caret(1, 99);
    assert_eq!(e.caret(), (1, 10));
    e.newline();
    assert_eq!((e.caret(), e.line(2)), ((2, 2), "  "));
    e.insert("label 2;");
    assert_eq!(e.text(), "row {\n  label 1;\n  label 2;\n}");
    // Enter inside the indentation carries only what is left of the caret.
    e.set_caret(2, 1);
    e.newline();
    assert_eq!(([e.line(2), e.line(3)], e.caret()), ([" ", "  label 2;"], (3, 1)));
    e.backspace();
    assert_eq!((e.line(3), e.caret()), (" label 2;", (3, 0)));
    // Backspace at a line's start joins it to the line above.
    e.backspace();
    assert_eq!((e.line(2), e.caret()), ("  label 2;", (2, 1)));
    e.set_caret(2, 0);
    e.backspace();
    assert_eq!((e.line(1), e.caret()), ("  label 1;  label 2;", (1, 10)));
    e.end();
    e.delete();
    assert_eq!(e.text(), "row {\n  label 1;  label 2;}");
    e.set_caret(0, 0);
    e.backspace();
    // Tabs become two spaces, \r and other controls go, \n splits.
    e.insert("a\tb\r\nc\u{7}");
    assert_eq!([e.line(0), e.line(1)], ["a  b", "crow {"]);
    assert_eq!((e.caret(), e.line_count()), ((1, 1), 3));
    e.insert("éü");
    e.left();
    e.delete();
    assert_eq!(e.line(1), "cérow {");
}

#[test]
fn editor_arrows_at_line_ends_and_clicks_on_a_grid() {
    let mut e = Editor::new("abcdef\nab\nabcdef");
    e.set_caret(0, 6);
    e.right();
    assert_eq!(e.caret(), (1, 0));
    e.left();
    assert_eq!(e.caret(), (0, 6));
    e.set_caret(0, 5);
    let mut seen = Vec::new();
    for _ in 0..3 {
        e.down();
        seen.push(e.caret());
    }
    assert_eq!(seen, [(1, 2), (2, 5), (2, 6)]);
    e.right();
    e.home();
    e.left();
    assert_eq!(e.caret(), (1, 2));
    e.up();
    e.up();
    e.left();
    assert_eq!(e.caret(), (0, 0));

    let mut e = Editor::new("state x = 1;\nlabel x;");
    let clicks =
        [(3.4, 1.5, (1, 3)), (3.6, 0.3, (0, 4)), (99.0, 9.0, (1, 8)), (-1.0, -1.0, (0, 0))];
    for (col, row, caret) in clicks {
        e.click(8.0 * col, 17.0 * row, 8.0, 17.0);
        assert_eq!(e.caret(), caret, "{col} {row}");
    }
    e.click(f32::NAN, 20.0, 0.0, 17.0);
    assert_eq!(e.caret(), (1, 0));

    // Through Studio: a press relative to the content's corner lands on the
    // drawn grid, the text a 4-cell gutter and a 6 px inset into the well.
    let mut fs = fs();
    let mut s = Studio::new(DEFAULT_FILE);
    let well = hit(&frame(&mut s), 4);
    assert_eq!(well.sense, Sense::Text);
    let g = s.geo.unwrap();
    assert_eq!((g.cell, g.row), (8.0, 17.0));
    let at = |well, col: f32, row: f32| press(well, (4.0 + col) * 8.0, 6.0 + row * 17.0);
    assert!(send(&mut s, &mut fs, at(well, 3.4, 4.5)).0);
    assert_eq!(s.ed.caret(), (4, 3));
    // On an empty line the caret goes to its end; what is typed lands there.
    fs.write("/apps/t.app", b"one\ntwo\n\nfour").unwrap();
    let mut s = studio(&fs, "/apps/t.app");
    let well = hit(&frame(&mut s), 4);
    for ev in [at(well, 3.4, 2.5), text("X"), ctrl(Key::Char('s'))] {
        send(&mut s, &mut fs, ev);
    }
    assert_eq!(fs.read("/apps/t.app"), Ok(&b"one\ntwo\nX\nfour"[..]));
}

#[test]
fn failed_run_lists_problems_and_clicks_jump() {
    let mut fs = fs();
    fs.write("/apps/bad.app", b"state count = 0;\nlabel count;\nlabel nope;").unwrap();
    let mut s = Studio::new("/apps/bad.app");
    assert_eq!(s.title(), "Studio — bad.app");
    assert_eq!(send(&mut s, &mut fs, RUN).1, []);
    let p = s.problem.as_ref().unwrap();
    assert_eq!((p.code, p.pos), (Some(codes::UNKNOWN_NAME), Some((3, 7))));
    assert!(p.line().starts_with("E0302 3:7 "), "{p:?}");
    assert_eq!(hit(&frame(&mut s), 100).sense, Sense::Click);
    send(&mut s, &mut fs, click(100));
    assert_eq!(s.ed.caret(), (2, 6));

    // Fix it at the caret and run again: saved, then opened in a window.
    for _ in 0..4 {
        send(&mut s, &mut fs, key(Key::Delete));
    }
    send(&mut s, &mut fs, text("count"));
    assert_eq!(send(&mut s, &mut fs, ctrl(Key::Enter)).1, opened("/apps/bad.app"));
    assert!(s.problem.is_none());
    let saved = fs.read("/apps/bad.app").unwrap();
    assert_eq!(saved, b"state count = 0;\nlabel count;\nlabel count;");
    assert!(frame(&mut s).iter().all(|h| h.id != WidgetId(100)));

    // Columns count chars, not bytes.
    let mut s = Studio::new("/apps/lex.app");
    send(&mut s, &mut fs, text("label 1;\nlabel \"é\" $;"));
    send(&mut s, &mut fs, RUN);
    assert!(s.problem.unwrap().line().starts_with("E0001 2:11 "));
}

#[test]
fn studio_keys_save_new_and_scroll() {
    let mut fs = fs();
    let mut s = Studio::new("/tmp/deep/x.app");
    assert!(s.wants_text_input());
    // The first event loads (a missing file starts empty) and redraws.
    assert!(send(&mut s, &mut fs, AppEvent::Tick { now_ms: 1.0 }).0);
    // A browser may echo Enter and Tab as text: one newline, two spaces.
    let typed = [text("row {"), key(Key::Enter), text("\n"), key(Key::Tab), text("\t")];
    for ev in typed.into_iter().chain([text("label 1;")]) {
        send(&mut s, &mut fs, ev);
    }
    assert_eq!(s.ed.text(), "row {\n  label 1;");
    assert!(!send(&mut s, &mut fs, key(Key::Char('s'))).0);
    send(&mut s, &mut fs, ctrl(Key::Char('s')));
    assert_eq!(fs.read("/tmp/deep/x.app"), Ok(&b"row {\n  label 1;"[..]));

    assert_eq!(send(&mut s, &mut fs, click(3)).1, opened("studio:/apps/untitled.app"));
    assert_eq!(fs.read("/apps/untitled.app"), Ok(NEW_APP.as_bytes()));
    assert_eq!(send(&mut s, &mut fs, click(3)).1, opened("studio:/apps/untitled-2.app"));

    // Each frame keeps the caret in view; the wheel scrolls whole rows and
    // takes the caret along.
    let long: Vec<String> = (0..200).map(|i| format!("label {i};")).collect();
    let mut s = Studio::new("/apps/long.app");
    send(&mut s, &mut fs, text(&long.join("\n")));
    frame(&mut s);
    let rows = s.geo.unwrap().rows;
    assert!(rows > 10 && s.top == 200 - rows);
    s.ed.set_caret(0, 3);
    frame(&mut s);
    assert_eq!(s.top, 0);
    send(&mut s, &mut fs, wheel(17.0 * 30.5));
    assert_eq!((s.top, s.ed.caret()), (30, (30, 3)));
    send(&mut s, &mut fs, wheel(8.5));
    assert_eq!(s.top, 31);
    send(&mut s, &mut fs, wheel(-1e6));
    assert_eq!((s.top, s.ed.caret().0), (0, rows - 1));
    assert!(!send(&mut s, &mut fs, wheel(f32::NAN)).0);
    send(&mut s, &mut fs, key(Key::PageDown));
    frame(&mut s);
    assert_eq!((s.ed.caret().0, s.top), (2 * rows - 1, rows));
}

#[test]
fn host_counter_clicks_change_the_label() {
    let mut fs = fs();
    let mut h = AppHost::new(DEFAULT_FILE);
    assert_eq!(h.title(), "counter.app");
    assert!(send(&mut h, &mut fs, AppEvent::Resized { w: 640.0, h: 480.0 }).0);
    assert_eq!(labels(&h.nodes), ["Counter", "0"]);
    for (id, count) in [(1, "1"), (1, "2"), (0, "1")] {
        assert!(send(&mut h, &mut fs, click(id)).0);
        assert_eq!(labels(&h.nodes), ["Counter", count]);
    }
    let (list, hits) = themed(&mut h, 640.0, &THEMES[0]);
    let (minus, plus) = (hit(&hits, 0), hit(&hits, 1));
    assert!(minus.rect.y == plus.rect.y && plus.rect.x > minus.rect.x);
    // The count between the buttons sits on their middle line, as their
    // own labels do.
    let (left, right) = (minus.rect.x + minus.rect.w, plus.rect.x);
    let between = |i: &&Instance| (left..right).contains(&i.rect[0]) && i.rect[1] > minus.rect.y;
    let count = list.instances().iter().find(|i| i.kind == 4.0 && between(i));
    let [_, y, _, gh] = count.expect("the count").rect;
    let mid = minus.rect.y + minus.rect.h / 2.0;
    assert!((y + gh / 2.0 - mid).abs() <= 1.0, "{} {mid}", y + gh / 2.0);
    assert!(hit(&hits, 2).rect.y > plus.rect.y);
    assert!(h.fault.is_none() && !h.wants_text_input());
}

#[test]
fn host_input_round_trip() {
    let mut fs = fs();
    let mut h = host(&fs, "/apps/greeter.app");
    let field = hit(&frame(&mut h), INPUT);
    assert_eq!(field.sense, Sense::Text);
    assert!(!send(&mut h, &mut fs, text("x")).0);
    assert!(send(&mut h, &mut fs, press(field, 4.0, 4.0)).0);
    assert!(h.wants_text_input());
    send(&mut h, &mut fs, text("Ada\n"));
    assert_eq!(labels(&h.nodes)[1], "Hello, Ada!");
    send(&mut h, &mut fs, key(Key::Backspace));
    assert_eq!(labels(&h.nodes)[1], "Hello, Ad!");
    // The button that appeared keeps applang's id; pressing it drops focus.
    let wave = hit(&frame(&mut h), 0);
    send(&mut h, &mut fs, press(wave, 4.0, 4.0));
    send(&mut h, &mut fs, click(0));
    assert_eq!(labels(&h.nodes)[2], "You waved 1 times.");
    assert!(!h.wants_text_input());
    assert!(!send(&mut h, &mut fs, key(Key::Backspace)).0);
    assert_eq!(labels(&h.nodes)[1], "Hello, Ad!");
}

#[test]
fn host_faults_keep_the_old_state() {
    let mut fs = fs();
    let src = "state n = 0;\nlabel \"n = \" + n;\nbutton \"inc\" { n = n + 1; }\n\
               button \"spin\" { repeat 1000000 { n = n + 1; } }";
    fs.write("/apps/spin.app", src.as_bytes()).unwrap();
    let mut h = host(&fs, "/apps/spin.app");
    send(&mut h, &mut fs, click(0));
    assert_eq!(labels(&h.nodes), ["n = 1"]);
    send(&mut h, &mut fs, click(1));
    let fault = h.fault.as_ref().expect("a fault");
    assert_eq!(fault.code, Some(codes::FUEL_EXHAUSTED));
    assert!(fault.line().starts_with("E0206 4:"), "{fault:?}");
    assert_eq!(labels(&h.nodes), ["n = 1"]);
    frame(&mut h);
    // A clean event clears the status line.
    send(&mut h, &mut fs, click(0));
    assert_eq!(labels(&h.nodes), ["n = 2"]);
    assert!(h.fault.is_none());
}

#[test]
fn open_routes_names_and_shows_load_problems() {
    let mut fs = fs();
    fs.write("/apps/typo.app", b"state x = 1;\nlabel x + true;").unwrap();
    let titles = [
        ("studio", "Studio — counter.app"),
        ("studio:/apps/greeter.app", "Studio — greeter.app"),
        ("studio:clicker.app", "Studio — clicker.app"),
        ("/apps/clicker.app", "clicker.app"),
        ("studio.app", "studio.app"),
    ];
    for (name, title) in titles {
        let app = open(name).unwrap();
        assert_eq!(app.title(), title, "{name}");
        let icon = if title.starts_with("Studio") { STUDIO_ICON } else { APP_ICON };
        assert_eq!(app.icon(), icon, "{name}");
    }
    for name in ["terminal", "studio:", "studio-x", ".app/x", ""] {
        assert!(open(name).is_none(), "{name}");
    }
    assert_eq!(studio(&fs, "/apps/clicker.app").ed.text(), CLICKER);

    let mut h = AppHost::new("/apps/missing.app");
    assert!(send(&mut h, &mut fs, AppEvent::Tick { now_ms: 0.0 }).0);
    let msg = "error cannot read /apps/missing.app: no such file or directory";
    assert_eq!(broken(&h), msg);
    let mut h = host(&fs, "/apps/typo.app");
    assert!(broken(&h).starts_with("E0303 2:"));
    assert!(h.nodes.is_empty() && h.snippet.contains('^'));
    let edit = hit(&frame(&mut h), INPUT - 1).id;
    assert_eq!(send(&mut h, &mut fs, AppEvent::Click(edit)).1, opened("studio:/apps/typo.app"));
}

#[test]
fn paint_colors_chars_by_token() {
    use crate::studio::{Tok, paint};
    let line = r#"if labels == "é\"" { label 12; } // é"#;
    let mut toks = vec![Tok::Plain; 99];
    paint(line, &mut toks);
    assert_eq!(toks.len(), line.len());
    let names = [(Tok::Keyword, 'k'), (Tok::Str, 's'), (Tok::Number, 'n'), (Tok::Comment, 'd')];
    let name = |c| names.iter().find(|(k, _)| *k == c).map_or('.', |n| n.1);
    let got: String = line.char_indices().map(|(at, _)| name(toks[at])).collect();
    assert_eq!(got, "kk...........sssss...kkkkk.nn....dddd");
    assert_eq!(Problem::new(&lang::Diag::new_code(7, "x"), "").line(), "E0007 x");
    // In every theme: keywords in the accent, strings in its green, comments
    // faint, the current line raised, its number dim and the others faint.
    let mut fs = fs();
    fs.write("/apps/p.app", b"label \"hi\"; // c\nlabel 2;").unwrap();
    for theme in &THEMES {
        let (list, _) = themed(&mut studio(&fs, "/apps/p.app"), 640.0, theme);
        let ink = |c| list.instances().iter().filter(|i| i.kind == 4.0 && i.color == c).count();
        assert_eq!(ink(theme.accent), "labellabel".len(), "{}", theme.name);
        assert_eq!((ink(theme.ansi[2]), ink(theme.ansi[3])), (4, 1), "{}", theme.name);
        // Faint: "// c" and the 2; dim: the 1 and the note, "/apps/p.app".
        assert_eq!((ink(theme.text_faint), ink(theme.text_dim)), (4, 12), "{}", theme.name);
        // Raised: the Save and New buttons, and the current line's row.
        let lit = |i: &&Instance| i.kind == 0.0 && i.color == theme.surface_hi;
        let rows: Vec<f32> = list.instances().iter().filter(lit).map(|i| i.rect[3]).collect();
        assert_eq!(rows, [BUTTON_H, BUTTON_H, 17.0]);
    }
}

#[test]
fn studio_toolbar_note_stays_on_one_line() {
    // A long note (a dirty file that did not compile) is cut to one line
    // beside the buttons, or under them if fewer than 80 px are left there.
    let mut fs = fs();
    let mut s = Studio::new("/apps/a-rather-long-name-for-one-toolbar.app");
    send(&mut s, &mut fs, text("label nope;"));
    send(&mut s, &mut fs, RUN);
    let new = hit(&frame(&mut s), 3).rect;
    let room = |w: f32| w - PAD - (new.x + new.w + SPACING - ORIGIN.0);
    let small = THEMES[0].small();
    let (mut ts, row) = (text_system(), PAD + BUTTON_H + SPACING);
    let under = ts.line_height(small) + SPACING;
    for w in [640.0, 360.0, 300.0, 280.0, 200.0, 120.0] {
        let want = row + [under, 0.0][usize::from(room(w) >= 80.0)];
        assert_eq!(hit(&themed(&mut s, w, &THEMES[0]).1, 4).rect.y - ORIGIN.1, want, "{w}");
    }
    assert!(room(640.0) > 80.0 && room(200.0) < 80.0);
    let note = "x.app (modified) · did not compile";
    let mut fit = |room| ts.ellipsize(note, small, room);
    assert_eq!((fit(999.0), fit(2.0)), (note.to_string(), String::new()));
    let cut = fit(50.0);
    assert!(cut.ends_with('…') && note.starts_with(cut.trim_end_matches('…')));
    assert!(ts.measure(&cut, small) <= 50.0, "{cut}");
}

#[test]
fn host_cuts_renders_too_big_to_draw() {
    // applang bounds each string, not their sum: 50 labels of one 4,000-byte
    // state would be 200 KB of text to lay out every frame. The host keeps
    // what fits (32 KiB of text, 2,048 widgets) in render order, and says so.
    let mut fs = fs();
    let (s, row) = ("x ".repeat(2000), "row { label \"a\"; label s; label s; }");
    let src = ["state s = \"", &s, "\"; button \"clear\" { s = \"\"; }"].concat();
    let src = [src, "label s;".repeat(7), row.into(), "label s;".repeat(40)].concat();
    fs.write("/apps/big.app", src.as_bytes()).unwrap();
    let mut h = host(&fs, "/apps/big.app");
    assert_eq!(h.fault.as_ref().unwrap().line(), ["error ", crate::host::TOO_BIG].concat());
    // The button and seven labels (28,005 bytes), then the row cut after its
    // first long label (32,006): one more would pass 32,768.
    assert_eq!(h.nodes.len(), 9);
    assert!(matches!(&h.nodes[8], Node::Row { children } if children.len() == 2));
    assert_eq!(labels(&h.nodes).concat().len(), 8 * 4000 + 1);
    assert_eq!(hit(&frame(&mut h), 0).sense, Sense::Click);
    // Clearing the state makes everything fit again.
    send(&mut h, &mut fs, click(0));
    assert!(h.fault.is_none() && labels(&h.nodes).len() == 50);
    for (n, cut) in [(2100, true), (2048, false)] {
        fs.write("/apps/many.app", "label 1;".repeat(n).as_bytes()).unwrap();
        let h = host(&fs, "/apps/many.app");
        assert_eq!((h.nodes.len(), h.fault.is_some()), (2048, cut));
    }
}

#[test]
fn every_view_sits_on_device_pixels_in_every_theme() {
    // Studio (clean, and with a problem under the pointer) and AppHost
    // (running, faulted, and unable to run), at three pixel ratios: every
    // fill, border and gradient lands on device pixels, and nothing is
    // written in a color the theme does not have.
    let mut fs = fs();
    fs.write("/apps/bad.app", b"state n = 0;\nlabel nope;").unwrap();
    let spin = b"state n = 0;\nbutton \"spin\" { repeat 1000000 { n = n + 1; } }";
    fs.write("/apps/spin.app", spin).unwrap();
    let resized = AppEvent::Resized { w: 640.0, h: 420.0 };
    for dpr in [1.0, 1.5, 2.0] {
        for theme in &THEMES {
            let views = [
                ("studio", resized.clone(), 2),
                ("studio:/apps/bad.app", RUN, 100),
                ("/apps/counter.app", resized.clone(), 1),
                ("/apps/spin.app", click(0), 0),
                ("/apps/bad.app", resized.clone(), INPUT - 1),
            ];
            for (name, ev, hover) in views {
                let mut app = open(name).unwrap();
                send(app.as_mut(), &mut fs, ev);
                let (mut ts, mut list, mut hits) = (text_system(), DrawList::new(), Vec::new());
                ts.set_dpr(dpr);
                let id = Some(WidgetId(hover));
                let state = UiState { hover: id, pressed: id, focused: true, now_ms: 0.0 };
                let rect = RectF::new(0.0, 36.0, 640.0, 420.0);
                app.draw(&mut Ui::new(&mut list, &mut ts, rect, &mut hits, state, theme));
                assert!(hits.iter().any(|h| h.id == WidgetId(hover)), "{name}");
                let on = |v: f32| ((v * dpr) - (v * dpr).round()).abs() < 1e-3;
                for i in list.instances().iter().filter(|i| [0.0, 1.0, 5.0].contains(&i.kind)) {
                    let [x, y, w, h] = i.rect;
                    let ok = [x, y, x + w, y + h].iter().all(|&v| on(v));
                    assert!(ok, "{name} in {} at {dpr}: {i:?}", theme.name);
                }
                let white = Rgba(255, 255, 255, 255);
                let plain = ui::theme::mix(theme.text, theme.text_dim, 0.2);
                let known = [theme.text, theme.text_dim, theme.text_faint, theme.accent, white]
                    .into_iter()
                    .chain([theme.ansi[2], theme.ansi[3], theme.danger, theme.accent_text, plain]);
                let known: Vec<Rgba> = known.collect();
                for i in list.instances().iter().filter(|i| i.kind == 4.0) {
                    assert!(known.contains(&i.color), "{name}: {:?}", i.color);
                }
            }
        }
    }
}
