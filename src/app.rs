//! Application state.

use crate::nightscout::Entry;
use crate::units::Units;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    // Settings, // TODO: settings screen — see issues
}

pub struct App {
    pub units: Units,
    pub screen: Screen,
    /// Recent entries, newest first.
    pub entries: Vec<Entry>,
    pub last_error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(units: Units) -> Self {
        Self {
            units,
            screen: Screen::Dashboard,
            entries: Vec::new(),
            last_error: None,
            should_quit: false,
        }
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
