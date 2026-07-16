//! Alert evaluation: classify the latest reading against configured thresholds
//! and flag stale data.

use ratatui::style::Color;

use crate::config::Alerts;

const MS_PER_MIN: i64 = 60_000;

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

    /// Colour for the in-TUI banner.
    pub fn color(self) -> Color {
        match self {
            Alert::UrgentLow | Alert::UrgentHigh => Color::Red,
            Alert::Low | Alert::High => Color::Yellow,
            Alert::Stale => Color::Magenta,
            Alert::InRange => Color::Green,
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
    if age_ms > a.stale_minutes * MS_PER_MIN {
        return Alert::Stale;
    }
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
    fn is_alerting_only_for_out_of_range() {
        assert!(!Alert::InRange.is_alerting());
        assert!(Alert::Low.is_alerting());
        assert!(Alert::Stale.is_alerting());
    }
}
