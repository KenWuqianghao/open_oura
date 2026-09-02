//! Beat-level signal utilities (not an ecore port): turn the ring's IBI records
//! into timed beats and fixed-window statistics.
//!
//! The ring stores heart beats as inter-beat intervals in `ibi_and_amplitude`
//! (0x60, night) and `green_ibi_quality` (0x80, day). A record carries a handful of
//! consecutive intervals and one timestamp; the beats inside it are placed by
//! cumulative IBI from that timestamp. Windowed means/SDNN/RMSSD over those beats
//! are what a health store wants (1-minute heart rate, 5-minute HRV).

use crate::ported::hrv::{rmssd, sdnn};

/// One heart beat: its wall time (seconds, fractional) and the interval that
/// ended on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    pub t_s: f64,
    pub ibi_ms: u16,
}

/// Expand one IBI record into timed beats. `t0_s` is the record's time and is
/// taken as the time of the first beat; each following beat lands one interval
/// later. `good(i)` gates the i-th interval (quality flag); a bad interval still
/// advances time but yields no beat.
pub fn beats_from_record(t0_s: f64, ibi_ms: &[u16], good: impl Fn(usize) -> bool) -> Vec<Beat> {
    let mut out = Vec::with_capacity(ibi_ms.len());
    let mut t = t0_s;
    for (i, &ibi) in ibi_ms.iter().enumerate() {
        if i > 0 {
            t += ibi as f64 / 1000.0;
        }
        if good(i) && (300..=2000).contains(&ibi) {
            out.push(Beat { t_s: t, ibi_ms: ibi });
        }
    }
    out
}

/// Statistics over one fixed window of beats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowStat {
    /// Window start (seconds), aligned to a multiple of the window length.
    pub start_s: i64,
    pub n: usize,
    pub mean_bpm: f64,
    pub rmssd_ms: Option<f64>,
    pub sdnn_ms: Option<f64>,
}

/// Bucket `beats` (any order) into `window_s`-second windows aligned at
/// `floor(t / window_s)`, and report every window with at least `min_beats`.
/// Mean bpm is the mean of the per-beat instantaneous rates (60000 / IBI).
pub fn window_stats(beats: &[Beat], window_s: i64, min_beats: usize) -> Vec<WindowStat> {
    if window_s <= 0 || beats.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<&Beat> = beats.iter().collect();
    sorted.sort_by(|a, b| a.t_s.partial_cmp(&b.t_s).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::new();
    let mut current: Option<i64> = None;
    let mut ibis: Vec<u16> = Vec::new();

    let flush = |start: i64, ibis: &mut Vec<u16>, out: &mut Vec<WindowStat>| {
        if ibis.len() >= min_beats.max(1) {
            let mean_bpm =
                ibis.iter().map(|&i| 60_000.0 / i as f64).sum::<f64>() / ibis.len() as f64;
            out.push(WindowStat {
                start_s: start,
                n: ibis.len(),
                mean_bpm,
                rmssd_ms: rmssd(ibis),
                sdnn_ms: sdnn(ibis),
            });
        }
        ibis.clear();
    };

    for b in sorted {
        let start = (b.t_s / window_s as f64).floor() as i64 * window_s;
        if current != Some(start) {
            if let Some(prev) = current {
                flush(prev, &mut ibis, &mut out);
            }
            current = Some(start);
        }
        ibis.push(b.ibi_ms);
    }
    if let Some(prev) = current {
        flush(prev, &mut ibis, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_expands_by_cumulative_ibi_and_quality() {
        let beats = beats_from_record(100.0, &[1000, 1000, 500, 5000], |i| i != 2);
        // beat 0 at 100.0, beat 1 at 101.0, beat 2 gated out (time still advances
        // by 0.5 s), beat 3 implausible (5000 ms) dropped.
        assert_eq!(beats.len(), 2);
        assert_eq!(beats[0], Beat { t_s: 100.0, ibi_ms: 1000 });
        assert_eq!(beats[1], Beat { t_s: 101.0, ibi_ms: 1000 });
    }

    #[test]
    fn windows_align_and_gate() {
        // 60 bpm beats from t=59.5 for 120 s: windows 0, 60, 120 (last has 1 beat).
        let beats: Vec<Beat> = (0..121)
            .map(|i| Beat { t_s: 59.5 + i as f64, ibi_ms: 1000 })
            .collect();
        let stats = window_stats(&beats, 60, 3);
        assert_eq!(stats.len(), 2, "{stats:?}");
        assert_eq!(stats[0].start_s, 60);
        assert_eq!(stats[0].n, 60);
        assert!((stats[0].mean_bpm - 60.0).abs() < 1e-9);
        assert_eq!(stats[1].start_s, 120);
        // constant IBI → zero variability
        assert_eq!(stats[0].sdnn_ms, Some(0.0));
        assert_eq!(stats[0].rmssd_ms, Some(0.0));
        // the 1-beat window at 180 is below min_beats
        assert!(stats.iter().all(|s| s.start_s != 180));
    }

    #[test]
    fn five_minute_sdnn_matches_textbook() {
        let beats: Vec<Beat> = [800u16, 820, 810, 830]
            .iter()
            .enumerate()
            .map(|(i, &ibi)| Beat { t_s: 300.0 + i as f64, ibi_ms: ibi })
            .collect();
        let stats = window_stats(&beats, 300, 4);
        assert_eq!(stats.len(), 1);
        assert!((stats[0].sdnn_ms.unwrap() - 12.9099).abs() < 1e-3);
        assert!((stats[0].rmssd_ms.unwrap() - 17.3205).abs() < 1e-3);
    }
}
