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

/// A recurring time-of-day pattern worth naming.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Insight {
    pub kind: Pattern,
    /// Minutes since local midnight, inclusive start and end of the run.
    pub from_min: i64,
    pub to_min: i64,
    /// The worst value in the run, mg/dL — how low the lows got, or how high
    /// the highs.
    pub extreme: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// A quarter of readings or more sit below target at this time of day.
    Lows,
    /// The median sits above target at this time of day.
    Highs,
}

/// Shortest run worth reporting. Below this it's a blip in one bucket or two,
/// not a pattern you could act on.
const MIN_RUN_MIN: i64 = 45;

/// Name the recurring patterns in a profile.
///
/// The point of an AGP is that it shows *when* things go wrong, but reading
/// that off a percentile fan is a skill. These are the two findings that change
/// what someone does: a time of day where a quarter of readings are below
/// target (`p25 < low`), and one where the typical reading is above it
/// (`p50 > high`).
///
/// Lows use the 25th percentile rather than the median because a low that
/// happens on a quarter of days is already a pattern; waiting for the median to
/// drop below target would mean it happens on most days before it's mentioned.
pub fn insights(bands: &[Band], low: f64, high: f64) -> Vec<Insight> {
    let mut out = Vec::new();
    out.extend(runs(
        bands,
        MIN_RUN_MIN,
        Pattern::Lows,
        |b| b.p25 < low,
        |b| b.p05,
    ));
    out.extend(runs(
        bands,
        MIN_RUN_MIN,
        Pattern::Highs,
        |b| b.p50 > high,
        |b| b.p95,
    ));
    // Lows first: they're the ones that need acting on tonight.
    out.sort_by(|a, b| {
        (a.kind == Pattern::Highs)
            .cmp(&(b.kind == Pattern::Highs))
            .then((b.to_min - b.from_min).cmp(&(a.to_min - a.from_min)))
    });
    out
}

/// Contiguous runs of buckets matching `hit`, as insights.
///
/// Contiguity is in *time*, not in index: `profile` omits empty buckets, so two
/// adjacent entries in the slice can be hours apart, and treating them as one
/// run would invent a pattern across a gap with no data in it.
fn runs(
    bands: &[Band],
    min_len: i64,
    kind: Pattern,
    hit: impl Fn(&Band) -> bool,
    extreme_of: impl Fn(&Band) -> f64,
) -> Vec<Insight> {
    let mut out: Vec<Insight> = Vec::new();
    let mut run: Option<Insight> = None;
    let mut prev_minute: Option<i64> = None;

    for b in bands {
        let contiguous = prev_minute.is_none_or(|p| b.minute - p <= BUCKET_MIN);
        if hit(b) {
            match run.as_mut() {
                Some(r) if contiguous => {
                    r.to_min = b.minute;
                    r.extreme = pick_extreme(kind, r.extreme, extreme_of(b));
                }
                _ => {
                    push_if_long_enough(&mut out, run.take(), min_len);
                    run = Some(Insight {
                        kind,
                        from_min: b.minute,
                        to_min: b.minute,
                        extreme: extreme_of(b),
                    });
                }
            }
        } else {
            push_if_long_enough(&mut out, run.take(), min_len);
        }
        prev_minute = Some(b.minute);
    }
    push_if_long_enough(&mut out, run.take(), min_len);
    out
}

fn pick_extreme(kind: Pattern, a: f64, b: f64) -> f64 {
    match kind {
        Pattern::Lows => a.min(b),
        Pattern::Highs => a.max(b),
    }
}

fn push_if_long_enough(out: &mut Vec<Insight>, run: Option<Insight>, min_len: i64) {
    if let Some(r) = run {
        // A run spans from the start of its first bucket to the end of its
        // last, so a single bucket is BUCKET_MIN wide, not zero.
        if r.to_min - r.from_min + BUCKET_MIN >= min_len {
            out.push(r);
        }
    }
}

