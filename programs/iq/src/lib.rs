//! # iq: how well a model makes apps, measured
//!
//! Suite 2 of compusophyOS's evals: an app-making IQ benchmark whose tasks rise in tiers (1 a
//! counter; 2 a small tool; 3 a simple game or a richer tool; 4 a full classic game with its
//! rules; 5 levels, opponents or physics; 6 several of those combined) and whose checkers are
//! data: each task's check is a script in a small language ([`script`], its card [`CARD`]) that
//! drives the app made headlessly and says what must show. So a teacher model can write tasks,
//! and a verifier can judge them, mechanically:
//!
//! - [`grade`] grades a program for a task in stages, as Suite 1 does: it compiles
//!   (`applang::compile`), runs clean through the coder's smoke test on seeds 1 to 3
//!   (`coder::ai::fault`, as Studio checks a make; an icon line is not graded), and passes the
//!   check on seeds 1, 2 and 3. Deterministic and total: it is also the reward a model is trained
//!   on.
//! - [`verify`] keeps a task only if its reference program passes, a null app (one label) fails
//!   its check, and its check kills at least half (and at least 3) of the reference's mutants
//!   that count ([`mutants`]: a line dropped, a number made one more; one counts if it compiles,
//!   runs clean and is killed, or can be told from the reference by a fixed exploration that
//!   clicks, keys, taps, types and lets time pass, [`mutate::differs`]: an equivalent mutant is
//!   no slip): a check with no teeth is refused, and the refusal names the mutants it passes.
//!   Every refusal is coded.
//! - The suite is `evals/suites/iq.jsonl`, a task a line ([`suite`]), each with its provenance
//!   (who made it, from what prompt, under which verifier ([`verifier_hash`]), on what day).
//!   Families are held out, never wordings: [`held`] derives the split from a family's name, so
//!   it is never stored and never drifts.
//!
//! Codes, banded: E0701 to E0709 a check that does not read; E0721 to E0724 a program its check
//! fails; E0741 to E0749 a task refused; E0761 to E0766 a file that does not read ([`codes`]).
//! The dev binary `iq` verifies the suite, grades a program, scores a model's answers and prints
//! the split and the card.

#![forbid(unsafe_code)]

pub mod mutate;
pub mod script;
pub mod suite;
#[cfg(test)]
mod tests;

pub use mutate::{MAX_MUTANTS, Mutant, mutants};
pub use script::CARD;
pub use suite::{read_answers, read_suite, read_task, write_suite, write_task};

/// The stable codes of what iq refuses or fails, banded by what it is about.
pub mod codes {
    /// A statement that is no statement.
    pub const UNKNOWN: u16 = 701;
    /// A string with no closing quote, or an escape other than `\"` and `\\`.
    pub const STRING: u16 = 702;
    /// A statement's arguments missing, extra or of the wrong kind; a blank label.
    pub const ARGS: u16 = 703;
    /// A number out of its range: a square off its board, a wait past the limit.
    pub const RANGE: u16 = 704;
    /// A key no app can handle.
    pub const KEY: u16 = 705;
    /// A repeat without its colon, or of a repeat, a mark or an expect.
    pub const REPEAT: u16 = 706;
    /// A script that expects nothing.
    pub const NO_EXPECT: u16 = 707;
    /// A script past its limits: bytes, statements, actions or waits.
    pub const TOO_BIG: u16 = 708;
    /// `expect changed` or `expect same` with no mark before it.
    pub const NO_MARK: u16 = 709;
    /// The app faults as it first shows.
    pub const STARTS: u16 = 721;
    /// An action that could not be done (what it needs does not show) or that faulted the app.
    pub const ACTION: u16 = 722;
    /// An expectation that does not hold.
    pub const EXPECTED: u16 = 723;
    /// The check panicked: the harness's fault, never the program's.
    pub const PANICKED: u16 = 724;
    /// A task's id is not 1 to 48 of a-z, 0-9 and -.
    pub const BAD_ID: u16 = 741;
    /// A task's family is not 1 to 48 of a-z, 0-9 and -.
    pub const BAD_FAMILY: u16 = 742;
    /// A task's tier is not 1 to 6.
    pub const BAD_TIER: u16 = 743;
    /// A task asks nothing.
    pub const NO_ASK: u16 = 744;
    /// A task's check does not read.
    pub const BAD_CHECK: u16 = 745;
    /// A task's reference program fails its grade.
    pub const REF_FAILS: u16 = 746;
    /// A null app (one label) passes a task's check.
    pub const NULL_PASSES: u16 = 747;
    /// Fewer than [`super::MIN_COUNTED`] mutants of the reference compile and run clean.
    pub const FEW_MUTANTS: u16 = 748;
    /// A check kills fewer than half the counted mutants: it has no teeth.
    pub const TOOTHLESS: u16 = 749;
    /// A line that is not one JSON object.
    pub const NOT_JSON: u16 = 761;
    /// A member missing, unknown, given twice or of the wrong kind.
    pub const MEMBER: u16 = 762;
    /// Provenance that does not read: a verifier not 16 hex digits, a day not YYYY-MM-DD, no
    /// teacher.
    pub const PROVENANCE: u16 = 763;
    /// Two tasks with one id.
    pub const DUPLICATE: u16 = 764;
    /// An answer to a task the suite does not have.
    pub const NO_TASK: u16 = 765;
    /// A blank line.
    pub const BLANK: u16 = 766;
}

