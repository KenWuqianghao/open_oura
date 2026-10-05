//! SleepECG's feature extraction (`sleepecg.extract_features`, v0.5.x), ported
//! value for value with the `wrn-gru-mesa` parameters: 30 s epochs, a window of
//! 120 s before and 150 s after each epoch start, RR intervals from 0.3 s to 2 s,
//! a 4 Hz resample for the spectrum, and at most half of a window missing.
//!
//! Each epoch gives 36 values: 26 time-domain HRV features, 7 frequency-domain
//! HRV features (Task Force 1996; Shaffer 2017; Toichi 1997), then the clock time
//! of the recording start, the age and the sex. A value that cannot be computed is
//! NaN, as in NumPy. The numerical quirks of the original stay, because the model
//! was trained on them (for example, pNN50 divides by the longest window of the
//! night, not by the window itself).

pub(crate) const N_FEATURES: usize = 36;

const EPOCH_S: f64 = 30.0;
const LOOKBACK_S: f64 = 120.0;
const LOOKFORWARD_S: f64 = 150.0;
const MIN_RRI_S: f64 = 0.3;
const MAX_RRI_S: f64 = 2.0;
const FS_HZ: f64 = 4.0;
const MAX_NANS: f64 = 0.5;
/// Resampled points in one window: (120 s + 150 s) × 4 Hz.
const WINDOW: usize = 1080;
/// Resampled points between two epochs: 30 s × 4 Hz.
const STEP: usize = 120;
/// Highest spectrum bin in use: 0.4 Hz × 1080 / 4 Hz.
const MAX_BIN: usize = 108;

/// The recording facts that are model inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Subject {
    /// Local clock time of the window start, in seconds after midnight.
    pub start_sec_of_day: f64,
    /// Age in years.
    pub age: Option<f64>,
    /// `Some(true)` for female, `Some(false)` for male (SleepECG: 0 female, 1 male).
    pub female: Option<bool>,
}

/// Feature rows for `epochs` epochs. `intervals` are (time, RR interval) pairs in
/// seconds, sorted by time; the time is that of the beat that ends the interval,
/// in seconds after the window start (times before 0 feed the first windows).
/// SleepECG takes `np.diff(heartbeat_times)` with the later beat's time.
pub(crate) fn extract(intervals: &[(f64, f64)], epochs: usize, subject: &Subject) -> Vec<[f64; N_FEATURES]> {
    let (rri, rri_times): (Vec<f64>, Vec<f64>) = intervals
        .iter()
        .map(|&(t, rri)| {
            let rri = if (MIN_RRI_S..=MAX_RRI_S).contains(&rri) { rri } else { f64::NAN };
            (rri, t)
        })
        .unzip();
    let time = time_domain(&rri, &rri_times, epochs);
    let frequency = frequency_domain(&rri, &rri_times, epochs);
    let gender = subject.female.map_or(f64::NAN, |f| if f { 0.0 } else { 1.0 });
    let age = subject.age.unwrap_or(f64::NAN);
    (0..epochs)
        .map(|i| {
            let mut row = [f64::NAN; N_FEATURES];
            row[..26].copy_from_slice(&time[i]);
            row[26..33].copy_from_slice(&frequency[i]);
            row[33] = subject.start_sec_of_day;
            row[34] = age;
            row[35] = gender;
            row
        })
        .collect()
}

fn epoch_start(i: usize) -> f64 {
    i as f64 * EPOCH_S
}

// ── time domain ──────────────────────────────────────────────────────────────

fn time_domain(rri: &[f64], rri_times: &[f64], epochs: usize) -> Vec<[f64; 26]> {
    // the RR intervals whose time is in [epoch − 120 s, epoch + 150 s)
    let windows: Vec<&[f64]> = (0..epochs)
        .map(|i| {
            let lo = rri_times.partition_point(|&t| t < epoch_start(i) - LOOKBACK_S);
            let hi = rri_times.partition_point(|&t| t < epoch_start(i) + LOOKFORWARD_S);
            &rri[lo..hi.max(lo)]
        })
        .collect();
    // NumPy pads every window with NaN to the longest one; the pNN50/pNN20 means
    // run over that padded width.
    let padded_diffs = windows.iter().map(|w| w.len()).max().unwrap_or(0).saturating_sub(1);
    windows.iter().map(|nn| time_features(nn, padded_diffs)).collect()
}

