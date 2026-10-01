use super::*;
use applang::codes;
use gfx::{DrawList, RectF};
use ui::WidgetId;
use ui::{App, AppEvent, Cx, FontId, Hit, Key, Mods, Request, Sense, TextSystem, Ui, UiState};
use ui::{BUTTON_H, PAD, SPACING, TextStyle};

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
    let mut next_socket = 0;
    let mut cx = Cx::new(fs, 0.0, None, &mut next_socket);
    let redraw = app.event(ev, &mut cx);
    (redraw, cx.take_requests())
}

fn key(key: Key) -> AppEvent {
    let mods = Mods::default();
    AppEvent::Key { key, mods }
}

fn ctrl(key: Key) -> AppEvent {
    let base = Mods::default();
    let mods = Mods { ctrl: true, ..base };
    AppEvent::Key { key, mods }
}

fn text(t: &str) -> AppEvent {
    AppEvent::Text(t.to_string())
}

fn wheel(dy: f32) -> AppEvent {
    AppEvent::Wheel { x: 0.0, y: 0.0, dy }
}

fn opened(name: &str) -> Vec<Request> {
    let (name, floating) = (name.to_string(), false);
    vec![Request::Open { name, floating }]
}

fn press(x: f32, y: f32, id: WidgetId) -> AppEvent {
    let id = Some(id);
    AppEvent::PointerDown { x, y, id }
}

/// A press inside hit `h`, relative to the content's corner as the shell
/// sends it (hits are in the `Ui` rect's space).
fn tap(h: Hit) -> AppEvent {
    press(h.rect.x + 4.0 - ORIGIN.0, h.rect.y + 4.0 - ORIGIN.1, h.id)
}

fn text_system() -> TextSystem {
    let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
    ts.set_font(FontId::SansBold, BOLD.to_vec()).unwrap();
    ts.set_font(FontId::Mono, MONO.to_vec()).unwrap();
    ts
}

/// One focused frame of `app`, `w` x 480 at [`ORIGIN`]; returns its hits.
fn frame_w(app: &mut dyn App, w: f32) -> Vec<Hit> {
    let mut ts = text_system();
    let (mut list, mut hits) = (DrawList::new(), Vec::new());
    let base = UiState::default();
    let state = UiState { focused: true, ..base };
    let rect = RectF::new(ORIGIN.0, ORIGIN.1, w, 480.0);
    app.draw(&mut Ui::new(&mut list, &mut ts, rect, &mut hits, state));
    assert!(list.len() > 5);
    hits
}

fn frame(app: &mut dyn App) -> Vec<Hit> {
    frame_w(app, 640.0)
}

fn hit(hits: &[Hit], id: u32) -> Hit {
    let found = hits.iter().find(|h| h.id == WidgetId(id));
    *found.expect("no such hit")
}

fn labels(nodes: &[applang::Node]) -> Vec<String> {
    use applang::Node::{Col, Label, Row};
    let each = |n: &applang::Node| match n {
        Label { text } => vec![text.clone()],
        Row { children } | Col { children } => labels(children),
        _ => Vec::new(),
    };
    nodes.iter().flat_map(each).collect()
}

fn host(fs: &Vfs, path: &str) -> AppHost {
    let mut h = AppHost::new(path);
    h.load(fs);
    h
}

