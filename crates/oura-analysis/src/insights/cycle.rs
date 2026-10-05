//! Menstrual cycle estimate from logged period starts and night temperature.
//!
//! The calendar part uses the mean length of the wearer's recent cycles and a
//! luteal phase of 14 days. The temperature part uses the "three over six" rule of
//! the basal body temperature method (Marshall 1968, Br Med J 1:803): ovulation is
//! confirmed when three nights in sequence are 0.2 °C or more above the highest of
//! the six nights before them. This is an estimate for information. Do not use it
//! for contraception.

use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CycleEstimate {
    /// Day 1 is the first day of the last period.
    pub cycle_day: i64,
    /// `menstrual` | `follicular` | `fertile` | `luteal` | `late`
    pub phase: &'static str,
    pub mean_cycle_days: f64,
    /// Complete cycles behind the mean. 0 means the default of 28 days.
    pub cycles_used: usize,
    /// Day indices (days from the same origin as the input).
    pub last_period_day: i64,
    pub next_period_day: i64,
    pub ovulation_day: i64,
    /// True when the temperature rise confirmed the ovulation day.
    pub ovulation_confirmed: bool,
    pub fertile_start_day: i64,
    pub fertile_end_day: i64,
}

const DEFAULT_CYCLE_DAYS: f64 = 28.0;
const LUTEAL_DAYS: i64 = 14;
const PERIOD_DAYS: i64 = 5;
const MAX_CYCLES: usize = 6;
const RISE_C: f64 = 0.2;
/// A logged start more than this many days ago gives no estimate.
const STALE_DAYS: i64 = 60;

/// The first day of the temperature rise in `from..=to`, by the three-over-six
/// rule. `temps` is `(day, temperature deviation in °C)`.
fn temperature_rise(temps: &[(i64, f64)], from: i64, to: i64) -> Option<i64> {
    let at = |day: i64| temps.iter().find(|t| t.0 == day).map(|t| t.1);
    (from + 6..=to - 2).find(|&day| {
        let before: Vec<f64> = (day - 6..day).filter_map(at).collect();
        if before.len() < 5 {
            return false;
        }
        let high = before.iter().cloned().fold(f64::MIN, f64::max);
        (day..day + 3).all(|d| at(d).is_some_and(|t| t >= high + RISE_C))
    })
}

/// The cycle state on `today`. `period_starts` holds the first day of each logged
/// period as a day index, in any order. `None` with no start in the last 60 days.
pub fn cycle_estimate(period_starts: &[i64], today: i64, temps: &[(i64, f64)]) -> Option<CycleEstimate> {
    let mut starts: Vec<i64> = period_starts.iter().copied().filter(|d| *d <= today).collect();
    starts.sort_unstable();
    starts.dedup();
    let last = *starts.last()?;
    if today - last > STALE_DAYS {
        return None;
    }
    let lengths: Vec<f64> = starts
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64)
        .filter(|len| (21.0..=45.0).contains(len))
        .collect();
    let recent = &lengths[lengths.len().saturating_sub(MAX_CYCLES)..];
    let mean_cycle_days = if recent.is_empty() {
        DEFAULT_CYCLE_DAYS
    } else {
        recent.iter().sum::<f64>() / recent.len() as f64
    };
    let calendar_ovulation = last + mean_cycle_days.round() as i64 - LUTEAL_DAYS;
    let rise = temperature_rise(temps, last, today);
    // the rise starts the day after ovulation
    let ovulation_day = rise.map_or(calendar_ovulation, |d| d - 1);
    let next_period_day = match rise {
        Some(_) => ovulation_day + LUTEAL_DAYS,
        None => last + mean_cycle_days.round() as i64,
    };
    let (fertile_start_day, fertile_end_day) = (ovulation_day - 5, ovulation_day + 1);
    let cycle_day = today - last + 1;
    let phase = if today > next_period_day + 2 {
        "late"
    } else if cycle_day <= PERIOD_DAYS {
        "menstrual"
    } else if today < fertile_start_day {
        "follicular"
    } else if today <= fertile_end_day {
        "fertile"
    } else {
        "luteal"
    };
    Some(CycleEstimate {
        cycle_day,
        phase,
        mean_cycle_days: (mean_cycle_days * 10.0).round() / 10.0,
        cycles_used: recent.len(),
        last_period_day: last,
        next_period_day,
        ovulation_day,
        ovulation_confirmed: rise.is_some(),
        fertile_start_day,
        fertile_end_day,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_estimate_uses_the_mean_cycle_length() {
        // cycles of 30 and 32 days; today is day 10 of the third cycle
        let e = cycle_estimate(&[100, 130, 162], 171, &[]).unwrap();
        assert_eq!(e.cycle_day, 10);
        assert_eq!(e.mean_cycle_days, 31.0);
        assert_eq!(e.cycles_used, 2);
        assert_eq!(e.next_period_day, 193);
        assert_eq!(e.ovulation_day, 179);
        assert_eq!((e.fertile_start_day, e.fertile_end_day), (174, 180));
        assert_eq!(e.phase, "follicular");
        assert!(!e.ovulation_confirmed);
    }

    #[test]
    fn one_logged_period_uses_28_days() {
        let e = cycle_estimate(&[200], 202, &[]).unwrap();
        assert_eq!((e.phase, e.mean_cycle_days, e.cycles_used), ("menstrual", 28.0, 0));
        assert_eq!(e.next_period_day, 228);
        assert!(cycle_estimate(&[200], 270, &[]).is_none());
        assert!(cycle_estimate(&[], 270, &[]).is_none());
    }

    #[test]
    fn a_temperature_rise_confirms_ovulation() {
        // low phase to day 215, high phase from day 216
        let temps: Vec<(i64, f64)> = (200..222)
            .map(|d| (d, if d >= 216 { 0.35 } else { -0.05 }))
            .collect();
        let e = cycle_estimate(&[200], 221, &temps).unwrap();
        assert!(e.ovulation_confirmed);
        assert_eq!(e.ovulation_day, 215);
        assert_eq!(e.next_period_day, 229);
        assert_eq!(e.phase, "luteal");
    }
}
