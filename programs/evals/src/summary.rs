//! Reading a gain: two runs (or two models, each all its runs) side by side. Each side's pass
//! rate with its 95% Wilson interval, tokens and dollars per pass and seconds per make; the
//! difference with its 95% Newcombe interval; and, task by task (a task passes on a side when
//! most of its trials there did, fails when most failed, and is neither when half did), the tasks
//! that flipped, with McNemar's exact p for them. Records whose AI failed (stage `ai`) are not the
//! model's: they are counted apart, never as fails.

use crate::record::Record;

/// The 95% normal quantile.
const Z: f64 = 1.959_964;

/// A side's numbers: records graded (the AI's failures aside), passes, AI failures, requests
/// and those another model answered, tokens in and out, micro-dollars and milliseconds, over all
/// its records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub n: u32,
    pub pass: u32,
    pub errors: u32,
    pub tries: u32,
    pub fallback: u32,
    pub tokens: u64,
    pub usd_micros: u64,
    pub ms: u64,
}

/// The numbers of `rs`.
pub fn stats(rs: &[&Record]) -> Stats {
    let mut s = Stats::default();
    for r in rs {
        match r.stage.as_str() {
            "ai" => s.errors += 1,
            _ => (s.n, s.pass) = (s.n + 1, s.pass + u32::from(r.pass)),
        }
        (s.tries, s.fallback) = (s.tries + r.tries, s.fallback + r.fallback);
        s.tokens += r.tokens_in + r.tokens_out;
        (s.usd_micros, s.ms) = (s.usd_micros + r.usd_micros, s.ms + r.ms);
    }
    s
}

/// The 95% Wilson score interval of `k` passes in `n`.
pub fn wilson(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (n, p) = (f64::from(n), f64::from(k) / f64::from(n));
    let z2 = Z * Z;
    let mid = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
    let half = Z / (1.0 + z2 / n) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((mid - half).max(0.0), (mid + half).min(1.0))
}

/// The 95% Newcombe (hybrid score) interval of `b`'s pass rate less `a`'s.
pub fn newcombe(a: Stats, b: Stats) -> (f64, f64) {
    let rate = |s: Stats| if s.n == 0 { 0.0 } else { f64::from(s.pass) / f64::from(s.n) };
    let ((l1, u1), (l2, u2), (p1, p2)) =
        (wilson(a.pass, a.n), wilson(b.pass, b.n), (rate(a), rate(b)));
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

/// Each task of `rs` and whether most of its trials passed (the AI's failures aside; `None`
/// when as many failed), in order.
fn by_task(rs: &[&Record]) -> Vec<(String, Option<bool>)> {
    let mut tasks: Vec<(String, u32, u32)> = Vec::new();
    for r in rs.iter().filter(|r| r.stage != "ai") {
        let i = tasks.iter().position(|t| t.0 == r.task).unwrap_or_else(|| {
            tasks.push((r.task.clone(), 0, 0));
            tasks.len() - 1
        });
        (tasks[i].1, tasks[i].2) = (tasks[i].1 + u32::from(r.pass), tasks[i].2 + 1);
    }
    tasks.into_iter().map(|(t, k, n)| (t, (k * 2 != n).then_some(k * 2 > n))).collect()
}

/// `x` as a percentage.
fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// One side's line: its name, passes, rate and interval, tokens and dollars per pass, seconds
/// per make and the AI's failures.
fn line(name: &str, s: Stats) -> String {
    let (lo, hi) = wilson(s.pass, s.n);
    let rate = if s.n == 0 { 0.0 } else { f64::from(s.pass) / f64::from(s.n) };
    let per =
        |x: u64| if s.pass == 0 { "-".to_string() } else { format!("{}", x / u64::from(s.pass)) };
    let usd = if s.pass == 0 {
        "-".into()
    } else {
        format!("{:.4}", s.usd_micros as f64 / 1e6 / f64::from(s.pass))
    };
    let secs = s.ms / 1000 / u64::from((s.n + s.errors).max(1));
    format!(
        "{name:<28} {:>3}/{:<3} {:>5}  {:>4}-{:<4} {:>9} {:>8} {:>6} {:>6}\n",
        s.pass,
        s.n,
        pct(rate),
        pct(lo),
        pct(hi),
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
        "{:<28} {:>7} {:>5}  {:>9} {:>9} {:>8} {:>6} {:>6}\n",
        "", "pass", "rate", "95% CI", "tok/pass", "$/pass", "s/make", "AI err"
    );
    o += &line(an, sa);
    o += &line(bn, sb);
    let (lo, hi) = newcombe(sa, sb);
    let rate = |s: Stats| if s.n == 0 { 0.0 } else { f64::from(s.pass) / f64::from(s.n) };
    let d = (rate(sb) - rate(sa)) * 100.0;
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
    o
}

/// A markdown table of every run in `all`, in order: its model, date and commit, passes and rate
/// with its interval, by size, tokens and dollars per pass, seconds per make, the AI's failures,
/// and the requests the other model answered of all.
pub fn table(all: &[Record]) -> String {
    let mut o = String::from(
        "| run | model | commit | pass | rate (95% CI) | tiny / small / medium / hard | tokens/pass | $/pass | s/make | AI errors | other model |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
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
        let (lo, hi) = wilson(s.pass, s.n);
        let sizes: Vec<String> = ["tiny", "small", "medium", "hard"]
            .iter()
            .map(|z| {
                let t = stats(&rs.iter().copied().filter(|r| r.size == *z).collect::<Vec<_>>());
                format!("{}/{}", t.pass, t.n)
            })
            .collect();
        let rate = if s.n == 0 { 0.0 } else { f64::from(s.pass) / f64::from(s.n) };
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
            rs[0].meta.commit,
            s.pass,
            s.n,
            pct(rate),
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
