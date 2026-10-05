//! NightSignal: the overnight resting-heart-rate alert of Mishra et al. 2022
//! ("Real-time alerting system for COVID-19 and other stress events using
//! wearable data", Nat Med 28:164). Ported from the reference code
//! (github.com/StanfordBioinformatics/wearable-infection, `nightsignal.py`,
//! Apache-2.0; changes: Rust, values in and out, no files). See
//! `THIRD_PARTY_NOTICES.md`.
//!
//! The steps, as in the reference:
//! 1. The resting heart rate of a date is the integer mean of the integer
//!    heart-rate samples from 00:00 to 06:59 local time while at rest. The
//!    reference uses zero-step minutes; the ring gives heart rate during sleep.
//! 2. A single missing date between two dates with data gets their integer mean.
//! 3. The baseline of a date is the integer median of all dates up to and
//!    including it.
//! 4. A date is a red candidate at baseline + 4 bpm or more, a yellow candidate at
//!    baseline + 3 bpm or more.
//! 5. A date is red when it and the date before are red candidates, yellow when
//!    they are yellow candidates and the date is not red. All other dates are green.
//!
//! An alert means a sustained rise of resting heart rate. Infection, stress,
//! alcohol, travel and hard training can all cause one. It is not a diagnosis.

use serde::Serialize;

const YELLOW_BPM: i64 = 3;
const RED_BPM: i64 = 4;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Alert {
    Green,
    Yellow,
    Red,
}

/// One date of the NightSignal state machine.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DaySignal {
    /// Days since 1970-01-01 (local).
    pub day: i64,
    /// Resting heart rate of the date, bpm (integer, as in the reference).
    pub rhr: i64,
    /// Median of `rhr` over this and all earlier dates.
    pub baseline: i64,
    /// True when the date had no samples and got the mean of its neighbours.
    pub imputed: bool,
    pub alert: Alert,
}

/// Run NightSignal over the resting heart-rate samples of each date. Input: (local
/// day number, the bpm samples of that date from 00:00 to 06:59). Dates without
/// samples are skipped. Output is sorted by day.
pub fn nightsignal(days: &[(i64, Vec<f64>)]) -> Vec<DaySignal> {
    let mut avg: Vec<(i64, i64, bool)> = days
        .iter()
        .filter(|(_, hr)| !hr.is_empty())
        .map(|(day, hr)| {
            let sum: i64 = hr.iter().map(|h| h.trunc() as i64).sum();
            (*day, sum / hr.len() as i64, false)
        })
        .collect();
    avg.sort_by_key(|d| d.0);
    avg.dedup_by_key(|d| d.0);

    // one-date gaps: (today − prev = 2, next − today = 1) fills today − 1,
    // (today − prev = 1, next − today = 2) fills today + 1
    let mut filled = Vec::new();
    for i in 1..avg.len().saturating_sub(1) {
        let (prev, today, next) = (avg[i - 1], avg[i], avg[i + 1]);
        if next.0 - today.0 == 1 && today.0 - prev.0 == 2 {
            filled.push((today.0 - 1, (today.1 + prev.1) / 2, true));
        }
        if next.0 - today.0 == 2 && today.0 - prev.0 == 1 {
            filled.push((today.0 + 1, (today.1 + next.1) / 2, true));
        }
    }
    for f in filled {
        if !avg.iter().any(|d| d.0 == f.0) {
            avg.push(f);
        }
    }
    avg.sort_by_key(|d| d.0);

    let mut seen: Vec<i64> = Vec::with_capacity(avg.len());
    let mut out: Vec<DaySignal> = avg
        .iter()
        .map(|&(day, rhr, imputed)| {
            seen.push(rhr);
            DaySignal { day, rhr, baseline: int_median(&seen), imputed, alert: Alert::Green }
        })
        .collect();
    let red: Vec<bool> = out.iter().map(|d| d.rhr >= d.baseline + RED_BPM).collect();
    let yellow: Vec<bool> = out.iter().map(|d| d.rhr >= d.baseline + YELLOW_BPM).collect();
    for i in 1..out.len() {
        let consecutive = out[i].day - out[i - 1].day == 1;
        if consecutive && red[i] && red[i - 1] {
            out[i].alert = Alert::Red;
        } else if consecutive && yellow[i] && yellow[i - 1] {
            out[i].alert = Alert::Yellow;
        }
    }
    out
}

/// `int(statistics.median(values))`: the mean of the two middle values for an
/// even count, truncated.
fn int_median(values: &[i64]) -> i64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        ((v[n / 2 - 1] + v[n / 2]) as f64 / 2.0).trunc() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(rhr: &[(i64, f64)]) -> Vec<(i64, Vec<f64>)> {
        rhr.iter().map(|&(d, h)| (d, vec![h])).collect()
    }

    #[test]
    fn two_raised_nights_give_an_alert() {
        let mut input: Vec<(i64, f64)> = (0..10).map(|d| (d, 55.0)).collect();
        input.push((10, 59.0)); // +4: red candidate, alone
        input.push((11, 55.0));
        input.push((12, 58.0)); // +3, +3: yellow on the second night
        input.push((13, 58.0));
        input.push((14, 60.0)); // +5, +5: red on the second night
        input.push((15, 60.0));
        let out = nightsignal(&days(&input));
        let alert = |d: i64| out.iter().find(|s| s.day == d).unwrap().alert;
        assert_eq!(alert(10), Alert::Green);
        assert_eq!(alert(12), Alert::Green);
        assert_eq!(alert(13), Alert::Yellow);
        assert_eq!(alert(14), Alert::Yellow); // 13 and 14 are yellow candidates
        assert_eq!(alert(15), Alert::Red);
        assert_eq!(out[15].baseline, 55);
    }

    #[test]
    fn samples_truncate_and_one_day_gaps_fill() {
        let input = vec![(0, vec![55.9, 56.2, 57.0]), (2, vec![61.0]), (3, vec![60.0])];
        let out = nightsignal(&input);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].rhr, 56); // (55 + 56 + 57) / 3
        assert_eq!((out[1].day, out[1].rhr, out[1].imputed), (1, 58, true)); // (56 + 61) / 2
        assert_eq!(out[3].baseline, 59); // median(56, 58, 61, 60) = 59
    }

    #[test]
    fn a_gap_breaks_the_consecutive_rule() {
        let input: Vec<(i64, f64)> = vec![(0, 50.0), (1, 50.0), (2, 50.0), (3, 60.0), (6, 60.0)];
        let out = nightsignal(&days(&input));
        assert!(out.iter().all(|d| d.alert == Alert::Green));
    }
}
