//! A sleep score you can argue with.
//!
//! Every threshold traces to a paper rather than to a regression against Oura's
//! black box. Ported from Maxxis20's `sleep_score.rs` in open_health (MIT), where it
//! was validated against 661 nights of a real trends export: r = 0.814 with Oura's
//! own score, 78 % of nights within 10 points, median difference −5.
//!
//! ## Sources
//!
//! * **Ohayon M, et al. "National Sleep Foundation's sleep quality recommendations:
//!   first report." Sleep Health 2017;3(1):6–19.** Appropriateness bands for sleep
//!   efficiency, latency, WASO and long awakenings, by age.
//! * **Hirshkowitz M, et al. "National Sleep Foundation's sleep time duration
//!   recommendations." Sleep Health 2015;1(1):40–43.** Duration by age band.
//! * **Boulos MI, et al. "Normal polysomnography parameters in healthy adults: a
//!   systematic review and meta-analysis." Lancet Respir Med 2019;7(6):533–543.**
//!   Stage percentages in healthy adults.
//! * **de Zambotti M, et al. "The sleep of the ring: comparison of the ŌURA sleep
//!   tracker against polysomnography." Behav Sleep Med 2019;17(2):124–136.** Why
//!   architecture is weighted low: consumer PPG staging agrees with PSG far better
//!   on sleep/wake than on which stage, and deep sleep is the weakest call.
//!
//! The thresholds are transcribed from those papers' recommendation tables. They are
//! consensus *appropriateness* bands, not diagnostic cut-offs.
//!
//! ## Shape
//!
//! Each component scores 0–100 through a piecewise-linear curve pinned at the papers'
//! band edges, then components combine as a weighted mean. A component with no data
//! drops out and the remaining weights renormalise, so a night without a hypnogram
//! still scores on what it has. Resting HR and HRV are judged against **your own**
//! baseline, because the population spread of both dwarfs the night-to-night change
//! that means anything.

use super::{combine, curve, Contributor, Part, Score, Stats};

/// What a night needs to supply. Everything is optional: components with no input
/// drop out of the weighting rather than scoring zero.
#[derive(Default, Clone, Copy, Debug)]
pub struct NightInput {
    /// Minutes asleep from the hypnogram.
    pub asleep_min: Option<f64>,
    /// Minutes in bed. Used for the duration component only when `asleep_min` is
    /// missing (no stages); that component is then flagged provisional.
    pub in_bed_min: Option<f64>,
    pub efficiency_pct: Option<f64>,
    pub onset_latency_min: Option<f64>,
    pub waso_min: Option<f64>,
    pub awakenings: Option<f64>,
    pub deep_pct: Option<f64>,
    pub rem_pct: Option<f64>,
    /// Lowest resting HR of the night, with the wearer's own baseline.
    pub rhr: Option<f64>,
    pub rhr_baseline: Option<Stats>,
    pub hrv_ms: Option<f64>,
    pub hrv_baseline: Option<Stats>,
    /// Age in years — the recommendation bands are age-specific.
    pub age: f64,
}

/// Nights a personal baseline needs before it stops being provisional.
pub const BASELINE_DAYS: usize = 14;