fn time_features(nn: &[f64], padded_diffs: usize) -> [f64; 26] {
    let sd: Vec<f64> = nn.windows(2).map(|w| w[1] - w[0]).collect();
    let mean_nn = nanmean(nn);
    let max_nn = nanmax(nn);
    let min_nn = nanmin(nn);
    let sdnn = nanstd1(nn);
    let rmssd = nanmean(&sd.iter().map(|d| d * d).collect::<Vec<_>>()).sqrt();
    let sdsd = nanstd1(&sd);
    let nn50 = sd.iter().filter(|d| d.abs() > 0.05).count() as f64;
    let nn20 = sd.iter().filter(|d| d.abs() > 0.02).count() as f64;
    let (pnn50, pnn20) = if padded_diffs == 0 {
        (f64::NAN, f64::NAN)
    } else {
        (nn50 / padded_diffs as f64, nn20 / padded_diffs as f64)
    };
    let median_nn = nanmedian(nn);
    let mad_nn = nanmedian(&nn.iter().map(|x| (x - median_nn).abs()).collect::<Vec<_>>());
    let iqr_nn = nanpercentile(nn, 75.0) - nanpercentile(nn, 25.0);
    let cv_nn = sdnn / mean_nn;
    let cv_sd = sdsd / nanmean(&sd);
    let hr: Vec<f64> = nn.iter().map(|x| 60.0 / x).collect();
    let sd1 = (sdsd * sdsd * 0.5).sqrt();
    let sd2 = (2.0 * sdnn * sdnn - sd1 * sd1).sqrt();
    [
        mean_nn,
        max_nn,
        min_nn,
        max_nn - min_nn,
        sdnn,
        rmssd,
        sdsd,
        nn50,
        nn20,
        pnn50,
        pnn20,
        median_nn,
        mad_nn,
        iqr_nn,
        cv_nn,
        cv_sd,
        60.0 / mean_nn,
        60.0 / min_nn,
        60.0 / max_nn,
        nanstd1(&hr),
        sd1,
        sd2,
        std::f64::consts::PI * sd1 * sd2,
        sd1 / sd2,
        sd2 / sd1,
        (sd1 * sd2 * 16.0).log10(),
    ]
}

fn finite(x: &[f64]) -> Vec<f64> {
    x.iter().copied().filter(|v| !v.is_nan()).collect()
}

