use super::ast::{BinOp, Expr, Lit, Slot, Stmt, Type, Widget};
use super::codes::*;
use super::*;

fn code(src: &str) -> Option<u16> {
    compile(src).err().and_then(|e| e.code)
}

#[test]
fn every_failure_is_coded_and_a_whole_app_compiles() {
    // Deep widget nesting trips the depth cap, never a stack overflow,
    // and so do long flat operator chains (the AST spine eval walks).
    let deep = format!("{}label 1;{}", "row {".repeat(200), "}".repeat(200));
    let chain = format!("label {}0;", "1+".repeat(500));
    #[rustfmt::skip]
    let cases = [
        (&deep[..], TOO_DEEP), (&chain[..], TOO_DEEP),
        ("\"open", UNTERMINATED_STRING), ("\"line\nbreak\"", UNTERMINATED_STRING),
        ("\"bad \\q escape\"", BAD_ESCAPE), ("123abc", BAD_INT),
        ("label 99999999999999999999;", BAD_INT), ("@", UNEXPECTED_CHAR),
        ("/* open", UNTERMINATED_COMMENT), ("label a.b;", UNEXPECTED_CHAR),
        ("label 1; state x = 0;", UNEXPECTED_TOKEN), ("state x = y;", UNEXPECTED_TOKEN),
        ("state x = -true;", UNEXPECTED_TOKEN), ("button { }", UNEXPECTED_TOKEN),
        ("label 1", UNEXPECTED_TOKEN), ("row label 1; }", UNEXPECTED_TOKEN),
        ("widget", UNEXPECTED_TOKEN), ("on press \"a\" { }", UNEXPECTED_TOKEN),
        ("fn f(x) { }", UNEXPECTED_TOKEN), ("fn f(x: float) { }", UNEXPECTED_TOKEN),
        ("state n = 0; button \"b\" { n *= 2; }", UNEXPECTED_TOKEN),
        ("label nope;", UNKNOWN_NAME), ("state x = 1; button \"b\" { y = 2; }", UNKNOWN_NAME),
        ("input missing;", UNKNOWN_NAME), ("state x = 1; state x = 2;", DUP_STATE),
        ("state n = 0; input n;", TYPE_MISMATCH), ("if 1 { label 1; }", TYPE_MISMATCH),
        ("state x = 1; button \"b\" { x = \"s\"; }", TYPE_MISMATCH),
        ("state x = 1; button \"b\" { repeat true { } }", TYPE_MISMATCH),
        ("label 1 == \"1\";", TYPE_MISMATCH), ("label true + true;", TYPE_MISMATCH),
        ("label -true;", TYPE_MISMATCH),
        // A block-local disappears when its block ends.
        ("state x = 1; button \"b\" { if true { let t = 1; } x = t; }", UNKNOWN_NAME),
    ];
    for (src, want) in cases {
        assert_eq!(code(src), Some(want), "{src}");
    }
    let e = compile("label 1; state x = 0;").unwrap_err();
    assert!(e.message.contains("must come first"), "{e}");
    // A whole app compiles.
    let p = compile(
        "state count = 0; state name = \"w\\\"o\\\\r\\nld\"; state neg = -1_000;
         label \"Counter\"; /* a /* nested */ comment */
         row { button \"-\" { count = count - 1; } label count; }
         input name; // a line comment
         if count > 1000 { label \"big \" + count; } else if (count == 0) { } else { }",
    )
    .unwrap();
    let inits: Vec<_> = p.states().iter().map(|s| s.init.clone()).collect();
    assert_eq!(inits, [Lit::Int(0), Lit::Str("w\"o\\r\nld".into()), Lit::Int(-1000)]);
    assert_eq!(p.widgets().len(), 4);
    assert!(matches!(&p.widgets()[3], Widget::If { arms, .. } if arms.len() == 2));
    // `+` with a string concatenates; locals may shadow a state with another type.
    assert!(compile("state s = \"x\"; label 1 + 2; label \"n = \" + s;").is_ok());
    assert!(compile("state x = 1; button \"b\" { let x = \"s\"; x = \"t\"; }").is_ok());
    // Spans count bytes and cover multi-byte chars whole.
    let span = |src| compile(src).map(drop).unwrap_err().span;
    assert_eq!(span("label \"é\" é;"), Some(Span::new(11, 13)));
    assert_eq!(span("label !\"é\";"), Some(Span::new(6, 11)));
    // A short operator chain is within the depth cap.
    assert!(compile(&format!("label {}0;", "1+".repeat(40))).is_ok());
    // A stray `;` after a closing brace, or doubled, is nothing; a missing one is an error.
    let stray = "state n = 0;; grid 1, [1] { n = cell; }; row { label n;; }; \
                 button \"b\" { if true { n = 1; }; };";
    assert!(compile(stray).is_ok());
    assert_eq!(code("row { label 1 }"), Some(UNEXPECTED_TOKEN));
}

#[test]
fn v2_lists_functions_handlers_and_grids_check_and_fail_coded() {
    #[rustfmt::skip]
    let cases = [
        // Functions call only functions above them: no recursion, direct or not.
        ("fn f() { f(); }", CALL_BELOW), ("fn a() { b(); } fn b() { }", CALL_BELOW),
        ("label h();", UNKNOWN_NAME),
        ("fn f() -> int { if true { return 1; } }", MISSING_RETURN),
        ("fn f() -> int { }", MISSING_RETURN),
        ("fn f(x: int) { } fn f() { }", DUP_STATE), ("fn len() { }", DUP_STATE),
        ("fn f(x: int, x: int) { }", DUP_STATE),
        ("fn f(x: int) -> int { return x; } label f();", ARITY), ("label min(1);", ARITY),
        ("fn f() { } label f();", TYPE_MISMATCH), ("fn f() -> int { return; }", TYPE_MISMATCH),
        ("fn f() -> int { return true; }", TYPE_MISMATCH),
        ("button \"b\" { return 1; }", TYPE_MISMATCH), ("fn f() { return 2; }", TYPE_MISMATCH),
        // What draws changes nothing.
        ("label random(6);", IMPURE_RENDER),
        ("state n = 0; fn bump() -> int { n = n + 1; return n; } label bump();", IMPURE_RENDER),
        ("state xs = [0; 0]; fn add() -> int { push(xs, 1); return 0; } label add();",
         IMPURE_RENDER),
        ("state n = 0; fn f() -> int { return random(2); } every f() { }", IMPURE_RENDER),
        // Lists are typed, bounded and indexed by int.
        ("state xs = [];", UNEXPECTED_TOKEN), ("state xs = [1, true];", TYPE_MISMATCH),
        ("state xs = [0; 4097];", LIST_FULL), ("state xs = [[1]];", UNEXPECTED_TOKEN),
        ("state xs = [0; 3]; label xs;", TYPE_MISMATCH),
        ("state xs = [0; 3]; label xs[true];", TYPE_MISMATCH),
        ("state n = 0; label n[0];", TYPE_MISMATCH), ("button \"b\" { let x = []; }", TYPE_MISMATCH),
        ("state xs = [0; 3]; button \"b\" { push(xs, \"a\"); }", TYPE_MISMATCH),
        ("state xs = [0; 3]; button \"b\" { push(1, 2); }", TYPE_MISMATCH),
        ("state xs = [0; 3]; button \"b\" { remove(xs); }", ARITY),
        ("state xs = [0; 3]; label \"\" + xs;", TYPE_MISMATCH),
        ("state xs = [0; 3]; label xs == xs;", TYPE_MISMATCH),
        ("state xs = [0; 3]; button \"b\" { xs = [true; 3]; }", TYPE_MISMATCH),
        // Keys, grids and `cell`.
        ("on key \"shift\" { }", BAD_KEY), ("on key \"A\" { }", BAD_KEY),
        ("on key left { }", UNEXPECTED_TOKEN),
        ("state xs = [true; 3]; grid 3, xs;", TYPE_MISMATCH),
        ("state xs = [0; 3]; grid 3, xs, xs;", TYPE_MISMATCH),
        ("state n = 0; button \"b\" { n = cell; }", UNKNOWN_NAME),
        ("for i in 0..3 { label i; } label i;", UNKNOWN_NAME),
        ("every true { }", TYPE_MISMATCH),
        // `xs[i] += v` stores `xs[i] + v`, an item's type.
        ("state xs = [true; 2]; button \"b\" { xs[0] += true; }", TYPE_MISMATCH),
        ("state xs = [0; 2]; button \"b\" { xs[0] += \"a\"; }", TYPE_MISMATCH),
        ("state ws = [\"\"; 2]; button \"b\" { ws[0] -= 1; }", TYPE_MISMATCH),
        ("label 9223372036854775808;", BAD_INT), ("label -9223372036854775808;", BAD_INT),
    ];
    for (src, want) in cases {
        assert_eq!(code(src), Some(want), "{src}");
    }
    // All state fits in its limit as declared: a fill is measured before it is made (4,000
    // strings of 70 bytes would be 280,008), and the states together (two of 240,008).
    let src = format!("state n = 0; state a = [\"{}\"; 4000];", "x".repeat(70));
    let e = compile(&src).unwrap_err();
    let fill = Span::new(src.find('[').unwrap(), src.find(']').unwrap());
    assert_eq!((e.code, e.span), (Some(STATE_TOO_BIG), Some(fill)));
    let src = format!("state a = [\"{}\"; 4000]; state b = [\"{0}\"; 4000];", "x".repeat(60));
    let (e, b) = (compile(&src).unwrap_err(), src.find("b =").unwrap());
    assert_eq!((e.code, e.span), (Some(STATE_TOO_BIG), Some(Span::new(b, b + 1))));
    // Handlers and widgets, unlike functions, call functions in any order.
    let any = "state n = 0; every 100 { step(); } on key \"left\" { step(); } label twice(n);
               grid 1, [0] { step(); } fn step() { n += 1; } fn twice(x: int) -> int { return x * 2; }";
    assert!(compile(any).is_ok());
}

#[test]
fn plus_equals_keeps_its_index_whole_and_lists_know_their_shape() {
    let p = compile(
        "state a = [0; 3]; state b = [0; 3]; state c = [0; 3]; state d = [0; 3];
         state e = [0; 3]; saved state f = [\"\"; 0]; state n = 0;
         fn grow() { push(b, 1); }
         button \"x\" { a[n] += 1; a[0] -= n; c = [0; 3]; c = [1, 2, 3]; d = [1, 2]; e = [0; n]; }
         button \"y\" { let l = [0; 1]; push(l, 1); clear(l); f[0] += \"!\"; }",
    )
    .unwrap();
    let fixed: Vec<bool> = p.states().iter().map(|s| s.fixed).collect();
    assert_eq!(fixed, [true, false, true, false, false, true, true]);
    // `a[n] += 1` keeps its index once, apart from the value it adds.
    let Widget::Button { handler, .. } = &p.widgets()[0] else { panic!() };
    let Stmt::SetIndex { index: Expr::Var(i), op, value: Expr::Int(1, _), .. } = &handler.body[0]
    else {
        panic!("{:?}", handler.body[0])
    };
    assert_eq!((i.slot, *op), (Slot::State(6), Some(BinOp::Add)));
}

#[test]
fn a_v2_program_resolves_every_name_to_a_slot() {
    let p = compile(
        "state cells = [0; 9]; saved state best = 0; state tasks = [\"a\", \"b\",];
         fn at(x: int, y: int) -> int { let i = y * 3; return i + x; }
         fn reset() { for i in 0..len(cells) { cells[i] = 0; } best += 1; push(tasks, \"c\"); }
         every best * 100 { reset(); }
         on key \"left\" { cells[at(0, 1)] -= 1; }
         on key \"q\" { return; }
         for r in 0..3 { button \"row \" + r { cells[r] = 1; } }
         grid 3, cells { cells[cell] = 2; }
         grid 3, cells, [\"x\"; 9];",
    )
    .unwrap();
    assert_eq!(p.states().iter().map(|s| s.saved).collect::<Vec<_>>(), [false, true, false]);
    assert_eq!(
        p.states()[2].init,
        Lit::List(Type::Strs, vec![Lit::Str("a".into()), Lit::Str("b".into())])
    );
    let [at, reset] = p.fns() else { panic!("two fns") };
    assert!(at.pure && !reset.pure && at.ret == Some(Type::Int) && reset.ret.is_none());
    // In `at`: x and y are locals 0 and 1, the let 2.
    let Stmt::Return { value: Some(Expr::Binary(_, l, r, _)), .. } = &at.body[1] else { panic!() };
    let (Expr::Var(i), Expr::Var(x)) = (&**l, &**r) else { panic!("{l:?} {r:?}") };
    assert_eq!((i.slot, x.slot), (Slot::Local(2), Slot::Local(0)));
    // `best += 1` is `best = best + 1`, on state 1.
    let Stmt::Assign { target, value: Expr::Binary(..), .. } = &reset.body[1] else { panic!() };
    assert_eq!(target.slot, Slot::State(1));
    assert_eq!((p.everys().len(), p.keys().len(), p.widgets().len()), (1, 2, 3));
    let Widget::Grid { handler: Some(h), .. } = &p.widgets()[1] else { panic!() };
    assert_eq!(h.id, 1);
    // A for's variable is a local of the handlers inside it.
    let Widget::For { body, .. } = &p.widgets()[0] else { panic!() };
    let Widget::Button { handler, .. } = &body[0] else { panic!() };
    let Stmt::SetIndex { index: Expr::Var(r), .. } = &handler.body[0] else { panic!() };
    assert_eq!(r.slot, Slot::Local(0));
    // An if with an else on every path ends in return, and keys may be letters and digits.
    let src = "fn f(x: int) -> int { if x > 0 { return 1; } else if x < 0 { return -1; } \
               else { return 0; } } on key \"7\" { } on key \"space\" { }";
    assert!(compile(src).is_ok());
}

#[test]
fn saved_lines_read_back_as_literals() {
    let got = literals(
        "best = 12; // kept\nname = \"a\\\"b\"; xs = [1, -2]; none = [\"\"; 0];\n\
         low = -9223372036854775808;",
    );
    let want = [
        ("best", Lit::Int(12)),
        ("name", Lit::Str("a\"b".into())),
        ("xs", Lit::List(Type::Ints, vec![Lit::Int(1), Lit::Int(-2)])),
        ("none", Lit::List(Type::Strs, Vec::new())),
        ("low", Lit::Int(i64::MIN)),
    ];
    let got: Vec<_> = got.unwrap().into_iter().collect();
    assert_eq!(got, want.map(|(n, l)| (n.to_string(), l)));
    let bad =
        ["best = ;", "best 1;", "best = 1", "= 1;", "x = [1, true];", "x = 9223372036854775808;"];
    for bad in bad {
        assert!(literals(bad).is_err(), "{bad}");
    }
    // A saved line never fills past what all state may hold.
    let e = literals(&format!("xs = [\"{}\"; 4096];", "x".repeat(70))).unwrap_err();
    assert_eq!(e.code, Some(STATE_TOO_BIG));
}

#[test]
fn highlight_classes_every_token_and_goes_on_past_errors() {
    use Class::*;
    let src = "state s = \"é\\\"\"; // hi\nlabel s+12 @ é /* a */ true 1x \"a\\q\";\n\"open\n/* to";
    let got: Vec<_> = highlight(src).into_iter().map(|(s, c)| (&src[s.start..s.end], c)).collect();
    #[rustfmt::skip]
    let want = [
        ("state", Keyword), ("s", Name), ("=", Punct), ("\"é\\\"\"", Str), (";", Punct),
        ("// hi", Comment), ("label", Keyword), ("s", Name), ("+", Punct), ("12", Number),
        ("@", Error), ("é", Error), ("/* a */", Comment), ("true", Keyword), ("1x", Error),
        ("\"a\\q", Error), ("\";", Error), ("\"open", Error), ("/* to", Comment),
    ];
    assert_eq!(got, want);
    // On any input: no panic; ranges in order, apart, non-empty, on char boundaries.
    let alphabet = ["a", "1", " ", "\n", "\"", "\\", "/", "*", "é", "😀", "=", ";", "_", "n"];
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) as usize % n
    };
    for _ in 0..3000 {
        let src: String = (0..next(24)).map(|_| alphabet[next(alphabet.len())]).collect();
        let mut end = 0;
        for (span, _) in highlight(&src) {
            assert!(end <= span.start && span.start < span.end && span.end <= src.len());
            assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end));
            end = span.end;
        }
    }
}

#[test]
fn canvases_draw_only_where_a_canvas_calls_and_fail_coded() {
    let scene = "fn scene() { rect(0, 0, 9, 9, 1); circle(4, 4, 2, 3); ring(4, 4, 3, 1, 4); \
                 line(0, 0, 8, 8, 1, 9); text(\"hi\", 4, 4, 3, 11); sprite([\"1.1\"], 0, 0, 2); \
                 let b = [-1; 4]; b[1] = 11; pixels(b, 0, 0, 2, 1); }";
    let ok = [
        format!("{scene} canvas 9, 9, scene();"),
        format!("state n = 0; {scene} canvas 9, 9, scene() {{ n = x + y * 9; }}"),
        // What draws may call what draws, and what changes nothing; a state x beside a canvas
        // without a handler.
        "state x = 1; fn dot(x: int, y: int) { circle(x, y, 1, sin(90) / 1000); } \
         fn scene() { for i in 0..3 { dot(i, i + x); } } canvas 9, 9, scene();"
            .into(),
        // Before canvases a program could have its own `line` and `text`: without one, it may.
        "fn line(a: int) -> int { return a + cos(0); } fn text() { } label line(1); \
         button \"b\" { text(); }"
            .into(),
        // Pixels came later still: beside a canvas, its own `pixels` is its own.
        "fn pixels(n: int) { rect(n, 0, 1, 1, 1); } fn s() { pixels(2); } canvas 9, 9, s();".into(),
        // In a row, a loop and an if; drawing a shape alone.
        "fn s(i: int) { rect(i, 0, 1, 1, 1); } for i in 0..3 { row { if i > 0 { canvas 3, 1, \
         s(i) { } } } } canvas 1, 1, rect(0, 0, 1, 1, 2);"
            .into(),
    ];
    for src in &ok {
        assert!(compile(src).is_ok(), "{src}: {:?}", compile(src).err());
    }
    #[rustfmt::skip]
    let cases = [
        ("fn s() { } canvas 9, 9, 3;", UNEXPECTED_TOKEN), ("canvas 9, 9;", UNEXPECTED_TOKEN),
        // Shapes in a handler, an every, an on key, a widget's expression; with no canvas.
        ("fn s() { } canvas 9, 9, s() { rect(0, 0, 1, 1, 1); }", DRAW_OUTSIDE),
        ("fn s() { rect(0, 0, 1, 1, 1); } every 100 { s(); } canvas 9, 9, s();", DRAW_OUTSIDE),
        ("fn s() { line(0, 0, 1, 1, 1, 1); } on key \"a\" { s(); } canvas 9, 9, s();",
         DRAW_OUTSIDE),
        ("fn s() { rect(0, 0, 1, 1, 1); } fn v() -> int { s(); return 1; } label v(); \
          canvas 9, 9, s();", DRAW_OUTSIDE),
        ("button \"b\" { rect(0, 0, 1, 1, 1); }", DRAW_OUTSIDE),
        ("fn s() { sprite([\"1\"], 0, 0, 1); }", DRAW_OUTSIDE),
        // What draws changes nothing; nor does what a canvas calls.
        ("state n = 0; fn s() { n += 1; rect(0, 0, 1, 1, 1); } canvas 9, 9, s();",
         IMPURE_RENDER),
        ("fn s() { rect(0, 0, 1, 1, random(3)); } canvas 9, 9, s();", IMPURE_RENDER),
        ("state n = 0; fn s() { n += 1; } canvas 9, 9, s();", IMPURE_RENDER),
        // Each shape's arguments: their count and types.
        ("fn s() { rect(0, 0, 1, 1); } canvas 9, 9, s();", ARITY),
        ("fn s() { text([1], 0, 0, 1, 1); } canvas 9, 9, s();", TYPE_MISMATCH),
        ("fn s() { sprite(1, 0, 0, 1); } canvas 9, 9, s();", TYPE_MISMATCH),
        ("fn s() { pixels([\"1\"], 0, 0, 1, 1); } canvas 9, 9, s();", TYPE_MISMATCH),
        ("fn s() { pixels([1], 0, 0, 1); } canvas 9, 9, s();", ARITY),
        ("button \"b\" { pixels([1], 0, 0, 1, 1); }", DRAW_OUTSIDE),
        ("fn s() { rect(0, 0, 1, 1, \"red\"); } canvas 9, 9, s();", TYPE_MISMATCH),
        ("fn s() { } canvas \"9\", 9, s();", TYPE_MISMATCH),
        ("fn s() { } label sin(true);", TYPE_MISMATCH),
        // The tap's x and y hide no state or loop variable; beside a canvas the shapes, sin
        // and cos are built in.
        ("state x = 0; fn s() { } canvas 9, 9, s() { }", DUP_STATE),
        ("fn s() { } for y in 0..2 { canvas 9, 9, s() { } }", DUP_STATE),
        ("fn line() { } fn s() { } canvas 9, 9, s();", DUP_STATE),
        ("fn sin(a: int) -> int { return a; } fn s() { } canvas 9, 9, s();", DUP_STATE),
        ("fn s() { } canvas 9, 9, s() { let n = z; }", UNKNOWN_NAME),
    ];
    for (src, want) in cases {
        assert_eq!(code(src), Some(want), "{src}");
    }
    // Each says where to draw and what was wrong.
    let said = |src: &str| compile(src).unwrap_err().message;
    assert!(said("button \"b\" { rect(0, 0, 1, 1, 1); }").contains("add canvas W, H, scene();"));
    let m = said("fn s() { } canvas 9, 9, s() { circle(0, 0, 1, 1); }");
    assert!(m.contains("only a canvas, or a function a canvas calls"), "{m}");
    let m = said("state n = 0; fn s() { n = 1; rect(0, 0, 1, 1, 1); } canvas 9, 9, s();");
    assert!(m.starts_with("`s` draws, so it changes nothing"), "{m}");
    assert!(said("fn s() { line(0, 0, 1, 1); } canvas 1, 1, s();").contains("x1, y1, x2, y2"));
    let e = compile("fn s() { } canvas 9, 9, 3;").unwrap_err();
    assert!(
        e.message.contains("as in canvas 160, 120, scene();") && e.span == Some(Span::new(24, 25))
    );
    // The handler's x and y are its first locals after what it captured.
    let p = compile("fn s() { } for i in 0..2 { canvas 9, 9, s() { let n = y * 9 + x + i; } }")
        .unwrap();
    let Widget::For { body, .. } = &p.widgets()[0] else { panic!() };
    let Widget::Canvas { handler: Some(h), .. } = &body[0] else { panic!() };
    let Stmt::Let { value: Expr::Binary(_, sum, i, _), .. } = &h.body[0] else { panic!() };
    let Expr::Binary(_, yx, x, _) = &**sum else { panic!() };
    let Expr::Binary(_, y, _, _) = &**yx else { panic!() };
    let slot = |e: &Expr| if let Expr::Var(v) = e { v.slot } else { panic!() };
    assert_eq!([slot(y), slot(x), slot(i)], [Slot::Local(2), Slot::Local(1), Slot::Local(0)]);
}
