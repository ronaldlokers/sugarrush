//! Summary statistics over a set of readings: time-in-range, mean, GMI, and
//! glycaemic variability.

use crate::alert::Alert;
use crate::config::Alerts;
use crate::nightscout::Entry;

/// Share of readings in each clinical band, as percentages summing to 100.
///
/// Five bands rather than three: the international consensus on time in range
/// reports "very low" and "very high" separately, because 20% high and 20%
/// *very* high are different clinical pictures. The band edges follow the
/// configured thresholds, so what the stats panel counts as urgent is what the
/// alarm treats as urgent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tir {
    pub very_low: f64,
    pub low: f64,
    pub in_range: f64,
    pub high: f64,
    pub very_high: f64,
}

impl Tir {
    /// Everything below the target range (very low + low).
    pub fn below(&self) -> f64 {
        self.very_low + self.low
    }
}

/// Time-in-range over `entries`. Bounds are mg/dL and match [`crate::alert`]'s
/// classification: at or below `urgent_low` is very low, below `low` is low,
/// `[low, high]` is in range, above `high` is high, at or above `urgent_high`
/// is very high. `None` when there are no readings.
pub fn tir(
    entries: &[Entry],
    urgent_low: f64,
    low: f64,
    high: f64,
    urgent_high: f64,
) -> Option<Tir> {
    if entries.is_empty() {
        return None;
    }
    // Classify through `alert::from_value` rather than repeating its ladder.
    // The ordering is not obvious — `urgent_high` is checked before `high` —
    // and this test file's own comment claimed the bands "land where
    // alert::from_value puts it" while nothing enforced it. Changing a
    // boundary in one place used to desync the stats panel from the alarm.
    let alerts = Alerts {
        urgent_low,
        low,
        high,
        urgent_high,
        ..Alerts::default()
    };
    let total = entries.len() as f64;
    let (mut vlo, mut lo, mut hi, mut vhi) = (0.0, 0.0, 0.0, 0.0);
    for e in entries {
        match crate::alert::from_value(e.sgv, &alerts) {
            Alert::UrgentLow => vlo += 1.0,
            Alert::Low => lo += 1.0,
            Alert::High => hi += 1.0,
            Alert::UrgentHigh => vhi += 1.0,
            Alert::InRange | Alert::Stale => {}
        }
    }
    let pct = |n: f64| n / total * 100.0;
    let (very_low, low_pct, high_pct, very_high) = (pct(vlo), pct(lo), pct(hi), pct(vhi));
    Some(Tir {
        very_low,
        low: low_pct,
        high: high_pct,
        very_high,
        in_range: 100.0 - very_low - low_pct - high_pct - very_high,
    })
}

/// Mean sensor glucose in mg/dL, or `None` when there are no readings.
pub fn mean_mgdl(entries: &[Entry]) -> Option<f64> {
    if entries.is_empty() {
        return None;
    }
    let sum: f64 = entries.iter().map(|e| e.sgv).sum();
    Some(sum / entries.len() as f64)
}

