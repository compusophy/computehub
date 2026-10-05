//! Reading a gain: two runs (or two models, each all its runs) side by side. Each side's pass
//! rate with its 95% Wilson interval, tokens and dollars per pass and seconds per make; the
//! difference with its 95% Newcombe interval; and, task by task (a task passes on a side when
//! most of its trials there did, fails when most failed, and is neither when half did), the tasks
//! that flipped, with McNemar's exact p for them. Records whose AI failed (stage `ai`) or whose
//! smoke test faulted on its own (`harness`) are not the model's: they are counted apart, never
//! as fails. It warns when a side mixes suites, prompts, knobs or harnesses, or when the sides
//! were graded by different suites: those compare different measurements.
//!
//! Trials of one task are not independent draws (a task that fails one trial is likely to fail
//! the next), so a side's interval is as wide as its tasks warrant: the rate is pooled over its
//! records, its variance is the one clustered by task, and the interval is Wilson's at the
//! number of independent draws that variance amounts to (`n eff`): all the records when trials
//! vary as independent draws would, as few as the tasks when each passes all its trials or none.
//! More trials of the same tasks narrow it only as far as they tell more. Newcombe's interval
//! treats the sides as independent; on the same tasks it is the cautious one, and McNemar's test
//! on the tasks that flipped is the paired one.

use crate::record::{Record, hex};

/// The 95% normal quantile.
const Z: f64 = 1.959_964;

/// A side's numbers: records graded (the AI's and the harness's failures aside), passes, those
/// failures, the effective number of independent draws, requests and those another model
/// answered, tokens in and out, micro-dollars and milliseconds, over all its records.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub n: u32,
    pub pass: u32,
    pub errors: u32,
    pub eff: f64,
    pub tries: u32,
    pub fallback: u32,
    pub tokens: u64,
    pub usd_micros: u64,
    pub ms: u64,
}

impl Stats {
    /// The pass rate (0 with nothing graded).
    pub fn rate(&self) -> f64 {
        if self.n == 0 { 0.0 } else { f64::from(self.pass) / f64::from(self.n) }
    }
}

/// Whether `r` was graded: neither its AI nor the harness failed.
pub fn graded(r: &Record) -> bool {
    !matches!(r.stage.as_str(), "ai" | "harness")
}

/// The numbers of `rs`.
pub fn stats(rs: &[&Record]) -> Stats {
    let mut s = Stats::default();
    for r in rs {
        if graded(r) {
            (s.n, s.pass) = (s.n + 1, s.pass + u32::from(r.pass));
        } else {
            s.errors += 1;
        }
        (s.tries, s.fallback) = (s.tries + r.tries, s.fallback + r.fallback);
        s.tokens += r.tokens_in + r.tokens_out;
        (s.usd_micros, s.ms) = (s.usd_micros + r.usd_micros, s.ms + r.ms);
    }
    s.eff = effective(rs);
    s
}

/// Each task of `rs`, graded, with its passes and trials, in order.
fn tally(rs: &[&Record]) -> Vec<(String, u32, u32)> {
    let mut tasks: Vec<(String, u32, u32)> = Vec::new();
    for r in rs.iter().filter(|r| graded(r)) {
        let i = tasks.iter().position(|t| t.0 == r.task).unwrap_or_else(|| {
            tasks.push((r.task.clone(), 0, 0));
            tasks.len() - 1
        });
        (tasks[i].1, tasks[i].2) = (tasks[i].1 + u32::from(r.pass), tasks[i].2 + 1);
    }
    tasks
}

/// How many independent draws the graded records of `rs` amount to: p(1 - p) over the pooled
/// rate's variance clustered by task (the sum of each task's squared passes less its trials'
/// share of them, over the records squared), at most the records. With every record passed, or
/// none, nothing tells trials apart: as many as the tasks.
pub fn effective(rs: &[&Record]) -> f64 {
    let tasks = tally(rs);
    let (k, n) = tasks.iter().fold((0, 0), |(k, n), t| (k + t.1, n + t.2));
    if k == 0 || k == n {
        return tasks.len() as f64;
    }
    let (n, p) = (f64::from(n), f64::from(k) / f64::from(n));
    let spread: f64 = tasks.iter().map(|t| (f64::from(t.1) - f64::from(t.2) * p).powi(2)).sum();
    let var = spread / (n * n);
    if var > 0.0 { (p * (1.0 - p) / var).min(n) } else { n }
}

