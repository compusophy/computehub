//! `lab`, the model lab's commands (paths are the repo's, wherever it runs from):
//!
//! - `corpus`: builds the corpus and writes its manifest, `programs/lab/data/manifest.tsv`.
//! - `train`: trains tiny on the corpus's training programs (the manifest must be the corpus's
//!   as it is now), logging to `programs/lab/out/train.log` and keeping the weights at each
//!   held-out check in `programs/lab/out/tiny.bin`, and those with the lowest held-out loss in
//!   `best.bin`; it stops early after `--patience` checks (4; 0: never) with no new lowest.
//!   Options: `--steps --threads --seed --vocab --ctx --dim --layers --heads --batch --lr
//!   --patience`, and `--found 1` to train on the found programs without their variants; the
//!   defaults are what was trained (see `programs/lab/data/results.md`).
//! - `measure`: 100 programs from each set of weights kept and from two n-gram baselines,
//!   judged, each free and constrained by applang's lexer and by its parser;
//!   `programs/lab/data/results.md` (its last section, the runs before, kept), and every
//!   program in `programs/lab/out/samples/`.
//! - `bench`: a few training steps of the shape given, timed.
//!
//! At most 8 threads (of the machine's 16), whatever `--threads` asks. An option not known, or
//! a value that does not read as its kind, is refused.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::time::Instant;

use lab::constrain::Rule;
use lab::corpus::Corpus;
use lab::measure::{self, PROMPTS, Sample};
use lab::{DATA, FOUND_ONLY, OUT};
use tiny::{Config, Model, Ngram, Rng, Tokenizer, Trainer};