#[test]
fn samples_compile_and_install_once() {
    for (path, src) in SAMPLES {
        assert!(applang::compile(src).is_ok(), "{path}");
    }
    assert!(applang::compile(NEW_APP).is_ok());
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
    assert_eq!([e.line(2), e.line(3)], [" ", "  label 2;"]);
    assert_eq!(e.caret(), (3, 1));
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
    let clicks = [(3.4, 1.5, (1, 3)), (3.6, 0.3, (0, 4)), (99.0, 9.0, (1, 8))];
    for (col, row, caret) in clicks {
        e.click(8.0 * col, 17.0 * row, 8.0, 17.0);
        assert_eq!(e.caret(), caret, "{col} {row}");
    }
    e.click(-5.0, -5.0, 8.0, 17.0);
    assert_eq!(e.caret(), (0, 0));
    e.click(f32::NAN, 20.0, 0.0, 17.0);
    assert_eq!(e.caret(), (1, 0));

    // Through Studio: a press on the editor, relative to the content's
    // corner as the shell sends it, lands on the drawn grid: the text starts
    // a 4-cell gutter and a 6 px inset into the well.
    let mut fs = fs();
    let mut s = Studio::new(DEFAULT_FILE);
    let well = hit(&frame(&mut s), 4);
    assert_eq!(well.sense, Sense::Text);
    let g = s.geo.unwrap();
    assert_eq!((g.cell, g.row), (8.0, 17.0));
    let at = |well: Hit, col: f32, row: f32| {
        let x = well.rect.x - ORIGIN.0 + (4.0 + col) * 8.0;
        press(x, well.rect.y - ORIGIN.1 + 6.0 + row * 17.0, well.id)
    };
    assert!(send(&mut s, &mut fs, at(well, 3.4, 4.5)).0);
    assert_eq!(s.ed.caret(), (4, 3));
    // On an empty line the caret goes to its end; what is typed lands there.
    fs.write("/apps/t.app", b"one\ntwo\n\nfour").unwrap();
    let mut s = Studio::new("/apps/t.app");
    s.load(&fs);
    let well = hit(&frame(&mut s), 4);
    for ev in [at(well, 3.4, 2.5), text("X"), ctrl(Key::Char('s'))] {
        send(&mut s, &mut fs, ev);
    }
    assert_eq!(fs.read("/apps/t.app"), Ok(&b"one\ntwo\nX\nfour"[..]));
}

#[test]
fn failed_run_lists_problems_and_clicks_jump() {
    let mut fs = fs();
    let bad = b"state count = 0;\nlabel count;\nlabel nope;";
    fs.write("/apps/bad.app", bad).unwrap();
    let mut s = Studio::new("/apps/bad.app");
    assert_eq!(s.title(), "Studio — bad.app");
    assert_eq!(send(&mut s, &mut fs, RUN).1, []);
    let p = &s.problems[0];
    assert_eq!((p.code, p.pos), (Some(codes::UNKNOWN_NAME), Some((3, 7))));
    assert!(p.to_string().starts_with("E0302 3:7 "), "{p}");
    assert_eq!(hit(&frame(&mut s), 100).sense, Sense::Click);
    send(&mut s, &mut fs, AppEvent::Click(WidgetId(100)));
    assert_eq!(s.ed.caret(), (2, 6));

    // Fix it at the caret and run again: saved, then opened in a window.
    for _ in 0..4 {
        send(&mut s, &mut fs, key(Key::Delete));
    }
    send(&mut s, &mut fs, text("count"));
    let requests = send(&mut s, &mut fs, ctrl(Key::Enter)).1;
    assert_eq!(requests, opened("/apps/bad.app"));
    assert!(s.problems.is_empty());
    let saved = fs.read("/apps/bad.app").unwrap();
    assert_eq!(saved, b"state count = 0;\nlabel count;\nlabel count;");
    assert!(frame(&mut s).iter().all(|h| h.id != WidgetId(100)));

    // Columns count chars, not bytes.
    let mut s = Studio::new("/apps/lex.app");
    send(&mut s, &mut fs, text("label 1;\nlabel \"é\" $;"));
    send(&mut s, &mut fs, RUN);
    assert!(s.problems[0].to_string().starts_with("E0001 2:11 "));
}

