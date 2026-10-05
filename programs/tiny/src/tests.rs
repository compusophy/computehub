use super::*;

fn cfg(vocab: usize, ctx: usize, dim: usize, layers: usize, heads: usize) -> Config {
    Config { vocab, ctx, dim, layers, heads }
}

/// A model whose every parameter is drawn wider than init's (so each kernel's curvature shows).
fn shaken(c: Config, seed: u64) -> Model {
    let mut m = Model::new(c, seed).unwrap();
    let mut rng = Rng::new(seed ^ 0xabc);
    for p in &mut m.params {
        *p += (rng.normal() * 0.3) as f32;
    }
    m
}

#[test]
fn the_backward_pass_matches_finite_differences() {
    let c = cfg(11, 7, 8, 2, 2);
    let mut m = shaken(c, 3);
    let (x, y) = ([1, 5, 9, 2, 2, 7, 0], [5, 9, 2, 2, 7, 0, 3]);
    let mut acts = Acts::new(&c);
    let mut grad = vec![0f32; c.params()];
    let loss = m.grad(&mut acts, (&x, &y), 1.0 / x.len() as f32, &mut grad);
    assert!(loss > 0.5, "{loss}");
    let h = 1e-2f32;
    let mut numeric = vec![0f32; c.params()];
    for (i, n) in numeric.iter_mut().enumerate() {
        let p = m.params[i];
        m.params[i] = p + h;
        let up = f64::from(m.loss(&mut acts, &x, &y));
        m.params[i] = p - h;
        let down = f64::from(m.loss(&mut acts, &x, &y));
        m.params[i] = p;
        *n = ((up - down) / (2.0 * f64::from(h))) as f32;
    }
    // Tensor by tensor: the embeddings, each layer's twelve, the final layer norm's two.
    for (r, _) in c.tensors() {
        let (a, n) = (&grad[r.clone()], &numeric[r.clone()]);
        let norm =
            |v: &mut dyn Iterator<Item = f32>| v.map(|x| f64::from(x).powi(2)).sum::<f64>().sqrt();
        let diff = norm(&mut a.iter().zip(n).map(|(a, n)| a - n));
        let size = norm(&mut n.iter().copied());
        assert!(size > 1e-3, "tensor {r:?} has no gradient to check: {size}");
        // Measured: 3e-5 to 4.7e-4 (f32 rounding in the differences); checked under 1e-3.
        assert!(diff / size < 1e-3, "tensor {r:?}: relative error {}", diff / size);
    }
}

#[test]
fn a_tiny_model_learns_a_short_program_by_heart() {
    let text = "state n = 0;\nbutton \"+\" { n += 1; }\nlabel n;\n";
    let tok = Tokenizer::bytes();
    let mut seq = vec![EOS];
    seq.extend(tok.encode(text));
    seq.push(EOS);
    let c = cfg(tok.vocab(), 64, 32, 1, 2);
    let mut t = Trainer::new(Model::new(c, 1).unwrap(), 1);
    let first = t.step(std::slice::from_ref(&seq), 1e-2, 1.0).loss;
    let mut last = first;
    for _ in 0..150 {
        last = t.step(std::slice::from_ref(&seq), 1e-2, 1.0).loss;
    }
    assert!(first > 5.0 && last < 0.05, "{first} -> {last}");
    // Greedy from the end token, it writes the program back and stops.
    let out = generate(&t.model, &[EOS], 80, Some(EOS), (0.0, 0), &mut Rng::new(1));
    assert_eq!(tok.decode(&out), text);
    assert_eq!(out.last(), Some(&EOS));
}

