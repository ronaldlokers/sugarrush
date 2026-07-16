//! Application state.

use crate::nightscout::Entry;
use crate::units::Units;
use crate::view::View;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    // Settings, // TODO: settings screen — see issues
}

pub struct App {
    pub units: Units,
    pub screen: Screen,
    /// Entries loaded for the current window, newest first.
    pub entries: Vec<Entry>,
    /// The visible time window over the history.
    pub view: View,
    /// Concrete window bounds (epoch ms) from the last fetch, for rendering.
    pub view_start: i64,
    pub view_end: i64,
    /// When `Some`, a date-jump prompt is open holding the typed buffer.
    pub date_input: Option<String>,
    pub last_error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(units: Units) -> Self {
        Self {
            units,
            screen: Screen::Dashboard,
            entries: Vec::new(),
            view: View::default(),
            view_start: 0,
            view_end: 0,
            date_input: None,
            last_error: None,
            should_quit: false,
        }
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
