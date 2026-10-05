//! The model: its parameters in one flat vector, seeded init, the forward pass and the backward
//! pass written out by hand.
//!
//! The vector holds, in order: the token embeddings (`vocab` rows of `dim`, also the head), the
//! position embeddings (`ctx` rows), each layer's [`sizes`] tensors, and the final layer norm's
//! weight and bias. A weight is (out, in): row `o` is what output `o` reads.

use std::ops::Range;

use crate::ops::{self, attention, attention_back, gelu, gelu_back, layernorm, layernorm_back};
use crate::ops::{matmul, matmul_back, softmax_ce};
use crate::{Config, Error, Rng};

/// A layer's tensors, in order: their index in [`sizes`].
pub(crate) const LN1_W: usize = 0;
pub(crate) const QKV_W: usize = 2;
pub(crate) const PROJ_W: usize = 4;
pub(crate) const LN2_W: usize = 6;
pub(crate) const FC_W: usize = 8;
pub(crate) const FCP_W: usize = 10;

/// The sizes of a layer's tensors for width `c`: the first layer norm's weight and bias, the
/// queries', keys' and values' weight (3c by c) and bias, attention's out projection and bias,
/// the second layer norm, the MLP's up (4c by c) and down (c by 4c) and their biases.
pub(crate) fn sizes(c: usize) -> [usize; 12] {
    [c, c, 3 * c * c, 3 * c, c * c, c, c, c, 4 * c * c, 4 * c, 4 * c * c, c]
}

impl Config {
    /// Where tensor `k` of layer `l` lives.
    pub(crate) fn at(&self, l: usize, k: usize) -> Range<usize> {
        let sizes = sizes(self.dim);
        let start = (self.vocab + self.ctx) * self.dim
            + l * self.layer_size()
            + sizes[..k].iter().sum::<usize>();
        start..start + sizes[k]
    }

    pub(crate) fn wte(&self) -> Range<usize> {
        0..self.vocab * self.dim
    }

    pub(crate) fn wpe(&self) -> Range<usize> {
        self.vocab * self.dim..(self.vocab + self.ctx) * self.dim
    }

    /// The final layer norm's weight; its bias follows.
    pub(crate) fn lnf(&self) -> Range<usize> {
        let start = self.params() - 2 * self.dim;
        start..start + self.dim
    }

    /// Every tensor's place and whether weight decay applies to it (to matrices and embeddings,
    /// never to biases and layer norms).
    pub(crate) fn tensors(&self) -> Vec<(Range<usize>, bool)> {
        let mut out = vec![(self.wte(), true), (self.wpe(), true)];
        for l in 0..self.layers {
            out.extend(
                (0..12).map(|k| (self.at(l, k), matches!(k, QKV_W | PROJ_W | FC_W | FCP_W))),
            );
        }
        let f = self.lnf();
        out.extend([(f.clone(), false), (f.end..f.end + self.dim, false)]);
        out
    }
}

/// A weight and the bias right after it, from `p`: tensor `k` of layer `l` and the next.
fn pair<'a>(cfg: &Config, p: &'a [f32], l: usize, k: usize) -> (&'a [f32], &'a [f32]) {
    (&p[cfg.at(l, k)], &p[cfg.at(l, k + 1)])
}

/// [`pair`], mutable: the two are side by side.
fn pair_mut<'a>(
    cfg: &Config,
    g: &'a mut [f32],
    l: usize,
    k: usize,
) -> (&'a mut [f32], &'a mut [f32]) {
    let (w, b) = (cfg.at(l, k), cfg.at(l, k + 1));
    g[w.start..b.end].split_at_mut(w.len())
}

/// A model: its shape and its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub cfg: Config,
    pub params: Vec<f32>,
}