/// Coefficient of variation (%) — the standard deviation as a share of the
/// mean, the standard measure of glycaemic variability. Consensus target is
/// ≤ 36%; above that, hypos become much more likely at the same average.
/// `None` with fewer than two readings (no sample variance to speak of).
pub fn cv_pct(entries: &[Entry]) -> Option<f64> {
    if entries.len() < 2 {
        return None;
    }
    let mean = mean_mgdl(entries)?;
    if mean <= 0.0 {
        return None;
    }
    let n = entries.len() as f64;
    // Sample standard deviation (n-1): these readings are a sample of the
    // period, not the whole population of possible readings.
    let var = entries.iter().map(|e| (e.sgv - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt() / mean * 100.0)
}

/// Glucose Management Indicator (estimated A1c, %) from mean mg/dL.
/// GMI(%) = 3.31 + 0.02392 × mean. (Bergenstal et al. 2018.)
pub fn gmi(mean_mgdl: f64) -> f64 {
    3.31 + 0.02392 * mean_mgdl
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five TIR bands and the alarm must classify a value identically —
    /// they are the same clinical thresholds, and a user who sees "URGENT LOW"
    /// on the headline while the stats panel counts that reading as merely
    /// "low" has been shown two different answers to one question.
    ///
    /// Boundary-exact, because that is where the two used to be able to drift:
    /// the ladder is inclusive at `urgent_low`/`urgent_high` and exclusive at
    /// `low`/`high`.
    #[test]
    fn bands_agree_with_the_alarm_at_every_boundary() {
        let (ul, lo, hi, uh) = (55.0, 70.0, 180.0, 250.0);
        for step in 0..=600 {
            let sgv = step as f64;
            let t = tir(&[e(sgv)], ul, lo, hi, uh).unwrap();
            let band = if t.very_low > 0.0 {
                Alert::UrgentLow
            } else if t.low > 0.0 {
                Alert::Low
            } else if t.high > 0.0 {
                Alert::High
            } else if t.very_high > 0.0 {
                Alert::UrgentHigh
            } else {
                Alert::InRange
            };
            let alarm = crate::alert::from_value(
                sgv,
                &Alerts {
                    urgent_low: ul,
                    low: lo,
                    high: hi,
                    urgent_high: uh,
                    ..Alerts::default()
                },
            );
            assert_eq!(band, alarm, "band disagrees with the alarm at {sgv} mg/dL");
        }
    }

    fn e(sgv: f64) -> Entry {
        Entry {
            sgv,
            date: 0,
            direction: None,
        }
    }

    /// Defaults: 55 / 70 / 180 / 250.
    fn tir_default(entries: &[Entry]) -> Option<Tir> {
        tir(entries, 55.0, 70.0, 180.0, 250.0)
    }

    #[test]
    fn tir_none_when_empty() {
        assert!(tir_default(&[]).is_none());
    }

    #[test]
    fn tir_splits_five_bands() {
        // 1 very low, 1 low, 2 in range, 1 high, 1 very high = 6 readings.
        let entries = [e(45.0), e(60.0), e(100.0), e(120.0), e(200.0), e(300.0)];
        let t = tir_default(&entries).unwrap();
        assert!((t.very_low - 100.0 / 6.0).abs() < 1e-9);
        assert!((t.low - 100.0 / 6.0).abs() < 1e-9);
        assert!((t.in_range - 200.0 / 6.0).abs() < 1e-9);
        assert!((t.high - 100.0 / 6.0).abs() < 1e-9);
        assert!((t.very_high - 100.0 / 6.0).abs() < 1e-9);
        assert!((t.below() - 200.0 / 6.0).abs() < 1e-9);
        let total = t.very_low + t.low + t.in_range + t.high + t.very_high;
        assert!((total - 100.0).abs() < 1e-9);
    }

    #[test]
    fn tir_bands_match_the_alert_thresholds() {
        // Exactly on a threshold lands where alert::from_value puts it.
        assert_eq!(tir_default(&[e(55.0)]).unwrap().very_low, 100.0);
        assert_eq!(tir_default(&[e(69.0)]).unwrap().low, 100.0);
        assert_eq!(tir_default(&[e(70.0)]).unwrap().in_range, 100.0);
        assert_eq!(tir_default(&[e(180.0)]).unwrap().in_range, 100.0);
        assert_eq!(tir_default(&[e(181.0)]).unwrap().high, 100.0);
        assert_eq!(tir_default(&[e(250.0)]).unwrap().very_high, 100.0);
    }

    #[test]
    fn mean_and_gmi() {
        assert_eq!(mean_mgdl(&[e(100.0), e(200.0)]).unwrap(), 150.0);
        // GMI at mean 150 ≈ 6.9%.
        assert!((gmi(150.0) - 6.898).abs() < 0.01);
    }

    #[test]
    fn cv_needs_two_readings_and_scales_with_spread() {
        assert!(cv_pct(&[]).is_none());
        assert!(cv_pct(&[e(100.0)]).is_none());
        // Perfectly flat glucose has no variability.
        assert_eq!(cv_pct(&[e(100.0), e(100.0)]).unwrap(), 0.0);
        // 90/110 around a mean of 100: sample SD is 14.14 → CV 14.1%.
        let cv = cv_pct(&[e(90.0), e(110.0)]).unwrap();
        assert!((cv - 14.14).abs() < 0.01, "cv was {cv}");
        // Wider spread, same mean → higher CV.
        assert!(cv_pct(&[e(50.0), e(150.0)]).unwrap() > cv);
    }
}
