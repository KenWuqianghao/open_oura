//! Daytime stress zones from heart rate and HRV.
//!
//! Mental stress raises heart rate and lowers beat-to-beat variability (Kim et
//! al. 2018, Psychiatry Investig 15:235). Each awake, sedentary window gets an
//! index: how far its heart rate is above, and its RMSSD below, the wearer's own
//! daytime reference. The reference is the median and the median absolute
//! deviation of the wearer's sedentary windows of the days before, so the index
//! is relative: `stressed` means high for this person, not high for a population.
//! Windows with movement are not classified, because the ring cannot tell
//! physical load from mental load there.

use serde::Serialize;

use super::median;

/// One awake window (5 minutes is typical).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayWindow {
    pub start_s: i64,
    pub mean_bpm: f64,
    pub rmssd_ms: Option<f64>,
    /// Metabolic equivalent in the window, when known. 1.0 is rest.
    pub met: Option<f64>,
}

/// The wearer's daytime reference.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Reference {
    pub bpm_median: f64,
    pub bpm_spread: f64,
    pub ln_rmssd_median: Option<f64>,
    pub ln_rmssd_spread: f64,
    pub windows: usize,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    Restored,
    Relaxed,
    Engaged,
    Stressed,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DayStress {
    pub stressed_min: f64,
    pub engaged_min: f64,
    pub relaxed_min: f64,
    pub restored_min: f64,
    /// Windows with movement, not classified.
    pub active_min: f64,
    /// Sum of the four zones.
    pub measured_min: f64,
    pub mean_index: Option<f64>,
}

/// A window at or above this MET has movement.
pub const SEDENTARY_MET: f64 = 1.5;
/// A reference needs this many sedentary windows.
pub const MIN_REFERENCE_WINDOWS: usize = 48;
const MIN_BPM_SPREAD: f64 = 3.0;
const MIN_LN_RMSSD_SPREAD: f64 = 0.15;

fn sedentary(w: &DayWindow) -> bool {
    w.met.is_none_or(|m| m < SEDENTARY_MET) && (30.0..=220.0).contains(&w.mean_bpm)
}

/// Median and scaled median absolute deviation (1.4826 × MAD estimates the SD).
fn centre_spread(values: &[f64], floor: f64) -> Option<(f64, f64)> {
    let centre = median(values)?;
    let deviations: Vec<f64> = values.iter().map(|v| (v - centre).abs()).collect();
    Some((centre, (1.4826 * median(&deviations)?).max(floor)))
}

/// The reference from the windows of the days before. `None` with fewer than
/// [`MIN_REFERENCE_WINDOWS`] sedentary windows.
pub fn reference(windows: &[DayWindow]) -> Option<Reference> {
    let rest: Vec<&DayWindow> = windows.iter().filter(|w| sedentary(w)).collect();
    if rest.len() < MIN_REFERENCE_WINDOWS {
        return None;
    }
    let bpm: Vec<f64> = rest.iter().map(|w| w.mean_bpm).collect();
    let (bpm_median, bpm_spread) = centre_spread(&bpm, MIN_BPM_SPREAD)?;
    let ln: Vec<f64> = rest
        .iter()
        .filter_map(|w| w.rmssd_ms.filter(|r| *r > 0.0).map(f64::ln))
        .collect();
    let hrv = (ln.len() >= MIN_REFERENCE_WINDOWS / 2)
        .then(|| centre_spread(&ln, MIN_LN_RMSSD_SPREAD))
        .flatten();
    Some(Reference {
        bpm_median,
        bpm_spread,
        ln_rmssd_median: hrv.map(|h| h.0),
        ln_rmssd_spread: hrv.map_or(MIN_LN_RMSSD_SPREAD, |h| h.1),
        windows: rest.len(),
    })
}

/// The stress index of one window: the mean of the heart rate z-score and the
/// negative RMSSD z-score. `None` for a window with movement.
pub fn stress_index(window: &DayWindow, reference: &Reference) -> Option<f64> {
    if !sedentary(window) {
        return None;
    }
    let z_bpm = (window.mean_bpm - reference.bpm_median) / reference.bpm_spread;
    let z_hrv = match (window.rmssd_ms.filter(|r| *r > 0.0), reference.ln_rmssd_median) {
        (Some(r), Some(m)) => Some((r.ln() - m) / reference.ln_rmssd_spread),
        _ => None,
    };
    Some(match z_hrv {
        Some(z) => (z_bpm - z) / 2.0,
        None => z_bpm,
    })
}

pub fn zone(index: f64) -> Zone {
    match index {
        i if i >= 1.0 => Zone::Stressed,
        i if i >= 0.0 => Zone::Engaged,
        i if i >= -1.0 => Zone::Relaxed,
        _ => Zone::Restored,
    }
}

/// Minutes in each zone for the windows of one day. `window_min` is the length
/// of one window.
pub fn day_stress(windows: &[DayWindow], reference: &Reference, window_min: f64) -> DayStress {
    let mut out = DayStress::default();
    let mut sum = 0.0;
    let mut n = 0usize;
    for w in windows {
        let Some(index) = stress_index(w, reference) else {
            out.active_min += window_min;
            continue;
        };
        sum += index;
        n += 1;
        match zone(index) {
            Zone::Stressed => out.stressed_min += window_min,
            Zone::Engaged => out.engaged_min += window_min,
            Zone::Relaxed => out.relaxed_min += window_min,
            Zone::Restored => out.restored_min += window_min,
        }
    }
    out.measured_min = out.stressed_min + out.engaged_min + out.relaxed_min + out.restored_min;
    out.mean_index = (n > 0).then(|| (sum / n as f64 * 100.0).round() / 100.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(bpm: f64, rmssd: f64, met: f64) -> DayWindow {
        DayWindow { start_s: 0, mean_bpm: bpm, rmssd_ms: Some(rmssd), met: Some(met) }
    }

    fn usual_days() -> Vec<DayWindow> {
        // 60 windows around 70 bpm and 40 ms
        (0..60).map(|i| w(64.0 + (i % 13) as f64, 34.0 + (i % 7) as f64 * 2.0, 1.1)).collect()
    }

    #[test]
    fn reference_needs_enough_sedentary_windows() {
        assert!(reference(&usual_days()[..40]).is_none());
        let moving: Vec<DayWindow> = (0..60).map(|_| w(110.0, 12.0, 4.0)).collect();
        assert!(reference(&moving).is_none());
        let r = reference(&usual_days()).unwrap();
        assert_eq!(r.windows, 60);
        assert!((r.bpm_median - 70.0).abs() < 1.0);
    }

    #[test]
    fn high_heart_rate_with_low_hrv_is_stressed() {
        let r = reference(&usual_days()).unwrap();
        assert_eq!(zone(stress_index(&w(88.0, 18.0, 1.0), &r).unwrap()), Zone::Stressed);
        assert_eq!(zone(stress_index(&w(56.0, 75.0, 1.0), &r).unwrap()), Zone::Restored);
        // a walk is not classified
        assert!(stress_index(&w(105.0, 15.0, 3.2), &r).is_none());
    }

    #[test]
    fn day_totals_count_each_window_once() {
        let r = reference(&usual_days()).unwrap();
        let day = [w(88.0, 18.0, 1.0), w(88.0, 18.0, 1.0), w(56.0, 75.0, 1.0), w(105.0, 15.0, 3.2)];
        let d = day_stress(&day, &r, 5.0);
        assert_eq!(d.stressed_min, 10.0);
        assert_eq!(d.restored_min, 5.0);
        assert_eq!(d.active_min, 5.0);
        assert_eq!(d.measured_min, 15.0);
    }
}
