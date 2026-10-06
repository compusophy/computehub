//! `teach`, the teacher's commands (`cargo run -p compusophy-teach -- <command> --flag value`):
//!
//! ```text
//! teach tasks   --tier T --families a,b --per 3 --out evals/suites/iq.jsonl --ledger L
//!               [--card CARD] [--batch] [--budget USD]
//! teach solve   --suite S [--held H] --k 4 --rounds 3 --out solutions.jsonl --ledger L
//!               [--batch] [--budget USD]
//! teach export  --solutions S (--held H | --suite S) --out sft.jsonl
//! teach ask     --url URL --model M --suite S --split held|train|all [--held H]
//!               --out answers.jsonl [--gap MS]
//! teach prompts --suite S --split held|train|all [--held H] --out prompts.jsonl
//! teach cost    --ledger L
//! teach writer  --tier T --families a,b --per 3 [--card CARD]   (a Claude Code session teaches:)
//! teach import  --from tasks.jsonl --out evals/suites/iq.jsonl
//! teach replies --suite S --replies R.jsonl --teacher NAME --out solutions.jsonl [--held H]
//! ```
//!
//! `tasks` and `solve` also take `--model` (claude-opus-5-5), `--effort` (high), `--max-tokens`
//! (64000 for tasks, 32000 for solve) and `--day` (today, UTC). They append to `--out`; refused
//! tasks go to `<out>.refused.jsonl`, and a batch's id waits in `<out>.batch` while it runs.
//! Tasks are verified by [`iq::verify`] and written as `iq` writes them, so `--out` may be the
//! suite itself. `--families` names ideas: a variant of an idea the suite has is named under
//! that idea's root (`pong-ai`, never `ai-pong`), so [`iq::held`] holds it out with its twin.
//! `export`, `ask` and `prompts` write `--out` afresh. `--card` is the checker language's card
//! (default [`iq::CARD`]). `--held` lists the held-out families, one a line, each holding every
//! family of its root ([`iq::root`]); without it they are the suite's families [`iq::held`]
//! holds out. `prompts` writes, a line a task, the messages `ask` would send and the room and
//! temperature it asks at, for a local model to answer in batches. The API key is
//! `ANTHROPIC_API_KEY`, read from the environment only and never printed.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use teach::cost::{self, Ledger};
use teach::run::{self, Run};
use teach::seam::{self, Iq, Judge, Task};
use teach::wire::{Anthropic, OpenAi};
use teach::{Fail, TEACHER, append, codes, read};

const FLAGS: [&str; 23] = [
    "--from",
    "--replies",
    "--teacher",
    "--tier",
    "--families",
    "--per",
    "--card",
    "--out",
    "--ledger",
    "--batch",
    "--budget",
    "--suite",
    "--held",
    "--k",
    "--rounds",
    "--solutions",
    "--url",
    "--model",
    "--split",
    "--gap",
    "--effort",
    "--max-tokens",
    "--day",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(f) = go(&args) {
        eprintln!("{f}");
        std::process::exit(1);
    }
}

fn usage(why: impl Into<String>) -> Fail {
    Fail::new(codes::USAGE, why)
}

fn go(args: &[String]) -> Result<(), Fail> {
    let o = Opts::parse(args.get(1..).unwrap_or(&[]))?;
    match args.first().map(String::as_str) {
        Some("tasks") => tasks(&o),
        Some("solve") => solve(&o),
        Some("export") => export(&o),
        Some("ask") => ask(&o),
        Some("prompts") => prompts(&o),
        Some("writer") => writer(&o),
        Some("import") => import(&o),
        Some("replies") => replies(&o),
        Some("cost") => {
            print!("{}", cost::summary(&read(o.need("--ledger")?)?));
            Ok(())
        }
        _ => Err(usage(
            "usage: teach tasks|solve|export|ask|prompts|cost|writer|import|replies (src/main.rs)",
        )),
    }
}

/// `--flag value` pairs (`--batch` alone), each one of [`FLAGS`].
struct Opts(BTreeMap<String, String>);

impl Opts {
    fn parse(args: &[String]) -> Result<Opts, Fail> {
        let (mut m, mut it) = (BTreeMap::new(), args.iter());
        while let Some(a) = it.next() {
            if !FLAGS.contains(&a.as_str()) {
                return Err(usage(format!("{a}: not a flag teach knows")));
            }
            let v = match a.as_str() {
                "--batch" => "1".to_string(),
                _ => it.next().cloned().ok_or_else(|| usage(format!("{a} wants a value")))?,
            };
            m.insert(a.clone(), v);
        }
        Ok(Opts(m))
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(String::as_str)
    }

    fn need(&self, k: &str) -> Result<&str, Fail> {
        self.get(k).ok_or_else(|| usage(format!("{k} is needed")))
    }

    fn num<T: std::str::FromStr>(&self, k: &str, or: T) -> Result<T, Fail> {
        match self.get(k) {
            None => Ok(or),
            Some(v) => v.parse().map_err(|_| usage(format!("{k} {v}: not a number"))),
        }
    }
}

