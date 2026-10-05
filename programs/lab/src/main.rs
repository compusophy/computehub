//! `lab`, the model lab's commands (paths are the repo's, wherever it runs from):
//!
//! - `corpus`: builds the corpus and writes its manifest, `programs/lab/data/manifest.tsv`.
//! - `train`: trains tiny on the corpus's training programs (the manifest must be the corpus's
//!   as it is now), logging to `programs/lab/out/train.log` and keeping the weights at each
//!   held-out check in `programs/lab/out/tiny.bin`, and those with the lowest held-out loss in
//!   `best.bin`. Options: `--steps --threads --seed --vocab --ctx --dim --layers --heads
//!   --batch --lr`, and `--found 1` to train on the found programs without their variants;
//!   the defaults are what was trained (see `programs/lab/data/results.md`).
//! - `measure`: 100 programs from each set of weights kept and from two n-gram baselines,
//!   judged; `programs/lab/data/results.md`, and every program in `programs/lab/out/samples/`.
//! - `bench`: a few training steps of the shape given, timed.
//!
//! At most 8 threads (of the machine's 16), whatever `--threads` asks.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::time::Instant;

use lab::corpus::Corpus;
use lab::measure::{self, PROMPTS, Sample};
use lab::ngram::Ngram;
use lab::{DATA, OUT};
use tiny::{Acts, Config, Model, Rng, Tokenizer, Trainer};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = Opts::parse(args.get(1..).unwrap_or(&[]));
    let code = match args.first().map(String::as_str) {
        Some("corpus") => corpus(&o),
        Some("train") => train(&o),
        Some("measure") => measure(&o),
        Some("bench") => bench(&o),
        _ => {
            eprintln!("usage: lab corpus | train | measure | bench [--option value]...");
            2
        }
    };
    std::process::exit(code);
}

/// `--key value` pairs.
struct Opts(BTreeMap<String, String>);

impl Opts {
    fn parse(args: &[String]) -> Opts {
        let mut map = BTreeMap::new();
        for pair in args.chunks(2) {
            if let [k, v] = pair {
                map.insert(k.trim_start_matches('-').to_string(), v.clone());
            }
        }
        Opts(map)
    }

    fn get<T: std::str::FromStr>(&self, key: &str, or: T) -> T {
        self.0.get(key).and_then(|v| v.parse().ok()).unwrap_or(or)
    }

    fn threads(&self) -> usize {
        self.get("threads", 8usize).clamp(1, 8)
    }

    /// The shape asked for, its vocab given by the tokenizer.
    fn config(&self, vocab: usize) -> Config {
        let (ctx, dim) = (self.get("ctx", CTX), self.get("dim", DIM));
        Config {
            vocab,
            ctx,
            dim,
            layers: self.get("layers", LAYERS),
            heads: self.get("heads", HEADS),
        }
    }
}

/// The shape and training trained tonight (see `programs/lab/data/results.md`).
const VOCAB: usize = 512;
const CTX: usize = 1024;
const DIM: usize = 128;
const LAYERS: usize = 4;
const HEADS: usize = 4;
const BATCH: usize = 8;
const STEPS: usize = 600;
const LR: f32 = 3e-3;
const SEED: u64 = 1729;
/// Steps between held-out checks.
const EVAL: usize = 30;
/// The weights kept: the last checked, and those with the lowest held-out loss.
const LAST: &str = "tiny.bin";
const BEST: &str = "best.bin";

fn corpus(o: &Opts) -> i32 {
    let root = lab::root();
    let c = Corpus::build(&root, o.threads());
    let manifest = c.manifest();
    let path = root.join(DATA).join("manifest.tsv");
    if let Err(e) = fs::create_dir_all(root.join(DATA)).and_then(|_| fs::write(&path, &manifest)) {
        eprintln!("cannot write {}: {e}", path.display());
        return 1;
    }
    let bytes: usize = c.programs.iter().map(|p| p.text.len()).sum();
    println!("{}", manifest.lines().nth(5).unwrap_or(""));
    println!("{bytes} bytes; corpus {:016x}; written to {DATA}/manifest.tsv", c.id());
    0
}

