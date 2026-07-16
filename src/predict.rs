//! Short-term glucose forecasting.
//!
//! Predictions come from the uploader when available (Loop / OpenAPS, fetched
//! in [`crate::nightscout`]). When none are published, [`ar2`] computes a
//! simple AR2-style projection from the two most recent readings — the same
//! model Nightscout uses for its short-term forecast.

use crate::nightscout::Entry;

const BG_REF: f64 = 140.0;
/// AR2 autoregression coefficients (Nightscout's ar2 plugin).
const AR: [f64; 2] = [-0.723, 1.716];
const STEP_MS: i64 = 5 * 60_000;
/// Forecast horizon: 6 × 5 min = 30 minutes.
const STEPS: usize = 6;
const BG_MIN: f64 = 36.0;
const BG_MAX: f64 = 400.0;

/// Project the next 30 minutes from the latest two readings. Returns
/// `(epoch_ms, mg/dL)` future points, or empty if there isn't enough data.
pub fn ar2(entries: &[Entry]) -> Vec<(i64, f64)> {
    let (latest, prev) = match (entries.first(), entries.get(1)) {
        (Some(a), Some(b)) => (a, b),
        _ => return Vec::new(),
    };

    // Log-space state: y0 = older reading, y1 = newest.
    let mut y0 = (prev.sgv / BG_REF).ln();
    let mut y1 = (latest.sgv / BG_REF).ln();

    let mut out = Vec::with_capacity(STEPS);
    for i in 1..=STEPS as i64 {
        let y_next = AR[0] * y0 + AR[1] * y1;
        y0 = y1;
        y1 = y_next;
        let bg = (BG_REF * y_next.exp()).clamp(BG_MIN, BG_MAX);
        out.push((latest.date + i * STEP_MS, bg));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sgv: f64, date: i64) -> Entry {
        Entry {
            sgv,
            date,
            direction: None,
        }
    }

    #[test]
    fn empty_without_two_readings() {
        assert!(ar2(&[]).is_empty());
        assert!(ar2(&[entry(100.0, 0)]).is_empty());
    }

    #[test]
    fn projects_six_future_points_five_min_apart() {
        let now = 1_000_000_000_000;
        let out = ar2(&[entry(120.0, now), entry(115.0, now - STEP_MS)]);
        assert_eq!(out.len(), STEPS);
        assert_eq!(out[0].0, now + STEP_MS);
        assert_eq!(out[5].0, now + 6 * STEP_MS);
        // A steady value in should forecast a steady value (AR coeffs sum ~1).
        let flat = ar2(&[entry(100.0, now), entry(100.0, now - STEP_MS)]);
        assert!((flat[0].1 - 100.0).abs() < 1.0);
    }

    #[test]
    fn stays_within_physiological_clamp() {
        let now = 0;
        // A steep rise must not project past BG_MAX.
        let out = ar2(&[entry(390.0, now), entry(300.0, now - STEP_MS)]);
        assert!(out.iter().all(|&(_, bg)| (BG_MIN..=BG_MAX).contains(&bg)));
    }
}
