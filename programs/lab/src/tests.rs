use std::collections::BTreeSet;

use tiny::{EOS, Rng, Tokenizer};

use crate::augment::{rename, shuffle_states};
use crate::collect::{blocks, find, literals};
use crate::corpus::{compiles, runs, shape};
use crate::measure::{judge, program, prompt_tokens};
use crate::ngram::Ngram;

#[test]
fn literals_are_read_as_rust_reads_them() {
    let src = r####"
// "not this" in a comment
/* nor "this" /* nested "one" */ */
let a = "line\n\"quoted\" \u{e9}\x41";
let b = r#"raw "inside" \n"#;
let c = 'x'; let d = '"'; let e = '\''; fn f<'a>(s: &'a str) {}
let g = "joined \
         here";
let h = b"bytes"; let r#type = 1; let i = br"raw bytes";
"####;
    let found = literals(src);
    let texts: Vec<&str> = found.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(
        texts,
        ["line\n\"quoted\" \u{e9}A", "raw \"inside\" \\n", "joined here", "bytes", "raw bytes"]
    );
    let lines: Vec<usize> = found.iter().map(|f| f.0).collect();
    assert_eq!(lines, [4, 5, 7, 9, 9]);
}

#[test]
fn app_blocks_are_cut_out_closed_or_not() {
    let text = "Here.\n```app\nlabel 1;\n```\nand\n```app \nstate a = 0;\nlabel a;\n";
    assert_eq!(
        blocks(text),
        [(3, "label 1;\n".to_string()), (7, "state a = 0;\nlabel a;\n".to_string())]
    );
    assert!(blocks("```apple\nlabel 1;\n```\n").is_empty());
}

#[test]
fn the_repo_holds_the_shots_and_the_replays() {
    let found = find(&crate::root());
    let at = |path: &str| found.iter().filter(|f| f.path == path).collect::<Vec<_>>();
    let snake = at("programs/applang/shots/snake.app");
    assert!(snake.len() == 1 && compiles(&snake[0].text));
    // A replay's first write did not compile (it named `stack`, never declared); its second did.
    let (one, two) = (
        at("programs/coder/tests/fixtures/tetris-1.txt"),
        at("programs/coder/tests/fixtures/tetris-2.txt"),
    );
    assert!(one.len() == 1 && !compiles(&one[0].text));
    assert!(two.len() == 1 && compiles(&two[0].text));
    // The tests' programs, from their string literals: the counter demo among them.
    assert!(found.iter().any(
        |f| f.path == "programs/applang/src/tests.rs" && f.text.starts_with("state count = 0;")
    ));
    assert!(
        found
            .iter()
            .all(|f| !f.path.starts_with("programs/lab") && !f.path.starts_with("programs/tiny"))
    );
}

const COUNTER: &str = "// A counter.\nstate count = 0;\nstate step = 1;\nsaved state best = 0;\n\
                       fn bump(by: int) { count += by; best = max(best, count); }\n\
                       row { button \"+\" { bump(step); } label count; }\n\
                       label \"best \" + best;\n\
                       for i in 0..3 { label i; }\n";

#[test]
fn renaming_keeps_a_program_and_its_meaning() {
    let mut seen = BTreeSet::new();
    for seed in 0..20 {
        let out = rename(COUNTER, &mut Rng::new(seed)).expect("renamed");
        assert!(compiles(&out) && runs(&out), "{out}");
        // Strings, comments, built-ins and the words that are not names stay.
        assert!(
            out.starts_with("// A counter.\n")
                && out.contains("\"best \" + ")
                && out.contains("max(")
        );
        assert!(!out.contains("count +=") && !out.contains("label count;"), "{out}");
        assert_eq!(shape(&out), shape(COUNTER), "a renamed program keeps its shape");
        seen.insert(out);
    }
    assert!(seen.len() > 15, "{}", seen.len());
    // A canvas handler's x and y are never renamed, nor taken.
    let canvas =
        "state x0 = 0;\nfn scene() { rect(x0, 0, 4, 4, 1); }\ncanvas 10, 10, scene() { x0 = x; }\n";
    for seed in 0..10 {
        let out = rename(canvas, &mut Rng::new(seed)).unwrap();
        assert!(out.contains("= x; }") && compiles(&out), "{out}");
    }
    assert_eq!(rename("label 1;", &mut Rng::new(1)), None);
}