impl Model {
    /// A model of shape `cfg`, its weights drawn from `seed`: matrices and embeddings normal
    /// with deviation 0.02 (the projections into the residual stream 0.02 / sqrt(2 layers)),
    /// biases 0, layer norms' weights 1.
    pub fn new(cfg: Config, seed: u64) -> Result<Model, Error> {
        cfg.check()?;
        let mut params = vec![0f32; cfg.params()];
        let mut rng = Rng::new(seed);
        let mut fill = |r: Range<usize>, sd: f64| {
            for p in &mut params[r] {
                *p = (rng.normal() * sd) as f32;
            }
        };
        fill(cfg.wte(), 0.02);
        fill(cfg.wpe(), 0.01);
        let resid = 0.02 / (2.0 * cfg.layers as f64).sqrt();
        for l in 0..cfg.layers {
            fill(cfg.at(l, QKV_W), 0.02);
            fill(cfg.at(l, PROJ_W), resid);
            fill(cfg.at(l, FC_W), 0.02);
            fill(cfg.at(l, FCP_W), resid);
        }
        for l in 0..cfg.layers {
            for k in [LN1_W, LN2_W] {
                params[cfg.at(l, k)].fill(1.0);
            }
        }
        params[cfg.lnf()].fill(1.0);
        Ok(Model { cfg, params })
    }

    /// The forward pass over `tokens` (at most `ctx`), its activations kept in `acts`: the next
    /// token's probabilities at each position are in [`Acts::probs`] after. With `targets`, the
    /// mean cross-entropy against them.
    pub fn forward(&self, acts: &mut Acts, tokens: &[u32], targets: Option<&[u32]>) -> f32 {
        let Config { vocab: v, dim: c, heads, layers, .. } = self.cfg;
        let (t, p) = (tokens.len().min(self.cfg.ctx), &self.params);
        let tokens = &tokens[..t];
        acts.t = t;
        let (wte, wpe) = (&p[self.cfg.wte()], &p[self.cfg.wpe()]);
        for (i, (row, &tok)) in acts.encoded.chunks_exact_mut(c).zip(tokens).enumerate() {
            let e = &wte[(tok as usize).min(v - 1) * c..][..c];
            for ((o, e), q) in row.iter_mut().zip(e).zip(&wpe[i * c..][..c]) {
                *o = e + q;
            }
        }
        for l in 0..layers {
            let a = &mut *acts;
            let (tc, l1) = (t * c, l * t * c);
            let residual = if l == 0 { &a.encoded[..tc] } else { &a.res3[l1 - tc..l1] };
            let ln1 = &mut a.ln1[l1..l1 + tc];
            let (m, r) = (&mut a.ln1_mean[l * t..][..t], &mut a.ln1_rstd[l * t..][..t]);
            let (w, b) = pair(&self.cfg, p, l, LN1_W);
            layernorm((ln1, m, r), residual, w, b);
            let qkv = &mut a.qkv[3 * l1..][..3 * tc];
            let (w, b) = pair(&self.cfg, p, l, QKV_W);
            matmul(qkv, &a.ln1[l1..l1 + tc], w, Some(b), c);
            let att = &mut a.att[l * heads * t * t..][..heads * t * t];
            attention(&mut a.atty[l1..l1 + tc], att, &a.qkv[3 * l1..][..3 * tc], t, heads);
            let (w, b) = pair(&self.cfg, p, l, PROJ_W);
            matmul(&mut a.tmp[..tc], &a.atty[l1..l1 + tc], w, Some(b), c);
            let residual = if l == 0 { &a.encoded[..tc] } else { &a.res3[l1 - tc..l1] };
            for ((o, x), y) in a.res2[l1..l1 + tc].iter_mut().zip(residual).zip(&a.tmp[..tc]) {
                *o = x + y;
            }
            let (m, r) = (&mut a.ln2_mean[l * t..][..t], &mut a.ln2_rstd[l * t..][..t]);
            let (w, b) = pair(&self.cfg, p, l, LN2_W);
            layernorm((&mut a.ln2[l1..l1 + tc], m, r), &a.res2[l1..l1 + tc], w, b);
            let (w, b) = pair(&self.cfg, p, l, FC_W);
            matmul(&mut a.fch[4 * l1..][..4 * tc], &a.ln2[l1..l1 + tc], w, Some(b), c);
            gelu(&mut a.fch_gelu[4 * l1..][..4 * tc], &a.fch[4 * l1..][..4 * tc]);
            let (w, b) = pair(&self.cfg, p, l, FCP_W);
            matmul(&mut a.tmp[..tc], &a.fch_gelu[4 * l1..][..4 * tc], w, Some(b), 4 * c);
            for ((o, x), y) in a.res3[l1..l1 + tc].iter_mut().zip(&a.res2[l1..l1 + tc]).zip(&a.tmp)
            {
                *o = x + y;
            }
        }
        let a = &mut *acts;
        let tc = t * c;
        let last = if layers == 0 { &a.encoded[..tc] } else { &a.res3[(layers - 1) * tc..][..tc] };
        let f = self.cfg.lnf();
        let (w, b) = (&p[f.clone()], &p[f.end..f.end + c]);
        layernorm((&mut a.lnf[..tc], &mut a.lnf_mean[..t], &mut a.lnf_rstd[..t]), last, w, b);
        matmul(&mut a.probs[..t * v], &a.lnf[..tc], wte, None, c);
        match targets {
            Some(y) => (softmax_ce(&mut a.probs[..t * v], &y[..t], v) / t as f64) as f32,
            None => {
                a.probs[..t * v].chunks_exact_mut(v).for_each(ops::softmax);
                0.0
            }
        }
    }

