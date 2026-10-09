//! The eval runner, for development only (never shipped): runs a suite of `programs/evals`
//! against the free AI, replays a recorded run offline, and compares runs. Its one wire is the
//! system's `curl`, POSTing to the free AI as the desktop does (same origin, a streamed
//! chat-completions body) and paced under its limits; everything else is the library's.
//!
//! ```text
//! cargo run -p eval -- run [--model zai/glm-5.3] [--trials 1] [--first 1] [--tasks a,b]
//!                          [--max 150] [--run ID] [--date YYYY-MM-DD] [--commit C] [--resume]
//! cargo run -p eval -- replay --run ID [--write] [--loose]
//! cargo run -p eval -- summary [--a RUN|MODEL] [--b RUN|MODEL] [--markdown]
//! cargo run -p eval -- list
//! cargo run -p eval -- iq --suite S --model M --out OUT [--held H | --tasks a,b] [--url U]
//!                         [--name N] [--jobs 1] [--ms MS] [--per-hour 110]
//! cargo run -p eval -- team --suite S --answers A --helper-url U --out OUT [--held H]
//!                           [--helper M] [--name N] [--turns 3] [--jobs 1] [--traces T]
//! ```
//!
//! `iq` makes the IQ suite's tasks (the families `--held` lists, or `--tasks`, else all) as
//! Studio makes them (`coder::Make`: write, check, fix, keep the best) over `--url`: the free
//! AI by default, paced as `run` is; another URL (a llama-server's chat completions) unpaced,
//! `--jobs` at once, each request stopped once its program is in. Each make has `--ms` of its
//! requests' time (Studio's by default); `--per-hour` lowers the free AI's hourly pace, to share
//! it with another run. An answer line a task for `iq score` (the program the make installed, in
//! an app block; no reply if it installed none, and how the make went) is appended to `--out` as
//! it ends; tasks already there are skipped, so a run cut off resumes, and a make the AI failed
//! (E0901 to E0905: busy, unreachable) is not written, so a rerun makes it again.
//!
//! `team` is the team's first move (DESIGN.md, "The team"): each answer in `--answers` (a lead's
//! recorded replies, as `iq score` reads them) is a draft that a helper at `--helper-url` (a
//! llama-server's chat completions, unpaced, `--jobs` at once) repairs when it does not run clean,
//! in at most `--turns` turns ([`team::Repair`]), every answer judged by the make loop's test. An
//! answer line a task goes to `--out` (the best program; how the repair went in `team`), so
//! `iq score` scores the lead and helper together; tasks already there are skipped.
//!
//! `--dir D` reads and writes `D/results` and `D/replays` instead of the repo's `evals/`. Runs
//! and replays need a debug build (as above): a checker that panics fails its app only where
//! panics unwind, and the release profile aborts.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use coder::json::{Json, quote};
use evals::record::{Meta, Record, load};
use evals::replay::Replay;
use evals::run::{Answer, At, Feed, MODELS, Wire};
use makes::TASKS;

const URL: &str = "https://computehub-sigma.vercel.app/api/ai";
const ORIGIN: &str = "Origin: https://computehub-sigma.vercel.app";
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../evals");
/// The free AI takes 30 requests a minute and 120 an hour from one client: kept under both.
const PER_MINUTE: usize = 25;
const PER_HOUR: usize = 110;
const HOUR: u64 = 3_600_000;
/// How long the rest of a stream is read for its receipt once the make has its program.
const DRAIN: Duration = Duration::from_secs(20);

/// Milliseconds since the epoch.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Where the times of this machine's requests in the last hour are kept, so that runs one
/// after another keep under the free AI's limits together (it counts by client: this machine).
fn sent_file() -> PathBuf {
    std::env::temp_dir().join("compusophy-eval-sent")
}

/// The live wire: curl to `url`, at most `max` requests this run; to the free AI each paced by
/// this machine's requests of the last hour (`sent`, ms since the epoch), each at most 300 s.
struct Curl {
    sent: Vec<u64>,
    n: usize,
    max: usize,
    url: String,
    paced: bool,
    per_hour: usize,
}

