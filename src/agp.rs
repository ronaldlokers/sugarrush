//! Ambulatory Glucose Profile: fold several days of readings onto a single
//! 24-hour clock and summarise each time-of-day bucket as a percentile
//! envelope (the standard AGP median + 25/75 + 5/95 bands).

use chrono::{Local, TimeZone, Timelike};

use crate::nightscout::Entry;

/// Number of time-of-day buckets — 15-minute resolution over 24h.
pub const BUCKETS: usize = 96;
/// Minutes per bucket.
pub const BUCKET_MIN: i64 = 24 * 60 / BUCKETS as i64;

/// Percentile envelope for one time-of-day bucket, in mg/dL.
#[derive(Debug, Clone, Copy)]
pub struct Band {
    /// Minutes since local midnight at the bucket centre.
    pub minute: i64,
    pub p05: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}

/// Group `entries` by local time-of-day and compute the percentile envelope for
/// each populated bucket, ordered by time of day. Empty buckets are skipped.
pub fn profile(entries: &[Entry]) -> Vec<Band> {
    let mut buckets: Vec<Vec<f64>> = vec![Vec::new(); BUCKETS];
    for e in entries {
        if let Some(dt) = Local.timestamp_millis_opt(e.date).single() {
            let minute = dt.hour() as i64 * 60 + dt.minute() as i64;
            let idx = (minute / BUCKET_MIN).clamp(0, BUCKETS as i64 - 1) as usize;
            buckets[idx].push(e.sgv);
        }
    }
    let mut out = Vec::new();
    for (i, vals) in buckets.into_iter().enumerate() {
        if vals.is_empty() {
            continue;
        }
        let mut v = vals;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        out.push(Band {
            minute: i as i64 * BUCKET_MIN + BUCKET_MIN / 2,
            p05: percentile(&v, 0.05),
            p25: percentile(&v, 0.25),
            p50: percentile(&v, 0.50),
            p75: percentile(&v, 0.75),
            p95: percentile(&v, 0.95),
        });
    }
    out
}

/// Linear-interpolated percentile of a sorted slice; `q` in `[0, 1]`.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        n => {
            let rank = q * (n - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
        }
    }
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

    /// Local midnight + `minutes`, on a fixed reference day, as epoch ms.
    fn at(day: i64, minutes: i64) -> i64 {
        // 2026-01-01 00:00 local + day days + minutes.
        let base = Local
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        base + (day * 24 * 60 + minutes) * 60_000
    }

    #[test]
    fn empty_profile_for_no_entries() {
        assert!(profile(&[]).is_empty());
    }

    #[test]
    fn folds_days_onto_one_clock() {
        // Same time of day across three days lands in one bucket.
        let e = [
            entry(100.0, at(0, 480)), // 08:00 day 0
            entry(120.0, at(1, 480)), // 08:00 day 1
            entry(140.0, at(2, 480)), // 08:00 day 2
        ];
        let bands = profile(&e);
        assert_eq!(bands.len(), 1);
        let b = bands[0];
        // Bucket centre for the 08:00 slot.
        assert_eq!(b.minute, 480 + BUCKET_MIN / 2);
        assert_eq!(b.p50, 120.0); // median of 100/120/140
        assert!(b.p05 <= b.p25 && b.p25 <= b.p50 && b.p50 <= b.p75 && b.p75 <= b.p95);
    }

    #[test]
    fn separate_times_make_separate_bands() {
        let e = [entry(90.0, at(0, 60)), entry(200.0, at(0, 720))];
        let bands = profile(&e);
        assert_eq!(bands.len(), 2);
        // Ordered by time of day.
        assert!(bands[0].minute < bands[1].minute);
    }

    #[test]
    fn percentile_interpolates() {
        let v = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&v, 0.0), 10.0);
        assert_eq!(percentile(&v, 1.0), 40.0);
        assert_eq!(percentile(&v, 0.5), 25.0); // midpoint of 20 and 30
    }
}
