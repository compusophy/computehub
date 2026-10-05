//! The kernels, forward and backward, over rows of plain slices: a row is one token's vector.
//! Weights are stored (out, in): row `o` of a weight is what output `o` reads, so a forward is
//! dot products and both backward halves are `axpy`s. Every sum runs in a fixed order, so the
//! same inputs give the same bits.

/// The sum of `a[i] * b[i]`, over the shorter's length, in eight running lanes (which the
/// compiler keeps in vector registers) added up in a fixed order.
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut lanes = [0f32; 8];
    let (mut ca, mut cb) = (a.chunks_exact(8), b.chunks_exact(8));
    for (x, y) in (&mut ca).zip(&mut cb) {
        for ((l, x), y) in lanes.iter_mut().zip(x).zip(y) {
            *l += x * y;
        }
    }
    let mut rest = 0f32;
    for (x, y) in ca.remainder().iter().zip(cb.remainder()) {
        rest += x * y;
    }
    let [l0, l1, l2, l3, l4, l5, l6, l7] = lanes;
    (((l0 + l4) + (l1 + l5)) + ((l2 + l6) + (l3 + l7))) + rest
}

/// `y += a * x`.
pub(crate) fn axpy(y: &mut [f32], a: f32, x: &[f32]) {
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

/// `out` (rows of `oc`) = `inp` (rows of `ic`) times `w` (`oc` rows of `ic`) transposed, plus
/// `b` (one per output) if any. Four rows share each weight row read.
pub(crate) fn matmul(out: &mut [f32], inp: &[f32], w: &[f32], b: Option<&[f32]>, ic: usize) {
    let oc = w.len() / ic;
    let mut outs = out.chunks_exact_mut(oc * 4);
    let mut ins = inp.chunks_exact(ic * 4);
    for (o4, x4) in (&mut outs).zip(&mut ins) {
        let (x0, rest) = x4.split_at(ic);
        let (x1, rest) = rest.split_at(ic);
        let (x2, x3) = rest.split_at(ic);
        for (o, wrow) in w.chunks_exact(ic).enumerate() {
            let bias = b.map_or(0.0, |b| b[o]);
            let [d0, d1, d2, d3] = dot4([x0, x1, x2, x3], wrow);
            o4[o] = d0 + bias;
            o4[oc + o] = d1 + bias;
            o4[2 * oc + o] = d2 + bias;
            o4[3 * oc + o] = d3 + bias;
        }
    }
    for (orow, x) in
        outs.into_remainder().chunks_exact_mut(oc).zip(ins.remainder().chunks_exact(ic))
    {
        for (o, (val, wrow)) in orow.iter_mut().zip(w.chunks_exact(ic)).enumerate() {
            *val = dot(x, wrow) + b.map_or(0.0, |b| b[o]);
        }
    }
}

/// Four dot products with one `w` (each `x` at least as long), each exactly as [`dot`] sums it.
fn dot4(xs: [&[f32]; 4], w: &[f32]) -> [f32; 4] {
    let (n, full) = (w.len(), w.len() / 8 * 8);
    let xs = xs.map(|x| &x[..n]);
    let mut lanes = [[0f32; 8]; 4];
    for i in (0..full).step_by(8) {
        let wc: &[f32; 8] = w[i..i + 8].try_into().unwrap_or(&[0.0; 8]);
        for (lane, x) in lanes.iter_mut().zip(xs) {
            let xc: &[f32; 8] = x[i..i + 8].try_into().unwrap_or(&[0.0; 8]);
            for ((l, x), w) in lane.iter_mut().zip(xc).zip(wc) {
                *l += x * w;
            }
        }
    }
    let mut out = [0f32; 4];
    for ((o, lane), x) in out.iter_mut().zip(lanes).zip(xs) {
        let mut rest = 0f32;
        for (x, w) in x[full..].iter().zip(&w[full..]) {
            rest += x * w;
        }
        let [l0, l1, l2, l3, l4, l5, l6, l7] = lane;
        *o = (((l0 + l4) + (l1 + l5)) + ((l2 + l6) + (l3 + l7))) + rest;
    }
    out
}

/// The backward of [`matmul`]: `dinp` = `dout` times `w` (overwritten), and `dout` transposed
/// times `inp` added to `dw`, `dout`'s column sums to `db`. With `ic` a multiple of 8, the
/// products go by tiles of 4 rows by 8 columns summed in registers ([`madd`]), the rows left
/// over one at a time; each element's sum runs in the same order either way
/// ([`matmul_back_plain`]).
pub(crate) fn matmul_back(
    (dinp, dw, db): (&mut [f32], &mut [f32], Option<&mut [f32]>),
    dout: &[f32],
    inp: &[f32],
    w: &[f32],
    ic: usize,
) {
    let oc = w.len() / ic;
    let rows = dout.len() / oc;
    let (rt, ot) = if ic % 8 == 0 { (rows / 4 * 4, oc / 4 * 4) } else { (0, 0) };
    for i0 in (0..ic).step_by(8).filter(|_| rt > 0) {
        // dinp[t + k][i0..] = the sum over o of dout[t + k][o] w[o][i0..].
        for t in (0..rt).step_by(4) {
            let d: [&[f32]; 4] = [0, 1, 2, 3].map(|k| &dout[(t + k) * oc..][..oc]);
            let mut acc = [[0f32; 8]; 4];
            for (o, wrow) in w.chunks_exact(ic).enumerate() {
                madd(&mut acc, [d[0][o], d[1][o], d[2][o], d[3][o]], row8(wrow, i0));
            }
            for (k, a) in acc.iter().enumerate() {
                dinp[(t + k) * ic + i0..][..8].copy_from_slice(a);
            }
        }
    }
    for (di, d) in dinp.chunks_exact_mut(ic).zip(dout.chunks_exact(oc)).skip(rt) {
        di.fill(0.0);
        for (&g, wrow) in d.iter().zip(w.chunks_exact(ic)) {
            axpy(di, g, wrow);
        }
    }
    for i0 in (0..ic).step_by(8).filter(|_| ot > 0) {
        // dw[o + k][i0..] += the sum over t of dout[t][o + k] inp[t][i0..].
        for o in (0..ot).step_by(4) {
            let mut acc = [[0f32; 8]; 4];
            for (x, d) in inp.chunks_exact(ic).zip(dout.chunks_exact(oc)) {
                madd(&mut acc, [d[o], d[o + 1], d[o + 2], d[o + 3]], row8(x, i0));
            }
            for (k, a) in acc.iter().enumerate() {
                for (d, a) in dw[(o + k) * ic + i0..][..8].iter_mut().zip(a) {
                    *d += a;
                }
            }
        }
    }
    let mut sum = vec![0f32; ic];
    for (o, dwrow) in dw.chunks_exact_mut(ic).enumerate().skip(ot) {
        sum.fill(0.0);
        for (x, d) in inp.chunks_exact(ic).zip(dout.chunks_exact(oc)) {
            axpy(&mut sum, d[o], x);
        }
        for (d, s) in dwrow.iter_mut().zip(&sum) {
            *d += s;
        }
    }
    if let Some(db) = db {
        for d in dout.chunks_exact(oc) {
            for (b, g) in db.iter_mut().zip(d) {
                *b += g;
            }
        }
    }
}

/// [`matmul_back`]'s products one element at a time (for its tests).
#[cfg(test)]
pub(crate) fn matmul_back_plain(
    (dinp, dw): (&mut [f32], &mut [f32]),
    dout: &[f32],
    inp: &[f32],
    w: &[f32],
    ic: usize,
) {
    let oc = w.len() / ic;
    for (di, d) in dinp.chunks_exact_mut(ic).zip(dout.chunks_exact(oc)) {
        di.fill(0.0);
        for (&g, wrow) in d.iter().zip(w.chunks_exact(ic)) {
            axpy(di, g, wrow);
        }
    }
    let mut sum = vec![0f32; ic];
    for (o, dwrow) in dw.chunks_exact_mut(ic).enumerate() {
        sum.fill(0.0);
        for (x, d) in inp.chunks_exact(ic).zip(dout.chunks_exact(oc)) {
            axpy(&mut sum, d[o], x);
        }
        for (d, s) in dwrow.iter_mut().zip(&sum) {
            *d += s;
        }
    }
}

/// `acc[k] += g[k] * v`, for a tile of four rows of eight.
#[inline(always)]
fn madd(acc: &mut [[f32; 8]; 4], g: [f32; 4], v: &[f32; 8]) {
    for (a, g) in acc.iter_mut().zip(g) {
        for (a, x) in a.iter_mut().zip(v) {
            *a += g * x;
        }
    }
}

/// The eight values of `v` from `at`.
#[inline(always)]
fn row8(v: &[f32], at: usize) -> &[f32; 8] {
    v[at..at + 8].try_into().unwrap_or(&[0.0; 8])
}

/// Layer norm over rows of `c`: each row less its mean, over its deviation, times `w` plus `b`;
/// each row's mean and reciprocal deviation are kept for the backward.
pub(crate) fn layernorm(
    (out, mean, rstd): (&mut [f32], &mut [f32], &mut [f32]),
    inp: &[f32],
    w: &[f32],
    b: &[f32],
) {
    let c = w.len();
    let rows = out.chunks_exact_mut(c).zip(inp.chunks_exact(c));
    for ((o, x), (m, r)) in rows.zip(mean.iter_mut().zip(rstd.iter_mut())) {
        let mu = x.iter().sum::<f32>() / c as f32;
        let var = x.iter().map(|v| (v - mu) * (v - mu)).sum::<f32>() / c as f32;
        let s = 1.0 / (var + 1e-5).sqrt();
        for (((o, x), w), b) in o.iter_mut().zip(x).zip(w).zip(b) {
            *o = (x - mu) * s * w + b;
        }
        (*m, *r) = (mu, s);
    }
}

/// The backward of [`layernorm`]: added to `dinp`, `dw` and `db`.
pub(crate) fn layernorm_back(
    (dinp, dw, db): (&mut [f32], &mut [f32], &mut [f32]),
    dout: &[f32],
    inp: &[f32],
    w: &[f32],
    (mean, rstd): (&[f32], &[f32]),
) {
    let c = w.len();
    let rows = dinp.chunks_exact_mut(c).zip(dout.chunks_exact(c)).zip(inp.chunks_exact(c));
    for (((dx, d), x), (&mu, &s)) in rows.zip(mean.iter().zip(rstd)) {
        let (mut dmean, mut dnorm_mean) = (0f32, 0f32);
        for ((x, d), w) in x.iter().zip(d).zip(w) {
            let dnorm = w * d;
            dmean += dnorm;
            dnorm_mean += dnorm * (x - mu) * s;
        }
        let (dmean, dnorm_mean) = (dmean / c as f32, dnorm_mean / c as f32);
        let each = dx.iter_mut().zip(dw.iter_mut()).zip(db.iter_mut());
        for (((dx, dw), db), ((x, d), w)) in each.zip(x.iter().zip(d).zip(w)) {
            let norm = (x - mu) * s;
            *db += d;
            *dw += norm * d;
            *dx += (w * d - dmean - norm * dnorm_mean) * s;
        }
    }
}

/// sqrt(2 / pi), for GELU's tanh form.
const GELU_S: f32 = 0.797_884_6;
const GELU_C: f32 = 0.044_715;

/// GELU, its tanh form.
pub(crate) fn gelu(out: &mut [f32], inp: &[f32]) {
    for (o, &x) in out.iter_mut().zip(inp) {
        *o = 0.5 * x * (1.0 + (GELU_S * (x + GELU_C * x * x * x)).tanh());
    }
}

/// The backward of [`gelu`], in place: `d` holds the gradient of its output and then of its
/// input `inp`.
pub(crate) fn gelu_back(d: &mut [f32], inp: &[f32]) {
    for (d, &x) in d.iter_mut().zip(inp) {
        let t = (GELU_S * (x + GELU_C * x * x * x)).tanh();
        let local =
            0.5 * (1.0 + t) + 0.5 * x * (1.0 - t * t) * GELU_S * (1.0 + 3.0 * GELU_C * x * x);
        *d *= local;
    }
}

/// Causal self-attention over `t` rows of `qkv` (each `3c`: the queries, keys and values of
/// `heads` heads); the outputs to `out` (rows of `c`), each head's probabilities to `att`
/// (`heads` blocks of `t` by `t`, zero above the diagonal).
pub(crate) fn attention(out: &mut [f32], att: &mut [f32], qkv: &[f32], t: usize, heads: usize) {
    let c = out.len() / t;
    let hs = c / heads;
    let scale = 1.0 / (hs as f32).sqrt();
    for (t1, orow) in out.chunks_exact_mut(c).enumerate() {
        for (h, o) in orow.chunks_exact_mut(hs).enumerate() {
            let q = &qkv[t1 * 3 * c + h * hs..][..hs];
            let row = &mut att[(h * t + t1) * t..][..t];
            let mut most = f32::NEG_INFINITY;
            for (t2, a) in row[..=t1].iter_mut().enumerate() {
                *a = dot(q, &qkv[t2 * 3 * c + c + h * hs..][..hs]) * scale;
                most = most.max(*a);
            }
            let mut sum = 0f32;
            for a in &mut row[..=t1] {
                *a = (*a - most).exp();
                sum += *a;
            }
            let inv = 1.0 / sum;
            for a in &mut row[..=t1] {
                *a *= inv;
            }
            row[t1 + 1..].fill(0.0);
            o.fill(0.0);
            for (t2, &a) in row[..=t1].iter().enumerate() {
                axpy(o, a, &qkv[t2 * 3 * c + 2 * c + h * hs..][..hs]);
            }
        }
    }
}

/// The backward of [`attention`]: `dqkv` (overwritten) from `dout`, with `datt` scratch of `t`.
pub(crate) fn attention_back(
    (dqkv, datt): (&mut [f32], &mut [f32]),
    dout: &[f32],
    qkv: &[f32],
    att: &[f32],
    (t, heads): (usize, usize),
) {
    let c = dout.len() / t;
    let hs = c / heads;
    let scale = 1.0 / (hs as f32).sqrt();
    dqkv.fill(0.0);
    for (t1, drow) in dout.chunks_exact(c).enumerate() {
        for (h, d) in drow.chunks_exact(hs).enumerate() {
            let row = &att[(h * t + t1) * t..][..=t1];
            let datt = &mut datt[..=t1];
            // out = att times v: to each probability and each value.
            for (t2, (da, &a)) in datt.iter_mut().zip(row).enumerate() {
                let v = t2 * 3 * c + 2 * c + h * hs;
                *da = dot(&qkv[v..v + hs], d);
                axpy(&mut dqkv[v..v + hs], a, d);
            }
            // The softmax, then the scaled dot products of the query and each key.
            let s = dot(row, datt);
            for (da, &a) in datt.iter_mut().zip(row) {
                *da = a * (*da - s) * scale;
            }
            let q = t1 * 3 * c + h * hs;
            for (t2, &da) in datt.iter().enumerate() {
                let k = t2 * 3 * c + c + h * hs;
                axpy(&mut dqkv[q..q + hs], da, &qkv[k..k + hs]);
                axpy(&mut dqkv[k..k + hs], da, &qkv[q..q + hs]);
            }
        }
    }
}

/// Softmax over each row of `v` in place, then the cross-entropy of each row's `target`: the
/// sum of `-ln p`, in f64.
pub(crate) fn softmax_ce(logits: &mut [f32], targets: &[u32], v: usize) -> f64 {
    let mut loss = 0f64;
    for (row, &y) in logits.chunks_exact_mut(v).zip(targets) {
        softmax(row);
        loss -= f64::from(row[y as usize].max(1e-30)).ln();
    }
    loss
}

/// Softmax in place.
pub(crate) fn softmax(row: &mut [f32]) {
    let most = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0f32;
    for x in row.iter_mut() {
        *x = (*x - most).exp();
        sum += *x;
    }
    let inv = 1.0 / sum;
    for x in row.iter_mut() {
        *x *= inv;
    }
}
