use super::*;

fn app(src: &str) -> App {
    let mut a = App::new(compile(src).unwrap(), Limits::default(), 7);
    a.render().unwrap();
    a
}

fn ev(a: &mut App, e: Event) -> Option<u16> {
    let r = a.handle(&e).err().and_then(|e| e.code);
    a.render().unwrap();
    r
}

fn click(a: &mut App, id: u32) -> Option<u16> {
    ev(a, Event::Click { id })
}

fn inp(state: &str, text: &str) -> Event {
    Event::Input { state: state.to_string(), text: text.to_string() }
}

fn vals(a: &App) -> Vec<Value> {
    a.state().map(|(_, v)| v.clone()).collect()
}

fn texts(nodes: &[Node]) -> Vec<String> {
    let each = |n: &Node| match n {
        Node::Label { text } | Node::Button { text, .. } => vec![text.clone()],
        Node::Row { children } | Node::Col { children } => texts(children),
        _ => Vec::new(),
    };
    nodes.iter().flat_map(each).collect()
}

fn shown(a: &mut App) -> Vec<String> {
    texts(&a.render().unwrap())
}

#[test]
fn the_counter_demo_end_to_end() {
    let mut a = app("state count = 0;
         row {
           button \"-\" { count = count - 1; }
           label count;
           button \"+\" { count += 1; }
         }
         if count >= 3 { label \"high\"; } else { label \"low\"; }");
    assert_eq!(shown(&mut a), ["-", "0", "+", "low"]);
    for id in [1, 1, 1, 0, 1] {
        assert_eq!(click(&mut a, id), None);
    }
    assert_eq!(shown(&mut a), ["-", "3", "+", "high"]);
}

#[test]
fn expressions_short_circuit_and_check_arithmetic() {
    let mut a = app("state n = 0; state s = \"\";
         button \"b\" { let t = n; if false && 1 / n == 0 || true { s = \"a\" + t + true; } }
         button \"neg\" { n = -(0 - 9223372036854775807 - 1); }
         button \"rep\" { repeat n - 1 { } }
         button \"mod\" { n = 7 % n; }
         button \"abs\" { n = abs(0 - 9223372036854775807 - 1); }");
    assert_eq!(click(&mut a, 0), None);
    assert_eq!(vals(&a), [Value::Int(0), Value::Str("a0true".into())]);
    for (id, code) in [
        (1, codes::OVERFLOW),
        (2, codes::NEGATIVE_REPEAT),
        (3, codes::DIV_BY_ZERO),
        (4, codes::OVERFLOW),
    ] {
        assert_eq!(click(&mut a, id), Some(code));
    }
}

#[test]
fn input_binds_two_ways_and_clips_hostile_text() {
    let mut a = app("state name = \"\"; input name; label \"hi \" + name;");
    a.handle(&inp("name", "Ada")).unwrap();
    let nodes = a.render().unwrap();
    assert_eq!(nodes[0], Node::Input { state: "name".to_string(), value: "Ada".to_string() });
    assert_eq!(texts(&nodes), ["hi Ada"]);
    // 5000 multi-byte chars clip at the cap, never mid-char.
    a.handle(&inp("name", &"é".repeat(5000))).unwrap();
    let [Value::Str(s)] = &vals(&a)[..] else { panic!("state gone") };
    assert!(s.len() <= Limits::default().max_str_bytes && s.chars().all(|c| c == 'é'));
}

#[test]
fn faults_roll_back_and_bad_events_are_coded() {
    let mut a = app("state x = 0; state y = 0; button \"boom\" { x = 99; y = 1 / y; }");
    // x's write happened before the fault, and was still rolled back.
    assert_eq!(click(&mut a, 0), Some(codes::DIV_BY_ZERO));
    assert_eq!(vals(&a), [Value::Int(0), Value::Int(0)]);
    let bad = [Event::Click { id: 7 }, inp("missing", ""), inp("x", "not a string state")];
    for ev in bad.into_iter().chain([Event::Tap { id: 0, cell: 0 }]) {
        assert_eq!(a.handle(&ev).unwrap_err().code, Some(codes::BAD_EVENT), "{ev:?}");
    }
    assert_eq!(vals(&a), [Value::Int(0), Value::Int(0)]);
    // A click names a handler the last render showed: one hidden since is gone.
    let mut a = app("state show = true; state n = 0;
         if show { button \"inc\" { n = n + 1; show = false; } }");
    assert_eq!((click(&mut a, 0), click(&mut a, 0)), (None, Some(codes::BAD_EVENT)));
    assert_eq!(vals(&a), [Value::Bool(false), Value::Int(1)]);
}

#[test]
fn strings_state_render_and_fuel_are_bounded() {
    // Self-concat trips the per-value cap.
    let mut a = app("state s = \"aaaa\"; button \"grow\" { repeat 60 { s = s + s; } }");
    assert_eq!(click(&mut a, 0), Some(codes::STR_TOO_LONG));
    // Many states each under the value cap trip the commit total instead.
    let states: String = (0..80).map(|i| format!("state s{i} = \"\";")).collect();
    let sets: String = (0..80).map(|i| format!("s{i} = \"{}\";", "b".repeat(3500))).collect();
    let mut a = app(&[states, "button \"fill\" {".into(), sets, "}".into()].concat());
    assert_eq!(click(&mut a, 0), Some(codes::STATE_TOO_BIG));
    assert!(a.state().all(|(_, v)| *v == Value::Str(String::new())));
    // Each label is under the string cap; 70 of them pass the render cap.
    let src = ["state s = \"", &"x".repeat(4000), "\";", &"label s;".repeat(70)].concat();
    let mut big = App::new(compile(&src).unwrap(), Limits::default(), 1);
    let e = big.render().unwrap_err();
    assert_eq!((e.code, e.span.is_some()), (Some(codes::RENDER_TOO_BIG), true));
    let fits = app(&src[..src.len() - 6 * 8]).render().unwrap();
    assert_eq!(texts(&fits).concat().len(), 64 * 4000);
    let src = "state x = 1; label x + x + x + x;";
    let lim = Limits { render_fuel: 3, ..Limits::default() };
    let mut a = App::new(compile(src).unwrap(), lim, 1);
    assert_eq!(a.render().unwrap_err().code, Some(codes::FUEL_EXHAUSTED));
    // An event has 1,000,000 steps: a hard drop's 100,000 fit, 2,000,000 do not.
    let mut a = app("state n = 0; button \"a\" { repeat 30000 { n += 1; } } \
                     button \"b\" { repeat 500000 { n += 1; } }");
    assert_eq!((click(&mut a, 0), a.steps() > 100_000), (None, false));
    a.handle(&Event::Click { id: 0 }).unwrap();
    assert!(a.steps() > 100_000, "{}", a.steps());
    assert_eq!(click(&mut a, 1), Some(codes::FUEL_EXHAUSTED));
}

#[test]
fn lists_functions_and_loops_run_and_fault_coded() {
    let mut a = app("state xs = [3, 1, 2]; state out = \"\"; state n = 0;
         fn sum() -> int { let t = 0; for i in 0..len(xs) { t += xs[i]; } return t; }
         fn sign(x: int) -> int { if x > 0 { return 1; } else if x < 0 { return -1; } else { return 0; } }
         fn first_big() -> int { for i in 0..len(xs) { if xs[i] > 2 { return i; } } return -1; }
         button \"go\" {
           push(xs, 4); insert(xs, 0, 9); remove(xs, 1); xs[0] -= 1;
           out = \"\" + sum() + \",\" + sign(-5) + \",\" + first_big() + \",\" + len(\"héllo\");
           out = out + \",\" + min(2, 5) + max(2, 5) + parse(\" 42 \", 0) + parse(\"x\", -1);
         }
         button \"oob\" { n = xs[len(xs)]; }
         button \"neg\" { xs[-1] = 0; }
         button \"full\" { xs = [0; 4096]; push(xs, 1); }
         button \"fill\" { xs = [1; n - 1]; }
         button \"rm\" { clear(xs); remove(xs, 0); }
         label sum();");
    assert_eq!(click(&mut a, 0), None);
    let want =
        [Value::List([8, 1, 2, 4].map(Value::Int).to_vec()), Value::Str("15,-1,0,5,2542-1".into())];
    assert_eq!(vals(&a)[..2], want);
    #[rustfmt::skip]
    let faults = [(1, codes::INDEX_OUT_OF_RANGE), (2, codes::INDEX_OUT_OF_RANGE),
        (3, codes::LIST_FULL), (4, codes::NEGATIVE_REPEAT), (5, codes::INDEX_OUT_OF_RANGE)];
    for (id, code) in faults {
        assert_eq!(click(&mut a, id), Some(code), "{id}");
    }
    assert_eq!(vals(&a)[..2], want, "every fault rolled back");
    // Calls nest at most 32 deep, as a chain of 40 functions finds.
    let mut chain = String::from("fn f0() -> int { return 0; }");
    for i in 1..40 {
        chain += &format!("fn f{i}() -> int {{ return f{}() + 1; }}", i - 1);
    }
    let mut deep = App::new(compile(&(chain + "label f39();")).unwrap(), Limits::default(), 1);
    assert_eq!(deep.render().unwrap_err().code, Some(codes::CALLS_TOO_DEEP));
}

#[test]
fn widget_loops_capture_their_values_and_grids_tap_by_square() {
    let mut a = app("state picks = [0; 3]; state hit = -1; state board = [0, 1, 8, 2];
         for i in 0..3 { button \"pick \" + i { picks[i] += 1; } }
         grid 2, board { hit = cell; board[cell] = 3; }
         grid 2, board, [\"a\", \"b\", \"c\", \"d\"];
         button \"break\" { board[0] = 9; }");
    let nodes = a.render().unwrap();
    assert_eq!(texts(&nodes), ["pick 0", "pick 1", "pick 2", "break"]);
    let cells = vec![0, 1, 8, 2];
    let words = ["a", "b", "c", "d"].map(String::from).to_vec();
    assert_eq!(nodes[3], Node::Grid { id: Some(3), cols: 2, cells: cells.clone(), texts: vec![] });
    assert_eq!(nodes[4], Node::Grid { id: None, cols: 2, cells, texts: words });
    assert_eq!((click(&mut a, 2), click(&mut a, 2), click(&mut a, 0)), (None, None, None));
    assert_eq!(ev(&mut a, Event::Tap { id: 3, cell: 1 }), None);
    let list = |v: [i64; 4]| Value::List(v.map(Value::Int).to_vec());
    let want = [Value::List([1, 0, 2].map(Value::Int).to_vec()), Value::Int(1), list([0, 3, 8, 2])];
    assert_eq!(vals(&a), want);
    // A square past the grid, a click on a grid and a tap on a button are bad events.
    for e in [Event::Tap { id: 3, cell: 4 }, Event::Click { id: 3 }, Event::Tap { id: 0, cell: 0 }]
    {
        assert_eq!(a.handle(&e).unwrap_err().code, Some(codes::BAD_EVENT));
    }
    // A square past 8 is a coded fault of the render, and so are bad columns and texts.
    a.handle(&Event::Click { id: 4 }).unwrap();
    assert_eq!(a.render().unwrap_err().code, Some(codes::BAD_GRID));
    for src in ["grid 0, [1];", "grid 101, [1];", "grid 1, [1], [\"a\", \"b\"];"] {
        let mut a = App::new(compile(src).unwrap(), Limits::default(), 1);
        assert_eq!(a.render().unwrap_err().code, Some(codes::BAD_GRID), "{src}");
    }
}

#[test]
fn ticks_keys_and_random_are_data_so_runs_replay() {
    let src = "state n = 0; state fast = 0; state speed = 300; state rolls = [0; 0];
         every speed { n += 1; }
         every 100 { fast += 1; push(rolls, random(6)); }
         on key \"left\" { speed = 0; }
         on key \"left\" { n += 100; }
         on key \"x\" { n = 0; speed = 1 / (fast - fast); }";
    let mut a = app(src);
    assert_eq!((a.timer(), a.keys()), (100, true));
    let run = |a: &mut App| {
        // 290 ms pass: 100 is due twice (at most once a tick), 300 not yet.
        let ticks = [100, 150, 0, 40].map(|ms| a.handle(&Event::Tick { ms }).unwrap());
        (ticks, vals(a))
    };
    let (ticks, got) = run(&mut a);
    assert_eq!(ticks, [true, true, false, false]);
    assert_eq!(got[..2], [Value::Int(0), Value::Int(2)]);
    // The same seed rolls the same; random stays below its bound.
    let (_, again) = run(&mut app(src));
    assert_eq!(got, again);
    let Value::List(rolls) = &got[3] else { panic!() };
    assert!(rolls.iter().all(|r| matches!(r, Value::Int(0..6))));
    let mut b = App::new(compile(src).unwrap(), Limits::default(), 99);
    b.render().unwrap();
    assert_ne!(run(&mut b).1, got, "another seed rolls otherwise");
    // A tick that makes 300 due; keys run every handler they name, in order; 0 pauses.
    assert!(a.handle(&Event::Tick { ms: 100 }).unwrap());
    assert_eq!(vals(&a)[0], Value::Int(1));
    assert_eq!(a.handle(&Event::Key { name: "left".into() }).ok(), Some(true));
    assert_eq!(a.handle(&Event::Key { name: "up".into() }).ok(), Some(false));
    let now = vals(&a);
    assert_eq!((&now[0], &now[2], a.timer()), (&Value::Int(101), &Value::Int(0), 100));
    assert_eq!(a.handle(&Event::Tick { ms: 5000 }).ok(), Some(true));
    assert_eq!(vals(&a)[0], Value::Int(101), "a paused every never runs");
    // A fault in a key handler rolls back the whole key.
    let e = a.handle(&Event::Key { name: "x".into() }).unwrap_err();
    assert_eq!((e.code, &vals(&a)[0]), (Some(codes::DIV_BY_ZERO), &Value::Int(101)));
    // random(0) faults and rolls the seed back with the rest.
    let mut r = app("state n = 0; button \"r\" { n = random(6); n = random(0); }");
    let seed = r.state.seed;
    assert_eq!(click(&mut r, 0), Some(codes::BAD_RANDOM));
    assert_eq!(r.state.seed, seed);
}

#[test]
fn saved_states_round_trip_and_changed_types_drop_with_a_note() {
    let src = "saved state best = 0; saved state names = [\"\"; 0]; saved state on = [true; 0];
         state other = 5; button \"b\" { best = -12; push(names, \"a \\\"q\\\"\\n\"); }";
    let mut a = app(src);
    assert_eq!(a.saved(), "best = 0;\nnames = [\"\"; 0];\non = [false; 0];\n");
    click(&mut a, 0);
    let saved = a.saved();
    assert_eq!(saved, "best = -12;\nnames = [\"a \\\"q\\\"\\n\"];\non = [false; 0];\n");
    let mut b = app(src);
    assert!(b.restore(&saved).is_empty());
    assert_eq!(vals(&b)[..2], vals(&a)[..2]);
    // A state no longer saved is skipped; a changed type is dropped with a note.
    let mut c = app("saved state best = \"\"; state names = [\"\"; 0];");
    let notes = c.restore(&saved);
    assert_eq!(notes, ["dropped the saved `best`: it is string now"]);
    assert_eq!(vals(&c), [Value::Str(String::new()), Value::List(Vec::new())]);
    assert_eq!(c.restore("best = ").len(), 1);
}

#[test]
fn diags_render_with_carets_and_the_card_is_real() {
    let src = "state x = 1;\nlabel x + true;";
    let r = compile(src).unwrap_err().render(src);
    assert!(r.contains("E0303") && r.contains("label x + true;"), "{r}");
}

/// The card's examples, joined into one program that compiles and passes the smoke test.
#[test]
fn every_card_example_compiles_and_smokes_clean() {
    let lines = [
        "state n = 0;   state name = \"\";   state on = false;",
        "state board = [0; 200];   state words = [\"a\", \"b\"];   state todos = [\"\"; 0];",
        "saved state best = 0;",
        "fn at(x: int, y: int) -> int { return y * 10 + x; }",
        "fn reset() { score = 0; }",
        "fn mark(on: bool) -> string { if on { return \"x\"; } return \"\"; }",
        "on key \"left\" {",
        "for i in 0..len(todos) {",
        "grid 10, board;",
        "grid 10, board {",
        "grid 2, [0, 0], words {",
        "\"score \" + n",
    ];
    let src = r#"
        state n = 0;   state name = "";   state on = false;
        state board = [0; 200];   state words = ["a", "b"];   state todos = [""; 0];
        saved state best = 0;
        state speed = 500; state score = 0; state xs = [0; 0];
        fn at(x: int, y: int) -> int { return y * 10 + x; }
        fn reset() { score = 0; }
        fn mark(on: bool) -> string { if on { return "x"; } return ""; }
        every 500 { n += 1; board[random(200)] = random(9); }
        every speed { score += 1; if len(todos) < 9 { push(todos, "t" + score); } }
        on key "left" { reset(); let i = 1; xs = [0; 2]; xs[i] = at(1, 2); push(xs, 1);
          insert(xs, 0, 2); remove(xs, 1); clear(xs); for j in 0..2 { n += j; } repeat 2 { }
          if n > 5 { return; } best = max(best, n); }
        label "score " + n;
        button mark(on) { on = !on; }
        input name;
        row { col { label min(1, 2) + max(1, 2) + abs(-3) + parse(name, 0) + len(name); } }
        if n > 3 { label 1; } else if n > 1 { label 2; } else { label 3; }
        for i in 0..len(todos) { label todos[i]; button "x" { remove(todos, i); } }
        grid 10, board;
        grid 10, board { board[cell] = (board[cell] + 1) % 9; }
        grid 2, [0, 0], words { on = !on; }
    "#;
    for l in lines.into_iter().chain(["every 500 {", "every speed {"]) {
        assert!(REFERENCE.contains(l) && src.contains(l), "{l}");
    }
    let report = smoke(compile(src).unwrap_or_else(|d| panic!("{}", d.render(src))), 3);
    assert!(report.fault.is_none() && report.warnings.is_empty(), "{report:?}");
    assert!(report.events > TICKS);
}

#[test]
fn the_shots_and_a_model_written_tetris_smoke_clean() {
    let tetris = include_str!("../tests/tetris.app");
    for (ask, src) in SHOTS.iter().copied().chain([("tetris", tetris)]) {
        let report = smoke(compile(src).unwrap_or_else(|d| panic!("{ask}: {}", d.render(src))), 5);
        assert!(report.fault.is_none() && report.warnings.is_empty(), "{ask}: {report:?}");
        assert!(report.most.0 < Limits::default().fuel / 2, "{ask}: {:?}", report.most);
        assert!(src.starts_with("// "), "{ask}: its first comment says what it is");
    }
    // The smoke test finds a fault a click away, and says what led to it.
    let src = "state xs = [0; 0]; button \"Go\" { push(xs, 1); } every 50 { xs[1] = 2; }";
    let report = smoke(compile(src).unwrap(), 1);
    let f = report.fault.expect("the tick indexes past the list");
    assert_eq!(f.diag.code, Some(codes::INDEX_OUT_OF_RANGE));
    assert_eq!(
        (f.during.as_str(), &f.before[..]),
        ("tick 1", &["clicking \"Go\"".to_string()][..])
    );
    // An app whose ticks do nothing is flagged; one that shows nothing faults (a model's reply
    // that lost its widgets).
    let report = smoke(compile("state n = 0; every 100 { n = 0; } label \"n\";").unwrap(), 1);
    assert_eq!(report.warnings, ["300 ticks never changed what it shows"]);
    let report = smoke(compile("state n = 0; every 100 { n = 0; }").unwrap(), 1);
    assert_eq!(report.fault.map(|f| f.diag.code), Some(Some(codes::SHOWS_NOTHING)));
    // Each button of the first screen is clicked as it shows by then: Start hides itself, which
    // moves the others' ids, and the Right after it is still Right.
    let src = "state on = false; state x = 0; if !on { button \"Start\" { on = true; } }
               button \"Left\" { x -= 1; } button \"Right\" { x += 1; } label x;";
    let report = smoke(compile(src).unwrap(), 1);
    assert!(report.fault.is_none(), "{report:?}");
}

#[test]
fn each_code_has_a_rule_and_the_card_says_what_models_got_wrong() {
    // A model's one slip: a label without `;` inside `if` braces. The card shows the form.
    assert!(REFERENCE.contains("also inside the\nbraces of if, row, col and for"));
    assert!(
        REFERENCE.contains("int (64-bit, checked)") && REFERENCE.contains("let works in any block")
    );
    #[rustfmt::skip]
    let all = [1, 2, 3, 4, 5, 101, 102, 203, 204, 205, 206, 211, 212, 214, 215, 216, 217, 218, 219,
        220, 221, 301, 302, 303, 304, 305, 306, 307, 308];
    for code in all {
        assert!(rule(code).ends_with('.'), "{code}");
    }
    assert_eq!(rule(codes::BAD_EVENT), "");
}
