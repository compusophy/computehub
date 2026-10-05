//! The baseline: an n-gram model over the same tokens the transformer reads. Its held-out loss
//! interpolates every order Witten-Bell's way, down to the uniform; it samples from the longest
//! context it has seen, backing off until it has seen one.

use std::collections::BTreeMap;

use tiny::{Rng, fnv};

/// What followed one context: how often, and how often each token.
#[derive(Debug, Clone, Default)]
struct Next {
    total: u32,
    /// Each token that followed and its count, by token.
    by: Vec<(u32, u32)>,
}

/// Counts of what follows each context of up to `n - 1` tokens.
#[derive(Debug, Clone)]
pub struct Ngram {
    pub n: usize,
    vocab: usize,
    /// Per context length k (0 to n - 1), by the FNV-1a hash of the context's tokens.
    counts: Vec<BTreeMap<u64, Next>>,
}

/// The key of a context.
fn key(ctx: &[u32]) -> u64 {
    let bytes: Vec<u8> = ctx.iter().flat_map(|t| t.to_le_bytes()).collect();
    fnv(&bytes)
}

impl Ngram {
    /// Counts `stream` (one sequence: its contexts run across the end tokens in it, as the
    /// transformer's windows do) for order `n` over `vocab` tokens.
    pub fn train(stream: &[u32], n: usize, vocab: usize) -> Ngram {
        let n = n.max(1);
        let mut counts = vec![BTreeMap::new(); n];
        for i in 0..stream.len() {
            for (k, map) in counts.iter_mut().enumerate().take(i + 1) {
                let next: &mut Next = map.entry(key(&stream[i - k..i])).or_default();
                next.total += 1;
                match next.by.binary_search_by_key(&stream[i], |b| b.0) {
                    Ok(at) => next.by[at].1 += 1,
                    Err(at) => next.by.insert(at, (stream[i], 1)),
                }
            }
        }
        Ngram { n, vocab, counts }
    }

    /// The chance of `w` after `ctx` (its last n - 1 tokens): the uniform, then each longer
    /// context seen, Witten-Bell: (count of w + kinds seen x lower) / (count + kinds seen).
    pub fn prob(&self, ctx: &[u32], w: u32) -> f64 {
        let mut p = 1.0 / self.vocab as f64;
        for k in 0..self.n.min(ctx.len() + 1) {
            let Some(next) = self.counts[k].get(&key(&ctx[ctx.len() - k..])) else { break };
            let kinds = next.by.len() as f64;
            let cw = next.by.binary_search_by_key(&w, |b| b.0).map_or(0, |at| next.by[at].1);
            p = (f64::from(cw) + kinds * p) / (f64::from(next.total) + kinds);
        }
        p
    }

    /// The mean of `-ln p` over every token of each sequence but its first, in nats.
    pub fn loss(&self, seqs: &[Vec<u32>]) -> f64 {
        let (mut sum, mut count) = (0f64, 0usize);
        for s in seqs {
            for i in 1..s.len() {
                sum -= self.prob(&s[..i], s[i]).ln();
                count += 1;
            }
        }
        sum / count.max(1) as f64
    }

    /// A token to follow `ctx`: drawn from what followed its longest context seen, each count
    /// raised to 1 / `temp`.
    pub fn sample(&self, ctx: &[u32], temp: f64, rng: &mut Rng) -> u32 {
        for k in (0..self.n.min(ctx.len() + 1)).rev() {
            let Some(next) = self.counts[k].get(&key(&ctx[ctx.len() - k..])) else { continue };
            let weights: Vec<f64> =
                next.by.iter().map(|b| f64::from(b.1).powf(1.0 / temp)).collect();
            let mut u = rng.unit() * weights.iter().sum::<f64>();
            for (w, b) in weights.iter().zip(&next.by) {
                if u < *w {
                    return b.0;
                }
                u -= w;
            }
            return next.by.last().map_or(tiny::EOS, |b| b.0);
        }
        tiny::EOS
    }

    /// Up to `max` tokens after `prompt`, stopping after [`tiny::EOS`] (kept).
    pub fn generate(&self, prompt: &[u32], max: usize, temp: f64, rng: &mut Rng) -> Vec<u32> {
        let mut seq = prompt.to_vec();
        let start = seq.len();
        while seq.len() - start < max {
            let from = seq.len().saturating_sub(self.n - 1);
            let next = self.sample(&seq[from..], temp, rng);
            seq.push(next);
            if next == tiny::EOS {
                break;
            }
        }
        seq.split_off(start)
    }
}
