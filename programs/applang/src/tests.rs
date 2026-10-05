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

fn ints(v: &[i64]) -> Value {
    Value::List(v.iter().copied().map(Value::Int).collect())
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
fn plus_equals_runs_its_index_once_and_fuel_pays_for_bytes() {
    // `counts[random(6)] += 1` reads and writes one square: the counts add up to the rolls.
    let mut a = app("state counts = [0; 6]; state rolls = 0;
         button \"roll\" { counts[random(6)] += 1; rolls += 1; }");
    (0..300).for_each(|_| assert_eq!(click(&mut a, 0), None));
    let Value::List(counts) = &vals(&a)[0] else { panic!() };
    let sum: i64 = counts.iter().map(|v| if let Value::Int(n) = v { *n } else { 0 }).sum();
    assert_eq!((sum, &vals(&a)[1]), (300, &Value::Int(300)));
    // An index that changes state runs once; the item is read before the value runs (as
    // `x += v` reads x first), and a value that empties the list faults, coded.
    let mut a = app("state xs = [0; 4]; state k = 0;
         fn next() -> int { k += 1; return k % 4; }
         fn set() -> int { xs[0] = 100; return 1; }
         fn empty() -> int { clear(xs); return 1; }
         button \"b\" { xs[next()] += 10; xs[next()] -= 1; xs[0] += set(); }
         button \"c\" { xs[0] += empty(); }");
    assert_eq!((click(&mut a, 0), vals(&a)), (None, vec![ints(&[1, 10, -1, 0]), Value::Int(2)]));
    assert_eq!(click(&mut a, 1), Some(codes::INDEX_OUT_OF_RANGE));
    // A copy costs a step per item and per 64 bytes: 4,096 copies of 4 KiB (16 MiB) cost
    // 266,240 steps, so an event makes at most 64 MB and a render 12.8 MB, faulting coded.
    let t = "let t = \"xxxxxxxxxxxxxxxx\"; repeat 8 { t = t + t; }";
    let mut a = app(&format!("state n = 0; button \"b\" {{ {t} let a = [t; 4096]; n = len(a); }}"));
    a.handle(&Event::Click { id: 0 }).unwrap();
    assert!(a.steps() > 4096 * 65, "{}", a.steps());
    let src = format!("button \"b\" {{ {t} repeat 4 {{ let a = [t; 4096]; }} }}");
    assert_eq!(click(&mut app(&src), 0), Some(codes::FUEL_EXHAUSTED));
    let src = format!("fn big() -> int {{ {t} let a = [t; 4096]; return 0; }} label big();");
    let mut a = App::new(compile(&src).unwrap(), Limits::default(), 1);
    assert_eq!(a.render().unwrap_err().code, Some(codes::FUEL_EXHAUSTED));
    // Text built a char at a time still fits in an event: 4,000 appends.
    let mut a = app("state s = \"\"; button \"b\" { repeat 4000 { s += \"x\"; } }");
    a.handle(&Event::Click { id: 0 }).unwrap();
    assert!(a.steps() < 400_000, "{}", a.steps());
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
fn a_changed_app_takes_back_only_saved_states_it_can_show() {
    // The board never changes length, so one saved by an older, smaller app is dropped; a list
    // that grows comes back at any length; a bad line loses only itself; the least int reads
    // back.
    let mut a = app("saved state board = [0; 9]; saved state words = [\"\"; 2];
         saved state low = 0; saved state best = 0;
         button \"add\" { push(words, \"w\"); } grid 3, board { board[cell] = 1; }");
    let notes = a.restore(
        "board = [1, 0, 0, 1];\nwords = [\"a\", \"b\", \"c\"];\nbest = 99999999999999999999;\n\
         low = -9223372036854775808;\n",
    );
    let bad = "dropped saved line 3: it did not read (E0003 integer literal out of range (max \
               9223372036854775807))";
    assert_eq!(notes, ["dropped the saved `board`: it holds 9 items now", bad]);
    let words = Value::List(["a", "b", "c"].map(|w| Value::Str(w.into())).to_vec());
    let want = [ints(&[0; 9]), words, Value::Int(i64::MIN), Value::Int(0)];
    assert_eq!(vals(&a), want);
    let mut b = app("saved state low = 0; label low;");
    assert!(b.restore(&a.saved()).is_empty() && vals(&b) == [Value::Int(i64::MIN)]);
    // A saved list this app cannot show starts it afresh, with a note; one it cannot show
    // afresh either is kept.
    let mut a = app("saved state top = [0; 5]; button \"x\" { push(top, 1); }
         for i in 0..5 { label top[i]; }");
    let notes = a.restore("top = [7, 7];");
    let note = "the saved state faults when it shows (E0215 index 2 is outside `top`, which has 2 \
                items); it starts afresh";
    assert_eq!((notes, vals(&a)), (vec![note.to_string()], vec![ints(&[0; 5])]));
    let src = "saved state xs = [0; 0]; button \"x\" { push(xs, 1); } label xs[1];";
    let mut b = App::new(compile(src).unwrap(), Limits::default(), 1);
    assert!(b.restore("xs = [4];").is_empty() && vals(&b) == [ints(&[4])]);
}

#[test]
fn ticks_roll_back_whole_and_smoke_runs_slow_timers_and_reopens() {
    // A faulted tick rolls back how far each `every` is too: its beat comes late, never lost.
    let mut a = app("state a = 0; state f = false; every 1000 { a += 1; }
         every 1000 { if f { a = 1 / 0; } } button \"break\" { f = true; } button \"fix\" { f = false; }");
    let tick = |a: &mut App| a.handle(&Event::Tick { ms: 500 }).map_err(|e| e.code);
    assert_eq!((tick(&mut a), click(&mut a, 0)), (Ok(false), None));
    assert_eq!((tick(&mut a), click(&mut a, 1)), (Err(Some(codes::DIV_BY_ZERO)), None));
    assert_eq!((tick(&mut a), vals(&a)[0].clone()), (Ok(true), Value::Int(1)));
    // A level-up every 20 s runs in a smoke test of a 50 ms game, and faults at level 4.
    let src = "state y = 0; state level = 1; state speeds = [500, 300, 200];
         every 50 { y = (y + 1) % 20; } every 20000 { level += 1; }
         label \"speed \" + speeds[level - 1] + \" \" + y;";
    let f = smoke(compile(src).unwrap(), 1).fault.expect("level 4 has no speed");
    let want = (Some(codes::INDEX_OUT_OF_RANGE), "the render after tick 60");
    assert_eq!((f.diag.code, f.during.as_str()), want);
    // A list saved apart from the list it is read with faults once it is opened again.
    let src = "saved state habits = [\"\"; 0]; state streak = [0; 0]; state name = \"\";
         input name; button \"Add\" { if name != \"\" { push(habits, name); push(streak, 0); } }
         for i in 0..len(habits) { label habits[i] + \": \" + streak[i]; }";
    let f = smoke(compile(src).unwrap(), 1).fault.expect("streak is not saved");
    assert_eq!(f.diag.code, Some(codes::INDEX_OUT_OF_RANGE));
    assert!(f.during.starts_with("the first render after closing and opening it again"));
    let fixed = src.replacen("state streak", "saved state streak", 1);
    assert!(smoke(compile(&fixed).unwrap(), 1).fault.is_none());
}

#[test]
fn the_snake_shot_steers_by_its_states_and_rests_when_still() {
    let snake = |seed| {
        let mut a = App::new(compile(SHOTS[0].1).unwrap(), Limits::default(), seed);
        a.render().unwrap();
        a
    };
    let at = |a: &App, i: usize| match &vals(a)[i] {
        Value::Int(n) => vec![*n],
        Value::List(items) => {
            items.iter().map(|v| if let Value::Int(n) = v { *n } else { -1 }).collect()
        }
        v => panic!("{v:?}"),
    };
    let (key, tick) = (|n: &str| Event::Key { name: n.into() }, Event::Tick { ms: 150 });
    // No timer runs before Start; up then left within one step turns up, never back.
    let mut a = snake(1);
    assert_eq!(a.timer(), 0);
    assert_eq!(click(&mut a, 1), None);
    assert_eq!(a.timer(), 150);
    for e in [key("up"), key("left"), tick.clone()] {
        assert_eq!(ev(&mut a, e), None);
    }
    assert_eq!((at(&a, 0), at(&a, 1), at(&a, 8)), (vec![4, 5, 5], vec![7, 7, 6], vec![150]));
    // Its board is pixels, 20 x 15 squares of 8 units: the body green, the food red.
    let Node::Canvas { draws, .. } = &a.render().unwrap()[2] else { panic!("the canvas") };
    let count = |c| draws[0].text.chars().filter(|&ch| ch == c).count();
    assert_eq!(
        (draws[0].shape, draws[0].at, count('2'), count('1')),
        (Shape::Pixels, [0, 0, 20, 8, 0], 3, 1)
    );
    // A tap left of the head, at square (0, 6), turns it left.
    assert_eq!(ev(&mut a, Event::Tap { id: 0, cell: (6 * 8 + 3) * 160 + 3 }), None);
    assert_eq!((at(&a, 4), at(&a, 5)), (vec![-1], vec![0]));
    // Going right from (5, 7): a tap straight behind turns nothing; one farther behind than
    // above, at (0, 5), turns up.
    let mut b = snake(2);
    click(&mut b, 1);
    for (y, way) in [(7, [1, 0]), (5, [0, -1])] {
        assert_eq!(ev(&mut b, Event::Tap { id: 0, cell: (y * 8 + 3) * 160 + 3 }), None);
        assert_eq!([at(&b, 4)[0], at(&b, 5)[0]], way);
    }
    // Food never starts on the body (seeds 132, 246 and 495 once put it there).
    for seed in 1..600 {
        let mut a = snake(seed);
        click(&mut a, 1);
        let (fx, fy) = (at(&a, 6)[0], at(&a, 7)[0]);
        assert!(fy != 7 || !(3..6).contains(&fx), "seed {seed}");
    }
    // The head runs into its body even where the food is drawn over it, and the timer stops.
    a.state.vals[0] = ints(&[6, 5, 5, 4, 4]);
    a.state.vals[1] = ints(&[6, 6, 7, 7, 6]);
    a.state.vals[2..8].clone_from_slice(&[1, 0, 1, 0, 5, 6].map(Value::Int));
    assert_eq!(ev(&mut a, tick), None);
    assert_eq!((at(&a, 8), a.timer()), (vec![0], 0));
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
        "canvas 160, 120, scene();",
        "canvas 160, 120, scene() {",
        "fn scene() { rect(0, 0, 160, 8, 4); circle(bx, by, 3, 3); text(score, 80, 4, 6, 9); }",
        "sprite([\"..3..\", \".333.\", \"33333\"], ",
        "pixels(board, x, y, 10, side);",
        "let b = [-1; 200]; b[at(x, y)] = 2;",
        "sin(d)  cos(d)",
    ];
    let src = r#"
        state n = 0;   state name = "";   state on = false;
        state board = [0; 200];   state words = ["a", "b"];   state todos = [""; 0];
        saved state best = 0;
        state speed = 500; state score = 0; state xs = [0; 0]; state bx = 80; state by = 60;
        fn at(x: int, y: int) -> int { return y * 10 + x; }
        fn reset() { score = 0; }
        fn mark(on: bool) -> string { if on { return "x"; } return ""; }
        fn scene() { rect(0, 0, 160, 8, 4); circle(bx, by, 3, 3); text(score, 80, 4, 6, 9); }
        fn squares(x: int, y: int, side: int) { let b = [-1; 200]; b[at(x, y)] = 2;
          pixels(b, x, y, 10, side); pixels(board, x, y, 10, side); }
        fn more() { ring(80, 60, 20, 2, 11); line(0, 119, 159, 0, 1, 10);
          sprite(["..3..", ".333.", "33333"], 70, 90, 2); scene(); squares(0, 10, 4); }
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
        canvas 160, 120, scene();
        canvas 160, 120, scene() { bx = x; by = y; }
        canvas 160, 120, more();
        label sin(d)  cos(d);
    "#;
    for l in lines.into_iter().chain(["every 500 {", "every speed {"]) {
        assert!(REFERENCE.contains(l) && src.contains(l), "{l}");
    }
    // The card's `sin(d)  cos(d)` as an expression, d a state.
    let src = &src.replace("sin(d)  cos(d)", "sin(score) + cos(score)");
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
    // A shot's second line is its icon, which the desktop reads whole (`icons::made`).
    for (ask, src) in SHOTS {
        let icon = icons::made::header(src.as_bytes()).map(icons::Made::parse);
        let second = src.lines().nth(1).unwrap_or_default();
        assert!(second.starts_with("// icon: ") && matches!(icon, Some(Ok(_))), "{ask}: {icon:?}");
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
        220, 221, 222, 223, 224, 225, 301, 302, 303, 304, 305, 306, 307, 308, 309];
    for code in all {
        assert!(rule(code).ends_with('.'), "{code}");
    }
    assert_eq!(rule(codes::BAD_EVENT), "");
}

/// A draw as a canvas's call gives it.
fn draw(shape: Shape, color: u8, at: [i16; 5], text: &str) -> Draw {
    Draw { shape, color, at, text: text.into() }
}

#[test]
fn canvases_draw_their_shapes_in_order_and_taps_say_where() {
    let mut a = app("state hits = [0; 0]; state n = 0;
         fn dot(i: int) { circle(i * 10, 5, 2, 3); }
         fn scene() {
           rect(0, 0, 40, 2, 1); ring(20, 10, 6, 2, 4); line(0, 19, 39, 0, 1, 9);
           text(42, 20, 10, 4, 11); text(\"hi\", 1, 1, 2, 10); sprite([\"1.2\", \".3\"], 30, 15, 1);
           pixels([1, 1, -1, 11, 0, 9], 2, 3, 3, 2);
           for i in 0..n { dot(i); }
         }
         label \"x\";
         canvas 40, 20, scene() { push(hits, x); push(hits, y); n = min(3, n + 1); }
         canvas 2 * 3, 1, scene();");
    let nodes = a.render().unwrap();
    let draws = vec![
        draw(Shape::Rect, 1, [0, 0, 40, 2, 0], ""),
        draw(Shape::Ring, 4, [20, 10, 6, 2, 0], ""),
        draw(Shape::Line, 9, [0, 19, 39, 0, 1], ""),
        draw(Shape::Text, 11, [20, 10, 4, 0, 0], "42"),
        draw(Shape::Text, 10, [1, 1, 2, 0, 0], "hi"),
        draw(Shape::Sprite, 0, [30, 15, 1, 0, 0], "1.2\n.3"),
        draw(Shape::Pixels, 0, [2, 3, 3, 2, 0], "11.b09"),
    ];
    let canvas = |id, w, draws| Node::Canvas { id, w, h: if w == 40 { 20 } else { 1 }, draws };
    assert_eq!(nodes[1..], [canvas(Some(0), 40, draws.clone()), canvas(None, 6, draws.clone())]);
    assert_eq!(draws.iter().map(Draw::ink).collect::<Vec<_>>(), [1, 1, 1, 3, 3, 4, 5]);
    // A tap is the unit y * w + x: its handler sees x and y.
    assert_eq!(ev(&mut a, Event::Tap { id: 0, cell: 3 * 40 + 7 }), None);
    assert_eq!(ev(&mut a, Event::Tap { id: 0, cell: 799 }), None);
    assert_eq!(vals(&a)[0], ints(&[7, 3, 39, 19]));
    let Node::Canvas { draws, .. } = &a.render().unwrap()[1] else { panic!() };
    assert_eq!(
        draws[7..],
        [draw(Shape::Circle, 3, [0, 5, 2, 0, 0], ""), draw(Shape::Circle, 3, [10, 5, 2, 0, 0], "")]
    );
    // A unit past it, a click on it and a tap on what has no handler are bad events.
    for e in
        [Event::Tap { id: 0, cell: 800 }, Event::Click { id: 0 }, Event::Tap { id: 1, cell: 0 }]
    {
        assert_eq!(a.handle(&e).unwrap_err().code, Some(codes::BAD_EVENT), "{e:?}");
    }
    // A handler's fault rolls the tap back.
    let mut b = app(
        "state n = 0; fn s() { rect(0, 0, 1, 1, 1); } canvas 4, 4, s() { n = 1; n = n / (x - 1); }",
    );
    assert_eq!(ev(&mut b, Event::Tap { id: 0, cell: 1 }), Some(codes::DIV_BY_ZERO));
    assert_eq!(vals(&b), [Value::Int(0)]);
    // The same seed and events draw the same, so a run replays.
    let src = "state x0 = 0; state ys = [0; 0]; fn s() { for i in 0..len(ys) { rect(x0 + i, ys[i], 1, 1, 2); } }
               every 50 { x0 = random(10); push(ys, random(20)); } canvas 30, 30, s();";
    let run = |seed| {
        let mut a = App::new(compile(src).unwrap(), Limits::default(), seed);
        (0..5)
            .map(|_| (a.handle(&Event::Tick { ms: 50 }).unwrap(), a.render().unwrap()))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(3), run(3));
    assert_ne!(run(3), run(4));
}

#[test]
fn shapes_past_a_canvas_fault_coded_and_sines_are_whole() {
    let scene = |body: &str| format!("state n = 0; fn s() {{ {body} }} canvas 9, 9, s();");
    #[rustfmt::skip]
    let cases = [
        ("rect(0, 0, 1, 1, 12);", "color is 12"), ("rect(0, 0, -1, 1, 1);", "w is -1"),
        ("ring(0, 0, 3, -1, 1);", "width is -1"), ("circle(40000, 0, 1, 1);", "x is 40000"),
        ("line(0, 0, 1, 1, -2, 1);", "width is -2"), ("text(\"a\\nb\", 0, 0, 1, 1);", "one line"),
        ("sprite([\"1\", \"2\\n\"], 0, 0, 1);", "one line"), ("text(1, 0, 0, -3, 1);", "size is -3"),
        ("sprite([\"1\"], 0, -32769, 1);", "y is -32769"),
        // 4,097 ink: a sprite of 4,096 squares, and itself.
        ("sprite([\"1\"; 4096], 0, 0, 1);", "4096 ink"),
        ("for i in 0..2049 { text(\"a\", i, 0, 1, 1); }", "4096 ink"),
        ("pixels([1], 0, 0, 1, -1);", "side is -1"),
        // 4,097 ink: 4,096 runs of one square, and itself.
        ("let b = [0; 4096]; for i in 0..2048 { b[i * 2] = 1; } pixels(b, 0, 0, 64, 1);",
         "4096 ink"),
    ];
    let pixels = [
        ("pixels([1, 2], 0, 0, 0, 1);", "1 to 64 cells a row, not 0"),
        ("pixels([0; 65], 0, 0, 65, 1);", "not 65"),
        ("pixels([1, 2, 3], 0, 0, 2, 1);", "no whole rows of 2: 1 left over"),
        ("pixels([0; 65], 0, 0, 1, 1);", "are 65 rows"),
        ("pixels([0, 12], 0, 0, 2, 1);", "cell 1 is 12"),
        ("pixels([-2], 0, 0, 1, 1);", "cell 0 is -2"),
        // 16,385 cells: four whole boards, and one more.
        ("for i in 0..4 { pixels([-1; 4096], 0, 0, 64, 1); } pixels([1], 0, 0, 1, 1);", "16384"),
    ];
    let bad = cases.iter().map(|c| (c, codes::BAD_DRAW));
    for ((body, said), code) in bad.chain(pixels.iter().map(|c| (c, codes::BAD_PIXELS))) {
        let mut a = App::new(compile(&scene(body)).unwrap(), Limits::default(), 1);
        let e = a.render().unwrap_err();
        assert_eq!(e.code, Some(code), "{body}");
        assert!(e.message.contains(said) && e.span.is_some(), "{body}: {}", e.message);
    }
    for (w, h) in [(0, 9), (9, 1025), (-1, 1)] {
        let src = format!("fn s() {{ }} canvas {w}, {h}, s();");
        let mut a = App::new(compile(&src).unwrap(), Limits::default(), 1);
        assert_eq!(a.render().unwrap_err().code, Some(codes::BAD_DRAW), "{w} x {h}");
    }
    // As much ink as a render holds draws, and 0 sizes draw (nothing): never clamped, never
    // dropped. So do as many cells of pixels, and none.
    app(&scene("sprite([\"1\"; 4094], 0, 0, 1); rect(0, 0, 0, 0, 0);"));
    app(&scene("for i in 0..4 { pixels([-1; 4096], 0, 0, 64, 0); } pixels([0; 0], 0, 0, 3, 1);"));
    // A thousand times the sine and cosine of a degree, any int a degree.
    let degs = "label sin(0) + \" \" + sin(30) + \" \" + sin(90) + \" \" + sin(210) + \" \" + sin(-90) + \
                \" \" + cos(0) + \" \" + cos(180) + \" \" + cos(450) + \" \" + sin(9223372036854775807);";
    let mut a = app(degs);
    assert_eq!(shown(&mut a), ["0 500 1000 -500 -1000 1000 -1000 0 122"]);
}

#[test]
fn the_smoke_test_taps_canvases_and_finds_a_picture_drawn_off_them() {
    // Drawn in the window's pixels, not the canvas's units: nothing ever reaches inside it.
    let src = "state n = 0; fn s() { circle(500, 500, 20, 1); text(n, 300, 200, 20, 9); }
               every 100 { n += 1; } label \"go\"; canvas 100, 100, s() { n = x; }";
    let report = smoke(compile(src).unwrap(), 1);
    let f = report.fault.expect("its shapes are off the canvas");
    assert_eq!((f.diag.code, f.during.as_str()), (Some(codes::OFF_CANVAS), "the whole smoke test"));
    assert_eq!(f.diag.span.map(|s| &src[s.start..s.start + 6]), Some("canvas"));
    assert!(f.diag.message.contains("100 x 100 units"), "{}", f.diag.message);
    // A canvas blank until it is tapped is fine: the smoke test taps it.
    let src = "state xs = [0; 0]; fn s() { for i in 0..len(xs) { rect(xs[i], 0, 1, 1, 2); } }
               label \"tap\"; canvas 50, 50, s() { if len(xs) < 99 { push(xs, x); } }";
    let report = smoke(compile(src).unwrap(), 2);
    assert!(report.fault.is_none(), "{report:?}");
    // A text wider than its canvas, as near as its characters and size say, faults where the
    // render drew it; one at its edge that fits does not (the desktop keeps it inside), nor one
    // whose point is off the canvas (the desktop leaves it there: a marquee).
    let src =
        |s: &str, x| format!("fn s() {{ text(\"{s}\", {x}, 4, 10, 9); }} canvas 100, 60, s();");
    let long = "Score 12   Best 40   Lives 3";
    let f = smoke(compile(&src(long, 2)).unwrap(), 1).fault.unwrap();
    assert_eq!((f.diag.code, f.during.as_str()), (Some(codes::TEXT_TOO_WIDE), "the first render"));
    assert!(f.diag.message.contains("needs about 117 units") && f.diag.span.is_some());
    let clean = |s: &str, x| smoke(compile(&src(s, x)).unwrap(), 1).fault.is_none();
    assert!(clean("Score 12", 100) && clean(long, 110));
    // Tic-tac-toe, breakout, paint, tetris and snake on a canvas play clean, from three seeds.
    for (name, src) in [
        ("tictactoe", include_str!("../tests/tictactoe.app")),
        ("breakout", include_str!("../tests/breakout.app")),
        ("paint", include_str!("../tests/paint.app")),
        ("blocks", include_str!("../tests/blocks.app")),
        ("snake", SHOTS[0].1),
    ] {
        for seed in 1..=3 {
            let report =
                smoke(compile(src).unwrap_or_else(|d| panic!("{name}: {}", d.render(src))), seed);
            assert!(
                report.fault.is_none() && report.warnings.is_empty(),
                "{name} {seed}: {report:?}"
            );
        }
    }
    // A tap at the middle of tic-tac-toe's board marks its middle square: an X of two lines.
    let mut t = app(include_str!("../tests/tictactoe.app"));
    let marks = |t: &mut App| match &t.render().unwrap()[1] {
        Node::Canvas { draws, .. } => draws.len(),
        _ => 0,
    };
    let before = marks(&mut t);
    assert_eq!(ev(&mut t, Event::Tap { id: 0, cell: 150 * 300 + 150 }), None);
    assert_eq!(marks(&mut t), before + 2);
    assert_eq!(vals(&t)[0], ints(&[0, 0, 0, 0, 1, 0, 0, 0, 0]));
}

/// `nodes`' canvases as uiwire carries them (Studio sends them so): each decodes as it is, with
/// the ink the runtime counted. How many there are.
fn wired(nodes: &[Node]) -> usize {
    use uiwire::Shape as S;
    let shapes = [S::Rect, S::Circle, S::Ring, S::Line, S::Text, S::Sprite, S::Pixels];
    let each = |n: &Node| match n {
        Node::Canvas { w, h, draws, .. } => {
            let wire = |d: &Draw| {
                let text = d.text.clone();
                uiwire::Draw { shape: shapes[d.shape as usize], color: d.color, at: d.at, text }
            };
            let wires: Vec<_> = draws.iter().map(wire).collect();
            assert!(draws.iter().zip(&wires).all(|(d, w)| d.ink() == w.ink()), "{draws:?}");
            let c = uiwire::Node::Canvas { id: 1, w: *w, h: *h, draws: wires };
            assert_eq!(uiwire::Node::decode(&c.encode()).as_ref(), Some(&c));
            1
        }
        Node::Row { children } | Node::Col { children } => wired(children),
        _ => 0,
    };
    nodes.iter().map(each).sum()
}

#[test]
fn paint_blocks_and_snake_play_on_pixels_that_cross_the_wire() {
    // Paint: a tap paints its square in the pen's color; one under the picture picks the pen.
    let mut p = app(include_str!("../tests/paint.app"));
    let tap = |w: u32, x: u32, y: u32| Event::Tap { id: 0, cell: y * w + x };
    for e in [tap(160, 12, 7), tap(160, 2 + 4 * 11 + 5, 170), tap(160, 159, 159)] {
        assert_eq!(ev(&mut p, e), None);
    }
    let Value::List(art) = &vals(&p)[0] else { panic!("the art") };
    let blank = Value::Int(-1);
    let painted: Vec<_> = art.iter().enumerate().filter(|(_, v)| **v != blank).collect();
    assert_eq!(painted, [(34, &Value::Int(1)), (1023, &Value::Int(4))]);
    assert_eq!(wired(&p.render().unwrap()), 1);
    assert_eq!(click(&mut p, 2), None);
    assert_eq!(vals(&p)[0], ints(&[-1; 1024]), "Clear");
    // Blocks: an I lying at the top over a bottom row full but for its four squares; a tap at
    // the bottom pushes it a row down, Drop drops it, and the row clears.
    let mut b = app(include_str!("../tests/blocks.app"));
    assert_eq!(click(&mut b, 3), None);
    let mut cells = vec![-1; 190];
    cells.extend([1, 1, 1, -1, -1, -1, -1, 1, 1, 1]);
    b.state.vals[0] = ints(&cells);
    b.state.vals[1..5].clone_from_slice(&[0, 0, 3, 0].map(Value::Int));
    assert_eq!((ev(&mut b, tap(80, 40, 150)), click(&mut b, 2)), (None, None));
    assert_eq!((&vals(&b)[0], &vals(&b)[5]), (&ints(&[-1; 200]), &Value::Int(10)));
    // A tap at its left moves the next piece left; a tick lets it fall a row.
    assert_eq!(ev(&mut b, tap(80, 5, 50)), None);
    assert_eq!(ev(&mut b, Event::Tick { ms: 400 }), None);
    assert_eq!(vals(&b)[3..5], [Value::Int(2), Value::Int(1)]);
    // Snake too, as it moves.
    let mut s = app(SHOTS[0].1);
    assert_eq!(click(&mut s, 1), None);
    for _ in 0..20 {
        assert_eq!(ev(&mut s, Event::Tick { ms: 150 }), None);
        assert_eq!((wired(&s.render().unwrap()), wired(&b.render().unwrap())), (1, 1));
    }
}

/// The taps `play` gives on paint `p` (its canvas 7) sent as the desktop sends them: how many.
fn send(p: &mut App, play: &mut uiview::Play, (id, cells): (u32, Vec<u32>)) -> usize {
    assert!(id == 7 && cells.len() <= uiview::FINE_TAPS as usize, "{cells:?}");
    for &cell in &cells {
        play.sent(false);
        assert_eq!(ev(p, Event::Tap { id: 0, cell }), None);
    }
    cells.len()
}

#[test]
fn paint_driven_by_a_drag_paints_every_square_it_crosses() {
    // Paint's canvas as the desktop lays it out on a phone: 160 x 180 units, 2 px a unit, a pad.
    // A finger goes down on square (1, 1) and drags in three samples to square (30, 20), the
    // window answering each tap with a frame and drawing it before the next sample is taken.
    let mut p = app(include_str!("../tests/paint.app"));
    let rect = gfx::RectF::new(20.0, 60.0, 320.0, 360.0);
    let board = uiview::Board { id: 7, nth: 0, rect, cols: 160, n: 160 * 180, fine: true };
    let (view, mut play) =
        (uiview::View { grids: vec![board], ..Default::default() }, uiview::Play::default());
    let at = |sx: f32, sy: f32| (20.0 + sx * 10.0 + 5.0, 60.0 + sy * 10.0 + 5.0);
    let answer = |play: &mut uiview::Play| {
        while play.busy() {
            play.answered();
        }
        play.drew();
    };
    let mut taps = 0;
    for (i, (sx, sy)) in
        [(1.0, 1.0), (11.0, 4.0), (21.0, 14.0), (30.0, 20.0)].into_iter().enumerate()
    {
        let ((x, y), id) = (at(sx, sy), (i == 0).then_some(7));
        let t = play.tap(&view, id, x, y);
        taps += send(&mut p, &mut play, t);
        answer(&mut play);
    }
    // Each square on the way painted, in a line with no gap: one to three a column, touching.
    let painted = |p: &App| -> Vec<usize> {
        let Value::List(art) = &vals(p)[0] else { panic!("the art") };
        (0..1024).filter(|&i| art[i] == Value::Int(1)).collect()
    };
    for col in 1..=30 {
        let rows: Vec<usize> =
            painted(&p).iter().filter(|&&i| i % 32 == col).map(|i| i / 32).collect();
        let touch = rows.windows(2).all(|w| w[1] == w[0] + 1);
        assert!((1..=3).contains(&rows.len()) && touch, "column {col}: {rows:?}");
    }
    assert!(painted(&p).len() < taps, "{taps} taps");
    // A quick stroke (as a script drags): its moves come while the press's tap is unanswered, so
    // none taps; once that is answered and drawn, where it went is held, and tapped then (at the
    // window's next frame), 300 px in one go: every square on row 25 painted.
    let t = play.tap(&view, Some(7), at(1.0, 25.0).0, at(1.0, 25.0).1);
    send(&mut p, &mut play, t);
    for sx in [10.0, 20.0, 31.0] {
        let (x, y) = at(sx, 25.0);
        assert!(play.tap(&view, None, x, y).1.is_empty() && !play.held());
    }
    answer(&mut play);
    assert!(play.held());
    let t = play.replay(&view);
    send(&mut p, &mut play, t);
    answer(&mut play);
    assert!(!play.held());
    let row: Vec<usize> = painted(&p).into_iter().filter(|i| i / 32 == 25).collect();
    assert_eq!(row, (25 * 32 + 1..=25 * 32 + 31).collect::<Vec<_>>());
    // It shows the pen's color as a swatch, and its squares' edges faintly (gray) under the paint.
    let Node::Canvas { draws, .. } = &p.render().unwrap()[1] else { panic!("the canvas") };
    let lines = draws.iter().filter(|d| d.shape == Shape::Line && d.color == 8).count();
    assert!(lines == 62 && draws.last().is_some_and(|d| d.shape == Shape::Rect && d.color == 1));
}