#[test]
fn weights_round_trip_and_a_changed_byte_is_caught() {
    let tok = Tokenizer::train(&["fn at(x: int) -> int { return x; } fn at(y: int)"], 270);
    let c = cfg(tok.vocab(), 12, 8, 2, 4);
    let m = Model::new(c, 9).unwrap();
    let bytes = save(&m, &tok, "made by a test").unwrap();
    assert_eq!(bytes.len(), 4 + 4 + 24 + 8 * tok.merges().len() + 4 + 14 + 8 + 4 * c.params() + 8);
    let (m2, tok2, note) = load(&bytes).unwrap();
    assert_eq!((m2 == m, tok2 == tok, note.as_str()), (true, true, "made by a test"));
    let hash = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
    assert_eq!(hash, fnv(&bytes[..bytes.len() - 8]));
    // Any byte changed, or the tail cut, is refused with its code.
    let mut bad = bytes.clone();
    bad[100] ^= 1;
    assert_eq!(load(&bad).map(|_| ()).unwrap_err().code(), 964);
    assert_eq!(load(&bytes[..bytes.len() - 1]).map(|_| ()).unwrap_err().code(), 964);
    assert_eq!(load(b"GPT2").map(|_| ()).unwrap_err(), Error::Magic);
    // A file whose hash is right but whose shape is not: a version, then a vocab, not known.
    let resealed = |edit: &dyn Fn(&mut Vec<u8>)| {
        let mut b = bytes[..bytes.len() - 8].to_vec();
        edit(&mut b);
        let h = fnv(&b);
        b.extend_from_slice(&h.to_le_bytes());
        load(&b).map(|_| ()).unwrap_err()
    };
    assert_eq!(resealed(&|b| b[4] = 2), Error::Version(2));
    assert_eq!(resealed(&|b| b[8] = 0), Error::Config);
    assert_eq!(resealed(&|b| b.push(0)), Error::Size);
    // A header asking for 2^24 - 257 merges (a vocab of 2^24) in a file of 200 bytes: refused
    // by its size, before room for them is made.
    let big = |b: &mut Vec<u8>| {
        b[8..12].copy_from_slice(&(1u32 << 24).to_le_bytes());
        b[28..32].copy_from_slice(&((1u32 << 24) - 257).to_le_bytes());
        b.truncate(200);
    };
    assert_eq!(resealed(&big), Error::Size);
    // A tokenizer that disagrees with the model is never saved.
    assert_eq!(save(&m, &Tokenizer::bytes(), "").unwrap_err(), Error::Config);
}

#[test]
fn the_kv_cache_gives_the_full_passes_probabilities() {
    let c = cfg(19, 9, 16, 2, 4);
    let m = shaken(c, 5);
    let tokens = [3, 1, 4, 1, 5, 9, 2, 6, 5];
    let mut acts = Acts::new(&c);
    m.forward(&mut acts, &tokens, None);
    let full = acts.probs().to_vec();
    let mut s = Session::new(&m);
    for (i, &t) in tokens.iter().enumerate() {
        let mut p = s.feed(t).to_vec();
        ops::softmax(&mut p);
        // The same sums in the same order: the same bits.
        assert_eq!(p, full[i * 19..][..19], "position {i}");
    }
    assert!(s.full() && s.len() == 9);
    assert_eq!(s.feed(0).len(), 19, "a full session reads nothing more");
}

#[test]
fn threads_change_no_bit_of_training() {
    let c = cfg(23, 8, 16, 2, 2);
    let batch: Vec<Vec<u32>> =
        (0..5).map(|b| (0..9).map(|i| ((b * 7 + i * i) % 23) as u32).collect()).collect();
    let run = |threads| {
        let mut t = Trainer::new(Model::new(c, 2).unwrap(), threads);
        let stats: Vec<Stats> = (0..3).map(|_| t.step(&batch, 3e-3, 1.0)).collect();
        let eval = t.eval(&batch);
        (t.model.params, stats, eval)
    };
    let (one, three) = (run(1), run(3));
    assert!(one == three, "one thread and three differ");
    assert!(one.1[2].loss < one.1[0].loss, "{:?}", one.1);
    assert!(one.1.iter().all(|s| s.norm > 0.0 && s.norm.is_finite()));
}

