//! Application state.

use crate::alert::{self, Alert};
use crate::config::Alerts;
use crate::nightscout::Entry;
use crate::units::Units;
use crate::view::View;

pub struct App {
    pub units: Units,
    /// Entries loaded for the current window, newest first.
    pub entries: Vec<Entry>,
    /// The visible time window over the history.
    pub view: View,
    /// Concrete window bounds (epoch ms) from the last fetch, for rendering.
    pub view_start: i64,
    pub view_end: i64,
    /// When `Some`, a date-jump prompt is open holding the typed buffer.
    pub date_input: Option<String>,
    /// Forecast points `(epoch_ms, mg/dL)`, live mode only.
    pub predictions: Vec<(i64, f64)>,
    /// Configured alert thresholds and behaviour.
    pub alerts: Alerts,
    /// Current alert state (only meaningful in live mode).
    pub alert: Alert,
    /// Last alert we sent a desktop notification for, to debounce repeats.
    last_notified: Option<Alert>,
    pub last_error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(units: Units, alerts: Alerts) -> Self {
        Self {
            units,
            entries: Vec::new(),
            view: View::default(),
            view_start: 0,
            view_end: 0,
            date_input: None,
            predictions: Vec::new(),
            alerts,
            alert: Alert::InRange,
            last_notified: None,
            last_error: None,
            should_quit: false,
        }
    }

    /// Recompute the alert state from the latest reading. Alerts only apply
    /// while following live data; browsing history never alerts. Returns the
    /// new state so the caller can react to transitions.
    pub fn evaluate_alert(&mut self, now_ms: i64) -> Alert {
        self.alert = if self.view.is_live() {
            match self.latest() {
                Some(e) => alert::evaluate(e.sgv, now_ms - e.date, &self.alerts),
                None => Alert::InRange,
            }
        } else {
            Alert::InRange
        };
        self.alert
    }

    /// If the alert level changed into an alerting state since the last desktop
    /// notification, return it (once) and record it. Returning to range or
    /// staying at the same level yields `None`, debouncing repeats.
    pub fn take_notification(&mut self) -> Option<Alert> {
        if self.last_notified == Some(self.alert) {
            return None;
        }
        self.last_notified = Some(self.alert);
        self.alert.is_alerting().then_some(self.alert)
    }

    /// Open the date-jump prompt.
    pub fn begin_date_input(&mut self) {
        self.date_input = Some(String::new());
    }

    /// Close the date-jump prompt without acting.
    pub fn cancel_date_input(&mut self) {
        self.date_input = None;
    }

    /// Most recent reading, if any.
    pub fn latest(&self) -> Option<&Entry> {
        self.entries.first()
    }

    /// Difference between the latest and previous reading, in mg/dL.
    pub fn delta_mgdl(&self) -> Option<f64> {
        match (self.entries.first(), self.entries.get(1)) {
            (Some(a), Some(b)) => Some(a.sgv - b.sgv),
            _ => None,
        }
    }

    pub fn toggle_units(&mut self) {
        self.units = self.units.toggle();
    }
}
