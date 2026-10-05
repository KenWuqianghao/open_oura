//! Resilience: the balance of stress and recovery in the last 14 days.
//!
//! Three parts, each 0–100: recovery in sleep (the sleep score and the night HRV
//! against the baseline), recovery in the day (the share of measured daytime in
//! the `restored` zone), and stress load (the share in the `stressed` zone, less is
//! better). The weights are 40/30/30. A part with no data drops out and the others
//! share its weight. The five level names are Oura's.

use serde::Serialize;

use crate::scores::curve;

/// One day of the window.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DayLoad {
    pub stressed_min: f64,
    pub restored_min: f64,
    /// Classified daytime minutes. 0 when the day has no daytime data.
    pub measured_min: f64,
    pub sleep_score: Option<f64>,
    /// Night HRV as a z-score against the wearer's baseline.
    pub hrv_z: Option<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Resilience {
    pub score: f64,
    /// `limited` | `adequate` | `solid` | `strong` | `exceptional`
    pub level: &'static str,
    pub days: usize,
    pub sleep_recovery: Option<f64>,
    pub daytime_recovery: Option<f64>,
    pub stress_load: Option<f64>,
}

const MIN_DAYS: usize = 5;
/// A day needs this much classified daytime to count for the daytime parts.
const MIN_MEASURED_MIN: f64 = 60.0;

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

pub fn level(score: f64) -> &'static str {
    match score {
        s if s >= 85.0 => "exceptional",
        s if s >= 75.0 => "strong",
        s if s >= 60.0 => "solid",
        s if s >= 45.0 => "adequate",
        _ => "limited",
    }
}

/// Resilience from the days of the window (at most the last 14 are used).
/// `None` when fewer than five days have data.
pub fn resilience(days: &[DayLoad]) -> Option<Resilience> {
    let days = &days[days.len().saturating_sub(14)..];
    let with_data = days
        .iter()
        .filter(|d| d.sleep_score.is_some() || d.hrv_z.is_some() || d.measured_min >= MIN_MEASURED_MIN)
        .count();
    if with_data < MIN_DAYS {
        return None;
    }
    let sleep: Vec<f64> = days
        .iter()
        .filter_map(|d| {
            let hrv = d.hrv_z.map(|z| {
                curve(z, &[(-2.0, 20.0), (-1.0, 50.0), (0.0, 75.0), (1.0, 95.0), (2.0, 100.0)])
            });
            match (d.sleep_score, hrv) {
                (Some(s), Some(h)) => Some(0.6 * s + 0.4 * h),
                (Some(s), None) => Some(s),
                (None, Some(h)) => Some(h),
                (None, None) => None,
            }
        })
        .collect();
    let daytime: Vec<&DayLoad> = days.iter().filter(|d| d.measured_min >= MIN_MEASURED_MIN).collect();
    let restored: Vec<f64> = daytime.iter().map(|d| d.restored_min / d.measured_min).collect();
    let stressed: Vec<f64> = daytime.iter().map(|d| d.stressed_min / d.measured_min).collect();

    let sleep_recovery = mean(&sleep);
    let daytime_recovery = mean(&restored)
        .map(|s| curve(s, &[(0.0, 20.0), (0.05, 50.0), (0.15, 80.0), (0.30, 100.0)]));
    let stress_load = mean(&stressed)
        .map(|s| curve(s, &[(0.05, 100.0), (0.15, 80.0), (0.30, 50.0), (0.50, 20.0)]));

    let parts = [(sleep_recovery, 0.4), (daytime_recovery, 0.3), (stress_load, 0.3)];
    let weight: f64 = parts.iter().filter(|p| p.0.is_some()).map(|p| p.1).sum();
    if weight <= 0.0 {
        return None;
    }
    let score = parts.iter().filter_map(|p| p.0.map(|v| v * p.1)).sum::<f64>() / weight;
    let score = score.round().clamp(0.0, 100.0);
    Some(Resilience {
        score,
        level: level(score),
        days: with_data,
        sleep_recovery: sleep_recovery.map(f64::round),
        daytime_recovery: daytime_recovery.map(f64::round),
        stress_load: stress_load.map(f64::round),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn good_sleep_and_calm_days_give_a_high_level() {
        let day = DayLoad {
            stressed_min: 20.0,
            restored_min: 150.0,
            measured_min: 600.0,
            sleep_score: Some(88.0),
            hrv_z: Some(0.8),
        };
        let r = resilience(&[day; 10]).unwrap();
        assert!(r.score >= 85.0, "{r:?}");
        assert_eq!(r.level, "exceptional");
        assert_eq!(r.days, 10);
    }

    #[test]
    fn stressed_days_with_poor_sleep_give_a_low_level() {
        let day = DayLoad {
            stressed_min: 300.0,
            restored_min: 0.0,
            measured_min: 600.0,
            sleep_score: Some(55.0),
            hrv_z: Some(-1.5),
        };
        let r = resilience(&[day; 7]).unwrap();
        assert_eq!(r.level, "limited");
    }

    #[test]
    fn sleep_only_data_still_gives_a_result() {
        let day = DayLoad { sleep_score: Some(80.0), ..Default::default() };
        let r = resilience(&[day; 6]).unwrap();
        assert_eq!(r.score, 80.0);
        assert!(r.daytime_recovery.is_none());
        assert!(resilience(&[day; 4]).is_none());
    }
}
