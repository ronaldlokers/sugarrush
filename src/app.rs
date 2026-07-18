//! Application state.

use std::cell::Cell;

use anyhow::Context;
use chrono::{Local, TimeZone, Timelike};
use ratatui::layout::Rect;

use crate::alert::{self, Alert};
use crate::config::{Alerts, AlertsConfig, Config, GraphStyle, MinimapConfig, Site};
use crate::nightscout::{DeviceStatus, Entry, Prediction, Treatment};
use crate::sound;
use crate::theme::{self, Theme, ThemeConfig};
use crate::units::Units;
use crate::view::{Span, View};

const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Settings,
}

/// Which view fills the graph pane, selected by the tab bar above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphView {
    /// Live timeline at 3h zoom.
    H3,
    /// Live timeline at 24h zoom.
    H24,
    /// Ambulatory Glucose Profile — percentile bands folded over N days.
    Agp,
}

impl GraphView {
    pub const ALL: [GraphView; 3] = [GraphView::H3, GraphView::H24, GraphView::Agp];

    pub fn label(self) -> &'static str {
        match self {
            GraphView::H3 => "3h",
            GraphView::H24 => "24h",
            GraphView::Agp => "AGP",
        }
    }

    /// Position in `ALL`, used to highlight the tab.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&v| v == self).unwrap_or(0)
    }

    /// Next/previous tab, wrapping. `dir` is +1 or -1.
    pub fn cycle(self, dir: i32) -> Self {
        let n = Self::ALL.len() as i32;
        Self::ALL[(self.index() as i32 + dir).rem_euclid(n) as usize]
    }
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
    Escalate,
    PredictHorizon,
    UrgentLow,
    Low,
    High,
    UrgentHigh,
    Stale,
    GraphStyle,
    AgpDays,
    MinimapEnabled,
    MinimapSpan,
    ThemeLow,
    ThemeInRange,
    ThemeHigh,
    ThemeUrgent,
    ThemePrediction,
    ThemeGraph,
    Colorblind,
}