/// The 95% Wilson score interval of a rate `p` from `n` independent draws.
pub fn wilson(p: f64, n: f64) -> (f64, f64) {
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let z2 = Z * Z;
    let mid = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
    let half = Z / (1.0 + z2 / n) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((mid - half).max(0.0), (mid + half).min(1.0))
}

/// The 95% Newcombe (hybrid score) interval of `b`'s pass rate less `a`'s, each side's Wilson
/// interval at its effective draws.
pub fn newcombe(a: Stats, b: Stats) -> (f64, f64) {
    let ((l1, u1), (l2, u2), (p1, p2)) =
        (wilson(a.rate(), a.eff), wilson(b.rate(), b.eff), (a.rate(), b.rate()));
    let d = p2 - p1;
    (
        d - ((p2 - l2).powi(2) + (u1 - p1).powi(2)).sqrt(),
        d + ((u2 - p2).powi(2) + (p1 - l1).powi(2)).sqrt(),
    )
}

/// McNemar's exact two-sided p for `b` and `c` discordant pairs.
pub fn mcnemar(b: u32, c: u32) -> f64 {
    let n = b + c;
    let mut tail = 0.0;
    let mut choose = 1.0;
    for i in 0..=b.min(c) {
        if i > 0 {
            choose = choose * f64::from(n - i + 1) / f64::from(i);
        }
        tail += choose;
    }
    (2.0 * tail / 2f64.powi(n as i32)).min(1.0)
}

/// The records `sel` names: a run's id, else a model's (all its runs).
pub fn select<'a>(all: &'a [Record], sel: &str) -> Vec<&'a Record> {
    let run: Vec<&Record> = all.iter().filter(|r| r.meta.run == sel).collect();
    if run.is_empty() { all.iter().filter(|r| r.meta.model == sel).collect() } else { run }
}

/// Each task of `rs` and whether most of its trials passed (the failures not the model's
/// aside; `None` when as many failed), in order.
fn by_task(rs: &[&Record]) -> Vec<(String, Option<bool>)> {
    let tasks = tally(rs).into_iter();
    tasks.map(|(t, k, n)| (t, (k * 2 != n).then_some(k * 2 > n))).collect()
}

/// `x` as a percentage.
fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// One of a record's hashes.
type Hash = fn(&Record) -> u64;

/// The distinct values of `f` over `rs`, as hex, in order.
fn kinds(rs: &[&Record], f: Hash) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in rs.iter().map(|r| hex(f(r))) {
        if !out.contains(&h) {
            out.push(h);
        }
    }
    out
}

/// What a record was measured with: its suite, prompt, knobs and harness.
const MEASURED: [(&str, Hash); 4] = [
    ("suite", |r| r.suite_hash),
    ("prompt", |r| r.prompt),
    ("knobs", |r| r.knobs),
    ("harness", |r| r.harness),
];

/// One side's line: its name, passes, rate and interval, effective draws, tokens and dollars
/// per pass, seconds per make and the failures not the model's.
fn line(name: &str, s: Stats) -> String {
    let (lo, hi) = wilson(s.rate(), s.eff);
    let per =
        |x: u64| if s.pass == 0 { "-".to_string() } else { format!("{}", x / u64::from(s.pass)) };
    let usd = if s.pass == 0 {
        "-".into()
    } else {
        format!("{:.4}", s.usd_micros as f64 / 1e6 / f64::from(s.pass))
    };
    let secs = s.ms / 1000 / u64::from((s.n + s.errors).max(1));
    format!(
        "{name:<28} {:>3}/{:<3} {:>5}  {:>4}-{:<4} {:>5.0} {:>9} {:>8} {:>6} {:>6}\n",
        s.pass,
        s.n,
        pct(s.rate()),
        pct(lo),
        pct(hi),
        s.eff,
        per(s.tokens),
        usd,
        secs,
        s.errors
    )
}