/// The tasks of the suite file at `path`: every line that is not blank must be one (E0989).
fn suite(path: &str) -> Result<Vec<Task>, Fail> {
    let text = read(path)?;
    let lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty());
    let task = |(i, l): (usize, &str)| {
        let bad = || Fail::new(codes::INPUT, format!("{path} line {}: not a task", i + 1));
        Task::parse(l).ok_or_else(bad)
    };
    lines.map(task).collect()
}

/// The held-out families: `--held`'s list, else those of `--suite`'s [`seam::held_of`] finds.
fn held(o: &Opts) -> Result<Vec<String>, Fail> {
    if let Some(h) = o.get("--held") {
        return Ok(seam::held(&read(h)?));
    }
    let tasks = suite(o.get("--suite").ok_or_else(|| usage("--held or --suite is needed"))?)?;
    Ok(seam::held_of(&tasks))
}

/// The judge of the suite at `path`, which grades by its tasks' checks (none read: it only
/// verifies new tasks).
fn judge(path: Option<&str>) -> Result<Iq, Fail> {
    let Some(path) = path else { return Ok(Iq { tasks: Vec::new() }) };
    let tasks = iq::read_suite(&read(path)?)
        .map_err(|r| Fail::new(codes::INPUT, format!("{path}: E{:04} {}", r.code, r.message)))?;
    Ok(Iq { tasks })
}

/// A run of `cmd` as the flags say, `room` tokens a request unless `--max-tokens` says.
fn teach_run<'a>(
    o: &Opts,
    cmd: &str,
    room: u32,
    teacher: &'a mut Anthropic,
    judge: &'a dyn Judge,
) -> Result<Run<'a>, Fail> {
    let day = o.get("--day").map_or_else(teach::today, String::from);
    let cap = match o.get("--budget") {
        Some(b) => {
            Some(cost::parse_usd(b).ok_or_else(|| usage(format!("--budget {b}: not dollars")))?)
        }
        None => None,
    };
    let ledger = Ledger {
        path: o.need("--ledger")?.into(),
        cmd: cmd.into(),
        day: day.clone(),
        cap,
        spent: 0,
    };
    let effort = o.get("--effort").unwrap_or("high");
    if !["low", "medium", "high", "xhigh", "max"].contains(&effort) {
        return Err(usage(format!("--effort {effort}: low, medium, high, xhigh or max")));
    }
    Ok(Run {
        teacher,
        judge,
        ledger,
        batch: o.get("--batch").is_some(),
        effort: effort.into(),
        max_tokens: o.num("--max-tokens", room)?,
        day,
    })
}

fn tasks(o: &Opts) -> Result<(), Fail> {
    let out = o.need("--out")?;
    let card = match o.get("--card") {
        Some(path) => read(path)?,
        None => iq::CARD.to_string(),
    };
    let tier: u8 = o.num("--tier", 0)?;
    if !(1..=6).contains(&tier) {
        return Err(usage("--tier: 1 to 6"));
    }
    let families: Vec<String> = o
        .need("--families")?
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(String::from)
        .collect();
    let taken: Vec<String> = std::fs::read_to_string(out)
        .unwrap_or_default()
        .lines()
        .filter_map(Task::parse)
        .map(|t| t.id)
        .collect();
    let mut teacher =
        Anthropic::new(o.get("--model").unwrap_or(TEACHER), &[out, ".batch"].concat())?;
    let judge = judge(None)?;
    let mut run = teach_run(o, "tasks", 64_000, &mut teacher, &judge)?;
    let w = run.tasks(&card, tier, &families, o.num("--per", 3)?, &taken);
    // Each kept line as iq writes it, so the suite reads back byte for byte.
    let canon =
        |l: &String| iq::read_task(l).map(|t| iq::write_task(&t) + "\n").unwrap_or(l.clone());
    append(out, &w.kept.iter().map(canon).collect::<String>())?;
    let mut refused = String::new();
    for (why, line) in &w.refused {
        eprintln!("  refused: {why}");
        refused += "{\"why\":";
        coder::json::put(&mut refused, why);
        refused += ",\"line\":";
        coder::json::put(&mut refused, line.trim_end());
        refused += "}\n";
    }
    if !refused.is_empty() {
        append(&[out.strip_suffix(".jsonl").unwrap_or(out), ".refused.jsonl"].concat(), &refused)?;
    }
    eprintln!(
        "{} tasks kept, {} refused; ${} spent",
        w.kept.len(),
        w.refused.len(),
        cost::usd(run.ledger.spent)
    );
    Ok(())
}

fn solve(o: &Opts) -> Result<(), Fail> {
    let out = o.need("--out")?;
    let (tasks, held) = (suite(o.need("--suite")?)?, held(o)?);
    let (k, rounds) = (o.num("--k", 4)?, o.num("--rounds", 3)?);
    let mut teacher =
        Anthropic::new(o.get("--model").unwrap_or(TEACHER), &[out, ".batch"].concat())?;
    let judge = judge(Some(o.need("--suite")?))?;
    let mut run = teach_run(o, "solve", 32_000, &mut teacher, &judge)?;
    let passes = run.solve(&tasks, &held, k, rounds, &mut |lines| append(out, lines))?;
    eprintln!("{passes} attempts passed; ${} spent", cost::usd(run.ledger.spent));
    Ok(())
}

