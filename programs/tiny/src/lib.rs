//! # tiny: a small transformer in plain Rust
//!
//! A decoder-only language model of GPT-2's shape: token and position embeddings, pre-norm
//! blocks of causal self-attention and a GELU MLP, a final layer norm, and the head tied to the
//! token embeddings. Everything is f32 and written out here, with no dependencies:
//!
//! - [`Tokenizer`]: bytes, one end token ([`EOS`]) and BPE merges learned from a corpus;
//! - [`Model`]: seeded init, [`Model::loss`] and [`Model::grad`], whose backward pass is written
//!   by hand (tested against finite differences);
//! - [`Trainer`]: AdamW with gradient clipping, the batch's sequences spread over threads;
//! - [`Session`] and [`generate`]: sampling (temperature, top-k) with a KV cache;
//! - [`save`] and [`load`]: one weights file with its tokenizer and a note, ending in its
//!   FNV-1a hash ([`fnv`]);
//! - [`Ngram`]: the baseline a model must beat, counts over the same tokens;
//! - [`par_map`]: work spread over threads, its results in order.
//!
//! Seeded and ordered: the same seed, data and steps make the same weights, bit for bit, on one
//! thread or eight (each sequence's gradient is its own, and they are summed in order).
//!
//! ```
//! use tiny::{Config, Model, Rng, Session, Tokenizer, generate};
//!
//! let tok = Tokenizer::bytes();
//! let cfg = Config { vocab: tok.vocab(), ctx: 16, dim: 8, layers: 1, heads: 2 };
//! let model = Model::new(cfg, 1).unwrap();
//! let prompt = tok.encode("label");
//! let out = generate(&model, &prompt, 4, None, (0.8, 8), &mut Rng::new(7));
//! assert_eq!(out.len(), 4);
//! let mut s = Session::new(&model);
//! assert_eq!(s.feed(prompt[0]).len(), tok.vocab());
//! ```

#![forbid(unsafe_code)]

mod bpe;
mod file;
mod infer;
mod model;
mod ngram;
mod ops;
#[cfg(test)]
mod tests;
mod train;

pub use bpe::{EOS, PIECE, Tokenizer};
pub use file::{fnv, load, save};
pub use infer::{Session, generate, sample};
pub use model::{Acts, Model};
pub use ngram::Ngram;
pub use train::{AdamW, Stats, Trainer};

/// A model's shape. The MLP is 4 times `dim` wide; each head is `dim / heads` wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Tokens the model knows (a [`Tokenizer`]'s [`Tokenizer::vocab`]).
    pub vocab: usize,
    /// The most tokens it sees at once: its positions.
    pub ctx: usize,
    /// The width of every token's vector.
    pub dim: usize,
    pub layers: usize,
    pub heads: usize,
}

/// The largest config a file may hold: 2^24 tokens, positions or widths, 256 layers.
const MOST: usize = 1 << 24;
/// The most floats a model's parameters, or one sequence's activations, may take: 2^29 (2 GiB),
/// so that either fits a wasm32 tab's memory and every size fits a 32-bit `usize`.
const FLOATS: usize = 1 << 29;

impl Config {
    /// The parameters in one layer: two layer norms, attention's in and out, the MLP's.
    fn layer_size(&self) -> usize {
        model::sizes(self.dim).iter().sum()
    }

    /// Every parameter, none if the count overflows: the embeddings, the layers (each
    /// `12 dim^2 + 13 dim`: [`model::sizes`]) and the final layer norm.
    fn count(&self) -> Option<usize> {
        let c = self.dim;
        let layer = c.checked_mul(c)?.checked_mul(12)?.checked_add(c.checked_mul(13)?)?;
        let embed = self.vocab.checked_add(self.ctx)?.checked_mul(c)?;
        embed.checked_add(self.layers.checked_mul(layer)?)?.checked_add(c.checked_mul(2)?)
    }

