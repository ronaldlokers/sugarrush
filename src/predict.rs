//! Short-term glucose forecasting.
//!
//! Predictions come from the uploader when available (Loop / OpenAPS, fetched
//! in [`crate::nightscout`]). When none are published, [`ar2`] computes a
//! simple AR2-style projection from the two most recent readings — the same
//! model Nightscout uses for its short-term forecast.

use crate::alert;
use crate::config::Alerts;
use crate::nightscout::{Entry, Prediction};
use crate::units::Units;

const BG_REF: f64 = 140.0;
/// AR2 autoregression coefficients (Nightscout's ar2 plugin).
const AR: [f64; 2] = [-0.723, 1.716];
const STEP_MS: i64 = 5 * 60_000;
/// Accepted spacing between the two readings the projection is built from.
/// CGMs post every 5 minutes; a little jitter is normal, a gap is not.
const MIN_STEP_MS: i64 = 3 * 60_000;
const MAX_STEP_MS: i64 = 8 * 60_000;
/// Forecast horizon: 6 × 5 min = 30 minutes.
const STEPS: usize = 6;
/// How far ahead [`ar2`] projects, in minutes. Uploader forecasts (Loop /
/// OpenAPS) may reach further; this is the local fallback's ceiling.
pub const HORIZON_MINUTES: i64 = STEPS as i64 * STEP_MS / 60_000;
const BG_MIN: f64 = 36.0;
const BG_MAX: f64 = 400.0;

/// Uncertainty half-width (mg/dL) added per 5-min step, so the AR2 projection
/// fans into a cone the further out it reaches.
const SPREAD_PER_STEP: f64 = 4.0;

/// Where the reading is heading: the midpoint of the furthest projected step,
/// in display units, with the band it lands in.
///
/// The midpoint rather than the whole cone, and the last step rather than the
/// nearest: a bar or a panel has room for one number, and the end of the
/// horizon is the part that changes what someone does next.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Outlook {
    pub in_min: i64,
    pub value: f64,
    /// The band that value lands in, so it can be coloured like the reading.
    pub class: &'static str,
    /// The band as a value rather than a name, for callers that colour or
    /// compare rather than print.
    #[serde(skip)]
    pub alert: alert::Alert,
}

/// Project [`ar2`] to its horizon and describe where it lands.
pub fn outlook(recent: &[Entry], alerts: &Alerts, units: Units) -> Option<Outlook> {
    let steps = ar2(recent);
    let last = steps.last()?;
    let midpoint = (last.low + last.high) / 2.0;
    let value = units.from_mgdl(midpoint);
    Some(Outlook {
        in_min: HORIZON_MINUTES,
        value: match units {
            Units::Mmol => (value * 10.0).round() / 10.0,
            Units::Mgdl => value.round(),
        },
        class: alert::from_value(midpoint, alerts).class(),
        alert: alert::from_value(midpoint, alerts),
    })
}

/// Project the next 30 minutes from the latest two readings as a widening
/// low–high band, or empty if there isn't enough data.
pub fn ar2(entries: &[Entry]) -> Vec<Prediction> {
    let (latest, prev) = match (entries.first(), entries.get(1)) {
        (Some(a), Some(b)) => (a, b),
        _ => return Vec::new(),
    };

    // The model assumes consecutive 5-minute samples. It never read the actual
    // gap, so a reading either side of a sensor dropout was extrapolated as if
    // it had happened in five minutes: a benign −0.5 mg/dL per minute across
    // 40 minutes projected below the low threshold and fired "heading low".
    // Sensor gaps are exactly when that fired, and exactly when a spurious
    // prediction is least welcome. No forecast beats a fabricated one.
    let gap = latest.date - prev.date;
    if !(MIN_STEP_MS..=MAX_STEP_MS).contains(&gap) {
        return Vec::new();
    }

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

    #[test]
    fn a_sensor_gap_produces_no_forecast_rather_than_a_wrong_one() {
        let now = 1_700_000_000_000;
        // Same two values, five minutes apart: a normal projection.
        let close = ar2(&[entry(90.0, now), entry(110.0, now - 5 * STEP_MS / 5)]);
        assert_eq!(close.len(), STEPS);

        // The same drop across a 40-minute gap is ~0.5 mg/dL per minute — flat.
        // Extrapolating it as a 5-minute step used to project 61 mg/dL at
        // +30 min and fire a false "heading low".
        let gapped = ar2(&[entry(90.0, now), entry(110.0, now - 40 * 60_000)]);
        assert!(gapped.is_empty(), "forecast built across a 40-minute gap");

        // Two readings in the same minute (duplicate uploaders) are no basis
        // for a trend either.
        assert!(ar2(&[entry(90.0, now), entry(110.0, now - 30_000)]).is_empty());

        // Ordinary jitter still forecasts.
        assert_eq!(
            ar2(&[entry(90.0, now), entry(95.0, now - 6 * 60_000)]).len(),
            STEPS
        );
    }
}