impl Curl {
    fn new(max: usize, url: &str) -> Curl {
        let since = now_ms().saturating_sub(HOUR);
        let kept = std::fs::read_to_string(sent_file()).unwrap_or_default();
        let sent = kept.lines().filter_map(|l| l.trim().parse().ok()).filter(|&t| t > since);
        let (url, paced) = (url.into(), url == URL);
        Curl { sent: sent.collect(), n: 0, max, url, paced, per_hour: PER_HOUR }
    }

    /// Waits until a request keeps under the limits, 2 s after the last at least.
    fn pace(&self) {
        let mut said = false;
        loop {
            let now = now_ms();
            let within =
                |ms: u64| self.sent.iter().filter(|&&t| now.saturating_sub(t) < ms).count();
            let gap = self.sent.last().is_some_and(|&t| now.saturating_sub(t) < 2000);
            if within(60_000) < PER_MINUTE && within(HOUR) < self.per_hour && !gap {
                return;
            }
            if within(HOUR) >= self.per_hour && !said {
                eprintln!("  waiting: {} requests in the last hour", within(HOUR));
                said = true;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Notes a request sent now.
    fn note(&mut self) {
        let now = now_ms();
        self.sent.retain(|&t| now.saturating_sub(t) < HOUR);
        self.sent.push(now);
        self.n += 1;
        let text: String = self.sent.iter().map(|t| format!("{t}\n")).collect();
        let _ = std::fs::write(sent_file(), text);
    }
}

impl Wire for Curl {
    fn post(&mut self, at: &At, body: &str, on: &mut dyn FnMut(&[u8], u64) -> Feed) -> Answer {
        let fail = |error: String| Answer { error, ..Answer::default() };
        if self.n >= self.max {
            return fail("the eval's request budget is spent".into());
        }
        if self.paced {
            self.pace();
        }
        let secs = if self.paced { "300" } else { "3600" };
        let args = ["-sS", "-N", "--max-time", secs, "-X", "POST", &self.url, "-H", ORIGIN];
        let more = ["-H", "Content-Type: application/json", "--data-binary", "@-"];
        let mut child = match Command::new("curl")
            .args(args)
            .args(more)
            .args(["-w", "%{stderr}\nstatus=%{http_code}\n"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return fail(format!("curl: {e}")),
        };
        if self.paced {
            self.note();
            eprintln!("  -> {} request {} ({} bytes)", at.task, at.n, body.len());
        } else {
            self.n += 1;
        }
        let (tx, rx) = channel();
        let mut out = child.stdout.take().expect("piped");
        std::thread::spawn(move || {
            let mut buf = vec![0; 16 * 1024];
            while let Ok(n @ 1..) = out.read(&mut buf) {
                if tx.send((Instant::now(), buf[..n].to_vec())).is_err() {
                    break;
                }
            }
        });
        let mut err = child.stderr.take().expect("piped");
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        let t0 = Instant::now();
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(body.as_bytes());
        }
        let ms = |t: Instant| t.saturating_duration_since(t0).as_millis() as u64;
        let (mut tail, mut feeding, mut until) = (Vec::new(), true, None::<Instant>);
        loop {
            let wait = until
                .map_or(Duration::from_secs(400), |u| u.saturating_duration_since(Instant::now()));
            match rx.recv_timeout(wait) {
                Ok((t, chunk)) => {
                    tail.extend_from_slice(&chunk);
                    tail.drain(..tail.len().saturating_sub(512));
                    match feeding.then(|| on(&chunk, ms(t))) {
                        // Only the free AI's receipt is worth reading on for: a local server
                        // stops writing when its client goes.
                        Some(Feed::Drain) if self.paced => {
                            (feeding, until) = (false, Some(Instant::now() + DRAIN))
                        }
                        Some(Feed::Drain | Feed::Stop) => {
                            let _ = child.kill();
                            break;
                        }
                        _ => {}
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let end = ms(Instant::now());
        let ok = child.wait().is_ok_and(|s| s.success());
        let stderr = errs.join().unwrap_or_default();
        let status =
            stderr.rsplit("status=").next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let said: Vec<&str> =
            stderr.lines().filter(|l| !l.starts_with("status=") && !l.trim().is_empty()).collect();
        let error = if ok || !feeding { String::new() } else { said.join(" ") };
        let error = if error.is_empty() && status == 0 && feeding {
            "couldn't reach the AI".into()
        } else {
            error
        };
        Answer { status, error, ms: end, receipt: uiwire::stat::receipt(&tail) }
    }
}

/// The value after `--name` in `args`.
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Whether the flag `--name` is in `args`.
fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Today's date (UTC), YYYY-MM-DD.
fn today() -> String {
    let days =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

/// The commit checked out, short, `-dirty` if the tree has changes.
fn commit() -> String {
    let git = |a: &[&str]| {
        Command::new("git")
            .args(a)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let head = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain"]).is_ok_and(|s| !s.is_empty());
    if dirty { head + "-dirty" } else { head }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn append(path: &str, text: &str) {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path);
    file.and_then(|mut f| f.write_all(text.as_bytes())).unwrap_or_else(|e| panic!("{path}: {e}"));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = arg(&args, "--dir").unwrap_or_else(|| ROOT.into());
    let results = format!("{root}/results/{}.jsonl", makes::ID);
    let replays = |run: &str| format!("{root}/replays/{}/{run}.jsonl", makes::ID);
    let cmd = args.first().map(String::as_str);
    if cfg!(panic = "abort") && matches!(cmd, Some("run" | "replay")) {
        return eprintln!(
            "runs and replays need a debug build (cargo run -p eval): a checker's panic fails its \
             app only where panics unwind"
        );
    }
    match cmd {
        Some("run") => {
            let model = arg(&args, "--model").unwrap_or_else(|| MODELS[0].into());
            if !MODELS.contains(&model.as_str()) {
                return eprintln!("--model: the free AI answers only {MODELS:?}");
            }
            let num =
                |name: &str, or: u32| arg(&args, name).and_then(|v| v.parse().ok()).unwrap_or(or);
            let (trials, first, max) = (num("--trials", 1), num("--first", 1), num("--max", 150));
            let date = arg(&args, "--date").unwrap_or_else(today);
            let short = model.rsplit('/').next().unwrap_or(&model).to_string();
            let run = arg(&args, "--run").unwrap_or_else(|| format!("{date}-{short}"));
            let meta = Meta {
                run: run.clone(),
                date,
                commit: arg(&args, "--commit").unwrap_or_else(commit),
                model,
            };
            let only = arg(&args, "--tasks").unwrap_or_default();
            let tasks: Vec<_> = TASKS
                .iter()
                .filter(|t| only.is_empty() || only.split(',').any(|o| o == t.id))
                .collect();
            // Where the records and exchanges go, made before any request is spent.
            for dir in [format!("{root}/results"), format!("{root}/replays/{}", makes::ID)] {
                std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{dir}: {e}"));
            }
            let had = load(&read(&results));
            let kept = |id: &str, trial: u32| {
                flag(&args, "--resume")
                    && had.iter().any(|r| {
                        (r.meta.run.as_str(), r.task.as_str(), r.trial, r.stage != "ai")
                            == (&run, id, trial, true)
                    })
            };
            let mut wire = Curl::new(max as usize, URL);
            if !wire.sent.is_empty() {
                eprintln!("{} requests from this machine in the last hour", wire.sent.len());
            }
            let mut failed = 0;
            for trial in first..first + trials {
                for t in tasks.iter().filter(|t| !kept(t.id, trial)) {
                    if wire.n + 5 > wire.max {
                        return eprintln!(
                            "stopped: the request budget ({max}) has no room for a whole make"
                        );
                    }
                    let (r, log) = evals::run::task(&meta, t, trial, &mut wire);
                    append(&replays(&run), &log.iter().map(|x| x.line()).collect::<String>());
                    append(&results, &r.line());
                    let usd = r.usd_micros as f64 / 1e6;
                    println!(
                        "{:<12} trial {trial}: {:<7} {:<8} {} tries {:>4} s ${usd:.4} {}",
                        r.task,
                        r.stage,
                        r.outcome,
                        r.tries,
                        r.ms / 1000,
                        r.reason
                    );
                    failed = if r.stage == "ai" { failed + 1 } else { 0 };
                    if failed >= 2 || r.reason.contains("E0902") {
                        return eprintln!(
                            "stopped: the AI failed ({}); {} requests sent",
                            r.reason, wire.n
                        );
                    }
                }
            }
            eprintln!("done: {} requests sent", wire.n);
        }
        Some("replay") => {
            let run = arg(&args, "--run").expect("--run ID");
            let mut wire = Replay::load(&read(&replays(&run)));
            wire.loose = flag(&args, "--loose");
            let text = read(&results);
            let (mut fresh, mut moved): (Vec<Record>, Vec<String>) = (Vec::new(), Vec::new());
            for r in load(&text).into_iter().filter(|r| r.meta.run == run) {
                let task = makes::find(&r.task).expect("a task of the suite");
                let (now, _) = evals::run::task(&r.meta, task, r.trial, &mut wire);
                let same = if now.line() == r.line() { "same" } else { "CHANGED" };
                println!(
                    "{:<12} trial {}: {same} {} -> {} {}",
                    r.task, r.trial, r.stage, now.stage, now.reason
                );
                if (now.prompt, now.knobs) != (r.prompt, r.knobs) {
                    moved.push(format!("{} trial {}", r.task, r.trial));
                }
                fresh.push(now);
            }
            wire.diverged.iter().for_each(|d| eprintln!("diverged: {d}"));
            if !flag(&args, "--write") {
                return;
            }
            // Written only when exact: every request as recorded, under the prompt and knobs
            // it was made with. Anything else needs a live run.
            if wire.loose {
                return eprintln!(
                    "not written: a loose replay only approximates a changed harness"
                );
            }
            if !wire.diverged.is_empty() {
                return eprintln!(
                    "not written: {} requests are not in the recording, so the harness asks \
                     differently now and the run needs a live run",
                    wire.diverged.len()
                );
            }
            if !moved.is_empty() {
                return eprintln!(
                    "not written: the coder's prompt or knobs changed since {} was made, which a \
                     live run measures",
                    moved.join(", ")
                );
            }
            // Each of its task's trials graded again, where it first came (a resumed one's
            // earlier lines go); every other line as it was.
            let (mut out, mut done) = (String::new(), Vec::new());
            for line in text.lines() {
                let Some(r) = Record::parse(line).filter(|r| r.meta.run == run) else {
                    out += &[line, "\n"].concat();
                    continue;
                };
                let at = (r.task, r.trial);
                if !done.contains(&at) {
                    out += &fresh
                        .iter()
                        .find(|n| (&n.task, n.trial) == (&at.0, at.1))
                        .map_or_else(String::new, Record::line);
                    done.push(at);
                }
            }
            std::fs::write(&results, out).expect("the results file");
        }
        Some("summary") => {
            let all = load(&read(&results));
            if flag(&args, "--markdown") {
                return print!("{}", evals::summary::table(&all));
            }
            let mut runs: Vec<&str> = Vec::new();
            for r in &all {
                runs.retain(|x| *x != r.meta.run);
                runs.push(&r.meta.run);
            }
            let pick = |name: &str, back: usize| {
                arg(&args, name)
                    .or_else(|| runs.iter().rev().nth(back).map(|s| s.to_string()))
                    .unwrap_or_default()
            };
            let (a, b) = (pick("--a", 1), pick("--b", 0));
            let (ra, rb) = (evals::summary::select(&all, &a), evals::summary::select(&all, &b));
            print!("{}", evals::summary::compare(&a, &ra, &b, &rb));
        }
        Some("list") => print!("{}", makes::listing()),
        Some("iq") => iq(&args),
        Some("team") => team(&args),
        _ => eprintln!("usage: eval run|replay|summary|list|iq|team (see tools/eval/src/main.rs)"),
    }
}

/// `iq`: see the crate docs.
fn iq(args: &[String]) {
    let need = |name: &str| arg(args, name).unwrap_or_else(|| panic!("{name} is needed"));
    let (suite, out, model) = (need("--suite"), need("--out"), need("--model"));
    let url = arg(args, "--url").unwrap_or_else(|| URL.into());
    let name = arg(args, "--name").unwrap_or_else(|| model.clone());
    let (held, only) = (arg(args, "--held").map(|h| read(&h)), arg(args, "--tasks"));
    let mut k = coder::Knobs::default();
    k.ms = arg(args, "--ms").and_then(|v| v.parse().ok()).unwrap_or(k.ms);
    let jobs = arg(args, "--jobs").and_then(|v| v.parse().ok()).unwrap_or(1usize);
    let jobs = if url == URL { 1 } else { jobs.max(1) };
    let per_hour = arg(args, "--per-hour").and_then(|v| v.parse().ok()).unwrap_or(PER_HOUR);
    let text = |j: &Json, key: &str| j.get(key).and_then(Json::text).unwrap_or("").to_string();
    let had: Vec<String> =
        read(&out).lines().filter_map(Json::parse).map(|j| text(&j, "task")).collect();
    // A dev tool's one run: its tasks' words live as long as it does.
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let tasks: Vec<makes::Task> = read(&suite)
        .lines()
        .filter_map(Json::parse)
        .filter(|j| {
            let (id, family) = (text(j, "id"), text(j, "family"));
            let asked = match (&only, &held) {
                (Some(o), _) => o.split(',').any(|o| o == id),
                (None, Some(h)) => h.lines().any(|f| f.trim() == family),
                (None, None) => true,
            };
            asked && !had.contains(&id)
        })
        .map(|j| makes::Task {
            id: leak(text(&j, "id")),
            size: "",
            ask: leak(text(&j, "ask")),
            check: |_| Ok(()),
        })
        .collect();
    eprintln!("{} tasks to make, {} made already", tasks.len(), had.len());
    let (next, file) = (Mutex::new(tasks.iter()), Mutex::new(()));
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                let mut wire = Curl::new(usize::MAX, &url);
                wire.per_hour = per_hour.min(PER_HOUR);
                while let Some(t) = next.lock().ok().and_then(|mut it| it.next()) {
                    let (d, _) = evals::run::make(t, &model, 1, k, &mut wire);
                    eprintln!("{:<28} {} ({} requests)", t.id, d.said(), d.receipt.turns.len());
                    if d.outcome == coder::Outcome::Failed {
                        continue;
                    }
                    let program = ["```app\n", d.draft.trim_end_matches('\n'), "\n```"].concat();
                    let turns: Vec<String> = d.receipt.turns.iter().map(|t| format!("{:?}", t.turn)).collect();
                    let line = format!(
                        "{{\"task\":{},\"model\":{},\"reply\":{},\"make\":{{\"said\":{},\"turns\":{},\"code\":{},\"ms\":{}}}}}\n",
                        quote(t.id),
                        quote(&name),
                        quote(if d.install { &program } else { "" }),
                        quote(&d.said()),
                        quote(&turns.join(" ")),
                        d.code,
                        d.receipt.ms
                    );
                    let _one = file.lock();
                    append(&out, &line);
                }
            });
        }
    });
}

/// The helper's whole reply to `body` over `wire` (for task `task`, its `n`th request), and
/// whether it ran out of room; or why there was none.
fn helper_reply(wire: &mut Curl, task: &str, n: u32, body: &str) -> Result<(String, bool), String> {
    let (mut stream, mut text) = (coder::json::Stream::default(), String::new());
    let at = At { task, trial: 1, n, hash: 0 };
    let answer = wire.post(&at, body, &mut |bytes, _| {
        stream.feed(bytes, &mut text, usize::MAX);
        Feed::More
    });
    stream.end(&mut text, usize::MAX);
    match answer.status {
        200..=299 if stream.error.is_empty() => Ok((text, stream.finish == "length")),
        status => Err(format!("HTTP {status} {} {}", answer.error, stream.error)),
    }
}

/// `team`: see the crate docs.
fn team(args: &[String]) {
    let need = |name: &str| arg(args, name).unwrap_or_else(|| panic!("{name} is needed"));
    let (suite, answers, out) = (need("--suite"), need("--answers"), need("--out"));
    let url = need("--helper-url");
    let helper = arg(args, "--helper").unwrap_or_else(|| "helper".into());
    let name = arg(args, "--name").unwrap_or_else(|| ["team-", &helper].concat());
    let turns: u8 = arg(args, "--turns").and_then(|v| v.parse().ok()).unwrap_or(3);
    let jobs = arg(args, "--jobs").and_then(|v| v.parse().ok()).unwrap_or(1usize).max(1);
    let held = arg(args, "--held").map(|h| read(&h));
    // Each helper turn, for study and for training the helpers: the message, the reply, how it
    // was judged.
    let traces = arg(args, "--traces");
    let text = |j: &Json, key: &str| j.get(key).and_then(Json::text).unwrap_or("").to_string();
    let had: Vec<String> =
        read(&out).lines().filter_map(Json::parse).map(|j| text(&j, "task")).collect();
    // The suite's asks, by task, for the families asked for.
    let asks: Vec<(String, String)> = read(&suite)
        .lines()
        .filter_map(Json::parse)
        .filter(|j| held.as_ref().is_none_or(|h| h.lines().any(|f| f.trim() == text(j, "family"))))
        .map(|j| (text(&j, "id"), text(&j, "ask")))
        .collect();
    // The lead's first answer to each of those tasks, not done already.
    let mut work: Vec<(String, String, String, String)> = Vec::new();
    for j in read(&answers).lines().filter_map(Json::parse) {
        let (task, lead, reply) = (text(&j, "task"), text(&j, "model"), text(&j, "reply"));
        let Some((_, ask)) = asks.iter().find(|a| a.0 == task) else { continue };
        if !had.contains(&task) && !work.iter().any(|w| w.0 == task) {
            work.push((task, ask.clone(), lead, reply));
        }
    }
    eprintln!("{} drafts to repair, {} done already", work.len(), had.len());
    let (next, file) = (Mutex::new(work.iter()), Mutex::new(()));
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                let mut wire = Curl::new(usize::MAX, &url);
                while let Some((task, ask, lead, reply)) =
                    next.lock().ok().and_then(|mut it| it.next())
                {
                    let t0 = Instant::now();
                    let draft = coder::edits::program(reply).map(|p| p.0.to_string());
                    let fixed = draft.map(|d| {
                        let (mut r, mut step) = team::Repair::start(ask, "", &d, &helper, turns);
                        let mut n = 0;
                        loop {
                            match step {
                                team::Next::Done(f) => break Ok(f),
                                team::Next::Ask(body) => {
                                    n += 1;
                                    match helper_reply(&mut wire, task, n, &body) {
                                        Ok((said, cut)) => {
                                            let asked = r.asked().to_string();
                                            step = r.reply(&said, cut);
                                            if let (Some(path), Some(s)) = (&traces, r.last()) {
                                                let line = format!(
                                                    "{{\"task\":{},\"turn\":{n},\"asked\":{},\"reply\":{},\"held\":{},\"before\":{},\"after\":{}}}
",
                                                    quote(task),
                                                    quote(&asked),
                                                    quote(&said),
                                                    quote(&format!("{:?}", s.held)),
                                                    s.before,
                                                    s.after
                                                );
                                                let _one = file.lock();
                                                append(path, &line);
                                            }
                                        }
                                        Err(e) => break Err(e),
                                    }
                                }
                            }
                        }
                    });
                    let f = match fixed {
                        Some(Ok(f)) => Some(f),
                        // The helper unreachable: not written, so a rerun repairs it.
                        Some(Err(e)) => {
                            eprintln!("{task:<28} helper failed: {e}");
                            continue;
                        }
                        None => None,
                    };
                    let ms = t0.elapsed().as_millis();
                    let (reply, said) = match &f {
                        Some(f) => {
                            let program = ["```app\n", f.src.trim_end_matches('\n'), "\n```"];
                            let held: Vec<String> =
                                f.steps.iter().map(|s| format!("{:?}", s.held)).collect();
                            let first = f.steps.first().map_or(0, |s| s.before);
                            let said = format!(
                                "{{\"lead\":{},\"helper\":{},\"turns\":{},\"held\":{},\"clean\":{},\"before\":{first},\"ms\":{ms}}}",
                                quote(lead),
                                quote(&helper),
                                f.steps.len(),
                                quote(&held.join(" ")),
                                f.clean
                            );
                            (program.concat(), said)
                        }
                        // No program in the lead's reply: nothing to repair.
                        None => (reply.clone(), format!("{{\"lead\":{},\"helper\":{},\"turns\":0,\"held\":\"\",\"clean\":false,\"before\":0,\"ms\":{ms}}}", quote(lead), quote(&helper))),
                    };
                    eprintln!("{task:<28} {said}");
                    let line = format!(
                        "{{\"task\":{},\"model\":{},\"reply\":{},\"team\":{said}}}\n",
                        quote(task),
                        quote(&name),
                        quote(&reply)
                    );
                    let _one = file.lock();
                    append(&out, &line);
                }
            });
        }
    });
}
