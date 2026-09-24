//! Readiness: ecore's eight contributors and the weights recovered from a trends
//! export (`docs/algorithms/score-weights.md`, R² = 0.969 on the combiner), scored
//! against the wearer's own 14-day baselines.
//!
//! Oura's contributor curves live in constant tables that do not read back from the
//! binary (`readiness_calculate @ 0x20897c`, weights `0x17bff8/0x17bfdc`). The curves
//! here are explicit, transparent stand-ins:
//!
//! | contributor | weight | input | curve |
//! | --- | --- | --- | --- |
//! | Resting heart rate | 17 | z vs your 14-day baseline | below baseline good, elevated bad |
//! | Previous night | 15 | last night's sleep score | identity |
//! | HRV balance | 15 | z vs your 14-day baseline | above baseline good |
//! | Body temperature | 13 | deviation from your baseline, °C | ±0.3 °C tolerated |
//! | Sleep balance | 12 | 14-day mean sleep ÷ sleep need | ≥ 1.0 is full marks |
//! | Previous day activity | 10 | yesterday's active kcal, z vs baseline | very hard days lower it |
//! | Recovery index | 10 | share of the night after resting HR bottomed | early minimum is good |
//! | Activity balance | 7 | 7-day mean active kcal ÷ 14-day mean | 0.8–1.2 is full marks |
//!
//! Every baseline-relative contributor is **provisional** until its window holds 14
//! days, and drops out entirely below 3 days. The final number renormalises over
//! whatever is present.

use super::{combine, curve, Contributor, Part, Score, Stats};

/// Days of history a readiness baseline needs before it stops being provisional.
pub const BASELINE_DAYS: usize = 14;

#[derive(Default, Clone, Copy, Debug)]
pub struct Input {
    /// Last night's sleep score (0–100).
    pub sleep_score: Option<f64>,
    /// Lowest resting HR of the night and the baseline of previous nights.
    pub rhr: Option<f64>,
    pub rhr_baseline: Option<Stats>,
    /// Nightly HRV (RMSSD, ms) and the baseline of previous nights.
    pub hrv_ms: Option<f64>,
    pub hrv_baseline: Option<Stats>,
    /// Tonight's skin temperature minus the baseline mean, °C, with the baseline.
    pub temp_deviation_c: Option<f64>,
    pub temp_baseline: Option<Stats>,
    /// Mean nightly sleep over the past 14 days divided by the personal sleep need.
    pub sleep_balance_ratio: Option<f64>,
    /// Days with sleep in that 14-day window.
    pub sleep_balance_days: usize,
    /// Yesterday's active kcal and the baseline of previous days.
    pub prev_day_active_kcal: Option<f64>,
    pub active_kcal_baseline: Option<Stats>,
    /// Share of the night (0–1) that came after resting HR reached its minimum.
    pub recovery_fraction: Option<f64>,
    /// 7-day mean active kcal divided by the 14-day mean.
    pub activity_balance_ratio: Option<f64>,
}

fn part(
    key: &'static str,
    name: &'static str,
    score: f64,
    raw_weight: f64,
    value: Option<f64>,
    unit: &'static str,
    provisional: bool,
    source: &'static str,
) -> Part {
    Part {
        contributor: Contributor { key, name, score, weight: 0.0, value, unit, provisional, source },
        raw_weight,
    }
}