/// The corpus as the repo has it now, if its manifest on disk says the same.
fn checked_corpus(threads: usize) -> Option<Corpus> {
    let root = lab::root();
    let c = Corpus::build(&root, threads);
    let on_disk = fs::read_to_string(root.join(DATA).join("manifest.tsv")).unwrap_or_default();
    if on_disk != c.manifest() {
        eprintln!("the corpus differs from {DATA}/manifest.tsv: run `lab corpus` first");
        return None;
    }
    Some(c)
}

/// Writes `bytes` to `name` (a path) in [`OUT`], through a temporary file so that a reader
/// never sees half.
fn put_out(name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = lab::root().join(OUT).join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let part = lab::root().join(OUT).join([name, ".part"].concat());
    fs::write(&part, bytes)?;
    fs::rename(&part, path)
}

/// The learning rate at `step` of `steps`: up from 0 over the first tenth (at most 100 steps),
/// then down a cosine to a tenth of `top`.
fn schedule(step: usize, steps: usize, top: f32) -> f32 {
    let warm = (steps / 10).clamp(1, 100);
    if step <= warm {
        return top * step as f32 / warm as f32;
    }
    let done = (step - warm) as f32 / (steps - warm).max(1) as f32;
    top * (0.1 + 0.45 * (1.0 + (std::f32::consts::PI * done).cos()))
}

fn train(o: &Opts) -> i32 {
    let threads = o.threads();
    let Some(c) = checked_corpus(threads) else { return 1 };
    let (seed, steps, batch, lr) =
        (o.get("seed", SEED), o.get("steps", STEPS), o.get("batch", BATCH), o.get("lr", LR));
    // `--found 1`: the programs found alone, without their variants.
    let found = o.get("found", 0u8) == 1;
    let kept = c.programs.iter().filter(|p| !found || p.of.is_none()).cloned().collect();
    let trained = Corpus { programs: kept };
    let texts: Vec<&str> = trained.train().map(|p| p.text.as_str()).collect();
    let tok = Tokenizer::train(&texts, o.get("vocab", VOCAB));
    let cfg = o.config(tok.vocab());
    let stream = lab::stream(&trained, &tok, seed);
    let held = lab::held(&c, &tok, cfg.ctx);
    let what = if found { " on the found programs alone" } else { "" };
    let Ok(model) = Model::new(cfg, seed) else {
        eprintln!("no model has the shape {cfg:?}");
        return 1;
    };
    let mut t = Trainer::new(model, threads);
    let mut log = String::new();
    let mut say = |line: String| {
        println!("{line}");
        log += &line;
        log.push('\n');
        let _ = put_out("train.log", log.as_bytes());
    };
    say(format!(
        "corpus {:016x}: {} programs trained on, {} held out; {} tokens ({:.2} bytes each)",
        c.id(),
        texts.len(),
        c.held().count(),
        stream.len(),
        texts.iter().map(|t| t.len()).sum::<usize>() as f64 / stream.len() as f64
    ));
    say(format!(
        "{cfg:?}: {} parameters; {steps} steps of {batch} x {}, lr {lr}, seed {seed}, {threads} threads",
        cfg.params(),
        cfg.ctx
    ));
    let (start, mut rng) = (Instant::now(), Rng::new(seed ^ 0xda7a));
    let (mut held_loss, mut best) = (f32::NAN, f32::INFINITY);
    for step in 1..=steps {
        let rate = schedule(step, steps, lr);
        let windows: Vec<Vec<u32>> = (0..batch)
            .map(|_| {
                let at = rng.below(stream.len().saturating_sub(cfg.ctx + 1).max(1));
                stream[at..(at + cfg.ctx + 1).min(stream.len())].to_vec()
            })
            .collect();
        let s = t.step(&windows, rate, 1.0);
        let secs = start.elapsed().as_secs_f64();
        let checked = step % EVAL == 0 || step == steps;
        if checked {
            held_loss = t.eval(&held);
        }
        if step % 10 == 0 || step == 1 || checked {
            let rate_tok = (step * batch * cfg.ctx) as f64 / secs;
            let held_now = if checked { format!(" held {held_loss:.4}") } else { String::new() };
            say(format!(
                "step {step:5} loss {:.4}{held_now} norm {:.3} lr {rate:.2e} {:.0} tok/s {:.1} min",
                s.loss,
                s.norm,
                rate_tok,
                secs / 60.0
            ));
        }
        // The last weights at each check, and the weights with the lowest held-out loss.
        let note = format!(
            "corpus {:016x} seed {seed}{what}: step {step} of {steps}, batch {batch} x {}, {} tokens, lr {lr}, {threads} threads, {:.1} minutes; loss {:.4}, held-out {held_loss:.4}",
            c.id(),
            cfg.ctx,
            step * batch * cfg.ctx,
            secs / 60.0,
            s.loss
        );
        let keep = |name: &str| {
            let bytes = tiny::save(&t.model, &tok, &note).map_err(|e| e.to_string())?;
            put_out(name, &bytes).map_err(|e| e.to_string())
        };
        let kept = match checked {
            true if held_loss < best => keep(BEST).and_then(|_| keep(LAST)),
            true => keep(LAST),
            false => Ok(()),
        };
        if let Err(e) = kept {
            eprintln!("cannot keep the weights: {e}");
            return 1;
        }
        if checked && held_loss < best {
            best = held_loss;
        }
    }
    0
}