fn nanmean(x: &[f64]) -> f64 {
    let v = finite(x);
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn nanmax(x: &[f64]) -> f64 {
    finite(x).into_iter().reduce(f64::max).unwrap_or(f64::NAN)
}

fn nanmin(x: &[f64]) -> f64 {
    finite(x).into_iter().reduce(f64::min).unwrap_or(f64::NAN)
}

/// Sample standard deviation (`ddof=1`); NaN with fewer than two values.
fn nanstd1(x: &[f64]) -> f64 {
    let v = finite(x);
    if v.len() < 2 {
        return f64::NAN;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

fn nanmedian(x: &[f64]) -> f64 {
    nanpercentile(x, 50.0)
}

/// NumPy's default (`linear`) percentile over the non-NaN values.
fn nanpercentile(x: &[f64], q: f64) -> f64 {
    let mut v = finite(x);
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let pos = q / 100.0 * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(v.len() - 1);
    let t = pos - lo as f64;
    // numpy's _lerp: a + (b − a)·t, from the upper end when t ≥ 0.5
    let (a, b) = (v[lo], v[hi]);
    if t >= 0.5 {
        b - (b - a) * (1.0 - t)
    } else {
        a + (b - a) * t
    }
}

// ── frequency domain ─────────────────────────────────────────────────────────

fn frequency_domain(rri: &[f64], rri_times: &[f64], epochs: usize) -> Vec<[f64; 7]> {
    if epochs == 0 {
        return Vec::new();
    }
    // np.arange(−120, last epoch + 150, 0.25) and a linear interpolation that is
    // NaN outside the beats and next to a rejected interval.
    let points = (epochs - 1) * STEP + WINDOW;
    let grid: Vec<f64> = (0..points).map(|j| -LOOKBACK_S + j as f64 / FS_HZ).collect();
    let resampled: Vec<f64> = grid.iter().map(|&t| interp(t, rri_times, rri)).collect();
    let dft = Dft::new();
    let blocks = BlockSums::new(&resampled, &dft);
    (0..epochs)
        .map(|k| {
            let window = &resampled[k * STEP..k * STEP + WINDOW];
            let psd = if blocks.complete(k) {
                Some(dft.boxcar(window, &blocks.spectrum(k, &dft)))
            } else {
                dft.partial(window)
            };
            psd.map_or([f64::NAN; 7], |psd| bands(&psd))
        })
        .collect()
}

/// `np.interp` at one point, NaN outside `[xp[0], xp[-1]]` (scipy `interp1d`).
fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if n == 0 || x < xp[0] || x > xp[n - 1] {
        return f64::NAN;
    }
    if n == 1 {
        return fp[0];
    }
    let j = xp.partition_point(|&v| v <= x).saturating_sub(1).min(n - 1);
    if x == xp[j] {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    slope * (x - xp[j]) + fp[j]
}

/// The one-sided periodogram bins 0..=108 of a 1080-point window, as scipy's
/// `periodogram` (density scaling) computes them in SleepECG's `_nanpsd`.
struct Dft {
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Dft {
    fn new() -> Self {
        let step = std::f64::consts::TAU / WINDOW as f64;
        Dft {
            cos: (0..WINDOW).map(|m| (m as f64 * step).cos()).collect(),
            sin: (0..WINDOW).map(|m| (m as f64 * step).sin()).collect(),
        }
    }

    /// A window without gaps (boxcar). `spectrum` holds the DFT bins 1..=108 of the
    /// raw window; mean removal changes only bin 0, which is computed here.
    fn boxcar(&self, window: &[f64], spectrum: &[(f64, f64)]) -> Vec<f64> {
        let mean = window.iter().sum::<f64>() / WINDOW as f64;
        let dc: f64 = window.iter().map(|x| x - mean).sum();
        let scale = 1.0 / (FS_HZ * WINDOW as f64);
        std::iter::once(dc * dc * scale)
            .chain(spectrum.iter().map(|(re, im)| 2.0 * (re * re + im * im) * scale))
            .collect()
    }

    /// A window with gaps: the valid samples, closed up, with a Hann window and
    /// zero-padded back to 1080 points; none when more than half is missing.
    fn partial(&self, window: &[f64]) -> Option<Vec<f64>> {
        let valid: Vec<f64> = window.iter().copied().filter(|v| !v.is_nan()).collect();
        if valid.is_empty() || (window.len() - valid.len()) as f64 / window.len() as f64 > MAX_NANS {
            return None;
        }
        let hann = hann_periodic(valid.len());
        let mean = valid.iter().sum::<f64>() / valid.len() as f64;
        let y: Vec<f64> = valid.iter().zip(&hann).map(|(x, w)| (x - mean) * w).collect();
        let scale = 1.0 / (FS_HZ * hann.iter().map(|w| w * w).sum::<f64>());
        Some(
            (0..=MAX_BIN)
                .map(|k| {
                    let (re, im) = self.bin(&y, k);
                    let p = (re * re + im * im) * scale;
                    // one-sided: every bin but DC (and Nyquist) counts twice
                    if k == 0 {
                        p
                    } else {
                        2.0 * p
                    }
                })
                .collect(),
        )
    }

    /// Σ y[n]·e^(−2πikn/1080) over the given samples.
    fn bin(&self, y: &[f64], k: usize) -> (f64, f64) {
        let (mut re, mut im) = (0.0, 0.0);
        let mut m = 0; // k·n mod 1080, without a division per sample
        for v in y {
            re += v * self.cos[m];
            im -= v * self.sin[m];
            m += k;
            if m >= WINDOW {
                m -= WINDOW;
            }
        }
        (re, im)
    }
}

/// The windows step by 120 samples and span 9 steps, so the DFT of a window is the
/// sum of its nine 120-sample block DFTs, each turned by its offset. Computing
/// each block once makes a night about eight times faster than a DFT per window.
struct BlockSums {
    /// Per block: has a NaN.
    gap: Vec<bool>,
    /// Per block: the bins 1..=108 of Σ x[n]·e^(−2πikn/1080), n in the block.
    bins: Vec<Vec<(f64, f64)>>,
}

const BLOCKS_PER_WINDOW: usize = WINDOW / STEP;

impl BlockSums {
    fn new(x: &[f64], dft: &Dft) -> Self {
        let blocks: Vec<&[f64]> = x.chunks_exact(STEP).collect();
        BlockSums {
            gap: blocks.iter().map(|b| b.iter().any(|v| v.is_nan())).collect(),
            bins: blocks
                .iter()
                .map(|b| {
                    if b.iter().any(|v| v.is_nan()) {
                        Vec::new()
                    } else {
                        (1..=MAX_BIN).map(|k| dft.bin(b, k)).collect()
                    }
                })
                .collect(),
        }
    }

    fn complete(&self, window: usize) -> bool {
        !self.gap[window..window + BLOCKS_PER_WINDOW].iter().any(|&g| g)
    }

    fn spectrum(&self, window: usize, dft: &Dft) -> Vec<(f64, f64)> {
        (1..=MAX_BIN)
            .map(|k| {
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..BLOCKS_PER_WINDOW {
                    let (a, b) = self.bins[window + i][k - 1];
                    let m = (k * STEP * i) % WINDOW;
                    let (c, s) = (dft.cos[m], dft.sin[m]);
                    re += a * c + b * s;
                    im += b * c - a * s;
                }
                (re, im)
            })
            .collect()
    }
}

/// scipy `get_window("hann", n)`: the periodic (DFT-even) Hann window.
fn hann_periodic(n: usize) -> Vec<f64> {
    let step = std::f64::consts::TAU / n as f64;
    (0..n).map(|i| 0.5 - 0.5 * (i as f64 * step).cos()).collect()
}

/// Total power, VLF, LF, LF norm, HF, HF norm, LF/HF from the bins 0..=108.
fn bands(psd: &[f64]) -> [f64; 7] {
    // np.fft.rfftfreq(1080, 0.25): k × (1 / 270)
    let val = 1.0 / (WINDOW as f64 / FS_HZ);
    let freq: Vec<f64> = (0..psd.len()).map(|k| k as f64 * val).collect();
    let band = |keep: &dyn Fn(f64) -> bool| -> f64 {
        let idx: Vec<usize> = (0..psd.len()).filter(|&k| keep(freq[k])).collect();
        idx.windows(2)
            .map(|w| (freq[w[1]] - freq[w[0]]) * (psd[w[1]] + psd[w[0]]) / 2.0)
            .sum()
    };
    let total = band(&|f| f <= 0.4);
    let vlf = band(&|f| 0.0033 < f && f <= 0.04);
    let lf = band(&|f| 0.04 < f && f <= 0.15);
    let hf = band(&|f| 0.15 < f && f <= 0.4);
    [
        total,
        vlf,
        lf,
        lf / (lf + hf) * 100.0,
        hf,
        hf / (lf + hf) * 100.0,
        lf / hf,
    ]
}
