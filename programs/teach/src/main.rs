//! `teach`, the teacher's commands (`cargo run -p compusophy-teach -- <command> --flag value`):
//!
//! ```text
//! teach tasks  --tier T --families a,b --per 3 --card CARD --out tasks.jsonl --ledger L
//!              [--batch] [--budget USD]
//! teach solve  --suite S --held H --k 4 --rounds 3 --out solutions.jsonl --ledger L
//!              [--batch] [--budget USD]
//! teach export --solutions S --held H --out sft.jsonl
//! teach ask    --url URL --model M --suite S --split held|train|all [--held H]
//!              --out answers.jsonl [--gap MS]
//! teach cost   --ledger L
//! ```
//!
//! `tasks` and `solve` also take `--model` (claude-opus-5-5), `--effort` (high), `--max-tokens`
//! (64000 for tasks, 32000 for solve) and `--day` (today, UTC). They append to `--out`; refused
//! tasks go to `<out>.refused.jsonl`, and a batch's id waits in `<out>.batch` while it runs.
//! `export` and `ask` write `--out` afresh. `--card` is the checker language's card (until iq
//! lands, a file). `--held` lists the held-out families, one a line. The API key is
//! `ANTHROPIC_API_KEY`, read from the environment only and never printed.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use teach::cost::{self, Ledger};
use teach::run::{self, Run};
use teach::seam::{self, Smoke, Task};
use teach::wire::{Anthropic, OpenAi};
use teach::{Fail, TEACHER, append, codes, read};

const FLAGS: [&str; 20] = [
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
        Some("cost") => {
            print!("{}", cost::summary(&read(o.need("--ledger")?)?));
            Ok(())
        }
        _ => Err(usage(
            "usage: teach tasks | solve | export | ask | cost (see programs/teach/src/main.rs)",
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

fn held(o: &Opts) -> Result<Vec<String>, Fail> {
    Ok(seam::held(&read(o.need("--held")?)?))
}

/// A run of `cmd` as the flags say, `room` tokens a request unless `--max-tokens` says.
fn teach_run<'a>(
    o: &Opts,
    cmd: &str,
    room: u32,
    teacher: &'a mut Anthropic,
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
        judge: &Smoke,
        ledger,
        batch: o.get("--batch").is_some(),
        effort: effort.into(),
        max_tokens: o.num("--max-tokens", room)?,
        day,
    })
}

fn tasks(o: &Opts) -> Result<(), Fail> {
    let out = o.need("--out")?;
    let card = read(o.need("--card")?)?;
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
    let mut run = teach_run(o, "tasks", 64_000, &mut teacher)?;
    let w = run.tasks(&card, tier, &families, o.num("--per", 3)?, &taken);
    append(out, &w.kept.concat())?;
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
    let mut run = teach_run(o, "solve", 32_000, &mut teacher)?;
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