fn bench(o: &Opts) -> i32 {
    let (threads, batch) = (o.threads(), o.get("batch", BATCH));
    let cfg = o.config(o.get("vocab", VOCAB));
    let Ok(model) = Model::new(cfg, 1) else { return 1 };
    let mut t = Trainer::new(model, threads);
    let mut rng = Rng::new(2);
    let windows: Vec<Vec<u32>> =
        (0..batch).map(|_| (0..=cfg.ctx).map(|_| rng.below(cfg.vocab) as u32).collect()).collect();
    t.step(&windows, 1e-3, 1.0);
    let start = Instant::now();
    let n = o.get("steps", 5usize);
    for _ in 0..n {
        t.step(&windows, 1e-3, 1.0);
    }
    let secs = start.elapsed().as_secs_f64() / n as f64;
    let toks = (batch * cfg.ctx) as f64;
    println!(
        "{cfg:?}: {} params; {:.3} s a step of {batch} x {}, {:.0} tok/s, {threads} threads",
        cfg.params(),
        secs,
        cfg.ctx,
        toks / secs
    );
    0
}

/// A writer of programs measured: its name, what made it, its held-out loss, its programs.
struct Writer {
    name: String,
    made: String,
    loss: f64,
    samples: Vec<Sample>,
}

/// The weights kept as `name`, if there are any, and readable.
fn weights(name: &str) -> Option<Result<(Model, Tokenizer, String), String>> {
    let bytes = fs::read(lab::root().join(OUT).join(name)).ok()?;
    Some(tiny::load(&bytes).map_err(|e| format!("{OUT}/{name}: {e}")))
}

/// The mean loss of `m` over `held`, a token at a time, in nats.
fn held_loss(m: &Model, held: &[Vec<u32>]) -> f64 {
    let mut acts = Acts::new(&m.cfg);
    let (mut sum, mut count) = (0f64, 0usize);
    for w in held {
        let n = w.len() - 1;
        sum += f64::from(m.loss(&mut acts, &w[..n], &w[1..])) * n as f64;
        count += n;
    }
    sum / count.max(1) as f64
}

