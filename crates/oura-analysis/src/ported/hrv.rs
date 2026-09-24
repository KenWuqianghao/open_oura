//! Heart-rate variability. Ported from ecore `hrv @ 0x1e7984`: RMSSD of the
//! inter-beat-interval series — the textbook `sqrt(mean(diff(ibi)^2))`. The ring
//! also reports a 5-min average RMSSD directly in `hrv_event`; this lets us
//! compute RMSSD over any IBI window we decode.
//! See docs/algorithms/hrv.md.

/// RMSSD (ms) over a sequence of inter-beat intervals (ms). Needs >= 2 intervals.
pub fn rmssd(ibi_ms: &[u16]) -> Option<f64> {
    if ibi_ms.len() < 2 {
        return None;
    }
    let mut sum_sq = 0f64;
    for w in ibi_ms.windows(2) {
        let d = w[1] as f64 - w[0] as f64;
        sum_sq += d * d;
    }
    Some((sum_sq / (ibi_ms.len() - 1) as f64).sqrt())
}

/// SDNN (ms): the sample standard deviation (n − 1) of the inter-beat intervals.
/// This is the quantity HealthKit's `heartRateVariabilitySDNN` type expects — it
/// is NOT RMSSD, which the ring reports in `hrv_event`. Needs >= 2 intervals.
pub fn sdnn(ibi_ms: &[u16]) -> Option<f64> {
    if ibi_ms.len() < 2 {
        return None;
    }
    let n = ibi_ms.len() as f64;
    let mean = ibi_ms.iter().map(|&v| v as f64).sum::<f64>() / n;
    let var = ibi_ms
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0);
    Some(var.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdnn_textbook() {
        // mean 815; deviations -15,5,-5,15 -> squares 225,25,25,225 = 500/3 -> 12.91
        let v = sdnn(&[800, 820, 810, 830]).unwrap();
        assert!((v - 12.9099).abs() < 1e-3, "{v}");
        assert!(sdnn(&[800]).is_none());
    }

    #[test]
    fn rmssd_textbook() {
        // diffs 20,-10,20 -> squares 400,100,400 = 900/3 = 300 -> ~17.32
        let v = rmssd(&[800, 820, 810, 830]).unwrap();
        assert!((v - 17.3205).abs() < 1e-3, "{v}");
    }
}