/// A coded failure: its code (see [`codes`], or applang's) and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coded {
    pub code: u16,
    pub message: String,
}

/// Why [`verify`] refuses a task, or a file does not read.
pub type Refusal = Coded;

impl Coded {
    pub fn new(code: u16, message: impl Into<String>) -> Coded {
        Coded { code, message: message.into() }
    }
}

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{:04} {}", self.code, self.message)
    }
}

/// Who made a task and how: the teacher (`hand`, or a model's name), the prompt it was asked
/// with ("" if none), the verifier that kept it ([`verifier_hash`]) and the day (YYYY-MM-DD).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct By {
    pub teacher: String,
    pub prompt: String,
    pub verifier: String,
    pub day: String,
}

/// One task: its id and family (a-z, 0-9, -), its tier (1 to 6), what a person asks for (with
/// the behavior its check relies on), its check (a [`script`]), its reference program (`ref` in
/// the file) and its provenance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub tier: u8,
    pub family: String,
    pub ask: String,
    pub check: String,
    pub reference: String,
    pub by: By,
}

/// How far a program got: the first stage it failed, or [`Stage::Pass`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Scoring's: the reply held no `app` block.
    Reply,
    /// It does not compile.
    Compile,
    /// It faults in the smoke test.
    Smoke,
    /// What grades it is at fault (a smoke test's bad event, a check that does not read or
    /// panicked): neither a pass nor a fail, counted apart.
    Harness,
    /// It fails its check.
    Check,
    Pass,
}

impl Stage {
    /// Every stage, in order.
    pub const ALL: [Stage; 6] =
        [Stage::Reply, Stage::Compile, Stage::Smoke, Stage::Harness, Stage::Check, Stage::Pass];

    /// Its name: `reply`, `compile`, `smoke`, `harness`, `check` or `pass`.
    pub fn name(self) -> &'static str {
        ["reply", "compile", "smoke", "harness", "check", "pass"][self as usize]
    }
}

/// A program's grade: the stage it reached, the code of what stopped it (0 if it passed) and
/// what was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grade {
    pub stage: Stage,
    pub code: u16,
    pub message: String,
}

impl Grade {
    pub fn pass(&self) -> bool {
        self.stage == Stage::Pass
    }

    fn fail(stage: Stage, code: u16, message: &str) -> Grade {
        Grade { stage, code, message: coder::ai::clip(message, 400) }
    }
}

/// How `src` fares at `task` ([`Grade`]): it compiles, runs clean through the smoke test on
/// seeds 1 to 3, and passes the task's check on seeds 1, 2 and 3. A check that does not read
/// is the task's fault, graded [`Stage::Harness`].
pub fn grade(task: &Task, src: &str) -> Grade {
    match script::parse(&task.check) {
        Ok(s) => grade_script(&s, src),
        Err(e) => {
            Grade::fail(Stage::Harness, e.code, &["its check does not read: ", &e.message].concat())
        }
    }
}

