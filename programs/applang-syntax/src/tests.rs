use super::ast::{Expr, Lit, Slot, Stmt, Type, Widget};
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
}

#[test]
fn v2_lists_functions_handlers_and_grids_check_and_fail_coded() {
    #[rustfmt::skip]
    let cases = [
        // Functions call only functions above them: no recursion, direct or not.
        ("fn f() { f(); }", CALL_BELOW), ("label g(); fn g() -> int { return 1; }", CALL_BELOW),
        ("fn a() { b(); } fn b() { }", CALL_BELOW), ("label h();", UNKNOWN_NAME),
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
    ];
    for (src, want) in cases {
        assert_eq!(code(src), Some(want), "{src}");
    }
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
    let got = literals("best = 12; // kept\nname = \"a\\\"b\"; xs = [1, -2]; none = [\"\"; 0];");
    let want = [
        ("best", Lit::Int(12)),
        ("name", Lit::Str("a\"b".into())),
        ("xs", Lit::List(Type::Ints, vec![Lit::Int(1), Lit::Int(-2)])),
        ("none", Lit::List(Type::Strs, Vec::new())),
    ];
    let got: Vec<_> = got.unwrap().into_iter().collect();
    assert_eq!(got, want.map(|(n, l)| (n.to_string(), l)));
    for bad in ["best = ;", "best 1;", "best = 1", "= 1;", "x = [1, true];"] {
        assert!(literals(bad).is_err(), "{bad}");
    }
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
