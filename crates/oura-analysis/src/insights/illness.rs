//! Rule-based check of the night's vital signs for signs of strain or illness.
//!
//! An infection usually raises resting heart rate, breathing rate and skin
//! temperature and lowers HRV one or more days before symptoms (Mishra et al.
//! 2020, Nat Biomed Eng 4:1208; Smarr et al. 2020, Sci Rep 10:21640). Each of the
//! four signs of last night is compared with the wearer's own range from the
//! nights before. This is the model-free stand-in for Oura's Symptom Radar
//! model; the field names and the status values are those of that model's output.
//! It is not a diagnosis.

use serde::Serialize;

/// The vital signs of one night. Temperature is the deviation from the wearer's
/// baseline in °C.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct NightVitals {
    pub breath: Option<f64>,
    pub lowest_hr: Option<f64>,
    pub hrv: Option<f64>,
    pub temp_deviation: Option<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Biomarker {
    /// `AverageBreath` | `LowestHeartRate` | `AverageHrv` | `TemperatureDeviation`
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: f64,
    pub lower: f64,
    pub upper: f64,
    pub indicates_symptoms: bool,
    /// `ELEVATED` | `DECREASED`
    pub reason: Option<&'static str>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct IllnessCheck {
    pub available: bool,
    /// `NO_SIGNS` | `MINOR_SIGNS` | `MAJOR_SIGNS` | `MISSING_LAST_NIGHT_SLEEP` |
    /// `MISSING_SLEEP_DATA`
    pub status: &'static str,
    pub traffic_light: &'static str,
    /// Sum of the sign weights, 0 when no sign is out of range.
    pub score: f64,
    pub decision: u8,
    pub days_with_data: usize,
    pub biomarkers: Vec<Biomarker>,
    /// Always `rules`: tells a client that no model made this result.
    pub basis: &'static str,
}

/// Nights of history that a range needs.
pub const MIN_HISTORY_NIGHTS: usize = 7;
const RANGE_SD: f64 = 1.5;

struct Sign {
    kind: &'static str,
    pick: fn(&NightVitals) -> Option<f64>,
    /// The smallest half-width of the range, absolute and as a share of the mean.
    floor_abs: f64,
    floor_rel: f64,
    /// True when a high value is the sign; false when a low value is.
    high: bool,
    weight: f64,
}

const SIGNS: [Sign; 4] = [
    Sign { kind: "AverageBreath", pick: |n| n.breath, floor_abs: 1.0, floor_rel: 0.0, high: true, weight: 1.0 },
    Sign { kind: "LowestHeartRate", pick: |n| n.lowest_hr, floor_abs: 3.0, floor_rel: 0.0, high: true, weight: 1.0 },
    Sign { kind: "AverageHrv", pick: |n| n.hrv, floor_abs: 0.0, floor_rel: 0.15, high: false, weight: 1.0 },
    Sign { kind: "TemperatureDeviation", pick: |n| n.temp_deviation, floor_abs: 0.3, floor_rel: 0.0, high: true, weight: 1.5 },
];

fn unavailable(status: &'static str, days_with_data: usize) -> IllnessCheck {
    IllnessCheck {
        available: false,
        status,
        traffic_light: "NO_SIGNS",
        score: 0.0,
        decision: 0,
        days_with_data,
        biomarkers: Vec::new(),
        basis: "rules",
    }
}

/// Check `last` against the nights in `history` (the nights before it, any order).
pub fn illness_check(last: Option<&NightVitals>, history: &[NightVitals]) -> IllnessCheck {
    let days_with_data = history
        .iter()
        .filter(|n| SIGNS.iter().any(|s| (s.pick)(n).is_some()))
        .count();
    let Some(last) = last else {
        return unavailable("MISSING_LAST_NIGHT_SLEEP", days_with_data);
    };
    let mut biomarkers = Vec::new();
    let mut score = 0.0;
    for sign in &SIGNS {
        let Some(value) = (sign.pick)(last) else { continue };
        let past: Vec<f64> = history.iter().filter_map(|n| (sign.pick)(n)).collect();
        if past.len() < MIN_HISTORY_NIGHTS {
            continue;
        }
        let mean = past.iter().sum::<f64>() / past.len() as f64;
        let sd = (past.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / past.len() as f64).sqrt();
        let half = (RANGE_SD * sd).max(sign.floor_abs).max(sign.floor_rel * mean.abs());
        let (lower, upper) = (mean - half, mean + half);
        let out = if sign.high { value > upper } else { value < lower };
        if out {
            let excess = if sign.high { value - upper } else { lower - value };
            // a value more than one half-width outside the range counts half again
            score += sign.weight * if excess > half { 1.5 } else { 1.0 };
        }
        let r2 = |v: f64| (v * 100.0).round() / 100.0;
        biomarkers.push(Biomarker {
            kind: sign.kind,
            value: r2(value),
            lower: r2(lower),
            upper: r2(upper),
            indicates_symptoms: out,
            reason: out.then_some(if sign.high { "ELEVATED" } else { "DECREASED" }),
        });
    }
    if biomarkers.is_empty() {
        return unavailable("MISSING_SLEEP_DATA", days_with_data);
    }
    let (status, decision) = match score {
        s if s >= 2.0 => ("MAJOR_SIGNS", 2),
        s if s >= 1.0 => ("MINOR_SIGNS", 1),
        _ => ("NO_SIGNS", 0),
    };
    IllnessCheck {
        available: true,
        status,
        traffic_light: status,
        score,
        decision,
        days_with_data,
        biomarkers,
        basis: "rules",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usual(i: usize) -> NightVitals {
        let wobble = (i % 3) as f64 - 1.0;
        NightVitals {
            breath: Some(14.0 + 0.2 * wobble),
            lowest_hr: Some(52.0 + wobble),
            hrv: Some(45.0 + 2.0 * wobble),
            temp_deviation: Some(0.05 * wobble),
        }
    }

    fn history() -> Vec<NightVitals> {
        (0..14).map(usual).collect()
    }

    #[test]
    fn a_usual_night_shows_no_signs() {
        let c = illness_check(Some(&usual(1)), &history());
        assert_eq!(c.status, "NO_SIGNS");
        assert_eq!(c.biomarkers.len(), 4);
        assert!(c.biomarkers.iter().all(|b| !b.indicates_symptoms));
    }

    #[test]
    fn one_sign_is_minor_and_more_are_major() {
        let mut night = usual(1);
        night.lowest_hr = Some(58.0);
        let c = illness_check(Some(&night), &history());
        assert_eq!((c.status, c.decision), ("MINOR_SIGNS", 1));
        let hr = c.biomarkers.iter().find(|b| b.kind == "LowestHeartRate").unwrap();
        assert_eq!(hr.reason, Some("ELEVATED"));

        night.temp_deviation = Some(0.9);
        night.hrv = Some(30.0);
        let c = illness_check(Some(&night), &history());
        assert_eq!(c.status, "MAJOR_SIGNS");
        let hrv = c.biomarkers.iter().find(|b| b.kind == "AverageHrv").unwrap();
        assert_eq!(hrv.reason, Some("DECREASED"));
    }

    #[test]
    fn missing_data_is_reported_not_guessed() {
        assert_eq!(illness_check(None, &history()).status, "MISSING_LAST_NIGHT_SLEEP");
        let c = illness_check(Some(&usual(0)), &history()[..3]);
        assert_eq!(c.status, "MISSING_SLEEP_DATA");
        assert!(!c.available);
    }
}
