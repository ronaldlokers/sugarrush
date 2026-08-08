//! Alert evaluation: classify the latest reading against configured thresholds
//! and flag stale data.

use crate::config::Alerts;

const MS_PER_MIN: i64 = 60_000;

/// How far (mg/dL) a reading must clear a threshold before the alert it
/// triggered clears — roughly one CGM noise step, and 0.2 mmol/L in the other
/// unit. Small enough that a genuine recovery still registers within a reading
/// or two.
const HYSTERESIS_MGDL: f64 = 4.0;

/// The current alert state, worst-case across value and freshness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    UrgentLow,
    Low,
    InRange,
    High,
    UrgentHigh,
    /// No fresh reading within the configured staleness window.
    Stale,
}

impl Alert {
    /// True for anything the user should be warned about.
    pub fn is_alerting(self) -> bool {
        !matches!(self, Alert::InRange)
    }

    /// True for states that warrant an audible alarm (urgent or no data).
    pub fn is_urgent(self) -> bool {
        matches!(self, Alert::UrgentLow | Alert::UrgentHigh | Alert::Stale)
    }

    /// Short human label for banners and notifications.
    pub fn label(self) -> &'static str {
        match self {
            Alert::UrgentLow => "URGENT LOW",
            Alert::Low => "LOW",
            Alert::InRange => "in range",
            Alert::High => "HIGH",
            Alert::UrgentHigh => "URGENT HIGH",
            Alert::Stale => "SENSOR GAP — no recent readings",
        }
    }

    /// Kebab-case identifier for Waybar CSS classes.
    pub fn class(self) -> &'static str {
        match self {
            Alert::UrgentLow => "urgent-low",
            Alert::Low => "low",
            Alert::InRange => "in-range",
            Alert::High => "high",
            Alert::UrgentHigh => "urgent-high",
            Alert::Stale => "stale",
        }
    }

    /// How much attention this state needs, lowest number first. Shared so the
    /// followers list and the watcher's alarm-tone choice can't disagree about
    /// whether a stale sensor outranks a mild low.
    pub fn severity(self) -> u8 {
        match self {
            Alert::UrgentLow => 0,
            Alert::UrgentHigh => 1,
            Alert::Stale => 2,
            Alert::Low => 3,
            Alert::High => 4,
            Alert::InRange => 5,
        }
    }

    /// The themed colour for this state. One definition, so the colourblind
    /// preset reaches every widget — it previously existed three times, and
    /// the copies disagreed about `Stale` (grey, magenta, and urgent), with two
    /// of them hardcoded past the theme entirely.
    pub fn color(self, theme: &crate::theme::Theme) -> ratatui::style::Color {
        match self {
            Alert::UrgentLow | Alert::UrgentHigh | Alert::Stale => theme.urgent,
            Alert::Low => theme.low,
            Alert::High => theme.high,
            Alert::InRange => theme.in_range,
        }
    }

    /// `notify-send` urgency keyword.
    pub fn urgency(self) -> &'static str {
        match self {
            Alert::UrgentLow | Alert::UrgentHigh | Alert::Stale => "critical",
            _ => "normal",
        }
    }
}

/// Classify a reading by value alone (ignoring staleness). Used for the
/// range label shown next to the current value.
pub fn from_value(sgv: f64, a: &Alerts) -> Alert {
    if sgv <= a.urgent_low {
        Alert::UrgentLow
    } else if sgv < a.low {
        Alert::Low
    } else if sgv >= a.urgent_high {
        Alert::UrgentHigh
    } else if sgv > a.high {
        Alert::High
    } else {
        Alert::InRange
    }
}

/// Classify a reading. Staleness takes precedence — a stale reading's value
/// can't be trusted as the current level.
pub fn evaluate(sgv: f64, age_ms: i64, a: &Alerts) -> Alert {
    evaluate_from(sgv, age_ms, a, Alert::InRange)
}