    /// The mean cross-entropy of predicting `targets` from `tokens`.
    pub fn loss(&self, acts: &mut Acts, tokens: &[u32], targets: &[u32]) -> f32 {
        self.forward(acts, tokens, Some(targets))
    }

    /// The forward pass, then the backward: the gradient of `scale` times the summed
    /// cross-entropy added to `grads` (as long as the parameters). Returns the mean loss.
    pub fn grad(
        &self,
        acts: &mut Acts,
        (tokens, targets): (&[u32], &[u32]),
        scale: f32,
        grads: &mut [f32],
    ) -> f32 {
        let loss = self.forward(acts, tokens, Some(targets));
        self.backward(acts, tokens, targets, scale, grads);
        loss
    }

    fn backward(&self, a: &mut Acts, tokens: &[u32], targets: &[u32], scale: f32, g: &mut [f32]) {
        let Config { vocab: v, dim: c, heads, layers, .. } = self.cfg;
        let (t, p, cfg) = (a.t, &self.params, &self.cfg);
        let tc = t * c;
        // The loss to the logits: p - 1 at the target, each times scale.
        for (row, &y) in a.probs[..t * v].chunks_exact_mut(v).zip(targets) {
            row[y as usize] -= 1.0;
            row.iter_mut().for_each(|d| *d *= scale);
        }
        let dwte = &mut g[cfg.wte()];
        matmul_back(
            (&mut a.dln[..tc], dwte, None),
            &a.probs[..t * v],
            &a.lnf[..tc],
            &p[cfg.wte()],
            c,
        );
        let f = cfg.lnf();
        let (dw, db) = g[f.start..f.end + c].split_at_mut(c);
        let last = if layers == 0 { &a.encoded[..tc] } else { &a.res3[(layers - 1) * tc..][..tc] };
        a.dres[..tc].fill(0.0);
        let stats = (&a.lnf_mean[..t], &a.lnf_rstd[..t]);
        layernorm_back((&mut a.dres[..tc], dw, db), &a.dln[..tc], last, &p[f], stats);
        for l in (0..layers).rev() {
            let l1 = l * tc;
            let (dw, db) = pair_mut(cfg, g, l, FCP_W);
            let (w, _) = pair(cfg, p, l, FCP_W);
            let dfc = &mut a.dfc[..4 * tc];
            matmul_back(
                (dfc, dw, Some(db)),
                &a.dres[..tc],
                &a.fch_gelu[4 * l1..][..4 * tc],
                w,
                4 * c,
            );
            gelu_back(&mut a.dfc[..4 * tc], &a.fch[4 * l1..][..4 * tc]);
            let (dw, db) = pair_mut(cfg, g, l, FC_W);
            let (w, _) = pair(cfg, p, l, FC_W);
            matmul_back(
                (&mut a.dln[..tc], dw, Some(db)),
                &a.dfc[..4 * tc],
                &a.ln2[l1..][..tc],
                w,
                c,
            );
            let (dw, db) = pair_mut(cfg, g, l, LN2_W);
            let (w, _) = pair(cfg, p, l, LN2_W);
            let stats = (&a.ln2_mean[l * t..][..t], &a.ln2_rstd[l * t..][..t]);
            layernorm_back(
                (&mut a.dres[..tc], dw, db),
                &a.dln[..tc],
                &a.res2[l1..][..tc],
                w,
                stats,
            );
            let (dw, db) = pair_mut(cfg, g, l, PROJ_W);
            let (w, _) = pair(cfg, p, l, PROJ_W);
            matmul_back(
                (&mut a.datty[..tc], dw, Some(db)),
                &a.dres[..tc],
                &a.atty[l1..][..tc],
                w,
                c,
            );
            let att = &a.att[l * heads * t * t..][..heads * t * t];
            let qkv = &a.qkv[3 * l1..][..3 * tc];
            let dqkv = (&mut a.dqkv[..3 * tc], &mut a.datt[..t]);
            attention_back(dqkv, &a.datty[..tc], qkv, att, (t, heads));
            let (dw, db) = pair_mut(cfg, g, l, QKV_W);
            let (w, _) = pair(cfg, p, l, QKV_W);
            matmul_back(
                (&mut a.dln[..tc], dw, Some(db)),
                &a.dqkv[..3 * tc],
                &a.ln1[l1..][..tc],
                w,
                c,
            );
            let (dw, db) = pair_mut(cfg, g, l, LN1_W);
            let (w, _) = pair(cfg, p, l, LN1_W);
            let residual = if l == 0 { &a.encoded[..tc] } else { &a.res3[l1 - tc..l1] };
            let stats = (&a.ln1_mean[l * t..][..t], &a.ln1_rstd[l * t..][..t]);
            layernorm_back((&mut a.dres[..tc], dw, db), &a.dln[..tc], residual, w, stats);
        }
        let (dwte, dwpe) = g[..cfg.wpe().end].split_at_mut(cfg.wte().end);
        for (i, (d, &tok)) in a.dres[..tc].chunks_exact(c).zip(tokens).enumerate() {
            ops::axpy(&mut dwte[(tok as usize).min(v - 1) * c..][..c], 1.0, d);
            ops::axpy(&mut dwpe[i * c..][..c], 1.0, d);
        }
    }
}

