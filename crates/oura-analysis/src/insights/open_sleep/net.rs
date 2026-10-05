//! The `wrn-gru-mesa` network, evaluated in f32 like Keras:
//!
//! Masking(−1) → BatchNorm(ε 0.001) → Dense(64) → ReLU → BiGRU(8) → BiGRU(8) →
//! Dense(4) → softmax over UNDEFINED, NREM, REM, WAKE.
//!
//! The GRUs use the Keras defaults: gate order z, r, h; `reset_after`; a separate
//! input and recurrent bias. A masked epoch (all 36 inputs equal −1) keeps the
//! state and gives a zero output (`zero_output_for_mask`).
//!
//! `wrn_gru_mesa.bin` holds the weights as little-endian f32 in Keras
//! `get_weights()` order (written by `tools/export_sleepecg.py`).

use std::sync::OnceLock;

use super::features::N_FEATURES;

const HIDDEN: usize = 64;
const UNITS: usize = 8;
const CLASSES: usize = 4;
const MASK: f32 = -1.0;
const BN_EPSILON: f32 = 0.001;

static BYTES: &[u8] = include_bytes!("wrn_gru_mesa.bin");

struct Gru {
    kernel: Vec<f32>,    // [input][3·UNITS]
    recurrent: Vec<f32>, // [UNITS][3·UNITS]
    bias_in: Vec<f32>,   // [3·UNITS]
    bias_rec: Vec<f32>,  // [3·UNITS]
    input: usize,
}

struct Weights {
    bn_gamma: Vec<f32>,
    bn_beta: Vec<f32>,
    bn_mean: Vec<f32>,
    bn_var: Vec<f32>,
    dense: Vec<f32>, // [N_FEATURES][HIDDEN]
    dense_bias: Vec<f32>,
    gru: [Gru; 4], // layer 1 forward, backward, layer 2 forward, backward
    out: Vec<f32>, // [2·UNITS][CLASSES]
    out_bias: Vec<f32>,
}

fn weights() -> &'static Weights {
    static WEIGHTS: OnceLock<Weights> = OnceLock::new();
    WEIGHTS.get_or_init(|| {
        let all: Vec<f32> = BYTES
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let mut at = 0;
        let mut take = |n: usize| {
            let v = all[at..at + n].to_vec();
            at += n;
            v
        };
        let bn_gamma = take(N_FEATURES);
        let bn_beta = take(N_FEATURES);
        let bn_mean = take(N_FEATURES);
        let bn_var = take(N_FEATURES);
        let dense = take(N_FEATURES * HIDDEN);
        let dense_bias = take(HIDDEN);
        let mut gru = |input: usize| {
            let kernel = take(input * 3 * UNITS);
            let recurrent = take(UNITS * 3 * UNITS);
            let bias = take(2 * 3 * UNITS);
            Gru {
                kernel,
                recurrent,
                bias_in: bias[..3 * UNITS].to_vec(),
                bias_rec: bias[3 * UNITS..].to_vec(),
                input,
            }
        };
        let gru = [gru(HIDDEN), gru(HIDDEN), gru(2 * UNITS), gru(2 * UNITS)];
        let out = take(2 * UNITS * CLASSES);
        let out_bias = take(CLASSES);
        assert_eq!(at, all.len(), "wrn_gru_mesa.bin does not match the network layout");
        Weights { bn_gamma, bn_beta, bn_mean, bn_var, dense, dense_bias, gru, out, out_bias }
    })
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl Gru {
    fn step(&self, x: &[f32], h: &[f32; UNITS]) -> [f32; UNITS] {
        let n = 3 * UNITS;
        let mut gx = self.bias_in.clone();
        for (i, xi) in x.iter().enumerate().take(self.input) {
            for (g, w) in gx.iter_mut().zip(&self.kernel[i * n..(i + 1) * n]) {
                *g += xi * w;
            }
        }
        let mut gh = self.bias_rec.clone();
        for (i, hi) in h.iter().enumerate() {
            for (g, w) in gh.iter_mut().zip(&self.recurrent[i * n..(i + 1) * n]) {
                *g += hi * w;
            }
        }
        let mut next = [0f32; UNITS];
        for u in 0..UNITS {
            let z = sigmoid(gx[u] + gh[u]);
            let r = sigmoid(gx[UNITS + u] + gh[UNITS + u]);
            let candidate = (gx[2 * UNITS + u] + r * gh[2 * UNITS + u]).tanh();
            next[u] = z * h[u] + (1.0 - z) * candidate;
        }
        next
    }

    /// Outputs for each step; `reverse` runs from the last step to the first.
    fn run(&self, xs: &[Vec<f32>], mask: &[bool], reverse: bool) -> Vec<[f32; UNITS]> {
        let mut out = vec![[0f32; UNITS]; xs.len()];
        let mut h = [0f32; UNITS];
        let order: Vec<usize> = if reverse { (0..xs.len()).rev().collect() } else { (0..xs.len()).collect() };
        for t in order {
            if mask[t] {
                continue;
            }
            h = self.step(&xs[t], &h);
            out[t] = h;
        }
        out
    }
}

fn bidirectional(forward: &Gru, backward: &Gru, xs: &[Vec<f32>], mask: &[bool]) -> Vec<Vec<f32>> {
    let f = forward.run(xs, mask, false);
    let b = backward.run(xs, mask, true);
    f.iter().zip(&b).map(|(f, b)| f.iter().chain(b).copied().collect()).collect()
}

/// Class probabilities `[UNDEFINED, NREM, REM, WAKE]` for each feature row.
pub(crate) fn predict(features: &[[f64; N_FEATURES]]) -> Vec<[f32; CLASSES]> {
    let w = weights();
    let inputs: Vec<[f32; N_FEATURES]> = features
        .iter()
        .map(|row| row.map(|v| if v.is_finite() { v as f32 } else { MASK }))
        .collect();
    let mask: Vec<bool> = inputs.iter().map(|row| row.iter().all(|&v| v == MASK)).collect();
    let hidden: Vec<Vec<f32>> = inputs
        .iter()
        .map(|row| {
            let mut y = w.dense_bias.clone();
            for (i, x) in row.iter().enumerate() {
                let inv = (w.bn_var[i] + BN_EPSILON).sqrt().recip() * w.bn_gamma[i];
                let norm = x * inv + (w.bn_beta[i] - w.bn_mean[i] * inv);
                for (o, k) in y.iter_mut().zip(&w.dense[i * HIDDEN..(i + 1) * HIDDEN]) {
                    *o += norm * k;
                }
            }
            y.iter().map(|v| v.max(0.0)).collect()
        })
        .collect();
    let first = bidirectional(&w.gru[0], &w.gru[1], &hidden, &mask);
    let second = bidirectional(&w.gru[2], &w.gru[3], &first, &mask);
    second
        .iter()
        .map(|x| {
            let mut logits = [0f32; CLASSES];
            for (c, logit) in logits.iter_mut().enumerate() {
                *logit = w.out_bias[c] + (0..2 * UNITS).map(|i| x[i] * w.out[i * CLASSES + c]).sum::<f32>();
            }
            let max = logits.iter().copied().fold(f32::MIN, f32::max);
            let exp = logits.map(|l| (l - max).exp());
            let sum: f32 = exp.iter().sum();
            exp.map(|e| e / sum)
        })
        .collect()
}