/// The comparison of side `a` (named `an`) with side `b`.
pub fn compare(an: &str, a: &[&Record], bn: &str, b: &[&Record]) -> String {
    let (sa, sb) = (stats(a), stats(b));
    let mut o = format!(
        "{:<28} {:>7} {:>5}  {:>9} {:>5} {:>9} {:>8} {:>6} {:>6}\n",
        "", "pass", "rate", "95% CI", "n eff", "tok/pass", "$/pass", "s/make", "errors"
    );
    o += &line(an, sa);
    o += &line(bn, sb);
    let (lo, hi) = newcombe(sa, sb);
    let d = (sb.rate() - sa.rate()) * 100.0;
    let (ta, tb) = (by_task(a), by_task(b));
    let (mut lost, mut won, mut ties) = (Vec::new(), Vec::new(), 0);
    for (t, pa) in &ta {
        if let Some((_, pb)) = tb.iter().find(|x| x.0 == *t) {
            match (pa, pb) {
                (Some(true), Some(false)) => lost.push(t.as_str()),
                (Some(false), Some(true)) => won.push(t.as_str()),
                (None, _) | (_, None) => ties += 1,
                _ => {}
            }
        }
    }
    o += &format!(
        "{bn} - {an}: {d:+.0} points, 95% CI {:+.0} to {:+.0}; {} tasks flipped, McNemar p = {:.2}; \
         {ties} passed half their trials on a side\n",
        lo * 100.0,
        hi * 100.0,
        lost.len() + won.len(),
        mcnemar(lost.len() as u32, won.len() as u32)
    );
    for (name, rs, s) in [(an, a, sa), (bn, b, sb)].into_iter().filter(|x| x.2.fallback > 0) {
        let (f, t) = (s.fallback, s.tries);
        let own: Vec<&Record> = rs.iter().copied().filter(|r| r.fallback == 0).collect();
        let o2 = stats(&own);
        o += &format!(
            "{name}: {f} of {t} requests were answered by the other model; the tasks whose every \
             request it answered itself pass {}/{}\n",
            o2.pass, o2.n
        );
    }
    o += &format!(
        "passed by {an} only: {}\n",
        if lost.is_empty() { "none".into() } else { lost.join(", ") }
    );
    o += &format!(
        "passed by {bn} only: {}\n",
        if won.is_empty() { "none".into() } else { won.join(", ") }
    );
    for (what, f) in MEASURED {
        let (ka, kb) = (kinds(a, f), kinds(b, f));
        for (name, k) in [(an, &ka), (bn, &kb)].into_iter().filter(|x| x.1.len() > 1) {
            o += &format!(
                "warning: {name} mixes {what} hashes ({}): its records are not one measurement\n",
                k.join(", ")
            );
        }
        match (&ka[..], &kb[..]) {
            ([x], [y]) if x != y && what == "suite" => {
                o += &format!(
                    "warning: the sides were graded by different suites ({x} and {y}), so the \
                     difference measures the suite's change too: grade the older run again \
                     offline first (replay --write)\n"
                )
            }
            ([x], [y]) if x != y => o += &format!("{an} and {bn} differ in {what}: {x} and {y}\n"),
            _ => {}
        }
    }
    o
}

/// A markdown table of every run in `all`, in order: its model, the commits its AI was asked at,
/// passes and rate with its interval, by size, tokens and dollars per pass, seconds per make,
/// the failures not the model's, and the requests the other model answered of all.
pub fn table(all: &[Record]) -> String {
    let mut o = String::from(
        "| run | model | commit | pass | rate (95% CI) | tiny / small / medium / hard | tokens/pass | $/pass | s/make | errors | other model |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    let mut runs: Vec<&str> = Vec::new();
    for r in all {
        if !runs.contains(&r.meta.run.as_str()) {
            runs.push(&r.meta.run);
        }
    }
    for run in runs {
        let rs: Vec<&Record> = all.iter().filter(|r| r.meta.run == run).collect();
        let s = stats(&rs);
        let (lo, hi) = wilson(s.rate(), s.eff);
        let sizes: Vec<String> = ["tiny", "small", "medium", "hard"]
            .iter()
            .map(|z| {
                let t = stats(&rs.iter().copied().filter(|r| r.size == *z).collect::<Vec<_>>());
                format!("{}/{}", t.pass, t.n)
            })
            .collect();
        let mut commits: Vec<&str> = Vec::new();
        for r in &rs {
            if !commits.contains(&r.meta.commit.as_str()) {
                commits.push(&r.meta.commit);
            }
        }
        let per = |x: u64| {
            if s.pass == 0 { "-".to_string() } else { (x / u64::from(s.pass)).to_string() }
        };
        let usd = if s.pass == 0 {
            "-".into()
        } else {
            format!("{:.4}", s.usd_micros as f64 / 1e6 / f64::from(s.pass))
        };
        o += &format!(
            "| {run} | {} | {} | {}/{} | {} ({}-{}) | {} | {} | {} | {} | {} | {}/{} |\n",
            rs[0].meta.model,
            commits.join(", "),
            s.pass,
            s.n,
            pct(s.rate()),
            pct(lo),
            pct(hi),
            sizes.join(" / "),
            per(s.tokens),
            usd,
            s.ms / 1000 / u64::from((s.n + s.errors).max(1)),
            s.errors,
            s.fallback,
            s.tries
        );
    }
    o
}