/// Classify a reading given the state it's succeeding, applying
/// [`HYSTERESIS_MGDL`] so an alert doesn't flap.
///
/// Entering an alert state uses the configured thresholds unchanged — a real
/// low must alarm on the first reading that crosses. *Leaving* one requires
/// clearing the threshold by the margin. Without this, a value hovering on a
/// boundary (e.g. 70 → 69 → 70 → 69, well within CGM noise) re-fires the
/// desktop notification, the audible alarm, and the push webhook on every
/// reading, which is how a user learns to ignore them.
pub fn evaluate_from(sgv: f64, age_ms: i64, a: &Alerts, prev: Alert) -> Alert {
    if age_ms > a.stale_minutes * MS_PER_MIN {
        return Alert::Stale;
    }
    let m = HYSTERESIS_MGDL;
    // Widen only the band we're currently in, in the direction of recovery.
    let (urgent_low, low, high, urgent_high) = match prev {
        Alert::UrgentLow => (a.urgent_low + m, a.low, a.high, a.urgent_high),
        Alert::Low => (a.urgent_low, a.low + m, a.high, a.urgent_high),
        Alert::High => (a.urgent_low, a.low, a.high - m, a.urgent_high),
        Alert::UrgentHigh => (a.urgent_low, a.low, a.high, a.urgent_high - m),
        _ => (a.urgent_low, a.low, a.high, a.urgent_high),
    };
    if sgv <= urgent_low {
        Alert::UrgentLow
    } else if sgv < low {
        Alert::Low
    } else if sgv >= urgent_high {
        Alert::UrgentHigh
    } else if sgv > high {
        Alert::High
    } else {
        Alert::InRange
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Alerts {
        Alerts::default() // 55 / 70 / 180 / 250, stale 15m
    }

    const FRESH: i64 = 0;

    #[test]
    fn in_range_midband() {
        assert_eq!(evaluate(100.0, FRESH, &cfg()), Alert::InRange);
    }

    #[test]
    fn low_and_urgent_low_boundaries() {
        assert_eq!(evaluate(69.0, FRESH, &cfg()), Alert::Low);
        assert_eq!(evaluate(55.0, FRESH, &cfg()), Alert::UrgentLow); // <= urgent_low
        assert_eq!(evaluate(40.0, FRESH, &cfg()), Alert::UrgentLow);
    }

    #[test]
    fn high_and_urgent_high_boundaries() {
        assert_eq!(evaluate(181.0, FRESH, &cfg()), Alert::High);
        assert_eq!(evaluate(250.0, FRESH, &cfg()), Alert::UrgentHigh); // >= urgent_high
        assert_eq!(evaluate(300.0, FRESH, &cfg()), Alert::UrgentHigh);
    }

    #[test]
    fn stale_overrides_value() {
        let sixteen_min = 16 * MS_PER_MIN;
        // Even a perfectly in-range value is a Stale alert when old.
        assert_eq!(evaluate(100.0, sixteen_min, &cfg()), Alert::Stale);
    }

    #[test]
    fn entering_an_alert_uses_the_raw_threshold() {
        // Hysteresis must never delay the *onset* of an alert.
        assert_eq!(
            evaluate_from(69.0, FRESH, &cfg(), Alert::InRange),
            Alert::Low
        );
        assert_eq!(
            evaluate_from(55.0, FRESH, &cfg(), Alert::Low),
            Alert::UrgentLow
        );
        assert_eq!(
            evaluate_from(181.0, FRESH, &cfg(), Alert::InRange),
            Alert::High
        );
    }

    #[test]
    fn leaving_an_alert_needs_the_margin() {
        // Low clears only once the reading is clear of 70 by the margin.
        assert_eq!(evaluate_from(71.0, FRESH, &cfg(), Alert::Low), Alert::Low);
        assert_eq!(
            evaluate_from(75.0, FRESH, &cfg(), Alert::Low),
            Alert::InRange
        );
        // Same on the urgent boundaries, in both directions.
        assert_eq!(
            evaluate_from(57.0, FRESH, &cfg(), Alert::UrgentLow),
            Alert::UrgentLow
        );
        assert_eq!(
            evaluate_from(60.0, FRESH, &cfg(), Alert::UrgentLow),
            Alert::Low
        );
        assert_eq!(
            evaluate_from(179.0, FRESH, &cfg(), Alert::High),
            Alert::High
        );
        assert_eq!(
            evaluate_from(247.0, FRESH, &cfg(), Alert::UrgentHigh),
            Alert::UrgentHigh
        );
    }

    #[test]
    fn staleness_still_wins_over_hysteresis() {
        let sixteen_min = 16 * MS_PER_MIN;
        assert_eq!(
            evaluate_from(100.0, sixteen_min, &cfg(), Alert::Low),
            Alert::Stale
        );
    }

    #[test]
    fn is_alerting_only_for_out_of_range() {
        assert!(!Alert::InRange.is_alerting());
        assert!(Alert::Low.is_alerting());
        assert!(Alert::Stale.is_alerting());
    }

    #[test]
    fn severity_puts_the_worst_first_and_ranks_no_data_high() {
        let mut all = [
            Alert::InRange,
            Alert::High,
            Alert::Low,
            Alert::Stale,
            Alert::UrgentHigh,
            Alert::UrgentLow,
        ];
        all.sort_by_key(|a| a.severity());
        assert_eq!(
            all,
            [
                Alert::UrgentLow,
                Alert::UrgentHigh,
                // Silence from a sensor outranks a mild low: a gap is when an
                // unnoticed low is possible.
                Alert::Stale,
                Alert::Low,
                Alert::High,
                Alert::InRange,
            ]
        );
    }

    #[test]
    fn every_state_takes_its_colour_from_the_theme() {
        let t = crate::theme::Theme::default();
        // Nothing may be hardcoded past the palette, or the colourblind preset
        // silently fails to recolour it.
        assert_eq!(Alert::Stale.color(&t), t.urgent);
        assert_eq!(Alert::UrgentLow.color(&t), t.urgent);
        assert_eq!(Alert::Low.color(&t), t.low);
        assert_eq!(Alert::High.color(&t), t.high);
        assert_eq!(Alert::InRange.color(&t), t.in_range);
    }
}
