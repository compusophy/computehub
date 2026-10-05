//! `iq`, the IQ benchmark's commands (a dev tool; paths default to the repository's):
//!
//! - `verify [file] [--stamp DAY] [--survivors]`: verifies every task of the suite
//!   (`evals/suites/iq.jsonl`): its kill rate and how long its reference takes to grade, or why it
//!   is refused; with `--survivors`, what each mutant its check passes changed. With `--stamp`,
//!   when every task verifies, each one's provenance is stamped with this verifier and the day,
//!   and the file written again.
//! - `grade <task-id> <program-file> [--suite file]`: the grade, as a JSON line.
//! - `score <answers.jsonl> [--suite file] [--each]`: each answer graded (its program taken as
//!   the coder takes it), then per model the pass rate by tier, by split and by stage; with
//!   `--each`, a JSON line per answer first.
//! - `split [file]`: each family, held out or trained on, and its tasks.
//! - `card`: the card a teacher writing checks is prompted with.
//! - `add --id --tier --family --ask --check FILE --ref FILE --day [--teacher hand] [--prompt]
//!   [--suite file]`: a task written by hand, verified, stamped and added to the suite.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use coder::json::quote;
use iq::{By, Tally, Task};

const USAGE: &str = "usage: iq verify [file] [--stamp DAY] [--survivors] | grade <task-id> \
                     <program-file> [--suite file] | score <answers.jsonl> [--suite file] \
                     [--each] | split [file] | card | add --id ID --tier N --family F --ask \
                     TEXT --check FILE --ref FILE --day YYYY-MM-DD [--teacher hand] [--prompt \
                     TEXT] [--suite file]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match Args::parse(args.get(1..).unwrap_or(&[])) {
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            2
        }
        Ok(a) => match args.first().map(String::as_str) {
            Some("verify") => verify(&a),
            Some("grade") => grade(&a),
            Some("score") => score(&a),
            Some("split") => split(&a),
            Some("card") => {
                println!("{}", iq::CARD);
                0
            }
            Some("add") => add(&a),
            _ => {
                eprintln!("{USAGE}");
                2
            }
        },
    };
    std::process::exit(code);
}

/// The words given and the `--key value` options (`--each` and `--survivors` take none).
struct Args {
    words: Vec<String>,
    opts: BTreeMap<String, String>,
}

/// The options known.
const KEYS: [&str; 11] =
    ["stamp", "suite", "id", "tier", "family", "ask", "check", "ref", "day", "teacher", "prompt"];

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let (mut words, mut opts, mut i) = (Vec::new(), BTreeMap::new(), 0);
        while i < args.len() {
            match args[i].strip_prefix("--") {
                Some(k @ ("each" | "survivors")) => _ = opts.insert(k.into(), String::new()),
                Some(k) if KEYS.contains(&k) => {
                    let v = args.get(i + 1).ok_or_else(|| format!("--{k} needs a value"))?;
                    opts.insert(k.to_string(), v.clone());
                    i += 1;
                }
                Some(k) => return Err(format!("--{k}: not an option")),
                None => words.push(args[i].clone()),
            }
            i += 1;
        }
        Ok(Args { words, opts })
    }

    fn opt(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }

    /// The suite's path: `--suite`, else the `n`th word, else the repository's.
    fn suite(&self, n: usize) -> PathBuf {
        let given = self.opt("suite").or(self.words.get(n).map(String::as_str));
        given.map_or_else(|| root().join(iq::suite::SUITE), PathBuf::from)
    }
}