/// Recommended sleep duration in hours for an age (Hirshkowitz 2015, Table 1):
/// `(may be appropriate low, recommended low, recommended high, may be appropriate high)`.
pub fn duration_band(age: f64) -> (f64, f64, f64, f64) {
    match age {
        a if a < 14.0 => (7.0, 9.0, 11.0, 12.0), // school age
        a if a < 18.0 => (7.0, 8.0, 10.0, 11.0), // teen
        a if a < 26.0 => (6.0, 7.0, 9.0, 11.0),  // young adult
        a if a < 65.0 => (6.0, 7.0, 9.0, 10.0),  // adult
        _ => (5.0, 7.0, 8.0, 9.0),               // older adult
    }
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

/// Score one night. `None` when nothing scoreable was supplied.
pub fn score_night(input: NightInput) -> Option<Score> {
    let mut parts: Vec<Part> = Vec::new();

    // ── duration (Hirshkowitz 2015) ─────────────────────────────────────────────
    // Weighted highest: the best-measured thing a ring reports and the one with the
    // strongest outcome literature behind it.
    let (duration_min, from_bed) = match (input.asleep_min, input.in_bed_min) {
        (Some(a), _) => (Some(a), false),
        (None, Some(b)) => (Some(b), true),
        _ => (None, false),
    };
    if let Some(minutes) = duration_min {
        let hours = minutes / 60.0;
        let (may_lo, rec_lo, rec_hi, may_hi) = duration_band(input.age);
        let score = curve(
            hours,
            &[
                (may_lo - 2.0, 0.0),
                (may_lo, 50.0),
                (rec_lo, 90.0),
                (rec_lo + 0.5, 100.0),
                (rec_hi, 100.0),
                (may_hi, 75.0),
                (may_hi + 2.0, 40.0),
            ],
        );
        parts.push(part(
            "duration",
            if from_bed { "Time in bed" } else { "Total sleep" },
            score,
            30.0,
            Some((hours * 10.0).round() / 10.0),
            "h",
            from_bed,
            "Hirshkowitz 2015 (NSF duration)",
        ));
    }

    // ── efficiency (Ohayon 2017): ≥85 % appropriate for every adult band ────────
    if let Some(efficiency) = input.efficiency_pct {
        parts.push(part(
            "efficiency",
            "Efficiency",
            curve(efficiency, &[(60.0, 0.0), (75.0, 40.0), (85.0, 85.0), (92.0, 100.0), (100.0, 100.0)]),
            20.0,
            Some(efficiency.round()),
            "%",
            false,
            "Ohayon 2017 (NSF quality)",
        ));
    }

    // ── sleep onset latency (Ohayon 2017): ≤15 min appropriate, >60 inappropriate ──
    if let Some(latency) = input.onset_latency_min {
        parts.push(part(
            "onset_latency",
            "Time to fall asleep",
            curve(latency, &[(0.0, 100.0), (15.0, 100.0), (30.0, 70.0), (45.0, 40.0), (60.0, 20.0), (120.0, 0.0)]),
            10.0,
            Some(latency.round()),
            "min",
            false,
            "Ohayon 2017 (NSF quality)",
        ));
    }

    // ── wake after sleep onset (Ohayon 2017): ≤20 min for 18–64, more for 65+ ───
    if let Some(waso) = input.waso_min {
        let appropriate = if input.age >= 65.0 { 30.0 } else { 20.0 };
        parts.push(part(
            "waso",
            "Awake during the night",
            curve(
                waso,
                &[
                    (0.0, 100.0),
                    (appropriate, 95.0),
                    (appropriate * 2.0, 65.0),
                    (appropriate * 3.0, 35.0),
                    (appropriate * 5.0, 0.0),
                ],
            ),
            15.0,
            Some(waso.round()),
            "min",
            false,
            "Ohayon 2017 (NSF quality)",
        ));
    }

    // ── awakenings (Ohayon 2017) ────────────────────────────────────────────────
    // The paper counts awakenings longer than 5 minutes. A hypnogram count includes
    // brief arousals the paper would not have counted, so this is scored leniently
    // and weighted low — an honest mismatch, not a hidden one.
    if let Some(awakenings) = input.awakenings {
        parts.push(part(
            "awakenings",
            "Awakenings",
            curve(awakenings, &[(0.0, 100.0), (2.0, 95.0), (5.0, 75.0), (10.0, 45.0), (20.0, 10.0)]),
            5.0,
            Some(awakenings),
            "",
            false,
            "Ohayon 2017 (NSF quality)",
        ));
    }

    // ── architecture (Boulos 2019): deliberately the smallest weight ────────────
    let mut architecture: Vec<f64> = Vec::new();
    let mut arch_value: Option<f64> = None;
    if let Some(deep) = input.deep_pct {
        architecture.push(curve(deep, &[(0.0, 0.0), (8.0, 50.0), (13.0, 95.0), (16.0, 100.0), (23.0, 100.0), (35.0, 85.0)]));
        arch_value = Some(deep.round());
    }
    if let Some(rem) = input.rem_pct {
        architecture.push(curve(rem, &[(0.0, 0.0), (10.0, 45.0), (20.0, 95.0), (22.0, 100.0), (25.0, 100.0), (35.0, 80.0)]));
    }
    if !architecture.is_empty() {
        parts.push(part(
            "architecture",
            "Stage balance",
            architecture.iter().sum::<f64>() / architecture.len() as f64,
            10.0,
            arch_value,
            "% deep",
            false,
            "Boulos 2019 (PSG meta-analysis); weighted low per de Zambotti 2019",
        ));
    }

    // ── restorative physiology, against the wearer's own baseline ───────────────
    let mut physiology: Vec<f64> = Vec::new();
    let mut provisional = false;
    let mut phys_value: Option<f64> = None;
    if let (Some(rhr), Some(base)) = (input.rhr, input.rhr_baseline) {
        let z = base.z(rhr);
        // Below your own baseline is good; an elevated resting HR is the classic
        // "something is off" signal (strain, alcohol, illness coming on).
        physiology.push(curve(z, &[(-2.0, 100.0), (0.0, 90.0), (1.0, 70.0), (2.0, 40.0), (3.5, 10.0)]));
        provisional |= !base.mature(BASELINE_DAYS);
        phys_value = Some(rhr.round());
    }
    if let (Some(hrv), Some(base)) = (input.hrv_ms, input.hrv_baseline) {
        let z = base.z(hrv);
        physiology.push(curve(z, &[(-3.0, 10.0), (-2.0, 35.0), (-1.0, 70.0), (0.0, 90.0), (1.5, 100.0)]));
        provisional |= !base.mature(BASELINE_DAYS);
    }
    if !physiology.is_empty() {
        parts.push(part(
            "physiology",
            "Heart rate & HRV",
            physiology.iter().sum::<f64>() / physiology.len() as f64,
            10.0,
            phys_value,
            "bpm",
            provisional,
            "Your own 14-day baseline",
        ));
    }

    combine(parts, "published norms (NSF 2015/2017, Boulos 2019) + your baseline")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adult(mut input: NightInput) -> NightInput {
        input.age = 40.0;
        input
    }

    #[test]
    fn a_textbook_night_scores_high() {
        let v = score_night(adult(NightInput {
            asleep_min: Some(8.0 * 60.0),
            efficiency_pct: Some(92.0),
            onset_latency_min: Some(12.0),
            waso_min: Some(15.0),
            awakenings: Some(1.0),
            deep_pct: Some(18.0),
            rem_pct: Some(22.0),
            ..Default::default()
        }))
        .unwrap();
        assert!(v.score >= 95.0, "{v:?}");
        assert!(!v.provisional);
    }

    #[test]
    fn short_broken_sleep_scores_low() {
        let v = score_night(adult(NightInput {
            asleep_min: Some(4.0 * 60.0),
            efficiency_pct: Some(66.0),
            onset_latency_min: Some(55.0),
            waso_min: Some(90.0),
            awakenings: Some(12.0),
            deep_pct: Some(5.0),
            rem_pct: Some(8.0),
            ..Default::default()
        }))
        .unwrap();
        assert!(v.score <= 40.0, "{v:?}");
    }

    /// A model-free night has no hypnogram, so it must still score on what it has.
    #[test]
    fn missing_components_renormalise_instead_of_scoring_zero() {
        let full = score_night(adult(NightInput {
            asleep_min: Some(8.0 * 60.0),
            efficiency_pct: Some(90.0),
            deep_pct: Some(18.0),
            rem_pct: Some(22.0),
            ..Default::default()
        }))
        .unwrap();
        let partial = score_night(adult(NightInput {
            asleep_min: Some(8.0 * 60.0),
            efficiency_pct: Some(90.0),
            ..Default::default()
        }))
        .unwrap();
        assert!((full.score - partial.score).abs() < 6.0, "full {} vs partial {}", full.score, partial.score);
        let weights: f64 = partial.contributors.iter().map(|c| c.weight).sum();
        assert!((weights - 1.0).abs() < 0.02, "weights must renormalise: {weights}");
    }

    #[test]
    fn time_in_bed_is_a_provisional_stand_in_for_sleep() {
        let v = score_night(adult(NightInput { in_bed_min: Some(7.5 * 60.0), ..Default::default() })).unwrap();
        assert!(v.provisional);
        assert_eq!(v.contributors[0].name, "Time in bed");
        assert!(score_night(adult(NightInput::default())).is_none());
    }

    #[test]
    fn duration_bands_follow_age() {
        let night = |hours: f64, age: f64| {
            score_night(NightInput { asleep_min: Some(hours * 60.0), age, ..Default::default() }).unwrap().score
        };
        assert_eq!(night(7.5, 40.0), 100.0);
        assert_eq!(night(7.5, 70.0), 100.0);
        assert!(night(9.5, 40.0) > night(9.5, 70.0));
        assert!(night(6.5, 16.0) < night(6.5, 40.0));
    }

    #[test]
    fn physiology_uses_the_wearers_own_baseline() {
        let base = NightInput { asleep_min: Some(8.0 * 60.0), rhr: Some(48.0), hrv_ms: Some(60.0), ..Default::default() };
        let stats = |mean: f64, sd: f64| Some(Stats { mean, sd, n: 20 });
        let calm = score_night(adult(NightInput { rhr_baseline: stats(50.0, 3.0), hrv_baseline: stats(55.0, 8.0), ..base })).unwrap();
        let strained = score_night(adult(NightInput { rhr_baseline: stats(42.0, 3.0), hrv_baseline: stats(80.0, 8.0), ..base })).unwrap();
        assert!(calm.score > strained.score, "{calm:?} vs {strained:?}");
        assert!(!calm.provisional);
        let young = score_night(adult(NightInput { rhr_baseline: Some(Stats { mean: 50.0, sd: 3.0, n: 4 }), ..base })).unwrap();
        assert!(young.provisional, "a 4-night baseline is provisional");
    }
}
