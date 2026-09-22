//! **Live daily scores — Sleep, Readiness, Activity — computed on the device.**
//!
//! These are *not* bit-for-bit ports. Oura's combiner structure and weights were
//! recovered from ecore and a trends-export regression (see
//! `docs/algorithms/score-weights.md`), but the per-contributor curves live in
//! constant tables that do not read back from the binary. So each scorer here keeps
//! the recovered **contributor set and weights** and scores every contributor with a
//! transparent piecewise-linear curve pinned to published recommendations or to the
//! wearer's own baseline. The output carries every contributor with its applied
//! weight, so a number can always be argued with.
//!
//! * [`sleep`]: literature-based night score (NSF 2015/2017, Boulos 2019). Ported
//!   from Maxxis20's contribution to open_health (`crates/oura-summary/src/sleep_score.rs`,
//!   MIT), validated there at r = 0.81 against 661 nights of Oura's own score.
//! * [`readiness`]: ecore's eight readiness contributors with the recovered weights,
//!   scored against personal 14-day baselines. Provisional until the baselines mature.
//! * [`activity`]: ecore's five activity contributors with the recovered weights and
//!   its `Y = [0, 25, 95, 100]` piecewise shape, against an explicit daily goal.
//!
//! See `docs/algorithms/live-scores.md`.

pub mod activity;
pub mod readiness;
pub mod sleep;

use serde::Serialize;

/// One contributor of a score: its 0–100 sub-score, the weight actually applied
/// after renormalising for missing inputs, and the raw value behind it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Contributor {
    pub key: &'static str,
    pub name: &'static str,
    /// 0–100.
    pub score: f64,
    /// Share of the final score, 0–1, after renormalisation.
    pub weight: f64,
    /// The input behind the sub-score, in `unit`, for the breakdown view.
    pub value: Option<f64>,
    pub unit: &'static str,
    /// True when the input rests on an immature baseline or a partial day.
    pub provisional: bool,
    /// Where the curve comes from, for the "sources" toggle.
    pub source: &'static str,
}

/// A 0–100 score with its breakdown.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Score {
    pub score: f64,
    pub contributors: Vec<Contributor>,
    /// True when any applied contributor is provisional.
    pub provisional: bool,
    pub basis: &'static str,
}

/// Mean, standard deviation, and sample count of a personal history window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub mean: f64,
    pub sd: f64,
    pub n: usize,
}

impl Stats {
    /// Population statistics of `values`; `None` when there are fewer than three.
    pub fn of(values: &[f64]) -> Option<Stats> {
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.len() < 3 {
            return None;
        }
        let n = finite.len() as f64;
        let mean = finite.iter().sum::<f64>() / n;
        let var = finite.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        Some(Stats { mean, sd: var.sqrt(), n: finite.len() })
    }

    /// Standard score of `x`. A near-zero SD is floored at 5 % of the mean (or
    /// 1e-6) so an unusually flat history cannot explode a small change.
    pub fn z(&self, x: f64) -> f64 {
        let floor = (self.mean.abs() * 0.05).max(1e-6);
        (x - self.mean) / self.sd.max(floor)
    }

    /// True once the window holds the days a personal baseline needs.
    pub fn mature(&self, days: usize) -> bool {
        self.n >= days
    }
}

/// Piecewise-linear interpolation through `(input, score)` anchors sorted by input.
/// Outside the ends it clamps, so an absurd input cannot produce an absurd score.
pub fn curve(x: f64, anchors: &[(f64, f64)]) -> f64 {
    if x <= anchors[0].0 {
        return anchors[0].1;
    }
    for pair in anchors.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x <= x1 {
            let t = if (x1 - x0).abs() < f64::EPSILON { 0.0 } else { (x - x0) / (x1 - x0) };
            return y0 + t * (y1 - y0);
        }
    }
    anchors[anchors.len() - 1].1
}

