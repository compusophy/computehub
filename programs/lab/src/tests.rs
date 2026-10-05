use std::collections::BTreeSet;
use std::fs;

use tiny::{EOS, Rng, Tokenizer};

use crate::augment::{drop_line, rename, shuffle_states, vary_numbers};
use crate::collect::{SKIP, blocks, find, literals};
use crate::constrain::{Lex, Rule, parses};
use crate::corpus::{Corpus, Program, compiles, content, hold_out, runs, shape};
use crate::measure::{PROMPTS, judge, program, prompt, write};

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
    // Nothing from the skipped folders: the lab's and tiny's own tests, and the evals' answer
    // keys (Suite 1's reference apps, which compile and run clean).
    let skipped =
        |f: &&crate::collect::Found| SKIP.iter().any(|s| f.path.starts_with(&[s, "/"].concat()));
    assert_eq!(found.iter().find(skipped), None);
    let refs = fs::read_dir(crate::root().join("programs/makes/refs")).unwrap();
    assert!(refs.flatten().any(|e| e.path().extension().is_some_and(|x| x == "app")));
}

/// The committed manifest is the corpus as the repo holds it now, as `lab train` and
/// `measure` require: a change that adds a program where `collect` looks, or changes what
/// compiles or runs clean, fails here, not at the next training run.
#[test]
fn the_manifest_is_the_corpus_the_repo_holds() {
    let root = crate::root();
    let on_disk = fs::read_to_string(root.join(crate::DATA).join("manifest.tsv")).unwrap();
    let built = Corpus::build(&root, 8).manifest();
    let rows = |m: &str| -> BTreeSet<String> { content(m).lines().map(String::from).collect() };
    let (want, have) = (rows(&built), rows(&on_disk));
    let (new, gone) = (want.difference(&have).count(), have.difference(&want).count());
    // Where the first programs found that the manifest lacks are (their variants follow them).
    let at: Vec<&str> = built
        .lines()
        .filter(|l| l.split('\t').nth(6) == Some("-"))
        .filter_map(|l| l.rsplit_once('\t'))
        .filter(|(row, _)| !have.contains(*row))
        .map(|(_, from)| from)
        .take(5)
        .collect();
    assert!(
        content(&built) == content(&on_disk),
        "the corpus differs from {}/manifest.tsv ({new} rows new, {gone} gone; found at \
         {at:?}): run `cargo run -p compusophy-lab --release -- corpus`, then train and measure",
        crate::DATA
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
fn a_line_dropped_and_numbers_changed_make_other_programs() {
    let mut dropped = BTreeSet::new();
    for seed in 0..30 {
        let Some(out) = drop_line(COUNTER, &mut Rng::new(seed)) else { continue };
        // One line of code fewer, its brackets closed on it; the comment stays.
        assert_eq!(out.lines().count() + 1, COUNTER.lines().count(), "{out}");
        assert!(out.starts_with("// A counter.\n"), "{out}");
        dropped.insert(out);
    }
    assert!(dropped.len() >= 4, "{}", dropped.len());
    assert!(dropped.iter().any(|t| compiles(t) && shape(t) != shape(COUNTER)));
    assert_eq!(drop_line("// only\nlabel 1;\n", &mut Rng::new(1)), None);
    let mut varied = BTreeSet::new();
    for seed in 0..20 {
        let Some(out) = vary_numbers(COUNTER, &mut Rng::new(seed)) else { continue };
        assert!(compiles(&out) && out.lines().count() == COUNTER.lines().count(), "{out}");
        varied.insert(out);
    }
    assert!(varied.len() >= 8, "{}", varied.len());
    assert_eq!(vary_numbers("label \"7\";\n", &mut Rng::new(1)), None, "strings stay");
}

/// Whether applang's lexer refuses `src`: the compiler's first error is a lexer code.
fn lex_error(src: &str) -> bool {
    applang::compile(src).is_err_and(|d| d.code.is_some_and(|c| (1..=5).contains(&c)))
}

/// Whether `src`'s brackets match, as applang's tokens have them (strings and comments aside).
fn matched(src: &str) -> bool {
    let mut open = Vec::new();
    for (span, class) in applang::highlight(src) {
        let t = &src[span.start..span.end];
        match (class, t) {
            (applang::Class::Punct, "(" | "[" | "{") => open.push(t),
            (applang::Class::Punct, ")") if open.pop() != Some("(") => return false,
            (applang::Class::Punct, "]") if open.pop() != Some("[") => return false,
            (applang::Class::Punct, "}") if open.pop() != Some("{") => return false,
            _ => {}
        }
    }
    open.is_empty()
}

#[test]
fn the_lexer_rule_is_applangs_lexer() {
    let snake = fs::read_to_string(crate::root().join("programs/applang/shots/snake.app")).unwrap();
    let ok = |src: &str| {
        let mut lex = Lex::default();
        lex.feed(src.as_bytes()) && lex.can_end()
    };
    assert!(ok(COUNTER) && ok(&snake));
    #[rustfmt::skip]
    let bad = [
        "label \"open\n;", "label \"a\\q\";", "label 3x;", "label 'a';", "a # b", "label 1 . 2;",
        "a & b", "/* open", "row { label 1; ]", "row { label 1;", "label 1; }", "label \"é",
    ];
    for src in bad {
        assert!(!ok(src), "{src:?}");
    }
    // Mutated programs: the rule refuses exactly what the lexer refuses, or brackets that do
    // not match.
    let bytes = b" \n\"\\/*.&|'#09az_(){}[];=-+!<>";
    let mut rng = Rng::new(5);
    for _ in 0..3000 {
        let mut m = COUNTER.as_bytes().to_vec();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(m.len() + 1);
            m.insert(at, bytes[rng.below(bytes.len())]);
        }
        let m = String::from_utf8(m).unwrap();
        assert_eq!(ok(&m), !lex_error(&m) && matched(&m), "{m}");
    }
}

#[test]
fn the_parser_rule_lets_every_program_through_as_it_is_written() {
    let snake = fs::read_to_string(crate::root().join("programs/applang/shots/snake.app")).unwrap();
    for src in [COUNTER, snake.as_str()] {
        let mut lex = Lex::default();
        for (i, b) in src.bytes().enumerate() {
            assert!(lex.feed(&[b]));
            if src.is_char_boundary(i + 1) {
                assert!(parses(&src[..=i], &lex, false), "refused at {i}: {:?}", &src[..=i]);
            }
        }
        assert!(parses(src, &lex, true));
    }
    // What no more text can mend: an error before the end of what is whole.
    let refused = |src: &str| {
        let mut lex = Lex::default();
        lex.feed(src.as_bytes());
        !parses(src, &lex, false)
    };
    for src in ["label \"a\" label ", "state = ", "label 1;\n)", "label ;", "row { if }"] {
        assert!(refused(src), "{src:?}");
    }
    // A name where only a name may not go is judged whole; one that may become a word, not yet.
    assert!(refused("label 1 abc") && !refused("label 1; la") && !refused("state n = 0;\ns"));
    // An operator, by each it can still be: `.` only as `..`, `-` as `-`, `-=` or `->`.
    assert!(refused("label 1 .") && !refused("label 1 -") && !refused("fn f() -"));
    let mut lex = Lex::default();
    lex.feed(b"label 1");
    assert!(!parses("label 1", &lex, true), "a program ends only where it parses");
}

#[test]
fn constrained_writers_keep_their_rules() {
    let tok = Tokenizer::bytes();
    for rule in [Rule::Lexer, Rule::Parser] {
        for seed in 0..4 {
            // A writer that draws any token let through, the end token often.
            let mut draw = |_: &[u32], allow: &dyn Fn(u32) -> bool, rng: &mut Rng| {
                if allow(EOS) && rng.below(4) == 0 {
                    return Some(EOS);
                }
                let ok: Vec<u32> = (0..tok.vocab() as u32).filter(|&t| allow(t)).collect();
                ok.get(rng.below(ok.len().max(1))).copied()
            };
            let mut rng = Rng::new(seed);
            let out = write(&tok, (seed as usize) % PROMPTS.len(), rule, &mut draw, &mut rng);
            let (text, ended) = program(&tok, (seed as usize) % PROMPTS.len(), &out);
            assert!(ended, "{rule:?} {seed}: {text}");
            assert!(!lex_error(&text) && matched(&text), "{rule:?}: {text}");
            if rule == Rule::Parser {
                let parsed = applang::compile(&text);
                assert!(parsed.is_ok() || parsed.is_err_and(|d| d.code >= Some(301)), "{text}");
            }
        }
    }
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
