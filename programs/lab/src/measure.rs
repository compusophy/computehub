//! Measuring a writer of programs: prompted by an app's header (its first comment line, as
//! every app Studio makes begins), it writes until its end token; each program is judged by
//! applang's own checker and runtime. Seeded, so a measure run again says the same.

use std::collections::{BTreeMap, BTreeSet};

use tiny::{EOS, Model, Rng, Session, Tokenizer};

use crate::corpus;
use crate::ngram::Ngram;

/// The headers prompted: five like apps in the corpus of 2026-10-05, five like none of its
/// apps.
pub const PROMPTS: [&str; 10] = [
    "// Counter: two buttons change one number; Reset sets it back to 0.",
    "// Todo: type a task and press Add; tap its box to check it off.",
    "// Snake: arrows or a tap steer the snake to the food; walls and its tail end it.",
    "// Paint: tap or drag on the canvas to paint squares in the color picked.",
    "// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.",
    "// Dice: press Roll to throw two dice and show their total.",
    "// Timer: Start counts down from 60 seconds; Stop pauses it.",
    "// Clock: a round face whose hands show the seconds since it opened.",
    "// Pong: drag the paddle to keep the ball in play; each return scores.",
    "// Quiz: three questions, each with buttons for its answers, and the score at the end.",
];

/// Samples per prompt: 10 prompts, 100 programs.
pub const EACH: usize = 10;
/// The most tokens a program may take.
pub const MAX_TOKENS: usize = 2048;
/// How a writer samples: temperature (chosen before anything was measured; [`SWEEP`] shows
/// others), and top-k (the transformer's).
pub const TEMP: f32 = 0.8;
pub const TOP_K: usize = 40;
/// The other temperatures the transformers are measured at.
pub const SWEEP: [f32; 3] = [0.2, 0.5, 1.0];

/// One program written, and the verdict on it.
#[derive(Debug, Clone, Default)]
pub struct Sample {
    /// Which of [`PROMPTS`].
    pub prompt: usize,
    /// The program: its header, then what was written.
    pub text: String,
    /// Whether the writer ended it (its end token) before [`MAX_TOKENS`].
    pub ended: bool,
    pub compiles: bool,
    /// Runs clean: [`corpus::runs`].
    pub runs: bool,
    /// No program of the corpus has its [`corpus::shape`]: not a copy, renamed or not.
    pub novel: bool,
    /// Why not: the compiler's code, else the smoke test's (or 0: its icon).
    pub code: Option<u16>,
    /// The share of what was written (after the header) that comes before the compiler's first
    /// error: 1 when it compiles. A broken program can still be mostly right.
    pub clean: f64,
}

/// Judges `text`: compiles, runs, is new to `known` (the corpus's shapes).
pub fn judge(prompt: usize, text: String, ended: bool, known: &BTreeSet<u64>) -> Sample {
    let novel = !known.contains(&corpus::shape(&text));
    let head = if text.starts_with(PROMPTS[prompt]) { PROMPTS[prompt].len() } else { 0 };
    let written = text.len().saturating_sub(head).max(1) as f64;
    let (compiles, code, clean) = match applang::compile(&text) {
        Err(d) => {
            let at = d.span.map_or(0, |s| s.start).saturating_sub(head);
            (false, Some(d.code.unwrap_or(0)), (at as f64 / written).min(1.0))
        }
        Ok(p) if p.widgets().is_empty() => (false, Some(applang::codes::SHOWS_NOTHING), 1.0),
        Ok(_) => (true, None, 1.0),
    };
    let fault = compiles.then(|| coder::ai::fault(&text, "", 3)).flatten();
    let code = code.or(fault.as_ref().map(|f| f.diag.code.unwrap_or(0)));
    let runs = compiles && fault.is_none();
    Sample { prompt, text, ended, compiles, runs, novel, code, clean }
}

/// A writer's prompt: the end token (as each program follows one in training) and the header's
/// tokens but its last; and the bytes that last stood for, with which the writer's first token
/// must begin. BPE joins the end of a line to what follows (`\nstate `, `.\n`), so a header
/// encoded alone, or with its newline, may end in tokens training never shows there; backed off
/// one token, the writer redraws it with whatever follows (token healing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub tokens: Vec<u32>,
    pub heal: Vec<u8>,
}

/// Prompt `p`'s [`Prompt`].
pub fn prompt(tok: &Tokenizer, p: usize) -> Prompt {
    let mut tokens = vec![EOS];
    tokens.extend(tok.encode(PROMPTS[p]));
    let heal = match tokens.len() {
        1 => Vec::new(),
        _ => tokens.pop().map_or(Vec::new(), |last| tok.piece(last).to_vec()),
    };
    Prompt { tokens, heal }
}

/// Whether `token` may come first after `p`: its bytes begin with the healed ones.
fn heals(tok: &Tokenizer, p: &Prompt, token: u32) -> bool {
    token != EOS && tok.piece(token).starts_with(&p.heal)
}