/// [`grade`] with the check read.
pub fn grade_script(s: &script::Script, src: &str) -> Grade {
    if let Err(d) = applang::compile(src) {
        return Grade::fail(Stage::Compile, d.code.unwrap_or(0), &coder::ai::problem(&d, src));
    }
    // A fault that `runs` is its icon line's alone, which is the desktop's, not graded here.
    if let Some(f) = coder::ai::fault(src, "", 3).filter(|f| !f.runs) {
        let code = f.diag.code.unwrap_or(0);
        let stage = if code == applang::codes::BAD_EVENT { Stage::Harness } else { Stage::Smoke };
        return Grade::fail(stage, code, &f.said);
    }
    // The check keeps from panicking; one that does is the harness's fault, caught where panics
    // unwind.
    match std::panic::catch_unwind(|| script::check(s, src)) {
        Ok(Ok(())) => Grade { stage: Stage::Pass, code: 0, message: String::new() },
        Ok(Err(e)) => Grade::fail(Stage::Check, e.code, &e.message),
        Err(_) => Grade::fail(Stage::Harness, codes::PANICKED, "the check panicked"),
    }
}

/// The program in a model's `reply`, as the coder takes it (`coder::edits::program`: the first
/// closed `app` block that begins with a comment, else the longest), and the grade it earns at
/// `task`; [`Stage::Reply`] if there is none.
pub fn grade_reply(task: &Task, reply: &str) -> Grade {
    match coder::edits::program(reply) {
        Some((src, _)) => grade(task, src),
        None => Grade::fail(Stage::Reply, 0, "the reply holds no app block"),
    }
}

/// The null app every check must fail: one that shows a label and does nothing.
pub const NULL: &str = "// Hello.\nlabel \"Hello\";\n";
/// The fewest mutants that must count for a check's teeth to be judged.
pub const MIN_COUNTED: usize = 3;

/// What [`verify`] found among the reference's [`mutants`]: how many were made; how many counted
/// (compiled, ran clean, and either failed the check or can be told from the reference by
/// [`mutate::differs`]); how many of those the check killed; how many passed it but showed just
/// as the reference does, so are no slip (equivalent); and what each counted one that passed
/// changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub made: usize,
    pub counted: usize,
    pub killed: usize,
    pub equivalent: usize,
    pub survivors: Vec<String>,
}

impl Verified {
    /// The kill rate in whole percent.
    pub fn percent(&self) -> usize {
        self.killed * 100 / self.counted.max(1)
    }
}

