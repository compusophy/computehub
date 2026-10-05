//! Training: a batch's sequences spread over threads (each sequence's gradient in its own
//! buffer, then all summed in the batch's order, so the thread count never changes a bit),
//! the gradient clipped by its global norm, and AdamW.

use std::thread;

use crate::{Acts, Model};

/// AdamW's settings and moments.
#[derive(Debug, Clone)]
pub struct AdamW {
    pub beta1: f32,
    pub beta2: f32,
    pub eps: f32,
    /// Weight decay, on matrices and embeddings only.
    pub decay: f32,
    m: Vec<f32>,
    v: Vec<f32>,
    /// Steps taken.
    t: i32,
}

impl AdamW {
    /// GPT-2's: betas 0.9 and 0.95, epsilon 1e-8, decay 0.1.
    pub fn new(params: usize) -> AdamW {
        let (m, v) = (vec![0.0; params], vec![0.0; params]);
        AdamW { beta1: 0.9, beta2: 0.95, eps: 1e-8, decay: 0.1, m, v, t: 0 }
    }

    /// One step of `lr` along `grad` (times `scale`), weight decay only where `decays` says.
    fn step(&mut self, p: &mut [f32], grad: &[f32], (lr, scale): (f32, f32), decays: &[bool]) {
        self.t += 1;
        let (b1, b2) = (self.beta1, self.beta2);
        let (c1, c2) = (1.0 - b1.powi(self.t), 1.0 - b2.powi(self.t));
        let each = p.iter_mut().zip(grad).zip(self.m.iter_mut().zip(self.v.iter_mut()));
        for (((p, &g), (m, v)), &decays) in each.zip(decays) {
            let g = g * scale;
            *m = b1 * *m + (1.0 - b1) * g;
            *v = b2 * *v + (1.0 - b2) * g * g;
            let step = (*m / c1) / ((*v / c2).sqrt() + self.eps);
            let decay = if decays { self.decay * *p } else { 0.0 };
            *p -= lr * (step + decay);
        }
    }
}

/// What a step saw: the batch's mean loss and the gradient's norm before clipping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub loss: f32,
    pub norm: f32,
}

/// A model, its optimizer, and the buffers its steps reuse.
#[derive(Debug)]
pub struct Trainer {
    pub model: Model,
    pub opt: AdamW,
    threads: usize,
    acts: Vec<Acts>,
    grads: Vec<Vec<f32>>,
    sum: Vec<f32>,
    decays: Vec<bool>,
}

impl Trainer {
    /// Trains `model` on `threads` threads (at least 1).
    pub fn new(model: Model, threads: usize) -> Trainer {
        let n = model.params.len();
        let mut decays = vec![false; n];
        for (r, d) in model.cfg.tensors() {
            decays[r].fill(d);
        }
        let threads = threads.max(1);
        let acts = Vec::new();
        Trainer {
            opt: AdamW::new(n),
            model,
            threads,
            acts,
            grads: Vec::new(),
            sum: vec![0.0; n],
            decays,
        }
    }

    /// One step on `batch`: each sequence's tokens predict the next (so each holds one more
    /// than it trains on, at most `ctx + 1`), the loss averaged over every position, the
    /// gradient clipped to a global norm of `clip` (none at 0), then AdamW with `lr`.
    pub fn step(&mut self, batch: &[Vec<u32>], lr: f32, clip: f32) -> Stats {
        let ctx = self.model.cfg.ctx;
        let all: usize = batch.iter().map(|s| positions(s, ctx)).sum();
        let scale = 1.0 / all.max(1) as f32;
        while self.grads.len() < batch.len() {
            self.grads.push(vec![0.0; self.model.params.len()]);
        }
        let losses = self.each(batch, Some(scale));
        // Sum the gradients in the batch's order, a slice of the parameters per thread.
        let grads = &self.grads[..batch.len()];
        let chunk = self.sum.len().div_ceil(self.threads);
        let sum_part = |(i, part): (usize, &mut [f32])| {
            part.fill(0.0);
            for g in grads {
                for (s, g) in part.iter_mut().zip(&g[i * chunk..]) {
                    *s += g;
                }
            }
        };
        if self.threads == 1 {
            self.sum.chunks_mut(chunk).enumerate().for_each(sum_part);
        } else {
            thread::scope(|s| {
                for part in self.sum.chunks_mut(chunk).enumerate() {
                    s.spawn(move || sum_part(part));
                }
            });
        }
        let norm = self.sum.iter().map(|&g| f64::from(g) * f64::from(g)).sum::<f64>().sqrt() as f32;
        let clipped = if clip > 0.0 && norm > clip { clip / norm } else { 1.0 };
        self.opt.step(&mut self.model.params, &self.sum, (lr, clipped), &self.decays);
        let loss = losses.iter().zip(batch).map(|(&l, s)| f64::from(l) * positions(s, ctx) as f64);
        Stats { loss: (loss.sum::<f64>() * f64::from(scale)) as f32, norm }
    }