fn measure(o: &Opts) -> i32 {
    let (threads, seed) = (o.threads(), o.get("seed", SEED));
    let last = match weights(LAST) {
        Some(Ok(w)) => w,
        Some(Err(e)) => {
            eprintln!("{e}");
            return 1;
        }
        None => {
            eprintln!("no weights at {OUT}/{LAST}: run `lab train` first");
            return 1;
        }
    };
    let Some(c) = checked_corpus(threads) else { return 1 };
    let id = format!("corpus {:016x}", c.id());
    let mut tinies = vec![("tiny, last", last)];
    if let Some(Ok(best)) = weights(BEST) {
        tinies.push(("tiny, lowest held-out loss", best));
    }
    if let Some((_, (_, _, note))) = tinies.iter().find(|t| !t.1.2.starts_with(&id)) {
        eprintln!("weights trained on another corpus: {note}");
        return 1;
    }
    let (cfg, tok) = (tinies[0].1.0.cfg, tinies[0].1.1.clone());
    let known: BTreeSet<u64> = c.programs.iter().map(|p| lab::corpus::shape(&p.text)).collect();
    let stream = lab::stream(&c, &tok, seed);
    let held = lab::held(&c, &tok, cfg.ctx);
    let start = Instant::now();
    let mut writers = Vec::new();
    let mut sweep = Vec::new();
    for (name, (m, _, note)) in &tinies {
        let samples = measure::from_model(m, &tok, &known, (seed, measure::TEMP), threads);
        writers.push(Writer {
            name: name.to_string(),
            made: note.clone(),
            loss: held_loss(m, &held),
            samples,
        });
        for t in measure::SWEEP {
            let samples = measure::from_model(m, &tok, &known, (seed, t), threads);
            sweep.push((format!("{name}, temperature {t}"), samples));
        }
    }
    // The baselines: every order's held-out loss; samples from the order with the lowest, and
    // from the highest, which copies most.
    let grams: Vec<(Ngram, f64)> = (2..=8)
        .map(|n| {
            let g = Ngram::train(&stream, n, tok.vocab());
            let l = g.loss(&held);
            (g, l)
        })
        .collect();
    let Some(low) = grams.iter().enumerate().min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|g| g.0)
    else {
        return 1;
    };
    for (i, why) in [(low, "the lowest held-out loss"), (grams.len() - 1, "the longest context")] {
        let (g, loss) = &grams[i];
        if writers.iter().any(|w| w.name.starts_with(&format!("{}-gram", g.n))) {
            continue;
        }
        let samples = measure::from_ngram(g, &tok, &known, seed, threads);
        let made =
            format!("counts of the training tokens, sampled from the longest context seen ({why})");
        writers.push(Writer { name: format!("{}-gram", g.n), made, loss: *loss, samples });
    }
    for w in &writers {
        let slug: String =
            w.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        for (i, x) in w.samples.iter().enumerate() {
            let _ = put_out(&format!("samples/{slug}-{i:03}.app"), x.text.as_bytes());
        }
    }
    let report = results(&c, &cfg, &grams, &writers, &sweep);
    let path = lab::root().join(DATA).join("results.md");
    if let Err(e) = fs::File::create(&path).and_then(|mut f| f.write_all(report.as_bytes())) {
        eprintln!("cannot write {}: {e}", path.display());
        return 1;
    }
    print!("{report}");
    eprintln!("sampled and judged in {:.0} s", start.elapsed().as_secs_f64());
    0
}