const USAGE: &str = "usage: lab corpus | train | measure | bench [--option value]...; options: \
                     --steps --threads --seed --vocab --ctx --dim --layers --heads --batch --lr \
                     --patience --found";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match Opts::parse(args.get(1..).unwrap_or(&[])) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let code = match args.first().map(String::as_str) {
        Some("corpus") => corpus(&o),
        Some("train") => train(&o),
        Some("measure") => measure(&o),
        Some("bench") => bench(&o),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// `--key value` pairs, each key one of [`USAGE`]'s.
struct Opts(BTreeMap<String, String>);

/// The options known.
const KEYS: [&str; 12] = [
    "steps", "threads", "seed", "vocab", "ctx", "dim", "layers", "heads", "batch", "lr",
    "patience", "found",
];

impl Opts {
    fn parse(args: &[String]) -> Result<Opts, String> {
        let mut map = BTreeMap::new();
        for pair in args.chunks(2) {
            let key = pair[0].strip_prefix("--").filter(|k| KEYS.contains(k));
            match (key, pair.get(1)) {
                (None, _) => return Err(format!("{}: not an option", pair[0])),
                (Some(k), None) => return Err(format!("--{k} needs a value")),
                (Some(k), Some(v)) => _ = map.insert(k.to_string(), v.clone()),
            }
        }
        Ok(Opts(map))
    }

    /// The value of `key`, or `or` if not given; a value that does not read as a `T` ends the
    /// run.
    fn get<T: std::str::FromStr>(&self, key: &str, or: T) -> T {
        let Some(v) = self.0.get(key) else { return or };
        v.parse().unwrap_or_else(|_| {
            eprintln!("--{key} {v}: not a {}\n{USAGE}", std::any::type_name::<T>());
            std::process::exit(2)
        })
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
/// The cosine's length, near where training stops (planned over 600, it stopped at 270).
const STEPS: usize = 300;
const LR: f32 = 3e-3;
const SEED: u64 = 1729;
/// Steps between held-out checks.
const EVAL: usize = 30;
/// Held-out checks in a row with no new lowest loss before training stops.
const PATIENCE: usize = 4;
/// The weights kept: the last checked, and those with the lowest held-out loss.
const LAST: &str = "tiny.bin";
const BEST: &str = "best.bin";
/// The heading of `results.md`'s last section, the runs before, which a measure keeps.
const EARLIER: &str = "## Earlier runs";

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
    let counts = manifest.lines().find(|l| l.contains(" programs: "));
    println!("{}", counts.unwrap_or(""));
    println!("{bytes} bytes; corpus {:016x}; written to {DATA}/manifest.tsv", c.id());
    0
}

/// The corpus as the repo has it now, if its manifest on disk holds the same programs (where
/// each was found may have moved).
fn checked_corpus(threads: usize) -> Option<Corpus> {
    let root = lab::root();
    let c = Corpus::build(&root, threads);
    let on_disk = fs::read_to_string(root.join(DATA).join("manifest.tsv")).unwrap_or_default();
    if lab::corpus::content(&on_disk) != lab::corpus::content(&c.manifest()) {
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
    let trained = lab::trained(&c, found);
    let texts: Vec<&str> = trained.train().map(|p| p.text.as_str()).collect();
    let tok = Tokenizer::train(&texts, o.get("vocab", VOCAB));
    let cfg = o.config(tok.vocab());
    let stream = lab::stream(&trained, &tok, seed);
    let held = lab::held(&c, &tok, cfg.ctx);
    let what = if found { FOUND_ONLY } else { "" };
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
        "corpus {:016x}: {} programs trained on, {} held out ({} found, measured); {} tokens \
         ({:.2} bytes each), {} held out",
        c.id(),
        texts.len(),
        c.held().count(),
        c.held().filter(|p| p.of.is_none()).count(),
        stream.len(),
        texts.iter().map(|t| t.len()).sum::<usize>() as f64 / stream.len() as f64,
        held.iter().map(|w| w.len() - 1).sum::<usize>()
    ));
    say(format!(
        "{cfg:?}: {} parameters; {steps} steps of {batch} x {}, lr {lr}, seed {seed}, {threads} threads",
        cfg.params(),
        cfg.ctx
    ));
    let (start, mut rng) = (Instant::now(), Rng::new(seed ^ 0xda7a));
    let (mut held_loss, mut best) = (f32::NAN, f32::INFINITY);
    // Checks since the lowest held-out loss; at `patience`, training stops.
    let (patience, mut stale, mut done) = (o.get("patience", PATIENCE), 0, steps);
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
        // The last weights at each check, and the weights with the lowest held-out loss. The
        // note says what made them and nothing that varies run to run (the time and threads
        // are in train.log), so the same training makes the same file, hash and all.
        let note = format!(
            "corpus {:016x} seed {seed}{what}: step {step} of {steps}, batch {batch} x {}, {} \
             tokens, lr {lr}; loss {:.4}, held-out {held_loss:.4}",
            c.id(),
            cfg.ctx,
            step * batch * cfg.ctx,
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
            (best, stale) = (held_loss, 0);
        } else if checked {
            stale += 1;
        }
        if patience > 0 && stale >= patience {
            say(format!("stopped: {stale} checks with no held-out loss under {best:.4}"));
            done = step;
            break;
        }
    }
    let minutes = start.elapsed().as_secs_f64() / 60.0;
    say(format!("trained: {done} steps in {minutes:.1} minutes on {threads} threads"));
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

/// A writer of programs measured: its name, what made it, its held-out loss, the rule its
/// tokens were drawn under, its programs.
struct Writer {
    name: String,
    made: String,
    loss: f64,
    rule: Rule,
    samples: Vec<Sample>,
}

/// The weights kept as `name`, if there are any, and readable; their note ends with the file's
/// own hash, which names it.
fn weights(name: &str) -> Option<Result<(Model, Tokenizer, String), String>> {
    let bytes = fs::read(lab::root().join(OUT).join(name)).ok()?;
    let loaded = tiny::load(&bytes).map_err(|e| format!("{OUT}/{name}: {e}"));
    // Loaded: the file ends in the hash of what comes before it.
    let hash = tiny::fnv(bytes.get(..bytes.len().saturating_sub(8)).unwrap_or(&[]));
    Some(loaded.map(|(m, tok, note)| (m, tok, format!("{note}; the file {hash:016x}"))))
}

fn measure(o: &Opts) -> i32 {
    let (threads, seed) = (o.threads(), o.get("seed", SEED));
    let mut tinies = Vec::new();
    for (name, file) in [("tiny, last", LAST), ("tiny, lowest held-out loss", BEST)] {
        let why = match weights(file) {
            Some(Ok(w)) => {
                tinies.push((name, w));
                continue;
            }
            Some(Err(e)) => e,
            None if file == LAST => format!("no weights at {OUT}/{LAST}: run `lab train` first"),
            None => continue,
        };
        eprintln!("{why}");
        return 1;
    }
    let Some(c) = checked_corpus(threads) else { return 1 };
    let id = format!("corpus {:016x}", c.id());
    if let Some((_, (_, _, note))) = tinies.iter().find(|t| !t.1.2.starts_with(&id)) {
        eprintln!("weights trained on another corpus: {note}");
        return 1;
    }
    // Both from one training run: one tokenizer, one stream.
    let (cfg, tok, note) = (tinies[0].1.0.cfg, tinies[0].1.1.clone(), tinies[0].1.2.clone());
    let found = note.contains(FOUND_ONLY);
    if tinies.iter().any(|t| t.1.1 != tok || t.1.2.contains(FOUND_ONLY) != found) {
        eprintln!("{OUT}/{LAST} and {OUT}/{BEST} are not from one training run: train again");
        return 1;
    }
    let known: BTreeSet<u64> = c.programs.iter().map(|p| p.shape).collect();
    // The baselines count what tiny was trained on, token for token.
    let stream = lab::stream(&lab::trained(&c, found), &tok, seed);
    let held = lab::held(&c, &tok, cfg.ctx);
    let start = Instant::now();
    let mut writers = Vec::new();
    for (name, (m, _, note)) in &tinies {
        let loss = f64::from(Trainer::new(m.clone(), threads).eval(&held));
        for rule in Rule::ALL {
            let samples =
                measure::from_model(m, &tok, &known, (seed, measure::TEMP, rule), threads);
            let (name, made) = (format!("{name}{}", rule.suffix()), note.clone());
            eprintln!("{name}: {:.0} s", start.elapsed().as_secs_f64());
            writers.push(Writer { name, made, loss, rule, samples });
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
    let low = grams.iter().enumerate().min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map_or(0, |g| g.0);
    for (i, why) in [(low, "the lowest held-out loss"), (grams.len() - 1, "the longest context")] {
        let (g, loss) = &grams[i];
        if writers.iter().any(|w| w.name.starts_with(&format!("{}-gram", g.n))) {
            continue;
        }
        for rule in Rule::ALL {
            let samples = measure::from_ngram(g, &tok, &known, (seed, rule), threads);
            let made = format!(
                "counts of the training tokens, sampled from the longest context seen ({why})"
            );
            let name = format!("{}-gram{}", g.n, rule.suffix());
            writers.push(Writer { name, made, loss: *loss, rule, samples });
        }
    }
    // Free first, then each rule, so a table reads as the rules' effect.
    writers.sort_by_key(|w| Rule::ALL.iter().position(|&r| r == w.rule));
    for Writer { name, samples, .. } in &writers {
        let slug: String =
            name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        for (i, x) in samples.iter().enumerate() {
            let _ = put_out(&format!("samples/{slug}-{i:03}.app"), x.text.as_bytes());
        }
    }
    let path = lab::root().join(DATA).join("results.md");
    // The runs before this one, as their own measure wrote them: kept below it.
    let before = fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    let earlier = before.find(EARLIER).map_or(String::new(), |at| ["\n", &before[at..]].concat());
    let report = results(&c, (&cfg, &held), &grams, &writers) + &earlier;
    if let Err(e) = fs::File::create(&path).and_then(|mut f| f.write_all(report.as_bytes())) {
        eprintln!("cannot write {}: {e}", path.display());
        return 1;
    }
    print!("{report}");
    eprintln!("sampled and judged in {:.0} s", start.elapsed().as_secs_f64());
    0
}

/// Each program that compiled (its text, [`lab::corpus::normal`]): who wrote it, how many
/// times each, and its verdict.
type Compiled<'a> = BTreeMap<String, (Vec<(&'a String, usize)>, &'a Sample)>;

/// The results file.
fn results(
    c: &Corpus,
    (cfg, windows): (&Config, &[Vec<u32>]),
    grams: &[(Ngram, f64)],
    writers: &[Writer],
) -> String {
    let found = c.programs.iter().filter(|p| p.of.is_none()).count();
    let runs = c.programs.iter().filter(|p| p.runs).count();
    let is = |p: &lab::corpus::Program, how: &str| {
        p.of.as_ref().is_some_and(|o| o.1.split(['+', '#']).any(|k| k == how))
    };
    let made = |how: &str| c.programs.iter().filter(|p| is(p, how)).count();
    let shapes: BTreeSet<u64> = c.programs.iter().map(|p| p.shape).collect();
    // The shapes only the variants with a line dropped or numbers changed have.
    let changed = |p: &&lab::corpus::Program| is(p, "drop") || is(p, "numbers");
    let others: BTreeSet<u64> =
        c.programs.iter().filter(|p| !changed(p)).map(|p| p.shape).collect();
    let new = shapes.len() - others.len();
    let held: Vec<&lab::corpus::Program> = c.held().filter(|p| p.of.is_none()).collect();
    let train_bytes: usize = c.train().map(|p| p.text.len()).sum();
    let found_bytes: usize = c.train().filter(|p| p.of.is_none()).map(|p| p.text.len()).sum();
    let held_bytes = held.iter().map(|p| p.text.len()).sum::<usize>();
    let mut out = String::from("# tiny writes applang: measured\n\n");
    out += "Made by `cargo run -p compusophy-lab --release -- measure` from the weights `-- train` \
            kept (`programs/lab/out/`, never committed); seeded, so run again on the corpus and \
            weights it names it says the same (`lab train` and `measure` refuse any other \
            corpus). Derived: never edit it by hand. The last section, the runs before, is kept \
            as their own measure wrote them, each under a heading of its own.\n\n";
    out += &format!(
        "**Corpus** `{:016x}` (`manifest.tsv`): {} programs, {found} found in the repo and {} \
         variants of them ({} with names renamed, {} with their states reordered, {} with a line \
         dropped, {} with numbers changed): {} shapes in all (a shape is a program but for its \
         names, comments and spacing). A line dropped or numbers changed makes a near-copy that \
         is another program: those add {new} shapes (the rest are renamed copies of one \
         another). {runs} run clean. Trained on: {train_bytes} bytes, {found_bytes} of them the \
         found programs'. Held out, never trained on: the {} found programs of a tenth of the \
         shapes, with every program sharing a shape with them and their variants; the held-out \
         loss is over those {} found programs, each once ({held_bytes} bytes).\n\n",
        c.id(),
        c.programs.len(),
        c.programs.len() - found,
        made("rename"),
        made("states"),
        made("drop"),
        made("numbers"),
        shapes.len(),
        held.len(),
        held.len(),
    );
    out += &format!(
        "**tiny**: {} parameters: vocab {} (the bytes, the end token and BPE merges), context {}, \
         width {}, {} layers of {} heads, f32.\n\n",
        cfg.params(),
        cfg.vocab,
        cfg.ctx,
        cfg.dim,
        cfg.layers,
        cfg.heads
    );
    for w in writers.iter().filter(|w| w.rule == Rule::Free) {
        out += &format!("- {}: {}.\n", w.name, w.made);
    }
    let tokens: usize = windows.iter().map(|w| w.len() - 1).sum();
    out += &format!(
        "\n**Held-out loss**, nats a token over the held-out programs (lower is better), and a \
         byte ({tokens} tokens over {held_bytes} bytes; a byte's loss compares across \
         tokenizers, a token's does not). The lowest-loss weights, and the n-gram sampled for \
         its loss, were picked by this same loss, which flatters them a little; training stops \
         by it too:\n\n"
    );
    out += "| model | a token | a byte |\n|---|---|---|\n";
    let per_byte = |l: f64| l * tokens as f64 / held_bytes.max(1) as f64;
    let free = writers.iter().filter(|w| w.rule == Rule::Free && w.name.starts_with("tiny"));
    for w in free {
        out += &format!("| {} | {:.3} | {:.3} |\n", w.name, w.loss, per_byte(w.loss));
    }
    for (g, l) in grams {
        out += &format!("| {}-gram, Witten-Bell | {l:.3} | {:.3} |\n", g.n, per_byte(*l));
    }
    let uniform = (cfg.vocab as f64).ln();
    out += &format!("| uniform | {uniform:.3} | {:.3} |\n\n", per_byte(uniform));
    out += &format!(
        "**Programs written**: {} prompts (an app's header line each, below) x {}, sampled at \
         temperature {} (tiny from its top {} tokens), up to {} tokens or the end token. Each \
         writer reads the end token and the header but its last token, and redraws that token \
         with what follows it (BPE joins a line's end to the next line's start, as `\\nstate `). \
         *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test \
         (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, \
         if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its \
         shape (its tokens but comments, names numbered as they first appear): not a copy, \
         renamed or not. *Clean*: the share of what it wrote that comes before its first error \
         (1 when it compiles), the mean. The compiler lexes a whole program before it parses, \
         so a lexer error hides any parser error before it: the program cut at a lexer error is \
         compiled again, and a parser error there comes first. The checker's errors (names, \
         types) count only where nothing before them is wrong. The first run, below, took the \
         compiler's first reported error, a lexer error anywhere before any parser error: its \
         clean shares overstate.\n\n\
         Each writer writes three ways. *Free*: any token. *Lexer-constrained*: only tokens that \
         keep the program lexically valid by applang's lexer (strings closed on their line, no \
         character the language lacks, no letter run into a number) with its brackets matched, \
         and the end token only where all is closed. *Parser-constrained*: that, and applang's \
         parser finds no error before the end of what is written (its last token, while it may \
         grow, judged by each thing it can still become), and the end token only where the \
         program parses; so nothing it writes has a lexer or parser error, and one cut at the \
         limit has its first error at its end. A token the parser refuses is drawn again \
         without it; after {} refusals every token is judged and the draw is among those it \
         lets through. *Dead end*: the rule lets no token through, and the writer stops there. \
         Neither rule looks at names or types, or runs anything: the checker and the smoke test \
         judge every program the same way.\n\n",
        PROMPTS.len(),
        measure::EACH,
        measure::TEMP,
        measure::TOP_K,
        measure::MAX_TOKENS,
        lab::constrain::TRIES
    );
    out += "| writer | programs | ended | dead end | compile | run clean | novel | novel, compile \
            | novel, run clean | clean |\n|---|---|---|---|---|---|---|---|---|---|\n";
    for w in writers {
        out += &measure::row(&w.name, &w.samples);
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
    // Every program that compiled, by any writer, once each: what the counts above are made of.
    let mut compiled: Compiled<'_> = BTreeMap::new();
    for (name, x) in writers.iter().flat_map(|w| w.samples.iter().map(move |x| (&w.name, x))) {
        if x.compiles {
            let by = &mut compiled.entry(lab::corpus::normal(&x.text)).or_insert((Vec::new(), x)).0;
            match by.last_mut() {
                Some((n, times)) if *n == name => *times += 1,
                _ => by.push((name, 1)),
            }
        }
    }
    out += &format!("\nEvery program that compiled ({}):\n", compiled.len());
    if compiled.is_empty() {
        out += "\nnone.\n";
    }
    const SHOWN: usize = 12;
    for (text, (names, x)) in compiled.iter().take(SHOWN) {
        let what = match (x.runs, x.novel) {
            (true, true) => "runs clean, novel",
            (true, false) => "runs clean, a copy",
            (false, true) => "faults, novel",
            (false, false) => "faults, a copy",
        };
        let by: Vec<String> = names
            .iter()
            .map(|(n, k)| if *k == 1 { n.to_string() } else { format!("{n}, {k} times") })
            .collect();
        out += &format!("\n{} ({what}):\n\n```app\n{text}```\n", by.join("; "));
    }
    if compiled.len() > SHOWN {
        out += &format!("\nand {} more, in `programs/lab/out/samples/`.\n", compiled.len() - SHOWN);
    }
    out
}