/// Score a morning. `None` when nothing scoreable was supplied.
pub fn score(input: Input) -> Option<Score> {
    let mut parts: Vec<Part> = Vec::new();
    let own = "Your own 14-day baseline";

    if let (Some(rhr), Some(base)) = (input.rhr, input.rhr_baseline) {
        let z = base.z(rhr);
        parts.push(part(
            "resting_hr",
            "Resting heart rate",
            curve(z, &[(-2.0, 100.0), (-0.5, 95.0), (0.5, 85.0), (1.5, 60.0), (2.5, 30.0), (4.0, 5.0)]),
            17.0,
            Some(rhr.round()),
            "bpm",
            !base.mature(BASELINE_DAYS),
            own,
        ));
    }

    if let Some(sleep) = input.sleep_score {
        parts.push(part(
            "previous_night",
            "Previous night",
            sleep.clamp(0.0, 100.0),
            15.0,
            Some(sleep.round()),
            "",
            false,
            "Last night's sleep score",
        ));
    }

    if let (Some(hrv), Some(base)) = (input.hrv_ms, input.hrv_baseline) {
        let z = base.z(hrv);
        parts.push(part(
            "hrv_balance",
            "HRV balance",
            curve(z, &[(-3.0, 5.0), (-2.0, 30.0), (-1.0, 65.0), (-0.3, 85.0), (0.3, 95.0), (1.5, 100.0)]),
            15.0,
            Some(hrv.round()),
            "ms",
            !base.mature(BASELINE_DAYS),
            own,
        ));
    }

    if let Some(dev) = input.temp_deviation_c {
        let mature = input.temp_baseline.map(|b| b.mature(BASELINE_DAYS)).unwrap_or(false);
        // Skin temperature moves with illness, alcohol, and the menstrual cycle;
        // ±0.3 °C is ordinary night-to-night scatter.
        parts.push(part(
            "temperature",
            "Body temperature",
            curve(dev.abs(), &[(0.0, 100.0), (0.3, 95.0), (0.5, 70.0), (0.8, 40.0), (1.2, 10.0)]),
            13.0,
            Some((dev * 100.0).round() / 100.0),
            "°C",
            !mature,
            own,
        ));
    }

    if let Some(ratio) = input.sleep_balance_ratio {
        parts.push(part(
            "sleep_balance",
            "Sleep balance",
            curve(ratio, &[(0.6, 0.0), (0.75, 35.0), (0.85, 65.0), (0.95, 90.0), (1.0, 100.0)]),
            12.0,
            Some((ratio * 100.0).round()),
            "% of need",
            input.sleep_balance_days < BASELINE_DAYS,
            "14-day sleep vs your sleep need",
        ));
    }

    if let (Some(kcal), Some(base)) = (input.prev_day_active_kcal, input.active_kcal_baseline) {
        let z = base.z(kcal);
        // A hard day yesterday asks for recovery today; a quiet one does not.
        parts.push(part(
            "previous_day_activity",
            "Previous day activity",
            curve(z, &[(-3.0, 100.0), (0.5, 100.0), (1.0, 90.0), (1.5, 75.0), (2.0, 60.0), (3.0, 35.0), (4.0, 20.0)]),
            10.0,
            Some(kcal.round()),
            "kcal",
            !base.mature(BASELINE_DAYS),
            own,
        ));
    }

    if let Some(fraction) = input.recovery_fraction {
        // Oura's recovery index: resting HR bottoming out in the first half of the
        // night means the body finished recovering early.
        parts.push(part(
            "recovery_index",
            "Recovery index",
            curve(fraction, &[(0.0, 0.0), (0.1, 25.0), (0.25, 55.0), (0.4, 85.0), (0.5, 100.0)]),
            10.0,
            Some((fraction * 100.0).round()),
            "% of night",
            false,
            "Night HR minimum → wake",
        ));
    }

    if let Some(ratio) = input.activity_balance_ratio {
        let mature = input.active_kcal_baseline.map(|b| b.mature(BASELINE_DAYS)).unwrap_or(false);
        parts.push(part(
            "activity_balance",
            "Activity balance",
            curve(ratio, &[(0.3, 40.0), (0.6, 75.0), (0.8, 100.0), (1.2, 100.0), (1.5, 75.0), (2.0, 40.0)]),
            7.0,
            Some((ratio * 100.0).round()),
            "% of usual",
            !mature,
            "7-day vs 14-day activity",
        ));
    }

    combine(parts, "ecore contributors + recovered weights, curves vs your baseline")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mature(mean: f64, sd: f64) -> Option<Stats> {
        Some(Stats { mean, sd, n: 20 })
    }

    fn rested() -> Input {
        Input {
            sleep_score: Some(90.0),
            rhr: Some(48.0),
            rhr_baseline: mature(50.0, 3.0),
            hrv_ms: Some(70.0),
            hrv_baseline: mature(60.0, 10.0),
            temp_deviation_c: Some(0.05),
            temp_baseline: mature(34.5, 0.2),
            sleep_balance_ratio: Some(1.02),
            sleep_balance_days: 14,
            prev_day_active_kcal: Some(400.0),
            active_kcal_baseline: mature(450.0, 120.0),
            recovery_fraction: Some(0.55),
            activity_balance_ratio: Some(1.0),
        }
    }

    #[test]
    fn a_rested_morning_scores_high() {
        let s = score(rested()).unwrap();
        assert!(s.score >= 93.0, "{s:?}");
        assert!(!s.provisional);
        assert_eq!(s.contributors.len(), 8);
        let weights: f64 = s.contributors.iter().map(|c| c.weight).sum();
        assert!((weights - 1.0).abs() < 0.02);
    }

    #[test]
    fn strain_and_a_fever_pull_it_down() {
        let s = score(Input {
            sleep_score: Some(55.0),
            rhr: Some(58.0),
            hrv_ms: Some(35.0),
            temp_deviation_c: Some(0.9),
            sleep_balance_ratio: Some(0.8),
            prev_day_active_kcal: Some(900.0),
            recovery_fraction: Some(0.1),
            ..rested()
        })
        .unwrap();
        assert!(s.score <= 45.0, "{s:?}");
    }

    #[test]
    fn young_baselines_are_provisional_and_missing_inputs_drop_out() {
        let s = score(Input {
            rhr_baseline: Some(Stats { mean: 50.0, sd: 3.0, n: 5 }),
            hrv_ms: None,
            recovery_fraction: None,
            ..rested()
        })
        .unwrap();
        assert!(s.provisional);
        assert_eq!(s.contributors.len(), 6);
        assert!(score(Input::default()).is_none());
    }
}
