//! Open sleep staging from heartbeat times: SleepECG's `wrn-gru-mesa` classifier
//! (Brunner & Hofer 2023, JOSS 8:5411; BSD-3-Clause), ported to pure Rust.
//!
//! The model was trained on 1971 MESA nights and tested on 1000 SHHS nights
//! (accuracy 0.75, Cohen's κ 0.54). It gives three classes per 30 s epoch: wake,
//! REM and non-REM. It cannot separate deep sleep from light sleep. It is the
//! fallback for a client that does not have Oura's sleep model and gets no
//! hypnogram from the ring. It is not a diagnosis. See
//! `docs/algorithms/open-sleep-staging.md`.

mod features;
mod net;

pub use features::Subject;

/// Seconds per epoch.
pub const EPOCH_S: f64 = 30.0;

/// Stage codes, the same as the other hypnograms: 2 non-REM, 3 REM, 4 wake.
/// This model has no deep-sleep code (1).
pub const NREM: i64 = 2;
pub const REM: i64 = 3;
pub const WAKE: i64 = 4;

/// One night staged by the open model.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenHypnogram {
    /// One code per 30 s epoch from the window start.
    pub stages: Vec<i64>,
    /// Mean probability of the chosen class, 0–1.
    pub confidence: f64,
}

/// Stage `epochs` epochs of 30 s. `intervals` are (time, RR interval) pairs in
/// seconds, sorted by time: the time of the beat that ends the interval, after the
/// window start. Intervals up to 120 s before the start and 150 s after the end
/// improve the edge epochs. Returns `None` for fewer than 2 epochs or when fewer
/// than half of the epochs have heart-rate data.
pub fn stage_night(intervals: &[(f64, f64)], epochs: usize, subject: &Subject) -> Option<OpenHypnogram> {
    if epochs < 2 {
        return None;
    }
    let rows = features::extract(intervals, epochs, subject);
    let covered = rows.iter().filter(|row| row[0].is_finite()).count();
    if covered * 2 < epochs {
        return None;
    }
    let probs = net::predict(&rows);
    let mut confidence = 0.0;
    let stages = probs
        .iter()
        .map(|p| {
            // UNDEFINED (index 0) is the padding class; choose among the real ones
            let (code, best) = [(NREM, p[1]), (REM, p[2]), (WAKE, p[3])]
                .into_iter()
                .fold((NREM, f32::MIN), |acc, c| if c.1 > acc.1 { c } else { acc });
            confidence += best as f64;
            code
        })
        .collect::<Vec<_>>();
    Some(OpenHypnogram { confidence: confidence / stages.len() as f64, stages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn golden() -> Vec<Value> {
        let v: Value = serde_json::from_str(include_str!("golden.json")).unwrap();
        v["cases"].as_array().unwrap().clone()
    }

    fn subject(case: &Value) -> Subject {
        Subject {
            start_sec_of_day: case["start_sec_of_day"].as_f64().unwrap(),
            age: case["age"].as_f64(),
            female: case["gender"].as_i64().map(|g| g == 0),
        }
    }

    /// SleepECG's `np.diff(heartbeat_times)`, timed at the later beat.
    fn intervals(case: &Value) -> Vec<(f64, f64)> {
        let t: Vec<f64> = case["heartbeat_times"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        t.windows(2).map(|w| (w[1], w[1] - w[0])).collect()
    }

    #[test]
    fn features_match_sleepecg() {
        for case in golden() {
            let epochs = case["epochs"].as_u64().unwrap() as usize;
            let rows = features::extract(&intervals(&case), epochs, &subject(&case));
            let ids = case["feature_ids"].as_array().unwrap();
            for (i, (row, want)) in rows.iter().zip(case["features"].as_array().unwrap()).enumerate() {
                for (j, w) in want.as_array().unwrap().iter().enumerate() {
                    let got = row[j];
                    match w.as_f64() {
                        None => assert!(!got.is_finite(), "epoch {i} {}: {got} should be NaN", ids[j]),
                        Some(w) => {
                            let tol = 1e-7 * w.abs().max(1e-3);
                            assert!((got - w).abs() <= tol, "epoch {i} {}: {got} vs {w}", ids[j]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn probabilities_match_sleepecg() {
        for case in golden() {
            let epochs = case["epochs"].as_u64().unwrap() as usize;
            let rows = features::extract(&intervals(&case), epochs, &subject(&case));
            let probs = net::predict(&rows);
            for (i, (p, want)) in probs.iter().zip(case["probs"].as_array().unwrap()).enumerate() {
                for (c, w) in want.as_array().unwrap().iter().enumerate() {
                    let w = w.as_f64().unwrap();
                    assert!((p[c] as f64 - w).abs() < 1e-4, "epoch {i} class {c}: {} vs {w}", p[c]);
                }
            }
        }
    }

    #[test]
    fn stages_follow_the_most_likely_class() {
        let case = &golden()[0];
        let epochs = case["epochs"].as_u64().unwrap() as usize;
        let night = stage_night(&intervals(case), epochs, &subject(case)).unwrap();
        assert_eq!(night.stages.len(), epochs);
        for (code, p) in night.stages.iter().zip(case["probs"].as_array().unwrap()) {
            let p: Vec<f64> = p.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            let best = (1..4).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap();
            assert_eq!(*code, [0, NREM, REM, WAKE][best]);
        }
        assert!(night.confidence > 0.0 && night.confidence <= 1.0);
    }

    #[test]
    fn a_night_without_heart_rate_is_not_staged() {
        let subject = Subject { start_sec_of_day: 82_800.0, age: Some(40.0), female: Some(true) };
        assert!(stage_night(&[], 100, &subject).is_none());
        // beats for the first 10 minutes of a 60-minute window only
        let beats: Vec<(f64, f64)> = (1..600).map(|i| (i as f64, 1.0)).collect();
        assert!(stage_night(&beats, 120, &subject).is_none());
        assert!(stage_night(&beats, 1, &subject).is_none());
    }
}
