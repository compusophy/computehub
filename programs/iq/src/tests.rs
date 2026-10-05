use super::*;
use script::{KINDS, parse};

/// The card's example lines (those that begin with exactly two spaces), as one script.
fn card_script() -> String {
    let examples = CARD.lines().filter(|l| l.starts_with("  ") && !l.starts_with("   "));
    examples.collect::<Vec<_>>().join("\n")
}

#[test]
fn the_card_parses_and_shows_every_statement() {
    let s = parse(&card_script()).unwrap_or_else(|e| panic!("{e}"));
    for kind in KINDS {
        assert!(s.lines.iter().any(|l| l.stmt.kind() == kind), "the card shows no {kind}");
    }
    // Every line of the card is prose or an example; no example is indented deeper.
    assert!(CARD.lines().all(|l| !l.starts_with("   ")));
    // What the card says of the limits is what the parser holds.
    let limits = format!("At most {} statements,\n5,000 actions", script::MAX_LINES);
    assert!(CARD.contains(&limits) && CARD.contains("3,600,000 ms of waits"), "{limits}");
    assert_eq!((script::MAX_ACTIONS, script::MAX_WAIT, script::MAX_REPEAT), (5000, 3_600_000, 500));
    assert!(CARD.contains("(500 at most)") && CARD.contains("seeds 1, 2 and 3"));
    assert_eq!(script::SEEDS, [1, 2, 3]);
}

/// The code a one-line script (with an expectation after it, so only the line is judged)
/// earns, or 0 if it reads.
fn code(line: &str) -> u16 {
    parse(&[line, "\nexpect says \"x\""].concat()).map_or_else(|e| e.code, |_| 0)
}

#[test]
fn a_script_that_does_not_read_is_refused_with_its_code_and_line() {
    use codes::*;
    let cases: &[(&str, u16)] = &[
        ("jump 3", UNKNOWN),
        ("\"click\"", UNKNOWN),
        ("expect", UNKNOWN),
        ("expect not after \"x\" = 1", UNKNOWN),
        ("expect nothing", UNKNOWN),
        ("click \"abc", STRING),
        ("click \"a\\nb\"", STRING),
        ("click", ARGS),
        ("click \"\"", ARGS),
        ("click \"a|\"", ARGS),
        ("click \"+\" \"-\"", ARGS),
        ("click 3", ARGS),
        ("wait x", ARGS),
        ("wait 1.5", ARGS),
        ("tap 1", ARGS),
        ("tapcell 0 0 3by3", ARGS),
        ("tapcell 0 0", ARGS),
        ("mark now", ARGS),
        ("expect number == 3", ARGS),
        ("expect number = three", ARGS),
        ("expect after \" score\" = 1", ARGS),
        ("expect after \"a|b\" = 1", ARGS),
        ("expect has \"+\" twice", ARGS),
        ("wait -1", RANGE),
        ("wait 3600001", RANGE),
        ("tap 1024 0", RANGE),
        ("tapcell 3 0 3x3", RANGE),
        ("tapcell 0 0 65x2", RANGE),
        ("tapcell 0 0 0x2", RANGE),
        ("type 0 \"x\"", RANGE),
        ("repeat 501: click \"+\"", RANGE),
        ("repeat 0: click \"+\"", RANGE),
        ("expect color 0 0 3x3 = 12", RANGE),
        ("expect squares -2 3x3 = 1", RANGE),
        ("key \"f1\"", KEY),
        ("key \"Left\"", KEY),
        ("press \"Go\" \"ab\"", KEY),
        ("repeat 3 click \"+\"", REPEAT),
        ("repeat 3:", REPEAT),
        ("repeat 3: repeat 2: click \"+\"", REPEAT),
        ("repeat 3: expect has \"+\"", REPEAT),
        ("repeat 3: mark", REPEAT),
        ("expect changed", NO_MARK),
    ];
    for &(line, want) in cases {
        assert_eq!(code(line), want, "{line}");
    }
    // What reads: comments, blank lines, a `#` in a string, escapes, negative numbers, every key.
    for line in [
        "# a comment",
        "",
        "   ",
        "click \"#1\" # the button labelled #1",
        "click \"say \\\"hi\\\" \\\\ bye\"",
        "expect number = -4",
        "expect after \"a b\" < -1",
        "key \"enter\"",
        "key \"q\"",
        "key \"7\"",
        "repeat 500: wait 10",
        "tapcell 63 63 64x64",
        "expect squares -1 3x3 >= 0",
    ] {
        assert_eq!(code(line), 0, "{line}");
    }
    // A refusal names its line; a script needs an expectation and stays within its limits.
    let e = parse("click \"+\"\n\nwait 10 20\nexpect says \"x\"").unwrap_err();
    assert!(e.code == codes::ARGS && e.message.starts_with("line 3: "), "{e}");
    assert!(e.message.contains("wait 1000"), "a refusal shows the usage: {e}");
    assert_eq!(parse("click \"+\"\n# expect says \"x\"").unwrap_err().code, NO_EXPECT);
    let many = "expect says \"x\"\n".repeat(script::MAX_LINES + 1);
    assert_eq!(parse(&many).unwrap_err().code, TOO_BIG);
    let busy = "repeat 500: click \"+\"\n".repeat(11) + "expect says \"x\"";
    assert_eq!(parse(&busy).unwrap_err().code, TOO_BIG);
    let slow = "repeat 2: wait 3600000\nexpect says \"x\"";
    assert_eq!(parse(slow).unwrap_err().code, TOO_BIG);
    assert_eq!(parse(&"#".repeat(script::MAX_BYTES + 1)).unwrap_err().code, TOO_BIG);
    // The statement as written, its comment aside, is what a failure quotes.
    let s = parse("  click \"#1\"   # go\nexpect says \"x\"").unwrap();
    assert_eq!((s.lines[0].no, s.lines[0].text.as_str()), (1, "click \"#1\""));
}