/// The repository's root, which this crate is two folders under.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The text of `path`, or what is wrong, said.
fn read(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The suite at `path`, or what is wrong, said.
fn suite_at(path: &std::path::Path) -> Result<Vec<Task>, String> {
    iq::read_suite(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// What a verification found among the mutants, said.
fn kills(v: &iq::Verified) -> String {
    format!(
        "kill {}/{} {}% ({} equivalent, of {} made)",
        v.killed,
        v.counted,
        v.percent(),
        v.equivalent,
        v.made
    )
}

/// Ends a command with what went wrong.
fn fail(why: String) -> i32 {
    eprintln!("{why}");
    1
}

fn ms(t: Instant) -> u128 {
    t.elapsed().as_millis()
}

fn verify(a: &Args) -> i32 {
    let path = a.suite(0);
    let mut tasks = match suite_at(&path) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let now = iq::verifier_hash();
    println!("verifier {now}: {} tasks in {}", tasks.len(), path.display());
    let mut refused = 0;
    for t in &mut tasks {
        let t0 = Instant::now();
        let v = iq::verify(t);
        let took = ms(t0);
        let split = if iq::held(&t.family) { "held" } else { "train" };
        let head = format!("{:<14} tier {}  {:<12} {:<5}", t.id, t.tier, t.family, split);
        match v {
            Ok(v) => {
                let t1 = Instant::now();
                iq::grade(t, &t.reference);
                let grade_ms = ms(t1);
                let stale = match t.by.verifier == now {
                    true => String::new(),
                    false => format!("  (stamped by {})", t.by.verifier),
                };
                println!("{head}  {}  grade {grade_ms} ms  verify {took} ms{stale}", kills(&v));
                if a.opt("survivors").is_some() {
                    v.survivors.iter().for_each(|s| println!("    lives: {s}"));
                }
                if let Some(day) = a.opt("stamp") {
                    t.by.verifier.clone_from(&now);
                    t.by.day = day.to_string();
                }
            }
            Err(e) => {
                refused += 1;
                println!("{head}  REFUSED {e}");
            }
        }
    }
    println!("{} verified, {refused} refused", tasks.len() - refused);
    if refused > 0 {
        return 1;
    }
    if a.opt("stamp").is_some() {
        if let Err(e) = iq::read_suite(&iq::write_suite(&tasks)) {
            return fail(format!("the stamp does not read: {e}"));
        }
        if let Err(e) = std::fs::write(&path, iq::write_suite(&tasks)) {
            return fail(format!("{}: {e}", path.display()));
        }
        println!("stamped {}", path.display());
    }
    0
}

fn grade(a: &Args) -> i32 {
    let (Some(id), Some(file)) = (a.words.first(), a.words.get(1)) else {
        return fail(USAGE.into());
    };
    let tasks = match suite_at(&a.suite(2)) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let Some(task) = tasks.iter().find(|t| t.id == *id) else {
        return fail(format!("E0765 the suite has no task {id}"));
    };
    let src = match read(std::path::Path::new(file)) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let t0 = Instant::now();
    let g = iq::grade(task, &src);
    println!("{}", line(id, "", &g, ms(t0)));
    i32::from(!g.pass())
}

/// A grade as a JSON line.
fn line(task: &str, model: &str, g: &iq::Grade, ms: u128) -> String {
    format!(
        "{{\"task\":{},\"model\":{},\"pass\":{},\"stage\":\"{}\",\"code\":{},\"ms\":{ms},\
         \"message\":{}}}",
        quote(task),
        quote(model),
        g.pass(),
        g.stage.name(),
        g.code,
        quote(&g.message)
    )
}

fn score(a: &Args) -> i32 {
    let Some(file) = a.words.first() else { return fail(USAGE.into()) };
    let tasks = match suite_at(&a.suite(1)) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let answers = match read(std::path::Path::new(file))
        .and_then(|t| iq::read_answers(&t).map_err(|e| format!("{file}: {e}")))
    {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let mut models: BTreeMap<String, (Tally, u128)> = BTreeMap::new();
    for ans in &answers {
        let Some(task) = tasks.iter().find(|t| t.id == ans.task) else {
            return fail(format!(
                "E0765 an answer is to {}, a task the suite does not have",
                ans.task
            ));
        };
        let t0 = Instant::now();
        let g = iq::grade_reply(task, &ans.reply);
        let took = ms(t0);
        if a.opt("each").is_some() {
            println!("{}", line(&ans.task, &ans.model, &g, took));
        }
        let m = models.entry(ans.model.clone()).or_default();
        m.0.add(task, &g);
        m.1 += took;
    }
    for (model, (tally, took)) in &models {
        let n: u32 = tally.stages.iter().sum();
        println!("{model}: {n} answers, {} ms a grade", took / u128::from(n.max(1)));
        print!("{}", tally.report());
    }
    0
}

fn split(a: &Args) -> i32 {
    let tasks = match suite_at(&a.suite(0)) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let mut families: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for t in &tasks {
        families.entry(&t.family).or_default().push(&t.id);
    }
    let mut count = [(0, 0); 2];
    for (f, ids) in &families {
        let held = iq::held(f);
        let c = &mut count[usize::from(held)];
        *c = (c.0 + 1, c.1 + ids.len());
        println!("{:<14} {:<5} {}", f, if held { "held" } else { "train" }, ids.join(" "));
    }
    println!(
        "train: {} families, {} tasks; held: {} families, {} tasks",
        count[0].0, count[0].1, count[1].0, count[1].1
    );
    0
}

fn add(a: &Args) -> i32 {
    let need = |k: &str| a.opt(k).ok_or_else(|| format!("add needs --{k}"));
    let task = (|| -> Result<Task, String> {
        let tier = need("tier")?;
        Ok(Task {
            id: need("id")?.into(),
            tier: tier.parse().map_err(|_| format!("--tier {tier}: not 1 to 6"))?,
            family: need("family")?.into(),
            ask: need("ask")?.into(),
            check: read(std::path::Path::new(need("check")?))?,
            reference: read(std::path::Path::new(need("ref")?))?,
            by: By {
                teacher: a.opt("teacher").unwrap_or("hand").into(),
                prompt: a.opt("prompt").unwrap_or("").into(),
                verifier: iq::verifier_hash(),
                day: need("day")?.into(),
            },
        })
    })();
    let task = match task {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let path = a.suite(usize::MAX);
    let mut tasks = match path.exists() {
        true => match suite_at(&path) {
            Ok(t) => t,
            Err(e) => return fail(e),
        },
        false => Vec::new(),
    };
    if tasks.iter().any(|t| t.id == task.id) {
        return fail(format!("E0764 the suite has a task {} already", task.id));
    }
    let t0 = Instant::now();
    match iq::verify(&task) {
        Err(e) => fail(format!("refused: {e}")),
        Ok(v) => {
            tasks.push(task);
            let text = iq::write_suite(&tasks);
            if let Err(e) = iq::read_suite(&text) {
                return fail(format!("the task does not read back: {e}"));
            }
            if let Err(e) = std::fs::write(&path, text) {
                return fail(format!("{}: {e}", path.display()));
            }
            println!("added: {}, verified in {} ms", kills(&v), ms(t0));
            0
        }
    }
}
