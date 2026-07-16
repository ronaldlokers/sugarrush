//! Application state.

use anyhow::Context;

use crate::alert::{self, Alert};
use crate::config::{Alerts, AlertsConfig, Config, Site};
use crate::nightscout::{DeviceStatus, Entry};
use crate::theme::{Theme, ThemeConfig};
use crate::units::Units;
use crate::view::View;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Settings,
}

/// Editable rows on the settings screen, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Units,
    Refresh,
    Desktop,
    UrgentLow,
    Low,
    High,
    UrgentHigh,
    Stale,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Units,
        Field::Refresh,
        Field::Desktop,
        Field::UrgentLow,
        Field::Low,
        Field::High,
        Field::UrgentHigh,
        Field::Stale,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Units => "Units",
            Field::Refresh => "Refresh interval",
            Field::Desktop => "Desktop notifications",
            Field::UrgentLow => "Urgent low",
            Field::Low => "Low",
            Field::High => "High",
            Field::UrgentHigh => "Urgent high",
            Field::Stale => "Stale after",
        }
    }
}

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
    /// Uploader/device metadata (live mode only).
    pub device: DeviceStatus,
    /// Epoch ms of the latest sensor start/change, if known.
    pub sensor_start_ms: Option<i64>,
    /// Configured alert thresholds and behaviour (mg/dL internally).
    pub alerts: Alerts,
    /// Current alert state (only meaningful in live mode).
    pub alert: Alert,
    /// Last alert we sent a desktop notification for, to debounce repeats.
    last_notified: Option<Alert>,

    // Settings / persistence.
    pub screen: Screen,
    /// Selected settings row.
    pub settings_sel: usize,
    /// Auto-refresh interval; editable at runtime.
    pub refresh_secs: u64,
    /// Set when `refresh_secs` changed so the run loop rebuilds its ticker.
    pub refresh_dirty: bool,
    /// Transient status line for the settings screen (e.g. "saved").
    pub status: Option<String>,
    /// Display colors.
    pub theme: Theme,
    /// Raw theme config, kept so it round-trips through a settings save.
    theme_config: ThemeConfig,
    /// Configured sites, and which one is active.
    pub sites: Vec<Site>,
    pub site_idx: usize,
    /// Set when the active site changed so the run loop rebuilds its client.
    pub site_dirty: bool,

    pub last_error: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(cfg: &Config, alerts: Alerts, sites: Vec<Site>) -> Self {
        Self {
            units: cfg.units,
            entries: Vec::new(),
            view: View::default(),
            view_start: 0,
            view_end: 0,
            date_input: None,
            predictions: Vec::new(),
            device: DeviceStatus::default(),
            sensor_start_ms: None,
            alerts,
            alert: Alert::InRange,
            last_notified: None,
            screen: Screen::Dashboard,
            settings_sel: 0,
            refresh_secs: cfg.refresh_secs,
            refresh_dirty: false,
            status: None,
            theme: cfg.theme.resolve(),
            theme_config: cfg.theme.clone(),
            sites,
            site_idx: 0,
            site_dirty: false,
            last_error: None,
            should_quit: false,
        }
    }

    /// The active site.
    pub fn active_site(&self) -> &Site {
        &self.sites[self.site_idx.min(self.sites.len().saturating_sub(1))]
    }

    /// Switch to the next configured site (no-op with a single site).
    pub fn next_site(&mut self) {
        if self.sites.len() > 1 {
            self.site_idx = (self.site_idx + 1) % self.sites.len();
            self.site_dirty = true;
            self.view.follow();
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

    // ---- Settings screen ----

    /// Toggle between the dashboard and settings screens.
    pub fn toggle_settings(&mut self) {
        self.screen = match self.screen {
            Screen::Dashboard => Screen::Settings,
            Screen::Settings => Screen::Dashboard,
        };
        self.status = None;
    }

    pub fn selected_field(&self) -> Field {
        Field::ALL[self.settings_sel.min(Field::ALL.len() - 1)]
    }

    /// Move the settings selection, wrapping at the ends.
    pub fn settings_move(&mut self, delta: isize) {
        let n = Field::ALL.len() as isize;
        let cur = self.settings_sel as isize;
        self.settings_sel = ((cur + delta).rem_euclid(n)) as usize;
        self.status = None;
    }

    /// Adjust the selected field by `dir` (-1 / +1), applied live.
    pub fn settings_adjust(&mut self, dir: i32) {
        // Threshold step: 0.1 mmol/L or 1 mg/dL, expressed in mg/dL.
        let step_mgdl = self.units.to_mgdl(match self.units {
            Units::Mmol => 0.1,
            Units::Mgdl => 1.0,
        });
        let d = dir as f64;
        match self.selected_field() {
            Field::Units => self.toggle_units(),
            Field::Desktop => self.alerts.desktop = !self.alerts.desktop,
            Field::Refresh => {
                let next = self.refresh_secs as i64 + dir as i64 * 5;
                self.refresh_secs = next.max(5) as u64;
                self.refresh_dirty = true;
            }
            Field::Stale => {
                let next = self.alerts.stale_minutes + dir as i64;
                self.alerts.stale_minutes = next.max(1);
            }
            Field::UrgentLow => {
                self.alerts.urgent_low = clamp_bg(self.alerts.urgent_low + d * step_mgdl)
            }
            Field::Low => self.alerts.low = clamp_bg(self.alerts.low + d * step_mgdl),
            Field::High => self.alerts.high = clamp_bg(self.alerts.high + d * step_mgdl),
            Field::UrgentHigh => {
                self.alerts.urgent_high = clamp_bg(self.alerts.urgent_high + d * step_mgdl)
            }
        }
        self.status = None;
    }

    /// Persist current settings back to config.toml. Sites and theme are
    /// preserved; thresholds are written in the active display unit.
    pub fn save_config(&mut self) {
        let result = Config::path().and_then(|p| {
            let body = toml::to_string_pretty(&self.build_config())
                .context("failed to serialize config")?;
            std::fs::write(&p, body).with_context(|| format!("failed to write {}", p.display()))?;
            Ok(p)
        });
        self.status = Some(match result {
            Ok(p) => format!("saved to {}", p.display()),
            Err(e) => format!("save failed: {e}"),
        });
    }

    /// Reconstruct a `Config` from current settings for persistence. A lone
    /// "default" site is written back in the legacy top-level form.
    fn build_config(&self) -> Config {
        let single_default = self.sites.len() == 1 && self.sites[0].name == "default";
        let (url, token, sites) = if single_default {
            (
                Some(self.sites[0].url.clone()),
                Some(self.sites[0].token.clone()),
                Vec::new(),
            )
        } else {
            (None, None, self.sites.clone())
        };
        let u = self.units;
        Config {
            url,
            token,
            sites,
            units: u,
            refresh_secs: self.refresh_secs,
            alerts: AlertsConfig {
                urgent_low: Some(u.from_mgdl(self.alerts.urgent_low)),
                low: Some(u.from_mgdl(self.alerts.low)),
                high: Some(u.from_mgdl(self.alerts.high)),
                urgent_high: Some(u.from_mgdl(self.alerts.urgent_high)),
                stale_minutes: Some(self.alerts.stale_minutes),
                desktop: Some(self.alerts.desktop),
            },
            theme: self.theme_config.clone(),
        }
    }

    /// Formatted value of a field for display on the settings screen.
    pub fn field_value(&self, field: Field) -> String {
        match field {
            Field::Units => self.units.label().to_string(),
            Field::Refresh => format!("{}s", self.refresh_secs),
            Field::Desktop => if self.alerts.desktop { "on" } else { "off" }.to_string(),
            Field::Stale => format!("{} min", self.alerts.stale_minutes),
            Field::UrgentLow => self.threshold(self.alerts.urgent_low),
            Field::Low => self.threshold(self.alerts.low),
            Field::High => self.threshold(self.alerts.high),
            Field::UrgentHigh => self.threshold(self.alerts.urgent_high),
        }
    }

    fn threshold(&self, mgdl: f64) -> String {
        format!("{} {}", self.units.format(mgdl), self.units.label())
    }
}

fn clamp_bg(mgdl: f64) -> f64 {
    mgdl.clamp(20.0, 500.0)
}
