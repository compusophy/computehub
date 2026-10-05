//! The words teach sends. The solver's and the fixer's are the coder's own ([`coder::prompt`]),
//! byte for byte as Studio sends them: the system prompt, then `Make: <ask>` for a new app, or
//! the fix turn (the ask, the program numbered, the coder's account of its fault) for a program
//! that faults. So a reply that passes is training data in the very format the trained model
//! will be asked in. The task writer's is teach's own; its hash (`by.prompt`) is pinned by a
//! test, so it changes only by decision.

use crate::seam::Task;

/// Who the writer writes for, before the coder's prompt.
const WRITER: &str = "You write tasks for IQ, a benchmark of how well a model makes apps in \
applang for Studio, the app maker of compusophyOS. A task is what a person asks Studio for, a \
checker that drives the app made and reads what it shows, and a reference program that passes \
the checker. Models are measured, and later trained, on these tasks, so each must be fair \
(every app that does what its ask says passes), discerning (an app that leaves part of it out \
fails) and unambiguous.\n\nThe model measured gets exactly this system prompt, then \"Make: \" \
and the ask:\n\n<model_prompt>\n";

/// Then the checker language's card.
const CHECKS: &str = "</model_prompt>\n\nCheckers are scripts in IQ's checker language:\n\n\
<checker_language>\n";

/// Then the tiers, the parts of a task and the reply's form.
const LADDER: &str = "</checker_language>\n\nThe tiers, from the simplest:\n\
1. A counter, or as small: a state or two and a button or two.\n\
2. A small tool: a timer, a converter, a todo list.\n\
3. A simple game or a richer tool: snake, memory, paint.\n\
4. A full classic game: tetris, 2048, minesweeper.\n\
5. A game with levels, opponents or physics.\n\
6. Ambitious: several of these combined in one app.\n\n\
The ask is what a person would type, then the observable behavior the checker relies on, said \
plainly and precisely: labels and buttons by their words, the keys, a board's size, what shows \
when, numbers that can be checked. For example: a counter: a label shows the count, starting \
at 0; a button labelled + adds one and a button labelled - takes one away. Describe only what \
is seen and done, never state names or code. Where the checker accepts alternatives (a button \
or a key; buttons or swatches), the ask names them; the look is otherwise free.\n\n\
The check is a script in the checker language that tests what the ask says and nothing it does \
not: it passes the ref, and fails an app that ignores part of the ask.\n\n\
The ref is a complete applang program as the measured model should reply with it, without its \
fence: its first comment, its icon line, then the app. It must compile, run clean through \
applang's smoke test (rendered, clicked, ticked, keyed, tapped and typed into on several seeds, \
closed and opened again), draw its icon and pass the check, in under 200 lines.\n\n\
Reply with one JSON object per line and nothing else (no fence, no prose), a line for each \
task:\n{\"id\":\"<family>-<a word or two>\",\"tier\":<tier>,\"family\":\"<family>\",\
\"ask\":\"...\",\"check\":\"...\",\"ref\":\"...\"}\nAn id is lowercase a-z, 0-9 and -, and \
begins with its family and -. Each task of a family is a different app or variant (snake that \
wraps around the edges; snake with walls and a speed that grows). Strings are JSON strings: a \
newline is \\n, a quote \\\".\n";

/// The task writer's system prompt, given the checker language's card (`iq::CARD`): who it
/// writes for, the coder's whole system prompt (applang's card and example replies among it:
/// [`applang::REFERENCE`], [`applang::SHOTS`]), the card, the tier ladder and the reply's form.
/// Nothing in it varies but the card, so the API caches it across a run.
pub fn writer(card: &str) -> String {
    [WRITER, &coder::prompt::system(), CHECKS, card.trim_end(), "\n", LADDER].concat()
}

/// The writer's request: `per` tasks of tier `tier` in `family`, none an id `taken`.
pub fn writer_user(tier: u8, family: &str, per: u32, taken: &[String]) -> String {
    let mut out = format!("Tier {tier}, family {family}: write {per} tasks.");
    if !taken.is_empty() {
        out += " Ids taken already (use none again, nor make the same app): ";
        out += &taken.join(", ");
        out.push('.');
    }
    out
}

/// The tasks in a writer's reply: each line that begins `{` read as one (its `by` left empty;
/// a ref given in an `app` fence taken out of it), else why not with the line.
pub fn tasks_in(reply: &str) -> Vec<Result<Task, (String, String)>> {
    let lines = reply.lines().map(str::trim).filter(|l| l.starts_with('{'));
    let task = |l: &str| match Task::parse(l) {
        Some(mut t) => {
            if let Some((src, true)) = coder::edits::program(&t.reference) {
                t.reference = src.to_string();
            }
            Ok(t)
        }
        None => Err(("a line that does not read as a task".into(), l.to_string())),
    };
    lines.map(task).collect()
}

/// The solver's system prompt: the coder's, byte for byte.
pub fn solver() -> String {
    coder::prompt::system()
}

/// The solver's request for a new app: the coder's first turn, byte for byte.
pub fn solve(ask: &str) -> String {
    coder::prompt::write(ask)
}

/// The fixer's request for `src`, made for `ask`: the coder's fix turn with the account it
/// builds of the program's fault ([`coder::ai::fault`], on Studio's seeds, from no saved
/// states, as a new app starts); none when it finds no fault.
pub fn fix(ask: &str, src: &str) -> Option<String> {
    let f = coder::ai::fault(src, "", coder::Knobs::default().seeds)?;
    Some(coder::prompt::fix(ask, src, &f.account))
}