/// The results file.
fn results(
    c: &Corpus,
    cfg: &Config,
    grams: &[(Ngram, f64)],
    writers: &[Writer],
    sweep: &[(String, Vec<Sample>)],
) -> String {
    let found = c.programs.iter().filter(|p| p.of.is_none()).count();
    let runs = c.programs.iter().filter(|p| p.runs).count();
    let mut out = String::from("# tiny writes applang: measured\n\n");
    out += "Made by `cargo run -p compusophy-lab --release -- measure` from the weights `-- train` \
            kept (`programs/lab/out/`, never committed); seeded, so run again it says the same. \
            Derived: never edit it by hand.\n\n";
    out += &format!(
        "**Corpus** `{:016x}` (`manifest.tsv`): {} programs, {found} found in the repo and {} \
         variants of them (names renamed, states reordered), {runs} of them running clean; {} \
         found programs held out with their variants, never trained on.\n\n",
        c.id(),
        c.programs.len(),
        c.programs.len() - found,
        c.held().filter(|p| p.of.is_none()).count(),
    );
    out += &format!(
        "**tiny**: {} parameters: vocab {} (the bytes, the end token and BPE merges), context {}, \
         width {}, {} layers of {} heads, f32, trained on 8 CPU threads.\n\n",
        cfg.params(),
        cfg.vocab,
        cfg.ctx,
        cfg.dim,
        cfg.layers,
        cfg.heads
    );
    for w in writers {
        out += &format!("- {}: {}.\n", w.name, w.made);
    }
    out += "\n**Held-out loss**, nats a token over the held-out programs (lower is better):\n\n";
    out += "| model | loss |\n|---|---|\n";
    for w in writers.iter().filter(|w| w.name.starts_with("tiny")) {
        out += &format!("| {} | {:.3} |\n", w.name, w.loss);
    }
    for (g, l) in grams {
        out += &format!("| {}-gram, Witten-Bell | {l:.3} |\n", g.n);
    }
    out += &format!("| uniform | {:.3} |\n\n", (cfg.vocab as f64).ln());
    out += &format!(
        "**Programs written**: {} prompts (an app's header line each, below) x {}, sampled at \
         temperature {} (tiny from its top {} tokens), up to {} tokens or the end token. \
         *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test \
         (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, \
         if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its \
         shape (its tokens but comments, names numbered as they first appear): not a copy, \
         renamed or not. *Clean*: the share of what it wrote that comes before the compiler's \
         first error (1 when it compiles), the mean.\n\n",
        PROMPTS.len(),
        measure::EACH,
        measure::TEMP,
        measure::TOP_K,
        measure::MAX_TOKENS
    );
    const HEAD: &str = "| writer | programs | ended | compile | run clean | novel | novel, \
                        compile | novel, run clean | clean |\n|---|---|---|---|---|---|---|---|---|\n";
    out += HEAD;
    for w in writers {
        out += &measure::row(&w.name, &w.samples);
    }
    out += "\nAt other temperatures (the same prompts and seeds):\n\n";
    out += HEAD;
    for (name, samples) in sweep {
        out += &measure::row(name, samples);
    }
    out += "\nWhy the rest did not run clean (the first error's code; 0: an icon that does not \
            draw):\n\n";
    for w in writers {
        out += &format!("- {}: {}\n", w.name, measure::reasons(&w.samples));
    }
    out += "\nPer prompt, compile / run clean of 10:\n\n| prompt |";
    for w in writers {
        out += &format!(" {} |", w.name);
    }
    out += "\n|---|";
    out += &"---|".repeat(writers.len());
    out.push('\n');
    let each: Vec<Vec<(usize, usize)>> =
        writers.iter().map(|w| measure::per_prompt(&w.samples)).collect();
    for (i, prompt) in PROMPTS.iter().enumerate() {
        out += &format!("| `{prompt}` |");
        for e in &each {
            out += &format!(" {} / {} |", e[i].0, e[i].1);
        }
        out.push('\n');
    }
    // The longest novel program tiny wrote that compiles, at any temperature.
    let tinies = writers.iter().filter(|w| w.name.starts_with("tiny"));
    let all = tinies.map(|w| (&w.name, &w.samples)).chain(sweep.iter().map(|(n, s)| (n, s)));
    let each = all.flat_map(|(n, s)| s.iter().map(move |x| (n, x)));
    let best = each.filter(|(_, x)| x.novel && x.compiles).max_by_key(|(_, x)| x.text.len());
    if let Some((name, x)) = best {
        let what = if x.runs { "runs clean" } else { "compiles but faults" };
        let text = lab::corpus::normal(&x.text);
        out += &format!(
            "\nThe longest novel program tiny wrote that compiles ({name}; it {what}):\n\n\
             ```app\n{text}```\n"
        );
    }
    out
}