#[test]
fn studio_keys_save_new_and_scroll() {
    let mut fs = fs();
    let mut s = Studio::new("/tmp/deep/x.app");
    assert!(s.wants_text_input());
    // The first event loads (a missing file starts empty) and redraws.
    assert!(send(&mut s, &mut fs, AppEvent::Tick { now_ms: 1.0 }).0);
    // A browser may echo Enter and Tab as text: one newline, two spaces.
    let typed = [text("row {"), key(Key::Enter), text("\n")];
    for ev in typed.into_iter().chain([key(Key::Tab), text("\t")]) {
        send(&mut s, &mut fs, ev);
    }
    send(&mut s, &mut fs, text("label 1;"));
    assert_eq!(s.ed.text(), "row {\n  label 1;");
    assert!(!send(&mut s, &mut fs, key(Key::Char('s'))).0);
    send(&mut s, &mut fs, ctrl(Key::Char('s')));
    assert_eq!(fs.read("/tmp/deep/x.app"), Ok(&b"row {\n  label 1;"[..]));

    let new = AppEvent::Click(WidgetId(3));
    let requests = send(&mut s, &mut fs, new.clone()).1;
    assert_eq!(requests, opened("studio:/apps/untitled.app"));
    assert_eq!(fs.read("/apps/untitled.app"), Ok(NEW_APP.as_bytes()));
    let requests = send(&mut s, &mut fs, new).1;
    assert_eq!(requests, opened("studio:/apps/untitled-2.app"));

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
    for _ in 0..2 {
        assert!(send(&mut h, &mut fs, AppEvent::Click(WidgetId(1))).0);
    }
    assert_eq!(labels(&h.nodes), ["Counter", "2"]);
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(0)));
    assert_eq!(labels(&h.nodes), ["Counter", "1"]);
    let hits = frame(&mut h);
    let (minus, plus) = (hit(&hits, 0), hit(&hits, 1));
    assert!(minus.rect.y == plus.rect.y && plus.rect.x > minus.rect.x);
    assert!(hit(&hits, 2).rect.y > plus.rect.y);
    assert!(h.fault.as_ref().is_none() && !h.wants_text_input());
}

#[test]
fn host_input_round_trip() {
    let mut fs = fs();
    let mut h = host(&fs, "/apps/greeter.app");
    let field = hit(&frame(&mut h), INPUT);
    assert_eq!(field.sense, Sense::Text);
    assert!(!send(&mut h, &mut fs, text("x")).0);
    assert!(send(&mut h, &mut fs, tap(field)).0);
    assert!(h.wants_text_input());
    send(&mut h, &mut fs, text("Ada\n"));
    assert_eq!(labels(&h.nodes)[1], "Hello, Ada!");
    send(&mut h, &mut fs, key(Key::Backspace));
    assert_eq!(labels(&h.nodes)[1], "Hello, Ad!");
    // The button that appeared keeps applang's id; pressing it drops focus.
    let wave = hit(&frame(&mut h), 0);
    send(&mut h, &mut fs, tap(wave));
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(0)));
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
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(0)));
    assert_eq!(labels(&h.nodes), ["n = 1"]);
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(1)));
    let fault = h.fault.as_ref().expect("a fault").clone();
    assert_eq!(fault.code, Some(codes::FUEL_EXHAUSTED));
    assert!(fault.to_string().starts_with("E0206 4:"), "{fault}");
    assert_eq!(labels(&h.nodes), ["n = 1"]);
    frame(&mut h);
    // A clean event clears the status line.
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(0)));
    assert_eq!(labels(&h.nodes), ["n = 2"]);
    assert!(h.fault.is_none());
}

