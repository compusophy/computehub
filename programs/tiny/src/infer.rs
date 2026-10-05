//! Inference: one token at a time with a KV cache (each position's keys and values kept, so a
//! step costs one token's work), and sampling with a temperature and top-k.

use crate::model::{FC_W, FCP_W, LN1_W, LN2_W, PROJ_W, QKV_W};
use crate::ops::{axpy, dot, gelu, layernorm, matmul, softmax};
use crate::{Config, EOS, Model, Rng};

/// A model reading one sequence: its keys and values so far, and one token's scratch.
#[derive(Debug, Clone)]
pub struct Session<'m> {
    model: &'m Model,
    /// Keys and values: per layer, `ctx` rows of `dim`.
    k: Vec<f32>,
    v: Vec<f32>,
    len: usize,
    x: Vec<f32>,
    h: Vec<f32>,
    qkv: Vec<f32>,
    y: Vec<f32>,
    f: Vec<f32>,
    g: Vec<f32>,
    att: Vec<f32>,
    logits: Vec<f32>,
}

impl<'m> Session<'m> {
    pub fn new(model: &'m Model) -> Session<'m> {
        let Config { vocab, ctx, dim: c, layers, .. } = model.cfg;
        let z = |n: usize| vec![0f32; n];
        Session {
            model,
            k: z(layers * ctx * c),
            v: z(layers * ctx * c),
            len: 0,
            x: z(c),
            h: z(c),
            qkv: z(3 * c),
            y: z(c),
            f: z(4 * c),
            g: z(4 * c),
            att: z(ctx),
            logits: z(vocab),
        }
    }

    /// The tokens read so far.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the context is full: [`Session::feed`] needs a [`Session::reset`] first.
    pub fn full(&self) -> bool {
        self.len == self.model.cfg.ctx
    }

    /// Forgets the sequence.
    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Reads `token` at the next position; the next token's logits. On a full context, the
    /// logits of the last position read, unchanged.
    pub fn feed(&mut self, token: u32) -> &[f32] {
        let m = self.model;
        let Config { vocab, ctx, dim: c, layers, heads } = m.cfg;
        if self.len == ctx {
            return &self.logits;
        }
        let (pos, p, hs) = (self.len, &m.params, c / heads);
        let tok = (token as usize).min(vocab - 1);
        let (e, q) = (&p[m.cfg.wte()][tok * c..][..c], &p[m.cfg.wpe()][pos * c..][..c]);
        for ((x, e), q) in self.x.iter_mut().zip(e).zip(q) {
            *x = e + q;
        }
        let (mut mean, mut rstd) = ([0f32], [0f32]);
        let scale = 1.0 / (hs as f32).sqrt();
        for l in 0..layers {
            let w = |k: usize| &p[m.cfg.at(l, k)];
            layernorm((&mut self.h, &mut mean, &mut rstd), &self.x, w(LN1_W), w(LN1_W + 1));
            matmul(&mut self.qkv, &self.h, w(QKV_W), Some(w(QKV_W + 1)), c);
            let at = (l * ctx + pos) * c;
            self.k[at..at + c].copy_from_slice(&self.qkv[c..2 * c]);
            self.v[at..at + c].copy_from_slice(&self.qkv[2 * c..]);
            let (ks, vs) = (&self.k[l * ctx * c..], &self.v[l * ctx * c..]);
            for (h, y) in self.y.chunks_exact_mut(hs).enumerate() {
                let qh = &self.qkv[h * hs..][..hs];
                let att = &mut self.att[..=pos];
                for (t2, a) in att.iter_mut().enumerate() {
                    *a = dot(qh, &ks[t2 * c + h * hs..][..hs]) * scale;
                }
                softmax(att);
                y.fill(0.0);
                for (t2, &a) in att.iter().enumerate() {
                    axpy(y, a, &vs[t2 * c + h * hs..][..hs]);
                }
            }
            matmul(&mut self.h, &self.y, w(PROJ_W), Some(w(PROJ_W + 1)), c);
            self.x.iter_mut().zip(&self.h).for_each(|(x, h)| *x += h);
            layernorm((&mut self.h, &mut mean, &mut rstd), &self.x, w(LN2_W), w(LN2_W + 1));
            matmul(&mut self.f, &self.h, w(FC_W), Some(w(FC_W + 1)), c);
            gelu(&mut self.g, &self.f);
            matmul(&mut self.h, &self.g, w(FCP_W), Some(w(FCP_W + 1)), 4 * c);
            self.x.iter_mut().zip(&self.h).for_each(|(x, h)| *x += h);
        }
        let f = m.cfg.lnf();
        let (w, b) = (&p[f.clone()], &p[f.end..f.end + c]);
        layernorm((&mut self.h, &mut mean, &mut rstd), &self.x, w, b);
        matmul(&mut self.logits, &self.h, &p[m.cfg.wte()], None, c);
        self.len += 1;
        &self.logits
    }
}

/// A token drawn from `logits`: at `temp` 0 (or below), the likeliest (the lowest id on a tie);
/// else from the `top_k` likeliest (all, at 0), their logits over `temp`, softmaxed.
pub fn sample(logits: &[f32], temp: f32, top_k: usize, rng: &mut Rng) -> u32 {
    let mut ranked: Vec<(f32, u32)> = logits.iter().zip(0u32..).map(|(&l, i)| (l, i)).collect();
    // Likeliest first; NaN last; ties by id, so the order is total and the same everywhere.
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    if temp <= 0.0 || ranked.len() == 1 {
        return ranked.first().map_or(EOS, |r| r.1);
    }
    let k = if top_k == 0 { ranked.len() } else { top_k.min(ranked.len()) };
    let mut probs: Vec<f32> = ranked[..k].iter().map(|r| r.0 / temp).collect();
    softmax(&mut probs);
    let mut u = rng.unit() as f32;
    for (p, r) in probs.iter().zip(&ranked) {
        if u < *p {
            return r.1;
        }
        u -= p;
    }
    ranked[k - 1].1
}

/// Up to `max` tokens after `prompt` (none: after [`EOS`]), each drawn by [`sample`], stopping
/// after `stop` if drawn (it is kept). When the context fills, the model reads the last half of
/// it again and goes on from there.
pub fn generate(
    model: &Model,
    prompt: &[u32],
    max: usize,
    stop: Option<u32>,
    (temp, top_k): (f32, usize),
    rng: &mut Rng,
) -> Vec<u32> {
    let ctx = model.cfg.ctx;
    let mut s = Session::new(model);
    let mut seq: Vec<u32> = if prompt.is_empty() { vec![EOS] } else { prompt.to_vec() };
    let mut logits = Vec::new();
    for &t in &seq[seq.len().saturating_sub(ctx)..] {
        logits = s.feed(t).to_vec();
    }
    let mut out = Vec::new();
    while out.len() < max {
        let next = sample(&logits, temp, top_k, rng);
        out.push(next);
        seq.push(next);
        if Some(next) == stop || out.len() == max {
            break;
        }
        if s.full() {
            s.reset();
            for &t in &seq[seq.len() - 1 - ctx / 2..seq.len() - 1] {
                s.feed(t);
            }
        }
        logits = s.feed(next).to_vec();
    }
    out
}