/// ecore's activity-style mapper: four input breakpoints `x` (worst → best) onto the
/// fixed `Y = [0, 25, 95, 100]` shape (`get_activity_score_raw @ 0x1d5788`).
/// `x` may run in either direction; the first entry is always the worst input.
pub fn ecore_curve(value: f64, x: [f64; 4]) -> f64 {
    const Y: [f64; 4] = [0.0, 25.0, 95.0, 100.0];
    if x[0] <= x[3] {
        curve(value, &[(x[0], Y[0]), (x[1], Y[1]), (x[2], Y[2]), (x[3], Y[3])])
    } else {
        curve(value, &[(x[3], Y[3]), (x[2], Y[2]), (x[1], Y[1]), (x[0], Y[0])])
    }
}

/// A contributor before combination: the raw ecore weight (percent).
pub(crate) struct Part {
    pub contributor: Contributor,
    pub raw_weight: f64,
}

/// Weighted mean with renormalisation: a contributor with no input drops out and the
/// others share its weight, so a night or day missing one signal is scored on what it
/// has instead of being punished for what the ring did not record.
pub(crate) fn combine(parts: Vec<Part>, basis: &'static str) -> Option<Score> {
    if parts.is_empty() {
        return None;
    }
    let total: f64 = parts.iter().map(|p| p.raw_weight).sum();
    if total <= 0.0 {
        return None;
    }
    let score = parts.iter().map(|p| p.contributor.score * p.raw_weight).sum::<f64>() / total;
    let contributors: Vec<Contributor> = parts
        .into_iter()
        .map(|p| Contributor {
            score: p.contributor.score.round(),
            weight: (p.raw_weight / total * 100.0).round() / 100.0,
            ..p.contributor
        })
        .collect();
    let provisional = contributors.iter().any(|c| c.provisional);
    Some(Score { score: score.round().clamp(0.0, 100.0), contributors, provisional, basis })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_clamps_outside_its_anchors() {
        let anchors = [(0.0, 10.0), (10.0, 90.0)];
        assert_eq!(curve(-5.0, &anchors), 10.0);
        assert_eq!(curve(50.0, &anchors), 90.0);
        assert_eq!(curve(5.0, &anchors), 50.0);
    }

    #[test]
    fn ecore_curve_runs_both_directions() {
        // more is better
        assert_eq!(ecore_curve(0.0, [0.0, 1.0, 3.0, 5.0]), 0.0);
        assert_eq!(ecore_curve(3.0, [0.0, 1.0, 3.0, 5.0]), 95.0);
        assert_eq!(ecore_curve(9.0, [0.0, 1.0, 3.0, 5.0]), 100.0);
        // less is better
        assert_eq!(ecore_curve(12.0, [12.0, 8.0, 5.0, 3.0]), 0.0);
        assert_eq!(ecore_curve(5.0, [12.0, 8.0, 5.0, 3.0]), 95.0);
        assert_eq!(ecore_curve(1.0, [12.0, 8.0, 5.0, 3.0]), 100.0);
    }

    #[test]
    fn stats_need_three_values_and_floor_sd() {
        assert!(Stats::of(&[1.0, 2.0]).is_none());
        let s = Stats::of(&[50.0, 50.0, 50.0]).unwrap();
        assert_eq!(s.mean, 50.0);
        // flat history: sd floored at 5 % of the mean, so +5 bpm is z = 2
        assert!((s.z(55.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn combine_renormalises_weights() {
        let part = |score: f64, w: f64| Part {
            contributor: Contributor {
                key: "k",
                name: "n",
                score,
                weight: 0.0,
                value: None,
                unit: "",
                provisional: false,
                source: "",
            },
            raw_weight: w,
        };
        let s = combine(vec![part(100.0, 30.0), part(50.0, 10.0)], "test").unwrap();
        assert_eq!(s.score, 88.0);
        let weights: f64 = s.contributors.iter().map(|c| c.weight).sum();
        assert!((weights - 1.0).abs() < 0.02);
        assert!(combine(vec![], "test").is_none());
    }
}