#[test]
fn a_gradient_over_the_clip_is_scaled_down_to_it() {
    let c = cfg(23, 8, 16, 2, 2);
    let batch: Vec<Vec<u32>> =
        (0..3).map(|b| (0..9).map(|i| ((b * 5 + i * 3) % 23) as u32).collect()).collect();
    // A plain step (no momentum, no decay, epsilon 1) moves each parameter by g / (|g| + 1) at
    // lr 1, so the gradient it took is read back from the move: g = u / (1 - |u|).
    let step = |clip: f32| {
        let mut t = Trainer::new(Model::new(c, 4).unwrap(), 2);
        (t.opt.beta1, t.opt.beta2, t.opt.eps, t.opt.decay) = (0.0, 0.0, 1.0, 0.0);
        let before = t.model.params.clone();
        let stats = t.step(&batch, 1.0, clip);
        let taken = before.iter().zip(&t.model.params).map(|(a, b)| {
            let u = f64::from(a - b);
            (u / (1.0 - u.abs())).powi(2)
        });
        (stats.norm, taken.sum::<f64>().sqrt())
    };
    let (norm, unclipped) = step(0.0);
    assert!(norm > 0.1 && (unclipped / f64::from(norm) - 1.0).abs() < 1e-4, "{unclipped} {norm}");
    // Over the clip: the gradient taken has the clip's norm; the stats say the norm before.
    let (before, clipped) = step(norm / 4.0);
    assert_eq!(before, norm);
    assert!((clipped / f64::from(norm / 4.0) - 1.0).abs() < 1e-4, "{clipped}");
    // Under it: untouched.
    assert_eq!(step(norm * 2.0).1, unclipped);
}

#[test]
fn sequences_too_short_to_train_add_nothing() {
    let c = cfg(11, 6, 8, 1, 2);
    let mut acts = Acts::new(&c);
    let m = Model::new(c, 6).unwrap();
    assert_eq!(m.loss(&mut acts, &[], &[]), 0.0);
    let mut grad = vec![0f32; c.params()];
    assert_eq!(m.grad(&mut acts, (&[], &[]), 1.0, &mut grad), 0.0);
    assert!(grad.iter().all(|&g| g == 0.0));
    let after = |batch: &[Vec<u32>]| {
        let mut t = Trainer::new(m.clone(), 2);
        let s = t.step(batch, 1e-2, 1.0);
        let eval = t.eval(batch);
        (t.model.params, s, eval)
    };
    // One token, or none: no position to train, so the step changes nothing.
    let (params, s, eval) = after(&[vec![4], vec![]]);
    assert!(params == m.params && s == Stats { loss: 0.0, norm: 0.0 } && eval == 0.0);
    // Beside a sequence that trains, they change nothing either.
    let (alone, s1, _) = after(&[vec![1, 2, 3]]);
    let (with, s2, _) = after(&[vec![1, 2, 3], vec![4], vec![]]);
    assert!(alone == with && s1 == s2 && alone != m.params);
}

#[test]
#[should_panic(expected = "a token id past the vocab")]
fn an_input_past_the_vocab_is_refused() {
    let c = cfg(11, 6, 8, 1, 2);
    Model::new(c, 1).unwrap().loss(&mut Acts::new(&c), &[1, 11], &[2, 3]);
}

#[test]
#[should_panic(expected = "a token id past the vocab")]
fn a_target_past_the_vocab_is_refused_alike() {
    let c = cfg(11, 6, 8, 1, 2);
    Model::new(c, 1).unwrap().loss(&mut Acts::new(&c), &[1, 2], &[2, 9999]);
}

#[test]
#[should_panic(expected = "a token id past the vocab")]
fn a_session_refuses_a_token_past_the_vocab() {
    let m = Model::new(cfg(11, 6, 8, 1, 2), 1).unwrap();
    Session::new(&m).feed(11);
}