#[test]
fn open_routes_names_and_shows_load_problems() {
    let mut fs = fs();
    let typo = b"state x = 1;\nlabel x + true;";
    fs.write("/apps/typo.app", typo).unwrap();
    let titles = [
        ("studio", "Studio — counter.app"),
        ("studio:/apps/greeter.app", "Studio — greeter.app"),
        ("studio:clicker.app", "Studio — clicker.app"),
        ("/apps/clicker.app", "clicker.app"),
        ("studio.app", "studio.app"),
    ];
    for (name, title) in titles {
        assert_eq!(open(name).unwrap().title(), title, "{name}");
        assert_eq!(open_in(name, &fs).unwrap().title(), title, "{name}");
    }
    for name in ["terminal", "studio:", "studio-x", ".app/x", ""] {
        assert!(open(name).is_none(), "{name}");
    }
    let mut s = Studio::new("/apps/clicker.app");
    s.load(&fs);
    assert_eq!(s.ed.text(), CLICKER);

    let mut h = AppHost::new("/apps/missing.app");
    assert!(send(&mut h, &mut fs, AppEvent::Tick { now_ms: 0.0 }).0);
    let msg = "error cannot read /apps/missing.app: no such file or directory";
    assert_eq!(h.problems[0].to_string(), msg);
    let mut h = host(&fs, "/apps/typo.app");
    assert!(h.problems[0].to_string().starts_with("E0303 2:"));
    assert!(h.nodes.is_empty() && h.snippet.contains('^'));
    let edit = hit(&frame(&mut h), INPUT - 1).id;
    let requests = send(&mut h, &mut fs, AppEvent::Click(edit)).1;
    assert_eq!(requests, opened("studio:/apps/typo.app"));
}

#[test]
fn paint_colors_chars_by_token() {
    use ui::theme::{ANSI, TEXT, TEXT_DIM};
    let line = r#"if labels == "é\"" { label 12; } // é"#;
    let mut colors = vec![TEXT; 99];
    crate::studio::paint(line, &mut colors);
    assert_eq!(colors.len(), line.len());
    let names = [(ANSI[5], 'k'), (ANSI[2], 's'), (ANSI[3], 'n'), (TEXT_DIM, 'd')];
    let name = |c| names.iter().find(|(k, _)| *k == c).map_or('.', |n| n.1);
    let got: String = line.char_indices().map(|(at, _)| name(colors[at])).collect();
    assert_eq!(got, "kk...........sssss...kkkkk.nn....dddd");
    let p = Problem::new(&lang::Diag::new_code(7, "x"), "");
    assert_eq!(p.to_string(), "E0007 x");
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
    let small = TextStyle::new(FontId::Sans, 12.0, ui::theme::TEXT_DIM);
    let (mut ts, row) = (text_system(), PAD + BUTTON_H + SPACING);
    let under = ts.line_height(small) + SPACING;
    for w in [640.0, 360.0, 300.0, 280.0, 200.0, 120.0] {
        let want = row + [under, 0.0][usize::from(room(w) >= 80.0)];
        assert_eq!(hit(&frame_w(&mut s, w), 4).rect.y - ORIGIN.1, want, "{w}");
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
    let fault = h.fault.as_ref().map(Problem::to_string).unwrap();
    assert_eq!(fault, ["error ", crate::host::TOO_BIG].concat());
    // The button and seven labels (28,005 bytes), then the row cut after its
    // first long label (32,006): one more would pass 32,768.
    assert_eq!(h.nodes.len(), 9);
    assert!(matches!(&h.nodes[8], applang::Node::Row { children } if children.len() == 2));
    assert_eq!(labels(&h.nodes).concat().len(), 8 * 4000 + 1);
    assert_eq!(hit(&frame(&mut h), 0).sense, Sense::Click);
    // Clearing the state makes everything fit again.
    send(&mut h, &mut fs, AppEvent::Click(WidgetId(0)));
    assert!(h.fault.is_none() && labels(&h.nodes).len() == 50);
    for (n, cut) in [(2100, true), (2048, false)] {
        fs.write("/apps/many.app", "label 1;".repeat(n).as_bytes()).unwrap();
        let h = host(&fs, "/apps/many.app");
        assert_eq!((h.nodes.len(), h.fault.is_some()), (2048, cut));
    }
}