/// An app that shows each thing a script reads: a count (keys add 10), a timer while Go is on, a
/// greeting typed, a 3 x 3 board of pixels that taps paint blue, and Start once the count is 3.
const PLAY: &str = "// A test.
state n = 0;
state on = false;
state t = 0;
state name = \"\";
state cells = [0; 9];
fn scene() { pixels(cells, 0, 0, 3, 10); }
every 100 { if on { t += 1; } }
on key \"up\" { n += 10; }
label \"Count \" + n;
label \"Time \" + t;
label \"Hello \" + name;
input name;
row { button \"+\" { n += 1; } button \"Go\" { on = !on; } }
if n >= 3 { button \"Start game\" { n = 100; } }
canvas 30, 30, scene() { cells[(y / 10) * 3 + x / 10] = 4; }
";

#[test]
fn a_script_drives_an_app_and_reads_what_it_shows() {
    let s = parse(
        "expect number = 0
         expect after \"count\" = 0
         expect not number > 0
         expect has \"+\"
         expect not has \"Start\"
         start                      # no Start yet: nothing
         repeat 3: click \"+\"
         expect has \"Start\"
         expect after \"count\" = 3
         key \"up\"
         expect after \"COUNT\" = 13
         press \"Nope\" \"up\"
         expect after \"count\" = 23
         press \"+\" \"up\"
         expect after \"count\" = 24
         type 1 \"Ada\"
         expect says \"hello ada\"
         expect not says \"grace|bob\"
         mark
         expect same
         click \"Go\"
         wait 500
         expect after \"time\" = 5
         expect changed
         tap 15 15
         expect color 1 1 3x3 = 4
         tapcell 0 2 3x3
         expect color 0 2 3x3 = 4
         expect color 2 2 3x3 != 4
         expect squares 4 3x3 = 2
         expect squares 0 3x3 = 7
         start
         expect number = 100",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(script::check(&s, PLAY), Ok(()));
    // Each failure is coded and names the seed, the line and what showed instead.
    let fails = |script: &str| script::check(&parse(script).unwrap(), PLAY).unwrap_err();
    let e = fails("click \"+\"\nexpect after \"count\" = 2");
    assert_eq!(e.code, codes::EXPECTED);
    assert!(e.message.starts_with("seed 1, line 2 `expect after \"count\" = 2`: after count is 1"));
    assert!(e.message.contains("Count 1"), "{e}");
    let e = fails("click \"Stop\"\nexpect says \"x\"");
    assert_eq!(e.code, codes::ACTION);
    assert!(e.message.contains("no button labelled Stop"), "{e}");
    let e = fails("repeat 2: type 3 \"x\"\nexpect says \"x\"");
    assert!(e.code == codes::ACTION && e.message.contains("its time 1 of 2"), "{e}");
    let e = fails("expect has \"Go!\"");
    assert!(e.message.contains("the buttons are [\"+\", \"Go\"]"), "{e}");
    let e = fails("mark\nclick \"+\"\nexpect same");
    assert!(e.message.contains("it changed"), "{e}");
    let e = fails("mark\nwait 100\nexpect changed");
    assert!(e.message.contains("nothing changed"), "{e}");
    let e = fails("expect squares 4 3x3 > 0");
    assert!(e.message.ends_with("0 squares are color 4"), "{e}");
    let e = fails("expect not number = 0");
    assert!(e.message.contains("its numbers are [0, 0]"), "{e}");
    // An app that faults as it first shows, and one with no board.
    let bad =
        script::check(&parse("expect says \"x\"").unwrap(), "state xs = [0; 0];\nlabel xs[1];");
    assert_eq!(bad.unwrap_err().code, codes::STARTS);
    let e = script::check(&parse("tapcell 0 0 3x3\nexpect says \"x\"").unwrap(), NULL).unwrap_err();
    assert!(e.code == codes::ACTION && e.message.contains("no 3x3 board"), "{e}");
}

#[test]
fn a_check_holds_on_every_seed() {
    let src = "state r = 0;\nbutton \"Roll\" { r = random(1000); }\nlabel \"R \" + r;";
    let mut p = makes::probe::Probe::start(src, 1).unwrap();
    p.click("Roll").unwrap();
    let first = p.after("r").unwrap();
    let s = parse(&format!("click \"Roll\"\nexpect after \"r\" = {first}")).unwrap();
    assert_eq!(script::run(&s, src, 1), Ok(()));
    let e = script::check(&s, src).unwrap_err();
    assert!(e.message.starts_with("seed 2") || e.message.starts_with("seed 3"), "{e}");
}

/// A task for a counter, its check and reference as a person would write them.
fn counter() -> Task {
    Task {
        id: "count-up".into(),
        tier: 1,
        family: "counter".into(),
        ask: "a counter: a label shows the count from 0; a button labelled + adds one".into(),
        check: "expect number = 0\nrepeat 3: click \"+\"\nexpect number = 3".into(),
        reference: "// A counter.\nstate n = 0;\nlabel \"Count \" + n;\nbutton \"+\" { n += 1; }\n"
            .into(),
        by: By {
            teacher: "hand".into(),
            prompt: String::new(),
            verifier: "0123456789abcdef".into(),
            day: "2026-10-05".into(),
        },
    }
}

#[test]
fn a_grade_is_the_first_stage_failed() {
    let t = counter();
    let g = grade(&t, &t.reference);
    assert_eq!((g.stage, g.code, g.pass()), (Stage::Pass, 0, true));
    let g = grade(&t, "label");
    assert_eq!((g.stage, g.code), (Stage::Compile, applang::codes::UNEXPECTED_TOKEN));
    assert!(g.message.starts_with("E0101 1:"), "{}", g.message);
    let faults = "state xs = [0; 0];\nbutton \"+\" { push(xs, 1); }\nevery 50 { xs[1] = 2; }\n";
    let g = grade(&t, &[faults, "label \"Count \" + len(xs);"].concat());
    assert_eq!((g.stage, g.code), (Stage::Smoke, applang::codes::INDEX_OUT_OF_RANGE));
    let g = grade(&t, "state n = 0;\nlabel \"Count \" + n;\nbutton \"+\" { n += 2; }");
    assert_eq!((g.stage, g.code), (Stage::Check, codes::EXPECTED));
    assert!(g.message.starts_with("seed 1, line 3 `expect number = 3`"), "{}", g.message);
    // An icon line the desktop cannot draw is not the grade's concern.
    let g = grade(&t, &["// A counter.\n// icon: blob 1 2\n", &t.reference].concat());
    assert!(g.pass(), "{g:?}");
    // A check that does not read is the task's fault.
    let g = grade(&Task { check: "jump".into(), ..counter() }, &t.reference);
    assert_eq!((g.stage, g.code), (Stage::Harness, codes::UNKNOWN));
    // Deterministic: the same program, the same grade.
    let wrong = "state n = 0;\nlabel \"Count \" + n;\nbutton \"+\" { n += 2; }";
    assert_eq!(grade(&t, wrong), grade(&t, wrong));
    // The program in a reply, as the coder takes it; none is the reply's stage.
    let reply = ["Here it is.\n```app\n", &t.reference, "```\nEnjoy."].concat();
    assert!(grade_reply(&t, &reply).pass());
    assert_eq!(grade_reply(&t, "I cannot.").stage, Stage::Reply);
    assert_eq!(
        Stage::ALL.map(Stage::name),
        ["reply", "compile", "smoke", "harness", "check", "pass"]
    );
}

#[test]
fn a_task_is_kept_only_if_its_check_has_teeth() {
    let v = verify(&counter()).unwrap_or_else(|e| panic!("{e}"));
    assert!(v.counted >= MIN_COUNTED && v.killed * 2 >= v.counted, "{v:?}");
    assert_eq!(v.killed + v.survivors.len(), v.counted);
    let refused = |t: Task| verify(&t).unwrap_err().code;
    assert_eq!(refused(Task { id: "Count".into(), ..counter() }), codes::BAD_ID);
    assert_eq!(refused(Task { id: "a".repeat(49), ..counter() }), codes::BAD_ID);
    assert_eq!(refused(Task { family: "a b".into(), ..counter() }), codes::BAD_FAMILY);
    assert_eq!(refused(Task { tier: 0, ..counter() }), codes::BAD_TIER);
    assert_eq!(refused(Task { tier: 7, ..counter() }), codes::BAD_TIER);
    assert_eq!(refused(Task { ask: " \n".into(), ..counter() }), codes::NO_ASK);
    assert_eq!(refused(Task { check: "jump".into(), ..counter() }), codes::BAD_CHECK);
    let wrong = "state n = 0;\nlabel \"Count \" + n;\nbutton \"+\" { n += 2; }";
    assert_eq!(refused(Task { reference: wrong.into(), ..counter() }), codes::REF_FAILS);
    let soft = "expect not says \"zebra\"";
    assert_eq!(refused(Task { check: soft.into(), ..counter() }), codes::NULL_PASSES);
    // Clicks + but expects nothing it changes: the null app fails, every slip passes.
    let blunt = "click \"+\"\nexpect not says \"zebra\"";
    let e = verify(&Task { check: blunt.into(), ..counter() }).unwrap_err();
    assert_eq!(e.code, codes::TOOTHLESS);
    assert!(
        e.message.contains("It passes: line 3 dropped") && e.message.contains("line 4:19"),
        "{e}"
    );
    // A reference too small to make slips of.
    let tiny = Task {
        check: "expect says \"hi there\"".into(),
        reference: "label \"hi there\";".into(),
        ..counter()
    };
    assert_eq!(refused(tiny), codes::FEW_MUTANTS);
}

#[test]
fn mutants_are_slips_and_equivalent_ones_are_told_apart() {
    let src = &counter().reference;
    let all = mutants(src);
    let whats: Vec<&str> = all.iter().map(|m| m.what.as_str()).collect();
    assert_eq!(
        whats,
        [
            "line 2 dropped: state n = 0;",
            "line 3 dropped: label \"Count \" + n;",
            "line 4 dropped: button \"+\" { n += 1; }",
            "line 2:11, 0 made 1: state n = 0;",
            "line 4:19, 1 made 2: button \"+\" { n += 1; }",
        ]
    );
    assert_eq!(mutants(src), all, "the same every time");
    // Capped, spread over the whole program, all distinct.
    let long: String = (0..60).map(|i| format!("label {i};\n")).collect();
    let many = mutants(&long);
    assert_eq!(many.len(), MAX_MUTANTS);
    assert!(many.iter().any(|m| m.what.contains("made")) && many[0].what.starts_with("line 1 "));
    // A starting value the app overwrites before it shows is no slip; a step of 2 is.
    let reset = "state n = 0;\nstate k = 7;\nlabel \"n \" + n;\nbutton \"+\" { k = 1; n += k; }\n";
    assert!(!mutate::differs(reset, &reset.replace("k = 7", "k = 8")));
    assert!(mutate::differs(reset, &reset.replace("n += k", "n += 2 * k")));
    // Timers, keys, taps and typing are explored too.
    assert!(mutate::differs(PLAY, &PLAY.replace("t += 1", "t += 2")));
    assert!(mutate::differs(PLAY, &PLAY.replace("n += 10", "n += 11")));
    assert!(mutate::differs(PLAY, &PLAY.replace("= 4; }", "= 5; }")));
    assert!(mutate::differs(PLAY, &PLAY.replace("\"Hello \"", "\"Hi \"")));
    assert!(!mutate::differs(PLAY, PLAY));
}

#[test]
fn tasks_read_and_write_one_way() {
    let mut t = counter();
    t.ask = "a \"quoted\" ask\twith a tab, a back\\slash, a bell \u{7} and é".into();
    t.by.prompt = "line one\nline two\r\n".into();
    let line = write_task(&t);
    assert!(line.contains("line one\\nline two\\r\\n") && line.contains("\\u0007"), "{line}");
    assert!(
        line.contains("\\\"quoted\\\"") && line.contains("back\\\\slash") && line.contains('é')
    );
    assert!(line.starts_with("{\"id\":\"count-up\",\"tier\":1,\"family\":\"counter\",\"ask\":"));
    let by = ",\"by\":{\"teacher\":\"hand\",\"prompt\":\"line one\\nline two\\r\\n\",\
              \"verifier\":\"0123456789abcdef\",\"day\":\"2026-10-05\"}}";
    assert!(line.ends_with(by), "{line}");
    assert_eq!(read_task(&line), Ok(t.clone()));
    // Members in another order read the same; the writer puts them back in its own.
    let shuffled = "{\"tier\":1,\"by\":{\"day\":\"2026-10-05\",\"verifier\":\"0123456789abcdef\",\
                    \"prompt\":\"\",\"teacher\":\"hand\"},\"id\":\"x\",\"family\":\"f\",\
                    \"ask\":\"a\",\"check\":\"c\",\"ref\":\"r\"}";
    let back = write_task(&read_task(shuffled).unwrap());
    assert!(back.starts_with("{\"id\":\"x\",\"tier\":1,\"family\":\"f\""), "{back}");
    let bad = |s: &str| read_task(s).unwrap_err().code;
    let good = write_task(&counter());
    assert_eq!(bad("{"), codes::NOT_JSON);
    assert_eq!(bad("[1]"), codes::NOT_JSON);
    assert_eq!(bad(&good.replace("\"ask\":", "\"asks\":")), codes::MEMBER);
    assert_eq!(bad(&good.replace("{\"id\":", "{\"x\":1,\"id\":")), codes::MEMBER);
    assert_eq!(bad(&good.replace("\"tier\":1", "\"tier\":\"1\"")), codes::MEMBER);
    assert_eq!(bad(&good.replace("\"tier\":1", "\"tier\":1.5")), codes::MEMBER);
    assert_eq!(bad(&good.replace("\"tier\":1", "\"tier\":1000")), codes::MEMBER);
    assert_eq!(bad(&good.replace("\"ref\":", "\"check\":\"\",\"ref\":")), codes::MEMBER);
    assert_eq!(bad(&good.replace("\"hand\"", "\" \"")), codes::PROVENANCE);
    assert_eq!(bad(&good.replace("0123456789abcdef", "0123456789ABCDEF")), codes::PROVENANCE);
    assert_eq!(bad(&good.replace("2026-10-05", "2026-10-5")), codes::PROVENANCE);
    // A suite: a task a line, each ended; no id twice, no blank line.
    let two = write_suite(&[counter(), Task { id: "other".into(), ..counter() }]);
    assert_eq!(write_suite(&read_suite(&two).unwrap()), two);
    let twice = write_suite(&[counter(), counter()]);
    assert_eq!(read_suite(&twice).unwrap_err().code, codes::DUPLICATE);
    let gap = [good.as_str(), "", good.as_str()].join("\n");
    let e = read_suite(&gap).unwrap_err();
    assert!(e.code == codes::BLANK && e.message.starts_with("line 2"), "{e}");
    // Answers: other members let be.
    let answers = "{\"task\":\"t\",\"model\":\"m\",\"reply\":\"r\",\"ms\":12}\n";
    let want = suite::Answer { task: "t".into(), model: "m".into(), reply: "r".into() };
    assert_eq!(read_answers(answers), Ok(vec![want]));
    assert_eq!(read_answers("{\"task\":\"t\",\"model\":\"m\"}").unwrap_err().code, codes::MEMBER);
}

#[test]
fn the_split_is_derived_from_the_family() {
    assert_eq!((fnv(b""), fnv(b"a")), (0xcbf2_9ce4_8422_2325, 0xaf63_dc4c_8601_ec8c));
    assert_eq!(held("counter"), fnv(b"counter") % 5 == 0);
    let families = ["counter", "tip", "stopwatch", "todo", "snake", "memory", "paint", "2048"];
    let out: Vec<bool> = families.iter().map(|f| held(f)).collect();
    assert_eq!(out, [true, false, false, false, false, false, false, false]);
    // About a fifth of all families.
    let n = (0..1000).filter(|i| held(&format!("family-{i}"))).count();
    assert!((150..250).contains(&n), "{n}");
    let v = verifier_hash();
    assert!(v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
}

#[test]
fn a_tally_counts_by_tier_split_and_stage() {
    let (t, mut held_t) = (Task { family: "tip".into(), ..counter() }, counter());
    held_t.family = (0..).map(|i| format!("f{i}")).find(|f| held(f)).unwrap();
    held_t.tier = 3;
    let mut tally = Tally::default();
    let pass = Grade { stage: Stage::Pass, code: 0, message: String::new() };
    let check = Grade { stage: Stage::Check, code: 723, message: String::new() };
    let harness = Grade { stage: Stage::Harness, code: 213, message: String::new() };
    for (task, g) in [(&t, &pass), (&t, &check), (&held_t, &pass), (&held_t, &harness)] {
        tally.add(task, g);
    }
    assert_eq!(tally.total(), (2, 3));
    assert_eq!((tally.tiers[0], tally.tiers[2]), ((1, 2), (1, 1)));
    assert_eq!(tally.split, [(1, 2), (1, 1)]);
    assert_eq!(tally.stages, [0, 0, 0, 1, 1, 2]);
    let report = tally.report();
    assert!(report.contains("pass 2/3 66%") && report.contains("1: 1/2 50%"), "{report}");
    assert!(report.contains("train 1/2 50%  held 1/1 100%") && report.contains("harness 1"));
}

/// The committed suite (`evals/suites/iq.jsonl`): it reads, every task verifies (each kill rate
/// and grade time said), it spans tiers 1 to 4 and six families or more, and it is written as
/// the writer writes it, byte for byte.
#[test]
fn the_committed_suite_verifies_and_round_trips() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../", "evals/suites/iq.jsonl");
    let text = std::fs::read_to_string(path).unwrap();
    let tasks = read_suite(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(write_suite(&tasks), text, "the suite round-trips byte for byte");
    let mut bad = Vec::new();
    for t in &tasks {
        let t0 = std::time::Instant::now();
        let g = grade(t, &t.reference);
        let took = t0.elapsed();
        match verify(t) {
            Ok(v) => eprintln!(
                "{:<10} tier {} kill {}/{} grade {took:?}",
                t.id, t.tier, v.killed, v.counted
            ),
            Err(e) => bad.push(format!("{}: {e}", t.id)),
        }
        assert!(g.pass(), "{}: {g:?}", t.id);
    }
    assert!(bad.is_empty(), "{bad:#?}");
    let tiers: std::collections::BTreeSet<u8> = tasks.iter().map(|t| t.tier).collect();
    let families: std::collections::BTreeSet<&str> =
        tasks.iter().map(|t| t.family.as_str()).collect();
    assert!(tasks.len() >= 8 && families.len() >= 6, "{} tasks, {families:?}", tasks.len());
    assert!((1..=4).all(|t| tiers.contains(&t)), "{tiers:?}");
    assert!(families.iter().any(|f| held(f)) && families.iter().any(|f| !held(f)));
}
