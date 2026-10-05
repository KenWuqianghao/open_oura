//! Sleep regularity and chronotype.
//!
//! The Sleep Regularity Index (Phillips et al. 2017, Sci Rep 7:3216) is the chance
//! that the wearer is in the same state (asleep or awake) at two times 24 h apart,
//! scaled so that 100 is a perfectly regular schedule and 0 is random. The clock
//! spread and the chronotype use circular statistics of the main sleep of each
//! day. The chronotype bands follow the midsleep distribution of the Munich
//! Chronotype Questionnaire (Roenneberg et al. 2007); the ring has no record of
//! work days, so all days count.

use serde::Serialize;

use super::{clock_stats, DAY_MIN};

/// One sleep period in local minutes from a fixed local midnight. All spans of
/// one call must use the same origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SleepSpan {
    pub start_min: i64,
    pub end_min: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Regularity {
    /// Sleep Regularity Index. 100 = the same schedule each day.
    pub sri: f64,
    /// Pairs of days (24 h apart) that the index compares.
    pub day_pairs: usize,
    /// Days with a main sleep.
    pub days: usize,
    pub bedtime_min: f64,
    pub bedtime_sd_min: f64,
    pub wake_min: f64,
    pub wake_sd_min: f64,
    /// Mean midpoint of the main sleep, minutes after local midnight.
    pub midpoint_min: f64,
    pub midpoint_sd_min: f64,
    /// `early` | `moderately_early` | `intermediate` | `moderately_late` | `late`
    pub chronotype: &'static str,
}

/// A main sleep is at least this long. Shorter periods are naps: they count for
/// the index (the wearer was asleep) but not for the clock statistics.
const MAIN_SLEEP_MIN: i64 = 180;
/// The index compares states at this step.
const EPOCH_MIN: i64 = 5;
/// Fewer day pairs than this give no result.
const MIN_DAY_PAIRS: usize = 4;

/// A sleep day runs from noon to noon, so one night is in one day.
fn sleep_day(minute: i64) -> i64 {
    (minute - 720).div_euclid(1440)
}

fn asleep(spans: &[SleepSpan], minute: i64) -> bool {
    spans.iter().any(|s| s.start_min <= minute && minute < s.end_min)
}

fn chronotype(midpoint_min: f64) -> &'static str {
    // A midpoint in the evening is before midnight: map it below zero.
    let m = if midpoint_min > 720.0 { midpoint_min - DAY_MIN } else { midpoint_min };
    match m {
        m if m < 150.0 => "early",
        m if m < 210.0 => "moderately_early",
        m if m < 270.0 => "intermediate",
        m if m < 330.0 => "moderately_late",
        _ => "late",
    }
}

/// Regularity of the sleep periods in `spans`. `None` when fewer than five
/// days with a main sleep have a neighbour day.
pub fn sleep_regularity(spans: &[SleepSpan]) -> Option<Regularity> {
    let spans: Vec<SleepSpan> = spans.iter().copied().filter(|s| s.end_min > s.start_min).collect();
    // the main sleep of each sleep day: the longest period with its midpoint there
    let mut main: std::collections::BTreeMap<i64, SleepSpan> = Default::default();
    for s in &spans {
        if s.end_min - s.start_min < MAIN_SLEEP_MIN {
            continue;
        }
        let day = sleep_day((s.start_min + s.end_min) / 2);
        let longer = main
            .get(&day)
            .is_none_or(|m| s.end_min - s.start_min > m.end_min - m.start_min);
        if longer {
            main.insert(day, *s);
        }
    }
    let (mut same, mut total, mut pairs) = (0usize, 0usize, 0usize);
    for day in main.keys() {
        if !main.contains_key(&(day + 1)) {
            continue;
        }
        pairs += 1;
        let base = day * 1440 + 720;
        for step in 0..(1440 / EPOCH_MIN) {
            let t = base + step * EPOCH_MIN;
            same += usize::from(asleep(&spans, t) == asleep(&spans, t + 1440));
            total += 1;
        }
    }
    if pairs < MIN_DAY_PAIRS {
        return None;
    }
    let sri = -100.0 + 200.0 * same as f64 / total as f64;
    let clock = |pick: fn(&SleepSpan) -> i64| -> Vec<f64> {
        main.values().map(|s| pick(s).rem_euclid(1440) as f64).collect()
    };
    let (bedtime_min, bedtime_sd_min) = clock_stats(&clock(|s| s.start_min))?;
    let (wake_min, wake_sd_min) = clock_stats(&clock(|s| s.end_min))?;
    let (midpoint_min, midpoint_sd_min) = clock_stats(&clock(|s| (s.start_min + s.end_min) / 2))?;
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    Some(Regularity {
        sri: r1(sri),
        day_pairs: pairs,
        days: main.len(),
        bedtime_min: r1(bedtime_min),
        bedtime_sd_min: r1(bedtime_sd_min),
        wake_min: r1(wake_min),
        wake_sd_min: r1(wake_sd_min),
        midpoint_min: r1(midpoint_min),
        midpoint_sd_min: r1(midpoint_sd_min),
        chronotype: chronotype(midpoint_min),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn night(day: i64, bed_min: i64, hours: i64) -> SleepSpan {
        let start = day * 1440 + bed_min;
        SleepSpan { start_min: start, end_min: start + hours * 60 }
    }

    #[test]
    fn the_same_schedule_each_day_scores_100() {
        let spans: Vec<SleepSpan> = (0..7).map(|d| night(d, 23 * 60, 8)).collect();
        let r = sleep_regularity(&spans).unwrap();
        assert_eq!(r.sri, 100.0);
        assert_eq!(r.day_pairs, 6);
        assert!(r.bedtime_sd_min < 0.5);
        assert_eq!(r.bedtime_min, 1380.0);
        assert_eq!(r.midpoint_min, 180.0);
        assert_eq!(r.chronotype, "moderately_early");
    }

    #[test]
    fn a_shifting_schedule_scores_lower() {
        // bedtime moves between 22:00 and 02:00 on alternate days
        let spans: Vec<SleepSpan> = (0..8)
            .map(|d| night(d, if d % 2 == 0 { 22 * 60 } else { 26 * 60 }, 7))
            .collect();
        let r = sleep_regularity(&spans).unwrap();
        assert!(r.sri < 70.0, "{}", r.sri);
        assert!(r.bedtime_sd_min > 90.0, "{}", r.bedtime_sd_min);
    }

    #[test]
    fn too_few_days_give_no_result() {
        let spans: Vec<SleepSpan> = (0..3).map(|d| night(d, 23 * 60, 8)).collect();
        assert!(sleep_regularity(&spans).is_none());
    }
}
