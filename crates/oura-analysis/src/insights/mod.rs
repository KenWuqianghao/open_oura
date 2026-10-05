//! **Insights: open estimates, not ports.**
//!
//! Oura computes these results with models or cloud scores that an independent
//! client cannot run. Each module here gives a documented estimate from published
//! methods and the wearer's own baseline. They take values in and give values out;
//! the caller owns all state. Expect them to follow Oura's results, not to match
//! them. See `docs/algorithms/insights.md`.
//!
//! * [`regularity`]: Sleep Regularity Index, clock-time spread, chronotype.
//! * [`bedtime`]: the bedtime window that gave the best nights.
//! * [`stress`]: daytime stress zones from heart rate and HRV.
//! * [`resilience`]: 14-day balance of stress and recovery.
//! * [`illness`]: rule-based check of the night's vital signs.
//! * [`nightsignal`]: NightSignal alert on a sustained rise of resting heart rate.
//! * [`cycle`]: calendar and temperature estimate of the menstrual cycle.
//! * [`open_sleep`]: wake / REM / non-REM from heartbeat times (SleepECG model).

pub mod bedtime;
pub mod cycle;
pub mod illness;
pub mod nightsignal;
pub mod open_sleep;
pub mod regularity;
pub mod resilience;
pub mod stress;

const DAY_MIN: f64 = 1440.0;

/// Circular mean and standard deviation of clock times (minutes after midnight).
/// A plain mean is wrong for times on both sides of midnight: 23:30 and 00:30
/// average to 12:00. Returns `(mean, sd)` in minutes, mean in `0..1440`.
pub(crate) fn clock_stats(minutes: &[f64]) -> Option<(f64, f64)> {
    if minutes.is_empty() {
        return None;
    }
    let n = minutes.len() as f64;
    let (mut s, mut c) = (0.0, 0.0);
    for m in minutes {
        let a = m.rem_euclid(DAY_MIN) / DAY_MIN * std::f64::consts::TAU;
        s += a.sin();
        c += a.cos();
    }
    let (s, c) = (s / n, c / n);
    let r = (s * s + c * c).sqrt().clamp(1e-12, 1.0);
    let mean = s.atan2(c).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * DAY_MIN;
    let sd = (-2.0 * r.ln()).max(0.0).sqrt() / std::f64::consts::TAU * DAY_MIN;
    Some((mean, sd))
}

/// Signed shortest distance from clock time `from` to clock time `to`, in
/// minutes, in `-720..720`. Positive when `to` is later.
pub(crate) fn clock_delta(from: f64, to: f64) -> f64 {
    (to - from + DAY_MIN / 2.0).rem_euclid(DAY_MIN) - DAY_MIN / 2.0
}

/// Median of `values`; `None` when empty.
pub(crate) fn median(values: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) { (v[mid - 1] + v[mid]) / 2.0 } else { v[mid] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_mean_crosses_midnight() {
        let (mean, sd) = clock_stats(&[23.5 * 60.0, 0.5 * 60.0]).unwrap();
        assert!(!(1.0..=1439.0).contains(&mean), "{mean}");
        assert!((sd - 30.0).abs() < 1.0, "{sd}");
        assert_eq!(clock_delta(23.0 * 60.0, 60.0), 120.0);
        assert_eq!(clock_delta(60.0, 23.0 * 60.0), -120.0);
    }
}
