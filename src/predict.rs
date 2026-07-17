//! Short-term glucose forecasting.
//!
//! Predictions come from the uploader when available (Loop / OpenAPS, fetched
//! in [`crate::nightscout`]). When none are published, [`ar2`] computes a
//! simple AR2-style projection from the two most recent readings — the same
//! model Nightscout uses for its short-term forecast.

use crate::nightscout::{Entry, Prediction};

const BG_REF: f64 = 140.0;
/// AR2 autoregression coefficients (Nightscout's ar2 plugin).
const AR: [f64; 2] = [-0.723, 1.716];
const STEP_MS: i64 = 5 * 60_000;
/// Forecast horizon: 6 × 5 min = 30 minutes.
const STEPS: usize = 6;
const BG_MIN: f64 = 36.0;
const BG_MAX: f64 = 400.0;

/// Uncertainty half-width (mg/dL) added per 5-min step, so the AR2 projection
/// fans into a cone the further out it reaches.
const SPREAD_PER_STEP: f64 = 4.0;

/// Project the next 30 minutes from the latest two readings as a widening
/// low–high band, or empty if there isn't enough data.
pub fn ar2(entries: &[Entry]) -> Vec<Prediction> {
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
        let center = BG_REF * y_next.exp();
        let spread = SPREAD_PER_STEP * i as f64;
        out.push(Prediction {
            at_ms: latest.date + i * STEP_MS,
            low: (center - spread).clamp(BG_MIN, BG_MAX),
            high: (center + spread).clamp(BG_MIN, BG_MAX),
        });
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
    fn projects_six_widening_bands_five_min_apart() {
        let now = 1_000_000_000_000;
        let out = ar2(&[entry(120.0, now), entry(115.0, now - STEP_MS)]);
        assert_eq!(out.len(), STEPS);
        assert_eq!(out[0].at_ms, now + STEP_MS);
        assert_eq!(out[5].at_ms, now + 6 * STEP_MS);
        // The band widens with the horizon.
        let w0 = out[0].high - out[0].low;
        let w5 = out[5].high - out[5].low;
        assert!(w5 > w0);
        // A steady value in should forecast a steady band centre.
        let flat = ar2(&[entry(100.0, now), entry(100.0, now - STEP_MS)]);
        let mid = (flat[0].low + flat[0].high) / 2.0;
        assert!((mid - 100.0).abs() < 1.0);
    }

    #[test]
    fn stays_within_physiological_clamp() {
        let now = 0;
        // A steep rise must not project past BG_MAX.
        let out = ar2(&[entry(390.0, now), entry(300.0, now - STEP_MS)]);
        assert!(out
            .iter()
            .all(|p| (BG_MIN..=BG_MAX).contains(&p.low) && (BG_MIN..=BG_MAX).contains(&p.high)));
    }
}
