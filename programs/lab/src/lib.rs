//! # lab: tiny on applang, made and measured
//!
//! A dev tool, never shipped (its binary runs on the machine that builds compusophyOS). The
//! first verifiable step from small total languages to models that compute:
//!
//! - [`corpus`]: every applang program in the repo that compiles ([`collect`] finds them, the
//!   evals' answer keys aside), once, content-addressed, with variants that keep their meaning
//!   ([`augment`]);
//! - [`tiny::Ngram`]: the baseline any model must beat, over the same tokens;
//! - [`measure`]: programs sampled from a model, each prompted by an app's header, judged by
//!   applang's own checker and runtime: does it compile, does it run clean, is it new; each
//!   free, and [`constrain`]ed by applang's lexer and by its parser.
//!
//! `cargo run -p compusophy-lab --release -- corpus | train | measure` (see `main.rs`).

#![forbid(unsafe_code)]

pub mod augment;
pub mod collect;
pub mod constrain;
pub mod corpus;
pub mod measure;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use tiny::{EOS, Rng, Tokenizer};

/// The repo's root, which this crate is two folders under.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// What a weights file's note says when it was trained on the found programs alone, without
/// their variants (`lab train --found 1`).
pub const FOUND_ONLY: &str = " on the found programs alone";

/// The corpus trained on: `c`, or with `found_only` its found programs alone.
pub fn trained(c: &corpus::Corpus, found_only: bool) -> corpus::Corpus {
    let programs = c.programs.iter().filter(|p| !found_only || p.of.is_none()).cloned().collect();
    corpus::Corpus { programs }
}

/// What is trained on: the training programs in an order drawn from `seed`, each after an end
/// token, and one end token last.
pub fn stream(c: &corpus::Corpus, tok: &Tokenizer, seed: u64) -> Vec<u32> {
    let mut texts: Vec<&str> = c.train().map(|p| p.text.as_str()).collect();
    let mut rng = Rng::new(seed);
    for i in (1..texts.len()).rev() {
        texts.swap(i, rng.below(i + 1));
    }
    let mut out = Vec::new();
    for t in texts {
        out.push(EOS);
        out.extend(tok.encode(t));
    }
    out.push(EOS);
    out
}

/// What the held-out loss is measured on: each held-out program found (its variants, renamed
/// copies, would count it again) between end tokens, cut into windows of `ctx + 1` tokens
/// (each overlapping the last by one, so every token is predicted once).
pub fn held(c: &corpus::Corpus, tok: &Tokenizer, ctx: usize) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for p in c.held().filter(|p| p.of.is_none()) {
        let mut seq = vec![EOS];
        seq.extend(tok.encode(&p.text));
        seq.push(EOS);
        let mut at = 0;
        while at + 1 < seq.len() {
            out.push(seq[at..(at + ctx + 1).min(seq.len())].to_vec());
            at += ctx;
        }
    }
    out
}

/// The committed data (the corpus's manifest, the results), from the root.
pub const DATA: &str = "programs/lab/data";
/// What is never committed (checkpoints, logs, samples), from the root: git ignores it.
pub const OUT: &str = "programs/lab/out";