/// Whether `s` is 1 to 48 of a-z, 0-9 and `-`.
fn slug(s: &str) -> bool {
    (1..=48).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Keeps a task a teacher made, or says why not: its id and family are slugs, its tier 1 to 6,
/// it asks something, its check reads, its reference passes [`grade`], the [`NULL`] app fails its
/// check, and its check kills at least half of the reference's [`mutants`] that count (they
/// compile and run clean, and are killed or can be told from the reference: an equivalent
/// mutant is no slip), of which there are at least [`MIN_COUNTED`].
pub fn verify(task: &Task) -> Result<Verified, Refusal> {
    let refuse = |code: u16, why: String| Err(Coded::new(code, why));
    if !slug(&task.id) {
        return refuse(
            codes::BAD_ID,
            format!("the id `{}` is not 1 to 48 of a-z, 0-9, -", task.id),
        );
    }
    if !slug(&task.family) {
        let why = format!("the family `{}` is not 1 to 48 of a-z, 0-9, -", task.family);
        return refuse(codes::BAD_FAMILY, why);
    }
    if !(1..=6).contains(&task.tier) {
        return refuse(codes::BAD_TIER, format!("the tier is {}, not 1 to 6", task.tier));
    }
    if task.ask.trim().is_empty() {
        return refuse(codes::NO_ASK, "it asks nothing".into());
    }
    let s = script::parse(&task.check)
        .map_err(|e| Coded::new(codes::BAD_CHECK, format!("its check does not read: {e}")))?;
    let g = grade_script(&s, &task.reference);
    if !g.pass() {
        let why = format!("its ref fails at {}: E{:04} {}", g.stage.name(), g.code, g.message);
        return refuse(codes::REF_FAILS, why);
    }
    if script::check(&s, NULL).is_ok() {
        return refuse(codes::NULL_PASSES, "an app that shows one label passes its check".into());
    }
    let all = mutants(&task.reference);
    let (mut counted, mut killed, mut equivalent, mut survivors) = (0, 0, 0, Vec::new());
    for m in &all {
        match grade_script(&s, &m.src).stage {
            Stage::Check => (counted, killed) = (counted + 1, killed + 1),
            Stage::Pass if mutate::differs(&task.reference, &m.src) => {
                counted += 1;
                survivors.push(m.what.clone());
            }
            Stage::Pass => equivalent += 1,
            _ => {}
        }
    }
    let v = Verified { made: all.len(), counted, killed, equivalent, survivors };
    if counted < MIN_COUNTED {
        let why = format!(
            "{counted} of its ref's {} mutants count (compile, run clean and differ from it); \
             {MIN_COUNTED} must",
            all.len()
        );
        return refuse(codes::FEW_MUTANTS, why);
    }
    if killed * 2 < counted {
        let why = format!(
            "its check kills {killed} of {counted} mutants of its ref ({}%); half must fail. It \
             passes: {}",
            v.percent(),
            v.survivors.join("; ")
        );
        return refuse(codes::TOOTHLESS, why);
    }
    Ok(v)
}

/// FNV-1a 64 of `bytes`.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Whether the family `family` is held out: never trained on, only measured. FNV-1a 64 of its
/// name, mod 5, is 0: about a fifth of the families, the same on every machine and every day.
pub fn held(family: &str) -> bool {
    fnv(family.as_bytes()) % 5 == 0
}

/// The verifier's hash, 16 hex digits: of this crate's code and that of every crate it stands
/// on (the probe, the coder's smoke test, applang), as its build script reads them. A change to
/// the checker language, the grade or the smoke test is a new verifier.
pub fn verifier_hash() -> String {
    env!("IQ_VERIFIER").to_string()
}

/// Passes and grades by tier, by split and by stage: a model's answers, scored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Per tier 1 to 6: passed, graded.
    pub tiers: [(u32, u32); 6],
    /// Trained-on families, then held-out ones: passed, graded.
    pub split: [(u32, u32); 2],
    /// How many reached each of [`Stage::ALL`].
    pub stages: [u32; 6],
}

impl Tally {
    /// Counts `g`, a grade at `task`. A [`Stage::Harness`] grade is counted apart, never graded.
    pub fn add(&mut self, task: &Task, g: &Grade) {
        self.stages[g.stage as usize] += 1;
        if g.stage == Stage::Harness {
            return;
        }
        let pass = u32::from(g.pass());
        let tier = &mut self.tiers[usize::from(task.tier.clamp(1, 6) - 1)];
        *tier = (tier.0 + pass, tier.1 + 1);
        let split = &mut self.split[usize::from(held(&task.family))];
        *split = (split.0 + pass, split.1 + 1);
    }

    /// Passed and graded, in all.
    pub fn total(&self) -> (u32, u32) {
        (self.split[0].0 + self.split[1].0, self.split[0].1 + self.split[1].1)
    }

    /// The tally as lines of text: the pass rate in all, by tier, by split and by stage.
    pub fn report(&self) -> String {
        let rate = |(p, n): (u32, u32)| match n {
            0 => "-".to_string(),
            n => format!("{p}/{n} {}%", p * 100 / n),
        };
        let mut out = format!("  pass {}\n  tiers", rate(self.total()));
        for (i, t) in self.tiers.iter().enumerate().filter(|t| t.1.1 > 0) {
            out += &format!("  {}: {}", i + 1, rate(*t));
        }
        out +=
            &format!("\n  train {}  held {}\n  stages", rate(self.split[0]), rate(self.split[1]));
        for (s, n) in Stage::ALL.iter().zip(self.stages) {
            out += &format!("  {} {n}", s.name());
        }
        out.push('\n');
        out
    }
}