    /// The floats of one sequence's [`Acts`], none if the count overflows: a layer's are
    /// `16 dim + 4` and `heads x ctx` (attention) a position, and the rest `vocab + 13 dim + 3`.
    fn acts(&self) -> Option<usize> {
        let (t, c) = (self.ctx, self.dim);
        let per = c.checked_mul(16)?.checked_add(self.heads.checked_mul(t)?)?.checked_add(4)?;
        let layers = self.layers.checked_mul(t)?.checked_mul(per)?;
        let rest = t.checked_mul(self.vocab.checked_add(c.checked_mul(13)?)?.checked_add(3)?)?;
        layers.checked_add(rest)
    }

    /// Every parameter: the embeddings, the layers and the final layer norm (`usize::MAX` for a
    /// shape so big the count overflows, which [`Config::check`] refuses).
    pub fn params(&self) -> usize {
        self.count().unwrap_or(usize::MAX)
    }

    /// Whether a model can have this shape: nothing zero or past 2^24 (layers past 256), `dim`
    /// a multiple of `heads`, and its parameters and one sequence's activations each at most
    /// 2^29 floats. Counted without overflow on any `usize`.
    pub fn check(&self) -> Result<(), Error> {
        let sizes = [self.vocab, self.ctx, self.dim, self.heads];
        let fits = |n: Option<usize>| n.is_some_and(|n| n <= FLOATS);
        let ok = sizes.iter().all(|&n| (1..=MOST).contains(&n))
            && (1..=256).contains(&self.layers)
            && self.dim % self.heads == 0
            && fits(self.count())
            && fits(self.acts());
        if ok { Ok(()) } else { Err(Error::Config) }
    }
}

/// What can go wrong reading a weights file, or making a model or tokenizer; each with a
/// stable code ([`Error::code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// E0961: the bytes do not start as a tiny weights file does.
    Magic,
    /// E0962: a format version this reader does not know.
    Version(u32),
    /// E0963: the file ends before its parts do, or runs on past them.
    Size,
    /// E0964: the bytes do not hash to the hash they end with.
    Hash { want: u64, got: u64 },
    /// E0965: a shape no model can have, or one its tokenizer disagrees with.
    Config,
    /// E0966: a merge (by its index) of a token not made before it, or making one longer than
    /// [`PIECE`] bytes.
    Merge(usize),
}

impl Error {
    /// The error's stable code.
    pub fn code(&self) -> u16 {
        match self {
            Error::Magic => 961,
            Error::Version(_) => 962,
            Error::Size => 963,
            Error::Hash { .. } => 964,
            Error::Config => 965,
            Error::Merge(_) => 966,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{:04}: ", self.code())?;
        match self {
            Error::Magic => write!(f, "not a tiny weights file"),
            Error::Version(v) => write!(f, "weights format version {v} is not known"),
            Error::Size => write!(f, "the file's size does not match its parts"),
            Error::Hash { want, got } => {
                write!(f, "the file hashes to {got:016x}, not the {want:016x} it ends with")
            }
            Error::Config => write!(f, "no model can have this shape"),
            Error::Merge(i) => {
                write!(f, "merge {i} names a token not made before it, or makes one too long")
            }
        }
    }
}

/// SplitMix64: a small, seeded, portable source of chance. Every draw is the same on every
/// machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), 53 bits of it.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in 0..n (n at least 1), without modulo bias worth the name: the high half of a
    /// 128-bit product.
    pub fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// A standard normal draw (Box-Muller).
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (1.0 - self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// `f` of each item, on up to `threads` threads (each taking the next item left), in the
/// items' order: the same results on any number of threads when `f` depends on its item alone.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (next, f) = (&AtomicUsize::new(0), &f);
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let work = move || {
            let mut mine = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { return mine };
                mine.push((i, f(item)));
            }
        };
        let handles: Vec<_> = (0..threads.max(1)).map(|_| s.spawn(work)).collect();
        let join = |h: std::thread::ScopedJoinHandle<'_, Vec<(usize, R)>>| match h.join() {
            Ok(mine) => mine,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        handles.into_iter().flat_map(join).collect()
    });
    done.sort_by_key(|d| d.0);
    done.into_iter().map(|d| d.1).collect()
}
