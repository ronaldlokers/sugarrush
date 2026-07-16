//! Summary statistics over a set of readings: time-in-range, mean, and GMI.

use crate::nightscout::Entry;

/// Fraction of readings below / within / above range, as percentages summing
/// to 100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tir {
    pub low: f64,
    pub in_range: f64,
    pub high: f64,
}

/// Time-in-range over `entries`, using mg/dL bounds `[low, high]` inclusive as
/// the in-range band. `None` when there are no readings.
pub fn tir(entries: &[Entry], low: f64, high: f64) -> Option<Tir> {
    if entries.is_empty() {
        return None;
    }
    let total = entries.len() as f64;
    let (mut lo, mut hi) = (0.0, 0.0);
    for e in entries {
        if e.sgv < low {
            lo += 1.0;
        } else if e.sgv > high {
            hi += 1.0;
        }
    }
    let low_pct = lo / total * 100.0;
    let high_pct = hi / total * 100.0;
    Some(Tir {
        low: low_pct,
        high: high_pct,
        in_range: 100.0 - low_pct - high_pct,
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

/// Glucose Management Indicator (estimated A1c, %) from mean mg/dL.
/// GMI(%) = 3.31 + 0.02392 × mean. (Bergenstal et al. 2018.)
pub fn gmi(mean_mgdl: f64) -> f64 {
    3.31 + 0.02392 * mean_mgdl
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(sgv: f64) -> Entry {
        Entry {
            sgv,
            date: 0,
            direction: None,
        }
    }

    #[test]
    fn tir_none_when_empty() {
        assert!(tir(&[], 70.0, 180.0).is_none());
    }

    #[test]
    fn tir_splits_correctly() {
        // 1 low, 2 in range, 1 high = 25 / 50 / 25.
        let entries = [e(60.0), e(100.0), e(120.0), e(200.0)];
        let t = tir(&entries, 70.0, 180.0).unwrap();
        assert_eq!(t.low, 25.0);
        assert_eq!(t.in_range, 50.0);
        assert_eq!(t.high, 25.0);
        assert!((t.low + t.in_range + t.high - 100.0).abs() < 1e-9);
    }

    #[test]
    fn boundaries_are_in_range() {
        // Exactly on the bounds counts as in range (inclusive band).
        let entries = [e(70.0), e(180.0)];
        let t = tir(&entries, 70.0, 180.0).unwrap();
        assert_eq!(t.in_range, 100.0);
    }

    #[test]
    fn mean_and_gmi() {
        assert_eq!(mean_mgdl(&[e(100.0), e(200.0)]).unwrap(), 150.0);
        // GMI at mean 150 ≈ 6.9%.
        assert!((gmi(150.0) - 6.898).abs() < 0.01);
    }
}
