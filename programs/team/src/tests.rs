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
fn another_try_samples_at_its_own_temperature() {
    let (_, next) = Repair::start_at("A counter", "", &broken(), "q3", 3, "0.7");
    assert!(ask(&next).contains("\"max_tokens\":4096,\"temperature\":0.7"));
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

#[test]
fn a_unified_diff_in_any_of_its_shapes_is_read_as_the_edit_blocks_it_means() {
    // Bare lines, a git diff's headers and hunk, and a hunk with the program's line numbers.
    let bare = "```diff\n- state n = 0\n+ state n = 0;\n```";
    let git = "```diff\ndiff --git a/app b/app\nindex 1..2 100644\n--- a/app\n+++ b/app\n\
               @@ -1,3 +1,3 @@ fn x\n // icon: line 5 12 19 12 line 12 5 12 19\n-state n = 0\n\
               +state n = 0;\n \n```";
    let numbered = "```diff\n@@ -3,1 +3,1 @@\n 2| // icon: line 5 12 19 12 line 12 5 12 19\n 3|-state n = 0\n 3|+state n = 0;\n```";
    for reply in [bare, git, numbered] {
        let (mut r, _) = Repair::start("a counter", "", &broken(), "base3b", 3);
        let f = done(r.reply(reply, false));
        assert!(f.clean, "{reply}: {f:?}");
        assert_eq!(f.steps[0].held, Held::Diff, "{reply}");
        assert_eq!(f.src.trim_end(), COUNTER.trim_end());
    }
    // No diff fence, or a hunk with nothing to find: nothing held.
    assert_eq!(diff::blocks("just words"), None);
    assert_eq!(diff::blocks("```diff\n+ only an added line\n```"), None);
    // A diff whose lines are not in the program: asked again with why.
    let (mut r, _) = Repair::start("a counter", "", &broken(), "base3b", 3);
    let again = r.reply("```diff\n- state m = 9\n+ state m = 9;\n```", false);
    assert!(ask(&again).contains("SEARCH"), "{again:?}");
    assert_eq!(r.last().map(|s| s.held), Some(Held::Missed));
}

#[test]
fn a_whole_program_that_is_another_program_is_not_a_fix() {
    // An example app in place of the draft: it runs clean, and is refused all the same.
    let other = "```app\n// Snake: eat and grow.\n// icon: ring 12 12 9\nstate len = 3;\nlabel \"Snake\";\nlabel \"Length \" + len;\nbutton \"Grow\" { len += 1; }\nbutton \"Reset\" { len = 3; }\nlabel \"Go\";\nlabel \"Eat\";\n```";
    let (mut r, _) = Repair::start("a counter", "", &broken(), "base05", 1);
    let f = done(r.reply(other, false));
    assert!(!f.clean && f.src == broken(), "{f:?}");
    // The draft rewritten whole but itself: kept.
    let (mut r, _) = Repair::start("a counter", "", &broken(), "base05", 1);
    let f = done(r.reply(&["```app\n", COUNTER, "```"].concat(), false));
    assert!(f.clean && f.steps[0].held == Held::Program, "{f:?}");
}

#[test]
fn a_one_line_block_quoting_part_of_one_line_edits_that_line() {
    // The icon line quoted without its `// icon:`: applied within the line.
    let draft = COUNTER.replace("line 12 5 12 19", "line 12 5 12 30");
    let (mut r, _) = Repair::start("a counter", "", &draft, "base3b", 2);
    let f = done(
        r.reply(&edit("line 5 12 19 12 line 12 5 12 30", "line 5 12 19 12 line 12 5 12 19"), false),
    );
    assert_eq!(f.steps[0].held, Held::Inline, "{f:?}");
    assert!(f.clean && f.src.trim_end() == COUNTER.trim_end(), "{f:?}");
    // Part of two lines, or of none: still a miss.
    let edits = [coder::edits::Edit { search: vec!["n".into()], replace: vec!["m".into()] }];
    assert_eq!(inline(COUNTER, &edits), None);
}
