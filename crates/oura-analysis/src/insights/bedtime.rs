//! Bedtime guidance: the bedtime window that gave the wearer's best nights.
//!
//! The centre of the window is the mean bedtime of the best third of the recent
//! nights, by sleep score. It is moved earlier when it leaves too little time for
//! the wearer's sleep need before the usual wake time. With too few scored nights,
//! the window comes from the usual wake time and the sleep need only.

use serde::Serialize;

use super::{clock_delta, clock_stats, DAY_MIN};

/// One main sleep. Clock times are minutes after local midnight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NightQuality {
    pub bedtime_min: f64,
    pub wake_min: f64,
    /// Sleep score 0–100 when the night has one.
    pub quality: Option<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BedtimeWindow {
    /// Minutes after local midnight, `0..1440`.
    pub start_min: f64,
    pub end_min: f64,
    pub usual_wake_min: f64,
    pub nights_used: usize,
    /// `best_nights` | `wake_time`
    pub basis: &'static str,
}

const MIN_NIGHTS: usize = 5;
const MIN_SCORED_NIGHTS: usize = 6;
const HALF_WIDTH_MIN: f64 = 30.0;
/// Time to fall asleep that the window allows for.
const LATENCY_MIN: f64 = 15.0;

/// The ideal bedtime window from recent main sleeps and the sleep need in
/// minutes. `None` with fewer than five nights.
pub fn ideal_bedtime(nights: &[NightQuality], need_min: f64) -> Option<BedtimeWindow> {
    if nights.len() < MIN_NIGHTS {
        return None;
    }
    let wakes: Vec<f64> = nights.iter().map(|n| n.wake_min).collect();
    let (usual_wake_min, _) = clock_stats(&wakes)?;
    let latest = (usual_wake_min - need_min - LATENCY_MIN).rem_euclid(DAY_MIN);

    let mut scored: Vec<(f64, f64)> = nights
        .iter()
        .filter_map(|n| n.quality.map(|q| (q, n.bedtime_min)))
        .collect();
    let (centre, nights_used, basis) = if scored.len() >= MIN_SCORED_NIGHTS {
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let best = &scored[..scored.len().div_ceil(3).max(3)];
        let beds: Vec<f64> = best.iter().map(|b| b.1).collect();
        let (mean, _) = clock_stats(&beds)?;
        // a best-night bedtime that is too late for the sleep need moves earlier
        let centre = if clock_delta(latest, mean) > 0.0 { latest } else { mean };
        (centre, best.len(), "best_nights")
    } else {
        (latest, nights.len(), "wake_time")
    };
    let round5 = |m: f64| ((m / 5.0).round() * 5.0).rem_euclid(DAY_MIN);
    Some(BedtimeWindow {
        start_min: round5(centre - HALF_WIDTH_MIN),
        end_min: round5(centre + HALF_WIDTH_MIN),
        usual_wake_min: round5(usual_wake_min),
        nights_used,
        basis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn night(bed_h: f64, wake_h: f64, quality: Option<f64>) -> NightQuality {
        NightQuality { bedtime_min: bed_h * 60.0, wake_min: wake_h * 60.0, quality }
    }

    #[test]
    fn window_follows_the_best_nights() {
        // early nights score high, late nights score low; wake at 07:30, need 8 h
        let mut nights = Vec::new();
        for _ in 0..3 {
            nights.push(night(22.5, 7.5, Some(88.0)));
            nights.push(night(24.5, 7.5, Some(62.0)));
            nights.push(night(23.5, 7.5, Some(70.0)));
        }
        let w = ideal_bedtime(&nights, 480.0).unwrap();
        assert_eq!(w.basis, "best_nights");
        assert_eq!(w.nights_used, 3);
        assert_eq!((w.start_min, w.end_min), (22.0 * 60.0, 23.0 * 60.0));
        assert_eq!(w.usual_wake_min, 450.0);
    }

    #[test]
    fn a_late_best_bedtime_moves_to_fit_the_sleep_need() {
        // every night starts at 01:00 and ends at 07:00: 8 h of sleep need a
        // bedtime of 22:45 or earlier
        let nights: Vec<NightQuality> = (0..6).map(|_| night(1.0, 7.0, Some(80.0))).collect();
        let w = ideal_bedtime(&nights, 480.0).unwrap();
        assert_eq!((w.start_min, w.end_min), (22.25 * 60.0, 23.25 * 60.0));
    }

    #[test]
    fn unscored_nights_use_the_wake_time() {
        let nights: Vec<NightQuality> = (0..5).map(|_| night(23.0, 7.0, None)).collect();
        let w = ideal_bedtime(&nights, 450.0).unwrap();
        assert_eq!(w.basis, "wake_time");
        assert_eq!(w.start_min, (7.0 * 60.0 - 450.0 - 15.0 - 30.0 + 1440.0) % 1440.0);
        assert!(ideal_bedtime(&nights[..4], 450.0).is_none());
    }
}