#[test]
fn bpe_learns_the_commonest_pairs_and_round_trips() {
    // The classic: aa is commonest; then (aa)a and ab tie, and the smaller pair wins.
    let tok = Tokenizer::train(&["aaabdaaabac"], 260);
    let (a, b) = (u32::from(b'a'), u32::from(b'b'));
    assert_eq!(tok.merges(), [(a, a), (a, b), (257, 258)]);
    assert_eq!(tok.encode("aaabdaaabac"), [259, 100, 259, 97, 99]);
    for text in ["", "aaab", "state é = \"ü\";\n", "\u{1f600} grid"] {
        assert_eq!(tok.decode(&tok.encode(text)), text);
    }
    assert_eq!(tok.decode(&[EOS, 97, 9999]), "a");
    assert_eq!(Tokenizer::from_merges(vec![(97, 300)]).unwrap_err(), Error::Merge(0));
    assert_eq!(Tokenizer::from_merges(vec![(97, EOS)]).unwrap_err().code(), 966);
    // Merges of merges double: the ninth would make 512 bytes, past PIECE, and is refused (a
    // file's 30 such merges would ask for a gigabyte).
    let doubling = |n: u32| (0..n).map(|i| if i == 0 { (97, 97) } else { (256 + i, 256 + i) });
    let eight = Tokenizer::from_merges(doubling(8).collect()).unwrap();
    assert_eq!(eight.piece(264).len(), PIECE);
    assert_eq!(Tokenizer::from_merges(doubling(9).collect()).unwrap_err(), Error::Merge(8));
    // Training never makes one: a run of 1000 a's doubles up to PIECE bytes and no further.
    let run = "a".repeat(1000);
    let tok = Tokenizer::train(&[&run, &run], 300);
    let longest = (257..tok.vocab() as u32).map(|t| tok.piece(t).len()).max();
    assert_eq!((tok.piece(264).len(), longest), (PIECE, Some(PIECE)));
    assert_eq!(tok.decode(&tok.encode(&run)), run);
}

#[test]
fn sampling_is_greedy_cold_and_follows_the_odds_warm() {
    let logits = [0.0, 3.0, 1.0, 3.0];
    let mut rng = Rng::new(4);
    assert_eq!(sample(&logits, 0.0, 0, &mut rng), 1, "ties go to the lower id");
    assert_eq!(sample(&logits, 1.0, 1, &mut rng), 1);
    let mut seen = [0u32; 4];
    for _ in 0..4000 {
        seen[sample(&logits, 1.0, 3, &mut rng) as usize] += 1;
    }
    // Top 3 of e^3, e^3, e^1: about 46.8%, 46.8%, 6.3%; the fourth never.
    assert_eq!(seen[0], 0);
    assert!((1700..2050).contains(&seen[1]) && (1700..2050).contains(&seen[3]), "{seen:?}");
    assert!((180..330).contains(&seen[2]), "{seen:?}");
    // A NaN of either sign (x86 makes them negative, wasm and ARM positive) is never drawn.
    for nan in [f32::NAN, -f32::NAN] {
        assert_eq!(sample(&[1.0, nan, 0.5], 0.0, 0, &mut rng), 0);
        assert!((0..200).all(|_| sample(&[0.0, nan, 0.0], 1.0, 0, &mut rng) != 1));
        assert!((0..200).all(|_| sample(&[nan, -1.0, nan], 2.0, 2, &mut rng) == 1));
    }
    // Nothing drawable: the lowest id, as when cold.
    let masked = [f32::NEG_INFINITY; 3];
    assert_eq!(sample(&masked, 1.0, 0, &mut rng), 0);
}

