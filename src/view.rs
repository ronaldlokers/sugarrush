//! Graph viewport: the visible time window and how it pans, zooms, and jumps.
//!
//! The window is defined by a [`Span`] (its width) and an anchor for its right
//! edge. When `end` is `None` the window follows "now" (live mode); once the
//! user pans or jumps into history it holds a fixed `Some(epoch_ms)` end.

use chrono::{Local, NaiveDate, TimeZone};

const MS_PER_MIN: i64 = 60_000;

/// Selectable widths for the visible time window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    H1,
    H3,
    H6,
    H12,
    H24,
}

impl Span {
    /// Ordered widest-to-narrowest for cycling.
    const ORDER: [Span; 5] = [Span::H1, Span::H3, Span::H6, Span::H12, Span::H24];

    pub fn minutes(self) -> i64 {
        match self {
            Span::H1 => 60,
            Span::H3 => 180,
            Span::H6 => 360,
            Span::H12 => 720,
            Span::H24 => 1440,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Span::H1 => "1h",
            Span::H3 => "3h",
            Span::H6 => "6h",
            Span::H12 => "12h",
            Span::H24 => "24h",
        }
    }

    /// Widen the window (fewer detail, more history on screen).
    pub fn wider(self) -> Self {
        let i = Self::ORDER.iter().position(|&s| s == self).unwrap_or(0);
        Self::ORDER[(i + 1).min(Self::ORDER.len() - 1)]
    }

    /// Narrow the window (more detail).
    pub fn narrower(self) -> Self {
        let i = Self::ORDER.iter().position(|&s| s == self).unwrap_or(0);
        Self::ORDER[i.saturating_sub(1)]
    }

    /// How many entries to request to comfortably fill this span, assuming
    /// readings as frequent as one per minute, with slack. Nightscout caps the
    /// count server-side, so over-asking is safe.
    pub fn fetch_count(self) -> usize {
        (self.minutes() as usize + 60) * 2
    }
}

/// The current viewport over the entry history.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub span: Span,
    /// Right edge of the window in epoch ms, or `None` to follow "now".
    pub end: Option<i64>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            span: Span::H3,
            end: None,
        }
    }
}

impl View {
    /// True when the window is anchored to real time.
    pub fn is_live(self) -> bool {
        self.end.is_none()
    }

    /// Concrete `(start_ms, end_ms)` for the window given the current time.
    pub fn bounds(self, now_ms: i64) -> (i64, i64) {
        let end = self.end.unwrap_or(now_ms);
        (end - self.span.minutes() * MS_PER_MIN, end)
    }

    /// Half a span; the step used when panning.
    fn step(self) -> i64 {
        self.span.minutes() * MS_PER_MIN / 2
    }

    /// Pan toward older data.
    pub fn pan_back(&mut self, now_ms: i64) {
        let end = self.end.unwrap_or(now_ms);
        self.end = Some(end - self.step());
    }

    /// Pan toward newer data; snapping back to live once it reaches now.
    pub fn pan_forward(&mut self, now_ms: i64) {
        if let Some(end) = self.end {
            let next = end + self.step();
            self.end = if next >= now_ms { None } else { Some(next) };
        }
    }

    /// Snap the window back to the live edge.
    pub fn follow(&mut self) {
        self.end = None;
    }

    pub fn zoom_out(&mut self) {
        self.span = self.span.wider();
    }

    pub fn zoom_in(&mut self) {
        self.span = self.span.narrower();
    }

    /// Jump to a calendar day: show that whole day at 24h zoom. Clamped to now
    /// so a today/future date lands back in live mode.
    pub fn jump_to(&mut self, date: NaiveDate, now_ms: i64) {
        self.span = Span::H24;
        // Right edge at local midnight following the chosen day.
        let next_midnight = date
            .succ_opt()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|dt| Local.from_local_datetime(&dt).single())
            .map(|dt| dt.timestamp_millis());
        self.end = match next_midnight {
            Some(ms) if ms < now_ms => Some(ms),
            _ => None,
        };
    }
}

/// Parse a `YYYY-MM-DD` string into a date.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000; // fixed epoch ms
    const HOUR: i64 = 3_600_000;

    #[test]
    fn live_window_ends_at_now() {
        let v = View::default();
        let (start, end) = v.bounds(NOW);
        assert_eq!(end, NOW);
        assert_eq!(start, NOW - 3 * HOUR); // default span is 3h
    }

    #[test]
    fn pan_back_anchors_and_shifts_by_half_span() {
        let mut v = View::default();
        v.pan_back(NOW);
        assert!(!v.is_live());
        // 3h span -> 1.5h step
        assert_eq!(v.end, Some(NOW - 90 * 60_000));
    }

    #[test]
    fn pan_forward_snaps_back_to_live() {
        let mut v = View::default();
        v.pan_back(NOW); // now at NOW - 1.5h
        v.pan_forward(NOW); // +1.5h reaches NOW -> live
        assert!(v.is_live());
    }

    #[test]
    fn zoom_cycles_and_clamps() {
        let mut v = View::default(); // H3
        v.zoom_in(); // H1
        assert_eq!(v.span, Span::H1);
        v.zoom_in(); // clamp at narrowest
        assert_eq!(v.span, Span::H1);
        v.zoom_out(); // H3
        v.zoom_out();
        v.zoom_out();
        v.zoom_out(); // H24
        v.zoom_out(); // clamp at widest
        assert_eq!(v.span, Span::H24);
    }

    #[test]
    fn jump_to_past_day_uses_24h_and_anchors() {
        let mut v = View::default();
        let date = parse_date("2000-01-01").unwrap();
        v.jump_to(date, NOW);
        assert_eq!(v.span, Span::H24);
        assert!(!v.is_live());
        let (start, end) = v.bounds(NOW);
        assert_eq!(end - start, 24 * HOUR);
    }

    #[test]
    fn jump_to_future_day_falls_back_to_live() {
        let mut v = View::default();
        let date = parse_date("2099-01-01").unwrap();
        v.jump_to(date, NOW);
        assert!(v.is_live());
    }

    #[test]
    fn parse_date_rejects_garbage() {
        assert!(parse_date("not-a-date").is_none());
        assert!(parse_date("2026-13-40").is_none());
        assert!(parse_date("2026-07-16").is_some());
    }
}