fn export(o: &Opts) -> Result<(), Fail> {
    let (out, held) = (o.need("--out")?, held(o)?);
    let (text, n) = run::export(&read(o.need("--solutions")?)?, &held);
    std::fs::write(out, text).map_err(|e| Fail::new(codes::FILE, format!("{out}: {e}")))?;
    eprintln!("{n:?}");
    if n.drift > 0 {
        let why =
            format!("{} passing attempts were made under another system prompt: left out", n.drift);
        eprintln!("{}", Fail::new(codes::DRIFT, why));
    }
    Ok(())
}

fn ask(o: &Opts) -> Result<(), Fail> {
    let split = o.need("--split")?;
    let held = match split {
        "held" | "train" => held(o)?,
        "all" => Vec::new(),
        _ => return Err(usage("--split: held, train or all")),
    };
    let tasks = suite(o.need("--suite")?)?;
    let mut chat = OpenAi { url: o.need("--url")?.into(), gap: o.num("--gap", 0)?, sent: false };
    let out = o.need("--out")?;
    let text = run::answers(&mut chat, o.need("--model")?, &tasks, split, &held);
    std::fs::write(out, text).map_err(|e| Fail::new(codes::FILE, format!("{out}: {e}")))
}

fn prompts(o: &Opts) -> Result<(), Fail> {
    let split = o.need("--split")?;
    let held = match split {
        "held" | "train" => held(o)?,
        "all" => Vec::new(),
        _ => return Err(usage("--split: held, train or all")),
    };
    let (tasks, out) = (suite(o.need("--suite")?)?, o.need("--out")?);
    let text = run::prompts(&tasks, split, &held);
    eprintln!("{} prompts", text.lines().count());
    std::fs::write(out, text).map_err(|e| Fail::new(codes::FILE, format!("{out}: {e}")))
}

/// The writer's prompt as `tasks` sends it (its system prompt, then each family's request), for a
/// Claude Code session to follow instead of the API.
fn writer(o: &Opts) -> Result<(), Fail> {
    let card = o.get("--card").map_or(Ok(iq::CARD.to_string()), read)?;
    print!("{}", teach::prompts::writer(&card));
    for f in o.need("--families")?.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        println!(
            "
---
{}",
            teach::prompts::writer_user(o.num("--tier", 1)?, f, o.num("--per", 3)?, &[])
        );
    }
    Ok(())
}

/// Task lines written elsewhere: each verified by iq, written as iq writes it, appended to
/// `--out` unless its id is taken there; refusals to `<from>.refused.jsonl`.
fn import(o: &Opts) -> Result<(), Fail> {
    let (from, out) = (o.need("--from")?, o.need("--out")?);
    let mut taken: Vec<String> =
        read(out).unwrap_or_default().lines().filter_map(Task::parse).map(|t| t.id).collect();
    let (judge, mut kept, mut refused) = (judge(None)?, String::new(), String::new());
    for line in read(from)?.lines().filter(|l| !l.trim().is_empty()) {
        let id = Task::parse(line).map(|t| t.id).unwrap_or_default();
        let got = match taken.contains(&id) {
            true => Err(format!("id {id} is taken")),
            false => {
                judge.verify_task(line).and_then(|_| iq::read_task(line).map_err(|r| r.message))
            }
        };
        match got {
            Ok(t) => (kept += &(iq::write_task(&t) + "\n"), taken.push(id)).1,
            Err(why) => {
                refused += &format!(
                    "{{\"why\":{},\"line\":{}}}
",
                    coder::json::quote(&why),
                    coder::json::quote(line)
                )
            }
        }
    }
    append(out, &kept)?;
    append(&[from.strip_suffix(".jsonl").unwrap_or(from), ".refused.jsonl"].concat(), &refused)?;
    eprintln!("{} kept, {} refused", kept.lines().count(), refused.lines().count());
    Ok(())
}

/// Replies written elsewhere to the solver's first turn, graded into attempt lines `export` reads.
fn replies(o: &Opts) -> Result<(), Fail> {
    let (path, out) = (o.need("--suite")?, o.need("--out")?);
    let (tasks, held, judge) = (suite(path)?, held(o)?, judge(Some(path))?);
    let day = o.get("--day").map_or_else(teach::today, String::from);
    let by = seam::By {
        teacher: o.need("--teacher")?.into(),
        prompt: teach::hex16(teach::fnv(teach::prompts::solver().as_bytes())),
        verifier: judge.id(),
        day,
    };
    let (lines, passes) = run::replies(&tasks, &held, &read(o.need("--replies")?)?, &judge, &by);
    append(out, &lines)?;
    eprintln!("{} replies graded, {passes} passed", lines.lines().count());
    Ok(())
}