#[test]
fn configs_are_checked_and_init_is_seeded() {
    assert_eq!(Model::new(cfg(10, 4, 6, 1, 4), 1).unwrap_err(), Error::Config);
    assert_eq!(Model::new(cfg(0, 4, 8, 1, 4), 1).unwrap_err().code(), 965);
    // Few parameters but activations past 2^29 floats (attention's ctx^2): refused.
    assert_eq!(cfg(257, 1 << 20, 2, 1, 1).check(), Err(Error::Config));
    assert!(cfg(257, 1 << 13, 2, 1, 1).check().is_ok());
    // Shapes whose counts overflow are refused, never wrapped (nor a panic in debug); on wasm32
    // the largest shapes a file may name overflow so.
    for huge in [cfg(usize::MAX, 2, 2, 1, 1), cfg(1 << 31, 1 << 31, 1 << 31, 1, 1)] {
        assert_eq!((huge.check(), huge.params()), (Err(Error::Config), usize::MAX));
    }
    let most = 1 << 24;
    assert_eq!(cfg(most, most, most, 256, 1).check(), Err(Error::Config));
    // Tonight's shape fits.
    assert!(cfg(512, 1024, 128, 4, 4).check().is_ok());
    let c = cfg(10, 4, 8, 3, 2);
    assert_eq!(c.params(), 14 * 8 + 3 * (12 * 64 + 13 * 8) + 16);
    let (a, b) = (Model::new(c, 1).unwrap(), Model::new(c, 1).unwrap());
    assert!(a == b && a != Model::new(c, 2).unwrap());
    // Layer norms start at 1 and biases at 0; the rest is spread about 0.
    let at = c.at(1, model::LN2_W);
    assert!(a.params[at.clone()].iter().all(|&p| p == 1.0));
    assert!(a.params[at.end..at.end + 8].iter().all(|&p| p == 0.0));
    let wte = &a.params[c.wte()];
    let sd = (wte.iter().map(|p| p * p).sum::<f32>() / wte.len() as f32).sqrt();
    assert!((0.012..0.028).contains(&sd), "{sd}");
}

#[test]
fn generation_slides_past_a_full_context() {
    let c = cfg(30, 6, 8, 1, 2);
    let m = shaken(c, 8);
    let out = generate(&m, &[1, 2, 3, 4, 5, 6, 7, 8], 20, None, (1.0, 0), &mut Rng::new(3));
    assert_eq!(out.len(), 20);
    assert!(out.iter().all(|&t| t < 30));
    let again = generate(&m, &[1, 2, 3, 4, 5, 6, 7, 8], 20, None, (1.0, 0), &mut Rng::new(3));
    assert_eq!(out, again, "seeded");
}

#[test]
fn tiled_products_sum_as_the_plain_ones_do() {
    let mut rng = Rng::new(11);
    let mut draw = |n: usize| (0..n).map(|_| rng.normal() as f32).collect::<Vec<f32>>();
    // Rows and outputs that are not whole tiles, and inputs that are.
    let (rows, ic, oc) = (7, 16, 6);
    let (dout, inp, w, dw0) = (draw(rows * oc), draw(rows * ic), draw(oc * ic), draw(oc * ic));
    let (mut a_in, mut a_w, mut a_b) = (vec![0f32; rows * ic], dw0.clone(), vec![1f32; oc]);
    let (mut b_in, mut b_w) = (vec![0f32; rows * ic], dw0);
    ops::matmul_back((&mut a_in, &mut a_w, Some(&mut a_b)), &dout, &inp, &w, ic);
    ops::matmul_back_plain((&mut b_in, &mut b_w), &dout, &inp, &w, ic);
    assert!(a_in == b_in && a_w == b_w);
    let col0: f32 = dout.chunks(oc).map(|d| d[0]).sum();
    assert_eq!(a_b[0], 1.0 + col0);
    // And the forward's four-row blocks give each row's plain dot products.
    let mut out = vec![0f32; rows * oc];
    ops::matmul(&mut out, &inp, &w, None, ic);
    for (t, o) in [(0, 0), (3, 5), (6, 2)] {
        assert_eq!(out[t * oc + o], ops::dot(&inp[t * ic..][..ic], &w[o * ic..][..ic]));
    }
}
