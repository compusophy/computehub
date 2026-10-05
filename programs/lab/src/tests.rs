use std::collections::BTreeSet;

use tiny::{EOS, Rng, Tokenizer};

use crate::augment::{rename, shuffle_states};
use crate::collect::{blocks, find, literals};
use crate::corpus::{Corpus, Program, compiles, content, hold_out, runs, shape};
use crate::measure::{PROMPTS, judge, program, prompt};
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
    // Among the tokens let through: from the longest context that had one, else none.
    for _ in 0..20 {
        assert_eq!(g.sample_where(&[1, 2], 1.0, &mut rng, &|t| t == 4), Some(4));
        assert_eq!(g.sample_where(&[1, 2], 1.0, &mut rng, &|t| t == EOS), Some(EOS));
    }
    assert_eq!(g.sample_where(&[1, 2], 1.0, &mut rng, &|t| t == 9), None);
}

#[test]
fn a_prompt_leaves_its_last_token_to_the_writer() {
    // A tokenizer that joins a line's end to what follows: `.\n`, then `.\ns`.
    let tok = Tokenizer::from_merges(vec![(46, 10), (257, 115)]).unwrap();
    let text = [PROMPTS[0], "\nstate n = 0;\nbutton \"+\" { n += 1; }\nlabel n;\n"].concat();
    let (alone, within) = (tok.encode(PROMPTS[0]), tok.encode(&text));
    assert_eq!(tok.piece(within[alone.len() - 1]), b".\ns", "the line's end joins what follows");
    let p = prompt(&tok, 0);
    // The end token, the header but its last token; that token's bytes left to the writer.
    assert_eq!((p.tokens[0], &p.tokens[1..]), (EOS, &alone[..alone.len() - 1]));
    assert_eq!(p.heal, tok.piece(alone[alone.len() - 1]));
    // As in training: the header's tokens but the last, then one starting with its bytes.
    assert_eq!(&within[..alone.len() - 1], &p.tokens[1..]);
    assert!(tok.piece(within[alone.len() - 1]).starts_with(&p.heal));
    // The writer's tokens from there make the program back.
    let mut out = within[alone.len() - 1..].to_vec();
    out.extend([EOS, 7, 7]);
    assert_eq!(program(&tok, 0, &out), (text, true));
}

#[test]
fn programs_are_judged_by_applang() {
    let tok = Tokenizer::bytes();
    let p = prompt(&tok, 0);
    assert_eq!((p.tokens[0], p.heal.as_slice()), (EOS, &b"."[..]));
    let mut out = tok.encode(".\nstate n = 0;\nbutton \"+\" { n += 1; }\nlabel n;\n");
    out.push(EOS);
    out.extend(tok.encode("ignored after the end"));
    let (text, ended) = program(&tok, 0, &out);
    assert!(ended && text.starts_with(PROMPTS[0]) && text.ends_with("}\nlabel n;\n"));
    assert!(text[PROMPTS[0].len()..].starts_with("\nstate n"));
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

/// A program of the corpus, `of` another (by its hash) if a variant.
fn program_of(text: &str, held: bool, of: Option<u64>) -> Program {
    Program {
        hash: tiny::fnv(text.as_bytes()),
        text: text.into(),
        shape: shape(text),
        runs: true,
        held,
        of: of.map(|h| (h, "rename#1".to_string())),
        from: if of.is_none() { vec!["here.app:1".into()] } else { Vec::new() },
    }
}

#[test]
fn the_stream_and_the_held_out_windows_cover_their_programs() {
    let held_one = program_of("label 22;\n", true, None);
    let c = Corpus {
        programs: vec![
            program_of("label 1;\n", false, None),
            program_of("label 3;\n", false, None),
            program_of("label 44;\n", true, Some(held_one.hash)),
            held_one,
        ],
    };
    let tok = Tokenizer::bytes();
    let s = crate::stream(&c, &tok, 7);
    assert_eq!((s.len(), s.iter().filter(|&&t| t == EOS).count()), (21, 3));
    assert_eq!((s[0], s[s.len() - 1]), (EOS, EOS));
    let trained = tok.decode(&s);
    assert!(!trained.contains("22") && !trained.contains("44"), "held out: never trained on");
    // Each held-out program found is predicted once, its variants never (they would count it
    // again): windows of ctx + 1 overlapping by one.
    let held = crate::held(&c, &tok, 4);
    assert_eq!(held.iter().map(|w| w.len()).collect::<Vec<_>>(), [5, 5, 4]);
    assert_eq!(held.iter().map(|w| w.len() - 1).sum::<usize>(), "label 22;\n".len() + 1);
    assert_eq!((held[0][0], held[2][3]), (EOS, EOS));
    // Trained on the found programs alone: the variants go.
    assert_eq!(crate::trained(&c, true).programs.len(), 3);
}

#[test]
fn programs_are_held_out_by_shape_with_their_kin() {
    let shaped = |n: usize| format!("state s{n} = {n};\nlabel s{n};\n");
    let kept = |t: &String| shape(t) % 10 != 0;
    // A found program of a held shape (0 mod 10), and the same renamed, found apart.
    let n = (0..400).find(|&n| !kept(&shaped(n))).unwrap();
    let a = program_of(&shaped(n), false, None);
    let b = program_of(&shaped(n).replace(&format!("s{n}"), "count"), false, None);
    // A variant of b of a shape not held by its own; a program found of that shape, renamed;
    // a variant of that; and a program of a shape of its own, kept.
    let more = |t: &str, k: usize| format!("{t}label {k};\n");
    let k = (0..400).find(|&k| kept(&more(&b.text, k))).unwrap();
    let b_variant = program_of(&more(&b.text, k), false, Some(b.hash));
    let e = program_of(&more(&b.text, k).replace("count", "other"), false, None);
    let e_variant = program_of(&more(&e.text, 1), false, Some(e.hash));
    let other = (0..400).map(shaped).find(|t| kept(t) && ![a.shape, e.shape].contains(&shape(t)));
    let c = program_of(&other.unwrap(), false, None);
    assert!(a.shape == b.shape && b_variant.shape == e.shape && kept(&e.text));
    let mut programs = vec![a, b, b_variant, e, e_variant, c];
    hold_out(&mut programs);
    let split: Vec<bool> = programs.iter().map(|p| p.held).collect();
    assert_eq!(split, [true, true, true, true, true, false]);
    // No shape on both sides.
    let held: BTreeSet<u64> = programs.iter().filter(|p| p.held).map(|p| p.shape).collect();
    assert!(programs.iter().filter(|p| !p.held).all(|p| !held.contains(&p.shape)));
}

#[test]
fn the_corpus_id_is_what_it_holds_not_where() {
    let found = program_of("label 1;\n", false, None);
    let mut c = Corpus { programs: vec![found] };
    let id = c.id();
    c.programs[0].from = vec!["moved.rs:90".into(), "elsewhere.rs:3".into()];
    assert_eq!(c.id(), id, "where a program was found is not what the corpus is");
    assert_eq!(
        content(&c.manifest()),
        content(&Corpus { programs: c.programs.clone() }.manifest())
    );
    c.programs[0].runs = false;
    assert_ne!(c.id(), id);
    let manifest = c.manifest();
    let rows = content(&manifest);
    assert!(!rows.contains('#') && !rows.contains("moved.rs") && rows.starts_with("hash\tshape"));
    assert_eq!(rows.lines().count(), 2);
}
