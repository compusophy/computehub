//! Measuring a writer of programs: prompted by an app's header (its first comment line, as
//! every app Studio makes begins), it writes until its end token; each program is judged by
//! applang's own checker and runtime. Seeded, so a measure run again says the same.

use std::collections::{BTreeMap, BTreeSet};

use applang::codes;
use tiny::{EOS, Model, Ngram, Rng, Session, Tokenizer};

use crate::constrain::{self, Guide, Rule};
use crate::corpus;

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
/// How a writer samples: temperature (chosen before anything was measured; the first run's
/// sweep of others, in `results.md`, found none better), and top-k (the transformer's).
pub const TEMP: f32 = 0.8;
pub const TOP_K: usize = 40;

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
    /// Why not: the first error's code ([`first_error`]), else the smoke test's (or 0: its icon).
    pub code: Option<u16>,
    /// The share of what was written (after the header) that comes before its first error
    /// ([`first_error`]): 1 when it compiles. A broken program can still be mostly right.
    pub clean: f64,
    /// Its rule let no token through: the writer stopped there ([`write`]).
    pub stuck: bool,
}

/// Where `text`'s first error is, and its code, the compiler having reported `d`. The compiler
/// lexes all before it parses, so a lexer error hides any parser error before it: the text cut
/// at a lexer error is compiled again, and a parser error there (not one the cut made, at its
/// end) comes first. The checker's errors count only when nothing before them is wrong.
pub fn first_error(text: &str, d: &applang::Diag) -> (usize, u16) {
    let (at, code) = (d.span.map_or(0, |s| s.start), d.code.unwrap_or(0));
    let cut = text.get(..at).filter(|_| (1..=5).contains(&code)).unwrap_or("");
    // The one parser error spanning a whole expression, a canvas's scene that is not a call:
    // one that runs to the cut is the cut's own.
    let called = |e: &applang::Diag, s: applang::Span| {
        e.message.starts_with("expected a call") && cut.get(s.end..).is_some_and(|r| r.trim() == "")
    };
    let earlier = applang::compile(cut).err().filter(|_| !cut.is_empty());
    match earlier.as_ref().and_then(|e| Some((e, e.span?, e.code?))) {
        Some((e, s, c)) if s.start < at && c < codes::DUP_STATE && !called(e, s) => (s.start, c),
        _ => (at, code),
    }
}