/// A forward pass's activations and a backward pass's scratch, for sequences up to a config's
/// `ctx` tokens: one per thread, reused.
#[derive(Debug, Clone)]
pub struct Acts {
    t: usize,
    encoded: Vec<f32>,
    ln1: Vec<f32>,
    ln1_mean: Vec<f32>,
    ln1_rstd: Vec<f32>,
    qkv: Vec<f32>,
    att: Vec<f32>,
    atty: Vec<f32>,
    res2: Vec<f32>,
    ln2: Vec<f32>,
    ln2_mean: Vec<f32>,
    ln2_rstd: Vec<f32>,
    fch: Vec<f32>,
    fch_gelu: Vec<f32>,
    res3: Vec<f32>,
    lnf: Vec<f32>,
    lnf_mean: Vec<f32>,
    lnf_rstd: Vec<f32>,
    probs: Vec<f32>,
    tmp: Vec<f32>,
    dres: Vec<f32>,
    dln: Vec<f32>,
    dfc: Vec<f32>,
    datty: Vec<f32>,
    dqkv: Vec<f32>,
    datt: Vec<f32>,
}

impl Acts {
    pub fn new(cfg: &Config) -> Acts {
        let Config { vocab: v, ctx: t, dim: c, layers: l, heads: h } = *cfg;
        let z = |n: usize| vec![0f32; n];
        Acts {
            t: 0,
            encoded: z(t * c),
            ln1: z(l * t * c),
            ln1_mean: z(l * t),
            ln1_rstd: z(l * t),
            qkv: z(3 * l * t * c),
            att: z(l * h * t * t),
            atty: z(l * t * c),
            res2: z(l * t * c),
            ln2: z(l * t * c),
            ln2_mean: z(l * t),
            ln2_rstd: z(l * t),
            fch: z(4 * l * t * c),
            fch_gelu: z(4 * l * t * c),
            res3: z(l * t * c),
            lnf: z(t * c),
            lnf_mean: z(t),
            lnf_rstd: z(t),
            probs: z(t * v),
            tmp: z(t * c),
            dres: z(t * c),
            dln: z(t * c),
            dfc: z(4 * t * c),
            datty: z(t * c),
            dqkv: z(3 * t * c),
            datt: z(t),
        }
    }

    /// The last forward pass's next-token probabilities, a row of `vocab` per position.
    pub fn probs(&self) -> &[f32] {
        let v = self.probs.len() / self.lnf_mean.len().max(1);
        &self.probs[..self.t * v]
    }
}