impl Field {
    pub const ALL: [Field; 27] = [
        Field::Units,
        Field::Refresh,
        Field::Desktop,
        Field::Sound,
        Field::Snooze,
        Field::QuietHours,
        Field::QuietStart,
        Field::QuietEnd,
        Field::QuietUrgentLow,
        Field::Escalate,
        Field::PredictHorizon,
        Field::UrgentLow,
        Field::Low,
        Field::High,
        Field::UrgentHigh,
        Field::Stale,
        Field::GraphStyle,
        Field::AgpDays,
        Field::MinimapEnabled,
        Field::MinimapSpan,
        Field::ThemeLow,
        Field::ThemeInRange,
        Field::ThemeHigh,
        Field::ThemeUrgent,
        Field::ThemePrediction,
        Field::ThemeGraph,
        Field::Colorblind,
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
            Field::Escalate => "Escalate after",
            Field::PredictHorizon => "Predict horizon",
            Field::UrgentLow => "Urgent low",
            Field::Low => "Low",
            Field::High => "High",
            Field::UrgentHigh => "Urgent high",
            Field::Stale => "Stale after",
            Field::GraphStyle => "Graph style",
            Field::AgpDays => "AGP days",
            Field::MinimapEnabled => "Minimap",
            Field::MinimapSpan => "Minimap span",
            Field::ThemeLow => "Color: low",
            Field::ThemeInRange => "Color: in range",
            Field::ThemeHigh => "Color: high",
            Field::ThemeUrgent => "Color: urgent",
            Field::ThemePrediction => "Color: forecast",
            Field::ThemeGraph => "Color: graph",
            Field::Colorblind => "Colorblind palette",
        }
    }

    /// The section this field belongs to. Fields are grouped contiguously in
    /// `ALL`, so a header is drawn whenever this changes between rows.
    pub fn group(self) -> &'static str {
        match self {
            Field::Units | Field::Refresh => "General",
            Field::Desktop
            | Field::Sound
            | Field::Snooze
            | Field::QuietHours
            | Field::QuietStart
            | Field::QuietEnd
            | Field::QuietUrgentLow
            | Field::Escalate => "Alarm",
            Field::PredictHorizon => "Predictions",
            Field::UrgentLow | Field::Low | Field::High | Field::UrgentHigh | Field::Stale => {
                "Thresholds"
            }
            Field::GraphStyle | Field::AgpDays | Field::MinimapEnabled | Field::MinimapSpan => {
                "Graph"
            }
            Field::ThemeLow
            | Field::ThemeInRange
            | Field::ThemeHigh
            | Field::ThemeUrgent
            | Field::ThemePrediction
            | Field::ThemeGraph
            | Field::Colorblind => "Theme",
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
    pub predictions: Vec<Prediction>,
    /// Uploader/device metadata + IOB/COB (live mode only).
    pub device: DeviceStatus,
    /// Carb/insulin treatments within the current window.
    pub treatments: Vec<Treatment>,
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
    /// When the current urgent episode began (for escalation timing).
    urgent_since: Option<i64>,
    /// Whether we've already pushed for the current urgent episode.
    pushed_episode: bool,
    /// Whether the current urgent episode has escalated.
    escalated: bool,
    /// Whether we've already notified for the current predicted crossing.
    predicted_notified: bool,

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
    /// Which view fills the graph pane (tab bar selection).
    pub graph_view: GraphView,
    /// Last `agp_days` of readings, newest first. Feeds the AGP profile and
    /// the stats panel's clinical-window TIR/mean/GMI.
    pub agp_entries: Vec<Entry>,
    /// When `agp_entries` was last fetched (ms since epoch), to throttle the
    /// heavy history fetch outside the AGP view.
    pub agp_fetched_ms: i64,
    /// How many days of history the AGP view and stats window fold over.
    pub agp_days: u32,

    // Minimap navigator.
    pub minimap_enabled: bool,
    /// Overview span in ms.
    pub minimap_span_ms: i64,
    /// Readings across the overview span, newest first.
    pub minimap_entries: Vec<Entry>,
    /// Inner rect of the minimap from the last draw, for mouse hit-testing.
    /// `Cell` so the immutable draw pass can record it.
    pub minimap_rect: Cell<Option<Rect>>,

    /// Running on synthetic data (`--demo`).
    pub demo: bool,
    /// Set when the config file permissions are too open (token is plaintext).
    pub perm_warning: bool,
    /// Whether the last fetch reached Nightscout.
    pub online: bool,
    /// Epoch ms of the last successful fetch.
    pub last_ok_ms: Option<i64>,
    /// Consecutive fetch failures, for backoff.
    fetch_fails: u32,
    /// Earliest epoch-ms to retry after a failure.
    next_retry_at: Option<i64>,

    pub last_error: Option<String>,
    pub should_quit: bool,
    /// Whether the keybinding help overlay is showing.
    pub show_help: bool,
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
            treatments: Vec::new(),
            sensor_start_ms: None,
            alerts,
            alert: Alert::InRange,
            last_notified: None,
            snooze_until: None,
            urgent_since: None,
            pushed_episode: false,
            escalated: false,
            predicted_notified: false,
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
            graph_view: GraphView::H3,
            agp_entries: Vec::new(),
            agp_fetched_ms: 0,
            agp_days: cfg.agp_days.clamp(1, 90),
            minimap_enabled: cfg.minimap.enabled,
            minimap_span_ms: cfg.minimap.span_hours.max(1) as i64 * MS_PER_HOUR,
            minimap_entries: Vec::new(),
            minimap_rect: Cell::new(None),
            demo: false,
            perm_warning: false,
            online: true,
            last_ok_ms: None,
            fetch_fails: 0,
            next_retry_at: None,
            last_error: None,
            should_quit: false,
            show_help: false,
        }
    }

    /// Record a successful fetch.
    pub fn mark_online(&mut self, now_ms: i64) {
        self.online = true;
        self.last_ok_ms = Some(now_ms);
        self.fetch_fails = 0;
        self.next_retry_at = None;
        self.last_error = None;
    }

    /// Record a failed fetch and schedule a backoff retry (5s → 60s).
    pub fn mark_offline(&mut self, now_ms: i64, err: String) {
        self.online = false;
        self.fetch_fails = self.fetch_fails.saturating_add(1);
        let secs = (5u64 << (self.fetch_fails.min(4) - 1)).min(60);
        self.next_retry_at = Some(now_ms + secs as i64 * 1000);
        self.last_error = Some(err);
    }

    /// True when an offline connection is due for a backoff retry.
    pub fn should_retry(&self, now_ms: i64) -> bool {
        !self.online && self.view.is_live() && self.next_retry_at.is_some_and(|t| now_ms >= t)
    }

    /// True when the graph pane shows the AGP profile rather than the timeline.
    pub fn is_agp(&self) -> bool {
        self.graph_view == GraphView::Agp
    }

    /// Select a graph-pane view. The timeline presets snap the window to their
    /// zoom at the live edge; AGP leaves the timeline untouched.
    pub fn set_graph_view(&mut self, v: GraphView) {
        self.graph_view = v;
        match v {
            GraphView::H3 => {
                self.view.span = Span::H3;
                self.view.follow();
            }
            GraphView::H24 => {
                self.view.span = Span::H24;
                self.view.follow();
            }
            GraphView::Agp => {}
        }
    }

    /// Move to the next/previous graph-pane tab (`dir` is +1 / -1).
    pub fn cycle_graph_view(&mut self, dir: i32) {
        self.set_graph_view(self.graph_view.cycle(dir));
    }

    /// The AGP lookback window in milliseconds.
    pub fn agp_span_ms(&self) -> i64 {
        self.agp_days as i64 * MS_PER_DAY
    }

    /// Entries to request to fill the AGP window (5-min cadence, with slack).
    pub fn agp_fetch_count(&self) -> usize {
        self.agp_days as usize * 24 * 12 + 200
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
                // No reading at all in the live window is itself a sensor gap —
                // but only once we've seen data (don't alarm during first-run
                // setup before any successful fetch).
                None if self.last_ok_ms.is_some() => Alert::Stale,
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

    /// First predicted low/high crossing from the current forecast, as
    /// `(rising, minutes_until)`. `rising` = heading high; else heading low.
    /// Only meaningful while in range and following live data.
    pub fn prediction_eta(&self, now_ms: i64) -> Option<(bool, i64)> {
        if self.alert != Alert::InRange {
            return None;
        }
        // The cone warns on the worst plausible path: its low edge for a low,
        // its high edge for a high.
        for p in &self.predictions {
            if p.low <= self.alerts.low {
                return Some((false, ((p.at_ms - now_ms) / 60_000).max(0)));
            }
            if p.high >= self.alerts.high {
                return Some((true, ((p.at_ms - now_ms) / 60_000).max(0)));
            }
        }
        None
    }

    /// A predictive-alert message if a crossing is forecast within the horizon
    /// and we haven't notified for it yet; debounced per episode.
    pub fn take_predictive(&mut self, now_ms: i64) -> Option<String> {
        let horizon = self.alerts.predict_horizon_minutes;
        match self.prediction_eta(now_ms) {
            Some((rising, mins)) if horizon > 0 && mins <= horizon => {
                if self.predicted_notified {
                    return None;
                }
                self.predicted_notified = true;
                let dir = if rising { "high" } else { "low" };
                Some(format!("heading {dir} in ~{mins} min"))
            }
            _ => {
                self.predicted_notified = false;
                None
            }
        }
    }

    /// The tone to play for the current alert.
    pub fn alarm_tone(&self) -> sound::Tone {
        match self.alert {
            Alert::UrgentLow => sound::Tone::Low,
            Alert::UrgentHigh => sound::Tone::High,
            _ => sound::Tone::Stale,
        }
    }

    /// Minutes left on an active snooze, if the alarm is currently silenced.
    pub fn snooze_remaining_min(&self, now_ms: i64) -> Option<i64> {
        self.snooze_until
            .filter(|t| *t > now_ms)
            .map(|t| (t - now_ms) / 60_000 + 1)
    }

    /// Silence the audible alarm for the configured snooze interval.
    pub fn snooze_alarm(&mut self, now_ms: i64) {
        if self.alert.is_urgent() {
            let mins = self.alerts.snooze_minutes.max(1);
            self.snooze_until = Some(now_ms + mins * 60_000);
            self.status = Some(format!("alarm snoozed {mins}m"));
        }
    }

    /// Track the urgent-episode lifecycle used for escalation and push.
    pub fn update_urgent(&mut self, now_ms: i64) {
        if self.alert.is_urgent() {
            if self.urgent_since.is_none() {
                self.urgent_since = Some(now_ms);
                self.pushed_episode = false;
                self.escalated = false;
            }
        } else {
            self.urgent_since = None;
        }
    }

    /// A message to POST to the push URL if one is warranted now — at urgent
    /// onset, then again on escalation after the configured delay. Fires at
    /// most once per trigger.
    pub fn take_push(&mut self, now_ms: i64) -> Option<String> {
        self.alerts.push_url.as_ref()?;
        if !self.alert.is_urgent() {
            return None;
        }
        let value = self
            .latest()
            .map(|e| format!(" · {} {}", self.units.format(e.sgv), self.units.label()))
            .unwrap_or_default();
        if !self.pushed_episode {
            self.pushed_episode = true;
            return Some(format!("sugarrush: {}{}", self.alert.label(), value));
        }
        if self.alerts.escalate_minutes > 0 && !self.escalated {
            if let Some(s) = self.urgent_since {
                if now_ms - s >= self.alerts.escalate_minutes * 60_000 {
                    self.escalated = true;
                    return Some(format!(
                        "sugarrush: STILL {} after {} min{}",
                        self.alert.label(),
                        self.alerts.escalate_minutes,
                        value
                    ));
                }
            }
        }
        None
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

    /// True when the current theme colors match the colorblind-safe palette.
    fn is_colorblind(&self) -> bool {
        self.theme_names
            .iter()
            .zip(theme::COLORBLIND_NAMES)
            .all(|(a, b)| a == b)
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
            Field::Escalate => {
                let next = self.alerts.escalate_minutes + dir as i64 * 5;
                self.alerts.escalate_minutes = next.clamp(0, 120);
            }
            Field::PredictHorizon => {
                let next = self.alerts.predict_horizon_minutes + dir as i64 * 5;
                self.alerts.predict_horizon_minutes = next.clamp(0, 60);
            }
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
            Field::AgpDays => {
                let next = self.agp_days as i64 + dir as i64;
                self.agp_days = next.clamp(1, 90) as u32;
                // The window changed, so the shared AGP/stats history buffer
                // must be refetched on the next refresh.
                self.agp_fetched_ms = 0;
            }
            Field::MinimapEnabled => self.minimap_enabled = !self.minimap_enabled,
            Field::MinimapSpan => {
                let next = self.minimap_span_ms / MS_PER_HOUR + dir as i64 * 6;
                self.minimap_span_ms = next.clamp(6, 72) * MS_PER_HOUR;
            }
            Field::Colorblind => {
                let names = if self.is_colorblind() {
                    theme::DEFAULT_NAMES
                } else {
                    theme::COLORBLIND_NAMES
                };
                self.theme_names = names.map(String::from);
                self.theme = theme::theme_from_names(&self.theme_names);
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
                escalate_minutes: Some(self.alerts.escalate_minutes),
                push_url: self.alerts.push_url.clone(),
                predict_horizon_minutes: Some(self.alerts.predict_horizon_minutes),
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
            agp_days: self.agp_days,
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
            Field::Escalate => {
                if self.alerts.escalate_minutes == 0 {
                    "off".to_string()
                } else {
                    format!("{} min", self.alerts.escalate_minutes)
                }
            }
            Field::PredictHorizon => {
                if self.alerts.predict_horizon_minutes == 0 {
                    "off".to_string()
                } else {
                    format!("{} min", self.alerts.predict_horizon_minutes)
                }
            }
            Field::Stale => format!("{} min", self.alerts.stale_minutes),
            Field::UrgentLow => self.threshold(self.alerts.urgent_low),
            Field::Low => self.threshold(self.alerts.low),
            Field::High => self.threshold(self.alerts.high),
            Field::UrgentHigh => self.threshold(self.alerts.urgent_high),
            Field::GraphStyle => self.graph_style.label().to_string(),
            Field::AgpDays => format!("{} days", self.agp_days),
            Field::MinimapEnabled => if self.minimap_enabled { "on" } else { "off" }.to_string(),
            Field::MinimapSpan => format!("{}h", self.minimap_span_ms / MS_PER_HOUR),
            Field::Colorblind => if self.is_colorblind() { "on" } else { "off" }.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn app() -> App {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        App::new(&cfg, alerts, sites)
    }

    fn entry(sgv: f64, date: i64) -> Entry {
        Entry {
            sgv,
            date,
            direction: None,
        }
    }

    #[test]
    fn dropout_without_history_does_not_alarm() {
        // Live, no readings, never fetched — first-run setup must not false-alarm.
        let mut a = app();
        assert_eq!(a.evaluate_alert(NOW), Alert::InRange);
    }

    #[test]
    fn dropout_after_data_is_stale() {
        // Once we've seen data, a live window with zero readings is a sensor gap.
        let mut a = app();
        a.last_ok_ms = Some(NOW);
        a.entries.clear();
        assert_eq!(a.evaluate_alert(NOW), Alert::Stale);
    }

    #[test]
    fn fresh_in_range_reading() {
        let mut a = app();
        a.entries = vec![entry(100.0, NOW)];
        assert_eq!(a.evaluate_alert(NOW), Alert::InRange);
    }

    #[test]
    fn old_reading_is_stale() {
        let mut a = app();
        a.entries = vec![entry(100.0, NOW - 20 * 60_000)]; // 20m > 15m stale window
        assert_eq!(a.evaluate_alert(NOW), Alert::Stale);
    }

    #[test]
    fn urgent_low_reading_alarms() {
        let mut a = app();
        a.entries = vec![entry(50.0, NOW)]; // <= 55 urgent-low
        assert_eq!(a.evaluate_alert(NOW), Alert::UrgentLow);
        assert!(a.alarm_active(NOW));
    }

    #[test]
    fn history_view_never_alarms() {
        let mut a = app();
        a.entries = vec![entry(40.0, NOW)];
        a.view.end = Some(NOW - 3_600_000); // pinned into history
        assert_eq!(a.evaluate_alert(NOW), Alert::InRange);
    }

    #[test]
    fn snooze_silences_then_re_arms() {
        let mut a = app();
        a.entries = vec![entry(40.0, NOW)];
        a.evaluate_alert(NOW);
        assert!(a.alarm_active(NOW));
        a.snooze_alarm(NOW);
        assert!(!a.alarm_active(NOW));
        assert!(a.snooze_remaining_min(NOW).is_some());
        // Returning to range clears the snooze so the next episode alarms again.
        a.entries = vec![entry(100.0, NOW)];
        a.evaluate_alert(NOW);
        assert!(a.snooze_remaining_min(NOW).is_none());
    }
}