/// Judges `text`: compiles, runs, is new to `known` (the corpus's shapes).
pub fn judge(prompt: usize, text: String, ended: bool, known: &BTreeSet<u64>) -> Sample {
    let novel = !known.contains(&corpus::shape(&text));
    let head = if text.starts_with(PROMPTS[prompt]) { PROMPTS[prompt].len() } else { 0 };
    let written = text.len().saturating_sub(head).max(1) as f64;
    let (compiles, code, clean) = match applang::compile(&text) {
        Err(d) => {
            let (at, code) = first_error(&text, &d);
            (false, Some(code), (at.saturating_sub(head) as f64 / written).min(1.0))
        }
        Ok(p) if p.widgets().is_empty() => (false, Some(codes::SHOWS_NOTHING), 1.0),
        Ok(_) => (true, None, 1.0),
    };
    let fault = compiles.then(|| coder::ai::fault(&text, "", 3)).flatten();
    let code = code.or(fault.as_ref().map(|f| f.diag.code.unwrap_or(0)));
    let runs = compiles && fault.is_none();
    Sample { prompt, text, ended, compiles, runs, novel, code, clean, stuck: false }
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

/// What draws a writer's next token: after `seq`, among the tokens `allow` lets through
/// (none if it has none to give).
pub(crate) type Draw<'a> = dyn FnMut(&[u32], &dyn Fn(u32) -> bool, &mut Rng) -> Option<u32> + 'a;

/// The tokens a writer writes after prompt `p` under `rule`, up to [`MAX_TOKENS`] or the end
/// token (kept), and whether it stopped at a dead end: each drawn among those the rule lets
/// through (the first healing the prompt). One the parser refuses is drawn again without it;
/// after [`constrain::TRIES`] refusals, every token is judged and the draw is among those the
/// parser lets through. With none to draw the writer stops there: a dead end (the first token
/// is then the header's own last).
pub(crate) fn write(
    tok: &Tokenizer,
    p: usize,
    rule: Rule,
    draw: &mut Draw<'_>,
    rng: &mut Rng,
) -> (Vec<u32>, bool) {
    let pr = prompt(tok, p);
    let mut guide = Guide::new(tok, rule, &tok.decode(&pr.tokens[1..]));
    let mut seq = pr.tokens.clone();
    let (start, mut stuck) = (seq.len(), false);
    while seq.len() - start < MAX_TOKENS {
        let first = seq.len() == start;
        let mut mask = guide.mask();
        for (t, m) in (0u32..).zip(mask.iter_mut()).filter(|_| first) {
            *m = *m && heals(tok, &pr, t);
        }
        let mut next = None;
        for _ in 0..constrain::TRIES {
            let Some(t) = draw(&seq, &|t: u32| mask[t as usize], rng) else { break };
            if guide.parses(t) {
                next = Some(t);
                break;
            }
            mask[t as usize] = false;
        }
        if next.is_none() {
            for (t, m) in (0u32..).zip(mask.iter_mut()) {
                *m = *m && guide.parses(t);
            }
            next = draw(&seq, &|t: u32| mask[t as usize], rng);
        }
        let last = tok.encode(PROMPTS[p]).last().copied().unwrap_or(EOS);
        let t = match next {
            Some(t) => t,
            None if first => last,
            None => {
                stuck = true;
                break;
            }
        };
        guide.push(t);
        seq.push(t);
        if t == EOS {
            break;
        }
    }
    (seq.split_off(start), stuck)
}

/// 100 programs written under `rule`, each by a writer `writer` makes for it, judged, on
/// `threads` threads: sample `i` of each prompt's [`EACH`] seeded by `seed` and `i`.
fn hundred<W: FnMut(&[u32], &dyn Fn(u32) -> bool, &mut Rng) -> Option<u32>>(
    tok: &Tokenizer,
    known: &BTreeSet<u64>,
    (seed, rule): (u64, Rule),
    threads: usize,
    writer: impl Fn() -> W + Sync,
) -> Vec<Sample> {
    let jobs: Vec<usize> = (0..PROMPTS.len() * EACH).collect();
    tiny::par_map(&jobs, threads, |&i| {
        let (p, mut rng) = (i / EACH, Rng::new(seed ^ (0x5eed_0000 + i as u64)));
        let (out, stuck) = write(tok, p, rule, &mut writer(), &mut rng);
        let (text, ended) = program(tok, p, &out);
        Sample { stuck, ..judge(p, text, ended, known) }
    })
}

/// 100 programs from `model` at `temp` under `rule`, judged, on `threads` threads.
pub fn from_model(
    m: &Model,
    tok: &Tokenizer,
    known: &BTreeSet<u64>,
    (seed, temp, rule): (u64, f32, Rule),
    threads: usize,
) -> Vec<Sample> {
    hundred(tok, known, (seed, rule), threads, || {
        let (ctx, mut s) = (m.cfg.ctx, Session::new(m));
        let (mut fed, mut logits) = (0, Vec::new());
        // The model reads what it has not yet; when its context fills, it reads the last half
        // of it again and goes on from there (as `tiny::generate` does).
        move |seq: &[u32], allow: &dyn Fn(u32) -> bool, rng: &mut Rng| {
            while fed < seq.len() {
                if s.full() {
                    s.reset();
                    for &t in &seq[fed - ctx / 2..fed] {
                        s.feed(t);
                    }
                }
                logits = s.feed(seq[fed]).to_vec();
                fed += 1;
            }
            let masked: Vec<f32> = (0u32..)
                .zip(&logits)
                .map(|(t, &l)| if allow(t) { l } else { f32::NEG_INFINITY })
                .collect();
            let any = (0u32..).zip(&logits).any(|(t, _)| allow(t));
            any.then(|| tiny::sample(&masked, temp, TOP_K, rng))
        }
    })
}

/// 100 programs from `g` under `rule`, as [`from_model`]; it draws from the longest context
/// it has seen followed by a token let through.
pub fn from_ngram(
    g: &Ngram,
    tok: &Tokenizer,
    known: &BTreeSet<u64>,
    (seed, rule): (u64, Rule),
    threads: usize,
) -> Vec<Sample> {
    hundred(tok, known, (seed, rule), threads, || {
        |seq: &[u32], allow: &dyn Fn(u32) -> bool, rng: &mut Rng| {
            let ctx = &seq[seq.len().saturating_sub(g.n - 1)..];
            g.sample_where(ctx, f64::from(TEMP), rng, allow)
        }
    })
}

/// A writer's line of the results table: samples, ended, stopped at a dead end, compile, run
/// clean, novel, novel and compile, novel and run clean, the mean clean share.
pub fn row(name: &str, s: &[Sample]) -> String {
    let n = |f: &dyn Fn(&Sample) -> bool| s.iter().filter(|x| f(x)).count();
    format!(
        "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {:.2} |\n",
        s.len(),
        n(&|x| x.ended),
        n(&|x| x.stuck),
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