impl Insight {
    /// `02:00–05:00` — the window this pattern covers.
    ///
    /// Derived from the bucket indices rather than by offsetting the stored
    /// centres: a centre is truncated (bucket 19 of 15 minutes is centred at
    /// 292.5 but stored as 292), so ±half a bucket lands a minute short and
    /// reports 04:59.
    pub fn window(&self) -> String {
        let fmt = |m: i64| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
        let start = self.from_min / BUCKET_MIN * BUCKET_MIN;
        let end = (self.to_min / BUCKET_MIN + 1) * BUCKET_MIN;
        format!("{}–{}", fmt(start), fmt(end))
    }

    /// A one-line finding, in display units.
    pub fn text(&self, units: crate::units::Units) -> String {
        let value = units.format(self.extreme);
        let unit = units.label();
        match self.kind {
            Pattern::Lows => format!("lows {} (down to {value} {unit})", self.window()),
            Pattern::Highs => format!("highs {} (up to {value} {unit})", self.window()),
        }
    }
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

    /// A profile with `p25` below target from `from` to `to` (minutes), and
    /// comfortably in range elsewhere.
    fn profile_with_low_window(from: i64, to: i64) -> Vec<Band> {
        (0..BUCKETS as i64)
            .map(|i| {
                let minute = i * BUCKET_MIN + BUCKET_MIN / 2;
                let low = (from..to).contains(&minute);
                Band {
                    minute,
                    p05: if low { 50.0 } else { 90.0 },
                    p25: if low { 62.0 } else { 100.0 },
                    p50: 110.0,
                    p75: 130.0,
                    p95: 150.0,
                }
            })
            .collect()
    }

    #[test]
    fn finds_an_overnight_low_pattern() {
        let bands = profile_with_low_window(120, 300); // 02:00–05:00
        let found = insights(&bands, 70.0, 180.0);
        assert_eq!(found.len(), 1);
        let i = &found[0];
        assert_eq!(i.kind, Pattern::Lows);
        assert_eq!(i.window(), "02:00–05:00");
        assert_eq!(i.extreme, 50.0); // reports how low it actually got
        assert!(i
            .text(crate::units::Units::Mgdl)
            .starts_with("lows 02:00–05:00"));
    }

    #[test]
    fn a_single_dip_is_not_a_pattern() {
        // One 15-minute bucket below target is noise, not something to act on.
        let bands = profile_with_low_window(120, 130);
        assert!(insights(&bands, 70.0, 180.0).is_empty());
    }

    #[test]
    fn a_gap_in_the_data_does_not_join_two_runs() {
        // Two short low windows either side of a stretch with no readings at
        // all. Bridging them would invent a pattern across hours of nothing.
        let bands: Vec<Band> = profile_with_low_window(0, 1440)
            .into_iter()
            .filter(|b| b.minute < 60 || b.minute > 600)
            .collect();
        let found = insights(&bands, 70.0, 180.0);
        assert_eq!(found.len(), 2, "expected two separate runs, got {found:?}");
        assert!(found[0].from_min > 600 || found[1].from_min > 600);
    }

    #[test]
    fn finds_highs_and_ranks_lows_first() {
        let mut bands = profile_with_low_window(120, 300);
        // A long post-dinner high: median above target 19:00–23:00.
        for b in bands.iter_mut() {
            if (1140..1380).contains(&b.minute) {
                b.p50 = 220.0;
                b.p95 = 260.0;
            }
        }
        let found = insights(&bands, 70.0, 180.0);
        assert_eq!(found.len(), 2);
        // Lows come first even though the high window is longer.
        assert_eq!(found[0].kind, Pattern::Lows);
        assert_eq!(found[1].kind, Pattern::Highs);
        assert_eq!(found[1].extreme, 260.0);
        assert!(found[1]
            .text(crate::units::Units::Mgdl)
            .contains("up to 260"));
    }

    #[test]
    fn a_profile_in_range_has_nothing_to_report() {
        let bands = profile_with_low_window(0, 0);
        assert!(insights(&bands, 70.0, 180.0).is_empty());
    }
}
