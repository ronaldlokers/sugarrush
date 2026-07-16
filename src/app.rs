//! Application state.

use std::cell::Cell;

use anyhow::Context;
use chrono::{Local, TimeZone, Timelike};
use ratatui::layout::Rect;

use crate::alert::{self, Alert};
use crate::config::{Alerts, AlertsConfig, Config, GraphStyle, MinimapConfig, Site};
use crate::nightscout::{DeviceStatus, Entry};
use crate::sound;
use crate::theme::{self, Theme, ThemeConfig};
use crate::units::Units;
use crate::view::View;

const MS_PER_HOUR: i64 = 3_600_000;

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
    Sound,
    Snooze,
    QuietHours,
    QuietStart,
    QuietEnd,
    QuietUrgentLow,
    UrgentLow,
    Low,
    High,
    UrgentHigh,
    Stale,
    GraphStyle,
    MinimapEnabled,
    MinimapSpan,
    ThemeLow,
    ThemeInRange,
    ThemeHigh,
    ThemeUrgent,
    ThemePrediction,
    ThemeGraph,
}

impl Field {
    pub const ALL: [Field; 23] = [
        Field::Units,
        Field::Refresh,
        Field::Desktop,
        Field::Sound,
        Field::Snooze,
        Field::QuietHours,
        Field::QuietStart,
        Field::QuietEnd,
        Field::QuietUrgentLow,
        Field::UrgentLow,
        Field::Low,
        Field::High,
        Field::UrgentHigh,
        Field::Stale,
        Field::GraphStyle,
        Field::MinimapEnabled,
        Field::MinimapSpan,
        Field::ThemeLow,
        Field::ThemeInRange,
        Field::ThemeHigh,
        Field::ThemeUrgent,
        Field::ThemePrediction,
        Field::ThemeGraph,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Units => "Units",
            Field::Refresh => "Refresh interval",
            Field::Desktop => "Desktop notifications",
            Field::Sound => "Audible alarm",
            Field::Snooze => "Snooze",
            Field::QuietHours => "Quiet hours",
            Field::QuietStart => "Quiet start",
            Field::QuietEnd => "Quiet end",
            Field::QuietUrgentLow => "Quiet: urgent-low sounds",
            Field::UrgentLow => "Urgent low",
            Field::Low => "Low",
            Field::High => "High",
            Field::UrgentHigh => "Urgent high",
            Field::Stale => "Stale after",
            Field::GraphStyle => "Graph style",
            Field::MinimapEnabled => "Minimap",
            Field::MinimapSpan => "Minimap span",
            Field::ThemeLow => "Color: low",
            Field::ThemeInRange => "Color: in range",
            Field::ThemeHigh => "Color: high",
            Field::ThemeUrgent => "Color: urgent",
            Field::ThemePrediction => "Color: forecast",
            Field::ThemeGraph => "Color: graph",
        }
    }

    /// For theme rows, the palette index (0..6) into the color roles.
    fn theme_index(self) -> Option<usize> {
        match self {
            Field::ThemeLow => Some(0),
            Field::ThemeInRange => Some(1),
            Field::ThemeHigh => Some(2),
            Field::ThemeUrgent => Some(3),
            Field::ThemePrediction => Some(4),
            Field::ThemeGraph => Some(5),
            _ => None,
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
    /// While `Some`, the audible alarm is silenced until this epoch-ms.
    snooze_until: Option<i64>,

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
    /// Color name per role (low/in_range/high/urgent/prediction/graph), edited
    /// on the settings screen and the source for theme persistence.
    theme_names: [String; 6],
    /// Configured sites, and which one is active.
    pub sites: Vec<Site>,
    pub site_idx: usize,
    /// Set when the active site changed so the run loop rebuilds its client.
    pub site_dirty: bool,
    /// How the graph draws readings.
    pub graph_style: GraphStyle,

    // Minimap navigator.
    pub minimap_enabled: bool,
    /// Overview span in ms.
    pub minimap_span_ms: i64,
    /// Readings across the overview span, newest first.
    pub minimap_entries: Vec<Entry>,
    /// Inner rect of the minimap from the last draw, for mouse hit-testing.
    /// `Cell` so the immutable draw pass can record it.
    pub minimap_rect: Cell<Option<Rect>>,

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
            snooze_until: None,
            screen: Screen::Dashboard,
            settings_sel: 0,
            refresh_secs: cfg.refresh_secs,
            refresh_dirty: false,
            status: None,
            theme: cfg.theme.resolve(),
            theme_names: names_from_config(&cfg.theme),
            sites,
            site_idx: 0,
            site_dirty: false,
            graph_style: cfg.graph_style,
            minimap_enabled: cfg.minimap.enabled,
            minimap_span_ms: cfg.minimap.span_hours.max(1) as i64 * MS_PER_HOUR,
            minimap_entries: Vec::new(),
            minimap_rect: Cell::new(None),
            last_error: None,
            should_quit: false,
        }
    }

    /// Handle a mouse press/drag over the minimap at screen column `col`:
    /// recenter the main window on the corresponding time. Returns true if the
    /// column fell within the strip (so the caller should refetch).
    pub fn minimap_seek(&mut self, col: u16, row: u16, now_ms: i64) -> bool {
        let Some(r) = self.minimap_rect.get() else {
            return false;
        };
        if r.width == 0 || row < r.y || row >= r.y + r.height {
            return false;
        }
        let col = col.clamp(r.x, r.x + r.width - 1);
        let frac = (col - r.x) as f64 / r.width as f64;
        let start = now_ms - self.minimap_span_ms;
        let target = start + (frac * self.minimap_span_ms as f64) as i64;
        // Center the main window on the target time, clamped to now (→ live).
        let half = self.view.span.minutes() * 60_000 / 2;
        let end = (target + half).min(now_ms);
        self.view.end = if end >= now_ms { None } else { Some(end) };
        true
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
        // A snooze only applies to the urgent episode that started it; once the
        // state clears, re-arm so the next urgent event alarms again.
        if !self.alert.is_urgent() {
            self.snooze_until = None;
        }
        self.alert
    }

    /// True when the audible alarm should currently sound.
    pub fn alarm_active(&self, now_ms: i64) -> bool {
        if !(self.alerts.sound && self.alert.is_urgent()) {
            return false;
        }
        if self.snooze_until.is_some_and(|t| now_ms < t) {
            return false;
        }
        // During quiet hours only urgent-low sounds (safety override).
        if let Some(dt) = Local.timestamp_millis_opt(now_ms).single() {
            let min_of_day = dt.hour() as i32 * 60 + dt.minute() as i32;
            if self.alerts.in_quiet_hours(min_of_day) {
                return self.alert == Alert::UrgentLow && self.alerts.quiet_urgent_low;
            }
        }
        true
    }

    /// The tone to play for the current alert.
    pub fn alarm_tone(&self) -> sound::Tone {
        match self.alert {
            Alert::UrgentLow => sound::Tone::Low,
            Alert::UrgentHigh => sound::Tone::High,
            _ => sound::Tone::Stale,
        }
    }

    /// Silence the audible alarm for the configured snooze interval.
    pub fn snooze_alarm(&mut self, now_ms: i64) {
        if self.alert.is_urgent() {
            let mins = self.alerts.snooze_minutes.max(1);
            self.snooze_until = Some(now_ms + mins * 60_000);
            self.status = Some(format!("alarm snoozed {mins}m"));
        }
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

    /// Enable quiet hours with a sensible default window if currently unset.
    fn ensure_quiet_hours(&mut self) {
        if self.alerts.quiet_start.is_none() {
            self.alerts.quiet_start = Some(23 * 60);
            self.alerts.quiet_end = Some(7 * 60);
        }
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
            Field::Sound => self.alerts.sound = !self.alerts.sound,
            Field::Snooze => {
                let next = self.alerts.snooze_minutes + dir as i64 * 5;
                self.alerts.snooze_minutes = next.clamp(1, 120);
            }
            Field::QuietHours => {
                if self.alerts.quiet_start.is_some() {
                    self.alerts.quiet_start = None;
                    self.alerts.quiet_end = None;
                } else {
                    self.alerts.quiet_start = Some(23 * 60); // 23:00
                    self.alerts.quiet_end = Some(7 * 60); // 07:00
                }
            }
            Field::QuietStart => {
                self.ensure_quiet_hours();
                if let Some(s) = self.alerts.quiet_start.as_mut() {
                    *s = (*s + dir * 30).rem_euclid(1440);
                }
            }
            Field::QuietEnd => {
                self.ensure_quiet_hours();
                if let Some(e) = self.alerts.quiet_end.as_mut() {
                    *e = (*e + dir * 30).rem_euclid(1440);
                }
            }
            Field::QuietUrgentLow => self.alerts.quiet_urgent_low = !self.alerts.quiet_urgent_low,
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
            Field::GraphStyle => self.graph_style = self.graph_style.cycle(dir),
            Field::MinimapEnabled => self.minimap_enabled = !self.minimap_enabled,
            Field::MinimapSpan => {
                let next = self.minimap_span_ms / MS_PER_HOUR + dir as i64 * 6;
                self.minimap_span_ms = next.clamp(6, 72) * MS_PER_HOUR;
            }
            f => {
                if let Some(i) = f.theme_index() {
                    self.theme_names[i] = theme::cycle_color(&self.theme_names[i], dir).to_string();
                    self.theme = theme::theme_from_names(&self.theme_names);
                }
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
                sound: Some(self.alerts.sound),
                snooze_minutes: Some(self.alerts.snooze_minutes),
                quiet_start: self.alerts.quiet_start.map(crate::config::fmt_hhmm),
                quiet_end: self.alerts.quiet_end.map(crate::config::fmt_hhmm),
                quiet_urgent_low: Some(self.alerts.quiet_urgent_low),
            },
            theme: ThemeConfig {
                low: Some(self.theme_names[0].clone()),
                in_range: Some(self.theme_names[1].clone()),
                high: Some(self.theme_names[2].clone()),
                urgent: Some(self.theme_names[3].clone()),
                prediction: Some(self.theme_names[4].clone()),
                graph: Some(self.theme_names[5].clone()),
            },
            graph_style: self.graph_style,
            minimap: MinimapConfig {
                enabled: self.minimap_enabled,
                span_hours: (self.minimap_span_ms / MS_PER_HOUR) as u32,
            },
        }
    }

    /// Formatted value of a field for display on the settings screen.
    pub fn field_value(&self, field: Field) -> String {
        match field {
            Field::Units => self.units.label().to_string(),
            Field::Refresh => format!("{}s", self.refresh_secs),
            Field::Desktop => if self.alerts.desktop { "on" } else { "off" }.to_string(),
            Field::Sound => if self.alerts.sound { "on" } else { "off" }.to_string(),
            Field::Snooze => format!("{} min", self.alerts.snooze_minutes),
            Field::QuietHours => if self.alerts.quiet_start.is_some() {
                "on"
            } else {
                "off"
            }
            .to_string(),
            Field::QuietStart => self
                .alerts
                .quiet_start
                .map(crate::config::fmt_hhmm)
                .unwrap_or_else(|| "—".into()),
            Field::QuietEnd => self
                .alerts
                .quiet_end
                .map(crate::config::fmt_hhmm)
                .unwrap_or_else(|| "—".into()),
            Field::QuietUrgentLow => if self.alerts.quiet_urgent_low {
                "on"
            } else {
                "off"
            }
            .to_string(),
            Field::Stale => format!("{} min", self.alerts.stale_minutes),
            Field::UrgentLow => self.threshold(self.alerts.urgent_low),
            Field::Low => self.threshold(self.alerts.low),
            Field::High => self.threshold(self.alerts.high),
            Field::UrgentHigh => self.threshold(self.alerts.urgent_high),
            Field::GraphStyle => self.graph_style.label().to_string(),
            Field::MinimapEnabled => if self.minimap_enabled { "on" } else { "off" }.to_string(),
            Field::MinimapSpan => format!("{}h", self.minimap_span_ms / MS_PER_HOUR),
            f => f
                .theme_index()
                .map(|i| self.theme_names[i].clone())
                .unwrap_or_default(),
        }
    }

    fn threshold(&self, mgdl: f64) -> String {
        format!("{} {}", self.units.format(mgdl), self.units.label())
    }
}

/// Six color names from the theme config, defaulting per role where unset.
fn names_from_config(tc: &ThemeConfig) -> [String; 6] {
    let d = theme::DEFAULT_NAMES;
    [
        tc.low.clone().unwrap_or_else(|| d[0].to_string()),
        tc.in_range.clone().unwrap_or_else(|| d[1].to_string()),
        tc.high.clone().unwrap_or_else(|| d[2].to_string()),
        tc.urgent.clone().unwrap_or_else(|| d[3].to_string()),
        tc.prediction.clone().unwrap_or_else(|| d[4].to_string()),
        tc.graph.clone().unwrap_or_else(|| d[5].to_string()),
    ]
}

fn clamp_bg(mgdl: f64) -> f64 {
    mgdl.clamp(20.0, 500.0)
}