/// The program a writer's tokens `out` make after prompt `p` (the first of them heals it):
/// the prompt's text, then the tokens up to the end token; and whether it came.
pub fn program(tok: &Tokenizer, p: usize, out: &[u32]) -> (String, bool) {
    let end = out.iter().position(|&t| t == EOS);
    let mut all = prompt(tok, p).tokens.split_off(1);
    all.extend_from_slice(&out[..end.unwrap_or(out.len())]);
    (tok.decode(&all), end.is_some())
}

/// Sample `i` of each prompt's [`EACH`]: its prompt and its seed (from `seed`).
fn job(i: usize, seed: u64) -> (usize, Rng) {
    (i / EACH, Rng::new(seed ^ (0x5eed_0000 + i as u64)))
}

/// 100 programs from `model` at `temp`, judged, on `threads` threads.
pub fn from_model(
    m: &Model,
    tok: &Tokenizer,
    known: &BTreeSet<u64>,
    (seed, temp): (u64, f32),
    threads: usize,
) -> Vec<Sample> {
    let jobs: Vec<usize> = (0..PROMPTS.len() * EACH).collect();
    corpus::par_map(&jobs, threads, |&i| {
        let (p, mut rng) = job(i, seed);
        let pr = prompt(tok, p);
        let mut s = Session::new(m);
        let mut logits = Vec::new();
        for &t in &pr.tokens {
            logits = s.feed(t).to_vec();
        }
        for (t, l) in (0u32..).zip(&mut logits) {
            if !heals(tok, &pr, t) {
                *l = f32::NEG_INFINITY;
            }
        }
        let mut out = vec![tiny::sample(&logits, temp, TOP_K, &mut rng)];
        let seq = [pr.tokens.as_slice(), &out].concat();
        out.extend(tiny::generate(m, &seq, MAX_TOKENS - 1, Some(EOS), (temp, TOP_K), &mut rng));
        let (text, ended) = program(tok, p, &out);
        judge(p, text, ended, known)
    })
}

/// 100 programs from `g`, as [`from_model`].
pub fn from_ngram(
    g: &Ngram,
    tok: &Tokenizer,
    known: &BTreeSet<u64>,
    seed: u64,
    threads: usize,
) -> Vec<Sample> {
    let jobs: Vec<usize> = (0..PROMPTS.len() * EACH).collect();
    corpus::par_map(&jobs, threads, |&i| {
        let (p, mut rng) = job(i, seed);
        let pr = prompt(tok, p);
        let temp = f64::from(TEMP);
        // Never seen after any context: the token backed off, as the header has it.
        let first = g.sample_where(&pr.tokens, temp, &mut rng, &|t| heals(tok, &pr, t));
        let last = tok.encode(PROMPTS[p]).last().copied().unwrap_or(EOS);
        let mut out = vec![first.unwrap_or(last)];
        let seq = [pr.tokens.as_slice(), &out].concat();
        out.extend(g.generate(&seq, MAX_TOKENS - 1, temp, &mut rng));
        let (text, ended) = program(tok, p, &out);
        judge(p, text, ended, known)
    })
}

/// A writer's line of the results table: samples, ended, compile, run clean, novel, novel and
/// compile, novel and run clean, the mean clean share.
pub fn row(name: &str, s: &[Sample]) -> String {
    let n = |f: &dyn Fn(&Sample) -> bool| s.iter().filter(|x| f(x)).count();
    format!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {:.2} |\n",
        s.len(),
        n(&|x| x.ended),
        n(&|x| x.compiles),
        n(&|x| x.runs),
        n(&|x| x.novel),
        n(&|x| x.novel && x.compiles),
        n(&|x| x.novel && x.runs),
        s.iter().map(|x| x.clean).sum::<f64>() / s.len().max(1) as f64,
    )
}

/// The commonest reasons a writer's programs failed, `E0101 x 30, ...`.
pub fn reasons(s: &[Sample]) -> String {
    let mut by: BTreeMap<u16, usize> = BTreeMap::new();
    for x in s.iter().filter(|x| !x.runs) {
        *by.entry(x.code.unwrap_or(0)).or_default() += 1;
    }
    let mut v: Vec<(u16, usize)> = by.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let each: Vec<String> = v.iter().map(|(c, n)| format!("E{c:04} x {n}")).collect();
    if each.is_empty() { "none".into() } else { each.join(", ") }
}

/// Per prompt: how many of its programs compiled and ran clean, `3 / 1`.
pub fn per_prompt(s: &[Sample]) -> Vec<(usize, usize)> {
    (0..PROMPTS.len())
        .map(|p| {
            let mine = s.iter().filter(|x| x.prompt == p);
            mine.fold((0, 0), |(c, r), x| (c + usize::from(x.compiles), r + usize::from(x.runs)))
        })
        .collect()
}