#[test]
fn states_are_reordered_and_nothing_else_moves() {
    let mut orders = BTreeSet::new();
    for seed in 0..12 {
        let Some(out) = shuffle_states(COUNTER, &mut Rng::new(seed)) else { continue };
        assert!(out.starts_with("// A counter.\nstate") || out.starts_with("// A counter.\nsaved"));
        assert!(out.ends_with("for i in 0..3 { label i; }\n") && compiles(&out));
        let mut a: Vec<&str> = out.lines().collect();
        let mut b: Vec<&str> = COUNTER.lines().collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        orders.insert(out);
    }
    assert!(orders.len() >= 4, "{}", orders.len());
    assert_eq!(shuffle_states("state a = 0; state b = 1;\nlabel a;\n", &mut Rng::new(1)), None);
}

#[test]
fn the_ngram_is_a_distribution_and_backs_off() {
    let stream: Vec<u32> = [EOS, 1, 2, 3, 1, 2, 4, EOS, 1, 2, 3, EOS].to_vec();
    let g = Ngram::train(&stream, 3, 260);
    for ctx in [&[][..], &[1, 2], &[9, 9]] {
        let sum: f64 = (0..260).map(|w| g.prob(ctx, w)).sum();
        assert!((sum - 1.0).abs() < 1e-9, "{ctx:?}: {sum}");
    }
    assert!(g.prob(&[1, 2], 3) > g.prob(&[1, 2], 4) && g.prob(&[1, 2], 4) > g.prob(&[1, 2], 5));
    // Sampling after 1 2 draws only what followed it; after an unseen context, backs off.
    let mut rng = Rng::new(3);
    for _ in 0..50 {
        assert!(matches!(g.sample(&[1, 2], 1.0, &mut rng), 3 | 4));
    }
    let out = g.generate(&[EOS, 1], 20, 1.0, &mut rng);
    assert_eq!(out.last(), Some(&EOS));
    assert!(g.loss(std::slice::from_ref(&stream)) < g.loss(&[vec![5, 6, 7, 8]]));
}

#[test]
fn programs_are_judged_by_applang() {
    let tok = Tokenizer::bytes();
    let p = prompt_tokens(&tok, 0);
    assert_eq!(p[0], EOS);
    let mut out = tok.encode("state n = 0;\nbutton \"+\" { n += 1; }\nlabel n;\n");
    out.push(EOS);
    out.extend(tok.encode("ignored after the end"));
    let (text, ended) = program(&tok, 0, &out);
    assert!(ended && text.starts_with(crate::measure::PROMPTS[0]) && text.ends_with("label n;\n"));
    let known: BTreeSet<u64> = BTreeSet::new();
    let s = judge(0, text.clone(), ended, &known);
    assert!(s.compiles && s.runs && s.novel && s.code.is_none());
    // A copy with its names changed, its comments and spacing too, is still a copy.
    let known: BTreeSet<u64> = [shape(&text)].into();
    let copy = text.replace(" n", " count").replace("// ", "//").replace(";\n", "; \n");
    assert!(compiles(&copy) && !judge(0, copy, true, &known).novel);
    assert!(judge(0, text.replace("1;", "2;"), true, &known).novel);
    let bad = judge(1, "label nope;\n".into(), false, &known);
    assert_eq!((bad.compiles, bad.runs, bad.code), (false, false, Some(302)));
    let faults = judge(2, "state t = 0;\nlabel \"avg \" + 1 / t;\n".into(), true, &known);
    assert_eq!((faults.compiles, faults.runs, faults.code), (true, false, Some(203)));
}

#[test]
fn the_stream_and_the_held_out_windows_cover_their_programs() {
    let p = |text: &str, held: bool| crate::corpus::Program {
        hash: tiny::fnv(text.as_bytes()),
        text: text.into(),
        runs: true,
        held,
        of: None,
        from: Vec::new(),
    };
    let c = crate::corpus::Corpus {
        programs: vec![p("label 1;\n", false), p("label 22;\n", true), p("label 3;\n", false)],
    };
    let tok = Tokenizer::bytes();
    let s = crate::stream(&c, &tok, 7);
    assert_eq!((s.len(), s.iter().filter(|&&t| t == EOS).count()), (21, 3));
    assert_eq!((s[0], s[s.len() - 1]), (EOS, EOS));
    assert!(!tok.decode(&s).contains("22"), "held-out programs are never trained on");
    // Each held-out token is predicted once: windows of ctx + 1 overlapping by one.
    let held = crate::held(&c, &tok, 4);
    assert_eq!(held.iter().map(|w| w.len()).collect::<Vec<_>>(), [5, 5, 4]);
    assert_eq!(held.iter().map(|w| w.len() - 1).sum::<usize>(), "label 22;\n".len() + 1);
    assert_eq!((held[0][0], held[2][3]), (EOS, EOS));
}
