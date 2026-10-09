use super::*;

/// A counter as GLM wrote one for the IQ suite.
const COUNTER: &str = "// Counter: + adds one, - takes one away, below 0 too.
// icon: line 5 12 19 12 line 12 5 12 19
state n = 0;

label \"Counter\";
label n;
row {
  button \"-\" { n -= 1; }
  button \"+\" { n += 1; }
}
";

/// The counter with its state's semicolon gone: it does not compile.
fn broken() -> String {
    COUNTER.replace("state n = 0;", "state n = 0")
}

/// An edit block from `search` to `replace`.
fn edit(search: &str, replace: &str) -> String {
    ["<<<<<<< SEARCH\n", search, "\n=======\n", replace, "\n>>>>>>> REPLACE\n"].concat()
}

fn ask(next: &Next) -> &str {
    match next {
        Next::Ask(body) => body,
        Next::Done(f) => panic!("over: {f:?}"),
    }
}

fn done(next: Next) -> Fixed {
    match next {
        Next::Done(f) => f,
        Next::Ask(_) => panic!("still asking"),
    }
}

#[test]
fn a_draft_that_runs_clean_needs_no_helper() {
    let (_, next) = Repair::start("A counter", "", COUNTER, "q3", 3);
    let f = done(next);
    assert!(f.clean && f.steps.is_empty() && f.src == COUNTER);
}

#[test]
fn a_helper_repairs_a_draft_with_edits_and_the_test_judges_it() {
    let (mut r, next) = Repair::start("A counter", "", &broken(), "q3", 3);
    let body = ask(&next);
    // Asked as Studio asks a fix: the helper's model, the fault's account, edit blocks wanted.
    assert!(body.starts_with("{\"model\":\"q3\",\"stream\":true"), "{body}");
    assert!(body.contains("did not compile") && body.contains("Reply with edit blocks."));
    assert!(body.contains("\"max_tokens\":4096,\"temperature\":0.2"));
    let f = done(r.reply(&edit("state n = 0", "state n = 0;"), false));
    assert!(f.clean && f.src == COUNTER, "{f:?}");
    assert_eq!(f.steps.len(), 1);
    assert_eq!((f.steps[0].held, f.steps[0].after), (Held::Edits, 0));
    assert!(f.steps[0].before != 0);
}

#[test]
fn edits_that_miss_are_asked_again_with_why_and_the_turns_run_out() {
    let (mut r, _) = Repair::start("A counter", "", &broken(), "q3", 2);
    let next = r.reply(&edit("state m = 9", "state m = 9;"), false);
    assert!(ask(&next).contains("Your edits did not apply"));
    // The second and last turn holds nothing: the draft is the best there is, not clean.
    let f = done(r.reply("I am not sure.", false));
    assert!(!f.clean && f.src == broken());
    let held: Vec<Held> = f.steps.iter().map(|s| s.held).collect();
    assert_eq!(held, [Held::Missed, Held::Nothing]);
}

#[test]
fn a_whole_program_counts_but_one_that_drops_most_of_the_draft_does_not() {
    let (mut r, _) = Repair::start("A counter", "", &broken(), "q3", 3);
    // Shorter by far: refused though it would run clean, and the helper is asked again.
    let tiny = "```app\n// Counter.\nstate n = 0;\nlabel n;\n```";
    let next = r.reply(tiny, false);
    assert!(ask(&next).contains("did not compile"));
    let whole = ["```app\n", COUNTER, "```"].concat();
    let f = done(r.reply(&whole, false));
    assert!(f.clean && f.src.trim_end() == COUNTER.trim_end(), "{f:?}");
    assert_eq!(f.steps[1].held, Held::Program);
}