    /// The mean loss over `seqs` (each predicting its next tokens), on the trainer's threads.
    pub fn eval(&mut self, seqs: &[Vec<u32>]) -> f32 {
        let (losses, ctx) = (self.each(seqs, None), self.model.cfg.ctx);
        let all: usize = seqs.iter().map(|s| positions(s, ctx)).sum();
        let sum: f64 =
            losses.iter().zip(seqs).map(|(&l, s)| f64::from(l) * positions(s, ctx) as f64).sum();
        (sum / all.max(1) as f64) as f32
    }

    /// Each sequence's mean loss, and with `scale` its gradient into its own buffer: thread
    /// `w` takes sequences `w`, `w + threads`, ...
    fn each(&mut self, batch: &[Vec<u32>], scale: Option<f32>) -> Vec<f32> {
        let workers = self.threads.min(batch.len()).max(1);
        while self.acts.len() < workers {
            self.acts.push(Acts::new(&self.model.cfg));
        }
        let model = &self.model;
        let one = |acts: &mut Acts, seq: &Vec<u32>, grad: Option<&mut Vec<f32>>| {
            let n = positions(seq, model.cfg.ctx);
            let (x, y) = (&seq[..n], &seq[1..=n]);
            match (scale, grad) {
                (Some(scale), Some(g)) => {
                    g.fill(0.0);
                    model.grad(acts, (x, y), scale, g)
                }
                _ => model.loss(acts, x, y),
            }
        };
        let mut losses = vec![0f32; batch.len()];
        let mut grads: Vec<Option<&mut Vec<f32>>> = match scale {
            Some(_) => self.grads.iter_mut().map(Some).collect(),
            None => batch.iter().map(|_| None).collect(),
        };
        let mut jobs: Vec<Jobs<'_>> = (0..workers).map(|_| Vec::new()).collect();
        for (i, g) in grads.drain(..batch.len()).enumerate() {
            jobs[i % workers].push((i, g));
        }
        let run = |acts: &mut Acts, jobs: Jobs<'_>| {
            jobs.into_iter().map(|(i, g)| (i, one(acts, &batch[i], g))).collect::<Vec<_>>()
        };
        let done: Vec<Vec<(usize, f32)>> = if workers == 1 {
            jobs.into_iter().map(|j| run(&mut self.acts[0], j)).collect()
        } else {
            thread::scope(|s| {
                let handles: Vec<_> = self
                    .acts
                    .iter_mut()
                    .zip(jobs)
                    .map(|(acts, j)| s.spawn(move || run(acts, j)))
                    .collect();
                let join = |h: thread::ScopedJoinHandle<'_, _>| match h.join() {
                    Ok(done) => done,
                    Err(panic) => std::panic::resume_unwind(panic),
                };
                handles.into_iter().map(join).collect()
            })
        };
        for (i, l) in done.into_iter().flatten() {
            losses[i] = l;
        }
        losses
    }
}

/// A worker's share of a batch: each sequence's index and, when training, its gradient's buffer.
type Jobs<'a> = Vec<(usize, Option<&'a mut Vec<f32>>)>;

/// The positions `seq` trains: one fewer than its tokens, at most `ctx`.
fn positions(seq: &[u32], ctx: usize) -> usize {
    seq.len().saturating_sub(1).min(ctx)
}
