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

/// Consecutive config-level failures (bad token / URL) before automatic
/// fetching pauses. More than one, so a server that briefly answers 401 during
/// a restart doesn't strand a working setup.
const CONFIG_FAIL_LIMIT: u32 = 3;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Settings,
    /// Every configured site at once, for watching someone else's readings.
    Followers,
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
    SiteUrl,
    SiteToken,
    Units,
    Refresh,
    Desktop,
    NotifyContent,
    Sound,
    Snooze,
    QuietHours,
    QuietStart,
    QuietEnd,
    QuietUrgentLow,
    Escalate,
    PushAlerts,
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
    pub const ALL: [Field; 31] = [
        Field::SiteUrl,
        Field::SiteToken,
        Field::Units,
        Field::Refresh,
        Field::Desktop,
        Field::NotifyContent,
        Field::Sound,
        Field::Snooze,
        Field::QuietHours,
        Field::QuietStart,
        Field::QuietEnd,
        Field::QuietUrgentLow,
        Field::Escalate,
        Field::PushAlerts,
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
            Field::SiteUrl => "Site URL",
            Field::SiteToken => "Read-only token",
            Field::Units => "Units",
            Field::Refresh => "Refresh interval",
            Field::Desktop => "Desktop notifications",
            Field::NotifyContent => "Notification detail",
            Field::Sound => "Audible alarm",
            Field::Snooze => "Snooze",
            Field::QuietHours => "Quiet hours",
            Field::QuietStart => "Quiet start",
            Field::QuietEnd => "Quiet end",
            Field::QuietUrgentLow => "Quiet: urgent-low sounds",
            Field::Escalate => "Escalate after",
            Field::PushAlerts => "Push alerts",
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
            Field::SiteUrl | Field::SiteToken => "Site",
            Field::Units | Field::Refresh => "General",
            Field::Desktop
            | Field::NotifyContent
            | Field::Sound
            | Field::Snooze
            | Field::QuietHours
            | Field::QuietStart
            | Field::QuietEnd
            | Field::QuietUrgentLow
            | Field::Escalate
            | Field::PushAlerts => "Alarm",
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

/// A settings row being edited as free text.
pub struct FieldEdit {
    pub field: Field,
    pub buffer: String,
    /// Render as dots rather than the typed characters.
    pub masked: bool,
}

pub struct App {
    pub units: Units,
    /// Entries loaded for the current window, newest first.
    pub entries: Vec<Entry>,
    /// The newest live reading, kept up to date even while the graph is
    /// showing history, so the alarm never follows the viewport.
    pub live_edge: Option<Entry>,
    /// When this app first evaluated an alert, so "never connected" can become
    /// an alarm rather than an indefinite silence. Recorded on first use
    /// rather than at construction, so it follows the same clock the alert
    /// path is given instead of the wall clock.
    first_seen_ms: Option<i64>,
    /// The visible time window over the history.
    pub view: View,
    /// Concrete window bounds (epoch ms) from the last fetch, for rendering.
    pub view_start: i64,
    pub view_end: i64,
    /// When `Some`, a date-jump prompt is open holding the typed buffer.
    pub date_input: Option<String>,
    /// When `Some`, a settings row is being edited as free text.
    pub field_edit: Option<FieldEdit>,
    /// Latest status per configured site, for the followers screen.
    pub followers: Vec<crate::follow::SiteStatus>,
    /// Settings have been changed but not written back to `config.toml`.
    /// Every edit applies live, so without this there's nothing to distinguish
    /// "changed and saved" from "changed and lost on quit".
    pub settings_dirty: bool,
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
    /// Which urgent state the current episode belongs to. An episode is tied to
    /// one variant, not to "urgent" in general — see `evaluate_alert`.
    episode_kind: Option<Alert>,
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
    /// Config values that had to be coerced on load (implausible thresholds,
    /// crossed bands). Shown in the footer — a silently corrected threshold is
    /// still a threshold the user thinks they set.
    pub config_warnings: Vec<String>,
    /// Whether `sugarrush watch` is currently alive, and whether it has been
    /// seen alive at all during this session. The pair matters: someone who
    /// doesn't run the daemon should see nothing, while someone whose daemon
    /// *died* must be told — those two are indistinguishable from one flag.
    pub watcher_alive: bool,
    pub watcher_seen: bool,
    /// Whether the last fetch reached Nightscout.
    pub online: bool,
    /// Set when a desktop notification could not be delivered — no
    /// notification daemon, or D-Bus refused it.
    pub notify_failed: bool,
    /// Epoch ms of the last successful fetch.
    pub last_ok_ms: Option<i64>,
    /// Consecutive fetch failures, for backoff.
    fetch_fails: u32,
    /// Consecutive failures that retrying can't fix (bad token / URL).
    config_fails: u32,
    /// Set once `config_fails` hits the limit: automatic fetching is paused
    /// until the user fixes the site or asks for a retry.
    pub fetch_paused: bool,
    /// Earliest epoch-ms to retry after a failure.
    next_retry_at: Option<i64>,

    pub last_error: Option<String>,
    /// Set when the reading fetch succeeded but a supplementary one didn't, so
    /// the dashboard can say which panels are stale instead of quietly showing
    /// old IOB/COB, markers, or device status as current.
    pub partial: Option<String>,
    pub should_quit: bool,
    /// Whether the keybinding help overlay is showing.
    pub show_help: bool,
}

impl App {
    pub fn new(cfg: &Config, alerts: Alerts, sites: Vec<Site>) -> Self {
        Self {
            units: cfg.units,
            entries: Vec::new(),
            live_edge: None,
            first_seen_ms: None,
            view: View::default(),
            view_start: 0,
            view_end: 0,
            date_input: None,
            field_edit: None,
            followers: Vec::new(),
            settings_dirty: false,
            predictions: Vec::new(),
            device: DeviceStatus::default(),
            treatments: Vec::new(),
            sensor_start_ms: None,
            alerts,
            alert: Alert::InRange,
            last_notified: None,
            snooze_until: None,
            urgent_since: None,
            episode_kind: None,
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
            config_warnings: Vec::new(),
            watcher_alive: false,
            watcher_seen: false,
            online: true,
            notify_failed: false,
            last_ok_ms: None,
            fetch_fails: 0,
            config_fails: 0,
            fetch_paused: false,
            next_retry_at: None,
            last_error: None,
            partial: None,
            should_quit: false,
            show_help: false,
        }
    }

    /// Note which supplementary fetches failed on this refresh (empty clears).
    pub fn set_partial(&mut self, missing: &[&str]) {
        self.partial = (!missing.is_empty()).then(|| missing.join(", "));
    }

    /// Record a successful fetch.
    pub fn mark_online(&mut self, now_ms: i64) {
        self.online = true;
        self.last_ok_ms = Some(now_ms);
        self.fetch_fails = 0;
        self.config_fails = 0;
        self.fetch_paused = false;
        self.next_retry_at = None;
        self.last_error = None;
    }

    /// Record a failed fetch and schedule a backoff retry (5s → 60s).
    ///
    /// `permanent` marks a failure that repeating the request can't fix (a bad
    /// token or URL). After [`CONFIG_FAIL_LIMIT`] of those in a row, automatic
    /// fetching pauses: retrying a rejected token every few seconds only risks
    /// tripping server-side rate limits, and the fix is in the user's hands.
    pub fn mark_offline(&mut self, now_ms: i64, err: String, permanent: bool) {
        self.online = false;
        // The outage message supersedes any per-panel notice.
        self.partial = None;
        self.fetch_fails = self.fetch_fails.saturating_add(1);
        if permanent {
            self.config_fails = self.config_fails.saturating_add(1);
        } else {
            self.config_fails = 0;
        }
        if self.config_fails >= CONFIG_FAIL_LIMIT {
            self.fetch_paused = true;
            self.next_retry_at = None;
            self.last_error = Some(format!("{err} · retries paused, press r to retry"));
            return;
        }
        // 5 · 10 · 20 · 40 · 60 · 60 … — the shift needs a fifth step to reach
        // the documented ceiling; capping it at four stopped at 40s.
        let secs = (5u64 << (self.fetch_fails.min(5) - 1)).min(60);
        self.next_retry_at = Some(now_ms + secs as i64 * 1000);
        self.last_error = Some(err);
    }

    /// Resume fetching after a pause — on an explicit refresh, or when the site
    /// config changes (a new site, or an edited URL/token).
    pub fn resume_fetching(&mut self) {
        self.fetch_paused = false;
        self.config_fails = 0;
        self.fetch_fails = 0;
        self.next_retry_at = None;
    }

    /// True when an offline connection is due for a backoff retry.
    ///
    /// Unlike the periodic refresh this doesn't require the live edge: reading
    /// history while the connection is down otherwise left it down, so
    /// returning to live meant an offline dashboard until the next manual
    /// refresh. A retry re-fetches whatever window is shown.
    pub fn should_retry(&self, now_ms: i64) -> bool {
        !self.online && !self.fetch_paused && self.next_retry_at.is_some_and(|t| now_ms >= t)
    }

    /// True when the periodic refresh should run (paused after a config error).
    pub fn should_auto_refresh(&self) -> bool {
        self.view.is_live() && !self.fetch_paused
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

    /// Open the text editor for the selected settings row, if it takes text.
    /// Returns true when an editor opened.
    pub fn begin_field_edit(&mut self) -> bool {
        let field = Field::ALL[self.settings_sel.min(Field::ALL.len() - 1)];
        let edit = match field {
            Field::SiteUrl => FieldEdit {
                field,
                // Pre-fill: a URL is usually being corrected, not replaced.
                buffer: self.active_site().url.clone(),
                masked: false,
            },
            // Start empty and masked — a token is replaced wholesale, and
            // pre-filling it would put the secret back on screen.
            Field::SiteToken => FieldEdit {
                field,
                buffer: String::new(),
                masked: true,
            },
            _ => return false,
        };
        self.field_edit = Some(edit);
        true
    }

    pub fn field_edit_push(&mut self, c: char) {
        if let Some(e) = self.field_edit.as_mut() {
            e.buffer.push(c);
        }
    }

    pub fn field_edit_backspace(&mut self) {
        if let Some(e) = self.field_edit.as_mut() {
            e.buffer.pop();
        }
    }

    pub fn cancel_field_edit(&mut self) {
        self.field_edit = None;
    }

    /// Apply the open editor. On a bad URL the editor stays open with the text
    /// intact so it can be fixed rather than retyped.
    pub fn commit_field_edit(&mut self) {
        let Some(edit) = self.field_edit.take() else {
            return;
        };
        let idx = self.site_idx.min(self.sites.len().saturating_sub(1));
        match edit.field {
            Field::SiteUrl => match crate::config::normalize_site_url(&edit.buffer) {
                Ok(url) => {
                    self.sites[idx].url = url;
                    self.site_dirty = true;
                    self.settings_dirty = true;
                    self.status = Some(if self.sites[idx].is_insecure() {
                        "site updated · ⚠ unencrypted http, the token is sent in clear".to_string()
                    } else {
                        format!("site set to {}", self.sites[idx].base_url())
                    });
                }
                Err(e) => {
                    self.status = Some(e.to_string());
                    self.field_edit = Some(edit);
                }
            },
            Field::SiteToken => {
                if edit.buffer.trim().is_empty() {
                    self.status = Some("token unchanged".to_string());
                } else {
                    self.sites[idx].token = edit.buffer.trim().to_string();
                    self.site_dirty = true;
                    self.settings_dirty = true;
                    self.status = Some("token updated · press w to save".to_string());
                }
            }
            _ => {}
        }
    }

    /// Toggle the followers list. Only meaningful with more than one site —
    /// with one, it would just be the dashboard with less on it.
    pub fn toggle_followers(&mut self) {
        if self.sites.len() < 2 {
            self.status = Some("add a second [[sites]] entry to follow others".to_string());
            return;
        }
        self.screen = match self.screen {
            Screen::Followers => Screen::Dashboard,
            _ => Screen::Followers,
        };
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
        self.settings_dirty = true;
    }

    /// The newest live reading, whatever the graph happens to be showing.
    ///
    /// The alarm must never depend on where the viewport is pointed: pressing
    /// `[` to look at last night used to make `evaluate_alert` return InRange,
    /// and — because the TUI keeps heartbeating — silenced the watch daemon
    /// too, so the whole system went quiet while someone read history.
    pub fn live_latest(&self) -> Option<&Entry> {
        if self.view.is_live() {
            self.entries.first()
        } else {
            self.live_edge.as_ref()
        }
    }

    /// Recompute the alert state from the newest live reading. Only the graph
    /// is historical when browsing; the alarm always tracks the present.
    /// Returns the new state so the caller can react to transitions.
    pub fn evaluate_alert(&mut self, now_ms: i64) -> Alert {
        let first_seen = *self.first_seen_ms.get_or_insert(now_ms);
        self.alert = {
            match self.live_latest() {
                // Carry the previous state in so a value sitting on a threshold
                // doesn't flap in and out of the alarm.
                Some(e) => alert::evaluate_from(e.sgv, now_ms - e.date, &self.alerts, self.alert),
                // No reading at all in the live window is itself a sensor gap —
                // but only once we've seen data (don't alarm during first-run
                // setup before any successful fetch).
                None if self.last_ok_ms.is_some() => Alert::Stale,
                // Never connected. Staying quiet is right for the seconds
                // before the first fetch and wrong forever after: a watcher
                // started with a bad token would otherwise report "in range"
                // all night. After the staleness window with nothing at all,
                // no data is a sensor gap like any other.
                None if now_ms - first_seen > self.alerts.stale_minutes * 60_000 => Alert::Stale,
                None => Alert::InRange,
            }
        };
        // An episode belongs to one urgent *state*, not to "urgent" in general.
        // Keying it on `is_urgent()` alone made Stale → UrgentLow one
        // continuous episode, so snoozing a sensor gap silenced the low that
        // arrived when the sensor came back, swallowed its onset push, and left
        // the escalation clock running from the gap — reporting "STILL URGENT
        // LOW after 20 min" for a low two minutes old.
        //
        // A different urgent state is a different emergency: re-arm everything.
        let kind = self.alert.is_urgent().then_some(self.alert);
        if kind != self.episode_kind {
            self.snooze_until = None;
            self.urgent_since = None;
            self.pushed_episode = false;
            self.escalated = false;
            self.episode_kind = kind;
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
        // Follow the projected path — the band centre — not the cone's edge.
        // The cone widens with the horizon by construction, so its edge crosses
        // a threshold even on perfectly flat glucose: at 75 mg/dL with the low
        // at 70, the edge alone would announce "heading low" forever.
        for p in &self.predictions {
            // Skip points already in the past. An uploader forecast is stamped
            // from when the pump published it, so a stale devicestatus is all
            // past points — and reporting one as a crossing "in ~0 min" reads
            // as an imminent low that has, in fact, already been forecast away.
            if p.at_ms <= now_ms {
                continue;
            }
            let centre = (p.low + p.high) / 2.0;
            if centre <= self.alerts.low {
                return Some((false, (p.at_ms - now_ms) / 60_000));
            }
            if centre >= self.alerts.high {
                return Some((true, (p.at_ms - now_ms) / 60_000));
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

    /// Write the clinical window to a CSV and a text summary in the working
    /// directory, reporting the result in the status line.
    ///
    /// Exports the same `agp_days` window the stats panel reports on — not the
    /// visible graph — so the summary someone hands to a clinician matches the
    /// numbers they were looking at when they pressed the key.
    pub fn export_window(&mut self, now_ms: i64) {
        if self.agp_entries.is_empty() {
            self.status = Some("nothing to export yet — no readings loaded".to_string());
            return;
        }
        // Same writer the CLI uses, so the two can't drift again, and the
        // absolute path is reported — a bare filename left the user guessing
        // which directory their health data landed in.
        match crate::export::write_pair(
            std::path::Path::new("."),
            &self.agp_entries,
            &self.alerts,
            self.units,
            self.agp_days,
            now_ms,
        ) {
            Ok(paths) => {
                let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                self.status = Some(format!(
                    "exported {} days to {}",
                    self.agp_days,
                    names.join(" + ")
                ));
            }
            Err(e) => self.status = Some(format!("export failed: {e}")),
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
        if !self.alerts.push_enabled {
            return None;
        }
        if !self.alert.is_urgent() {
            return None;
        }
        // The push is the only channel that leaves the machine, so it honours
        // the privacy setting the desktop notification already did: with
        // `notify_content` off, the reading is left out rather than shipped to
        // a third-party broker in clear text.
        let value = if self.alerts.notify_content {
            self.live_latest()
                .map(|e| format!(" · {} {}", self.units.format(e.sgv), self.units.label()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        // Name whose reading it is. A caregiver watching three people gets
        // "URGENT LOW" on their phone with no way to tell which one.
        let who = if self.sites.len() > 1 {
            format!("[{}] ", self.active_site().name)
        } else {
            String::new()
        };
        if !self.pushed_episode {
            self.pushed_episode = true;
            return Some(format!("sugarrush: {who}{}{value}", self.alert.label()));
        }
        if self.alerts.escalate_minutes > 0 && !self.escalated {
            if let Some(s) = self.urgent_since {
                if now_ms - s >= self.alerts.escalate_minutes * 60_000 {
                    self.escalated = true;
                    return Some(format!(
                        "sugarrush: {who}STILL {} after {} min{value}",
                        self.alert.label(),
                        self.alerts.escalate_minutes,
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

    // ---- Episode state, for the headless watcher ----
    //
    // `sugarrush watch` persists these across restarts so a restarted service
    // doesn't re-announce an ongoing low, restart an escalation timer, or
    // cancel a snooze someone deliberately set. They're read-only accessors
    // plus one restore hook rather than public fields, so the invariants stay
    // in this file.

    pub fn last_notified(&self) -> Option<Alert> {
        self.last_notified
    }

    pub fn urgent_since(&self) -> Option<i64> {
        self.urgent_since
    }

    pub fn episode_kind(&self) -> Option<Alert> {
        self.episode_kind
    }

    pub fn pushed_episode(&self) -> bool {
        self.pushed_episode
    }

    pub fn escalated(&self) -> bool {
        self.escalated
    }

    pub fn snooze_until(&self) -> Option<i64> {
        self.snooze_until
    }

    /// Resume a previously-running alert episode.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_episode(
        &mut self,
        last_notified: Option<Alert>,
        urgent_since: Option<i64>,
        pushed_episode: bool,
        escalated: bool,
        snooze_until: Option<i64>,
        episode_kind: Option<Alert>,
    ) {
        self.last_notified = last_notified;
        self.urgent_since = urgent_since;
        self.pushed_episode = pushed_episode;
        self.escalated = escalated;
        self.snooze_until = snooze_until;
        // Without this the first evaluate_alert after a restart would see a
        // "changed" variant and wipe the very episode we just restored.
        self.episode_kind = episode_kind;
    }

    // ---- Settings screen ----

    /// Toggle between the dashboard and settings screens.
    pub fn toggle_settings(&mut self) {
        self.screen = match self.screen {
            Screen::Settings => Screen::Dashboard,
            // From the followers list, settings is still one keypress away.
            Screen::Dashboard | Screen::Followers => Screen::Settings,
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
        // Every branch below either changes a persisted value or sets a status
        // message; marking here keeps the flag from being forgotten as rows are
        // added. A no-op row (an unset push URL) clears it again at the end.
        let was_dirty = self.settings_dirty;
        self.settings_dirty = true;
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
            // Thresholds are clamped against their neighbours as well as the
            // physiological range, so the four can never cross: a `low` above
            // `high` (or below `urgent_low`) would silently disable a band and
            // misclassify every reading in it.
            Field::UrgentLow => {
                self.alerts.urgent_low =
                    clamp_bg(self.alerts.urgent_low + d * step_mgdl).min(self.alerts.low)
            }
            Field::Low => {
                self.alerts.low = clamp_bg(self.alerts.low + d * step_mgdl)
                    .clamp(self.alerts.urgent_low, self.alerts.high)
            }
            Field::High => {
                self.alerts.high = clamp_bg(self.alerts.high + d * step_mgdl)
                    .clamp(self.alerts.low, self.alerts.urgent_high)
            }
            Field::UrgentHigh => {
                self.alerts.urgent_high =
                    clamp_bg(self.alerts.urgent_high + d * step_mgdl).max(self.alerts.high)
            }
            Field::SiteUrl | Field::SiteToken => {
                self.status = Some("press enter to edit".to_string());
                self.settings_dirty = was_dirty;
            }
            Field::NotifyContent => self.alerts.notify_content = !self.alerts.notify_content,
            Field::PushAlerts => {
                // Only meaningful with a URL configured; say so rather than
                // toggling a flag that can't do anything.
                if self.alerts.push_url.is_some() {
                    self.alerts.push_enabled = !self.alerts.push_enabled;
                } else {
                    self.status =
                        Some("set push_url in config.toml to enable push alerts".to_string());
                    self.settings_dirty = was_dirty;
                }
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
            // Atomic + owner-only: this file holds the only copy of the token.
            Config::write_atomic(&p, &body)?;
            Ok(p)
        });
        self.status = Some(match result {
            Ok(p) => {
                self.settings_dirty = false;
                format!("saved to {}", p.display())
            }
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
                push_enabled: Some(self.alerts.push_enabled),
                notify_content: Some(self.alerts.notify_content),
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
            Field::SiteUrl => {
                let site = self.active_site();
                // The demo's placeholder URL is not a real site; flagging it
                // would put a warning in every screenshot for nothing.
                if site.is_insecure() && !self.demo {
                    format!("{} ⚠ unencrypted", site.base_url())
                } else {
                    site.base_url().to_string()
                }
            }
            // Never render the token, not even partially: the settings screen
            // is the pane most likely to be screenshotted or screen-shared.
            Field::SiteToken => if self.active_site().token.is_empty() {
                "not set"
            } else {
                "set · ••••••"
            }
            .to_string(),
            Field::PredictHorizon => {
                let h = self.alerts.predict_horizon_minutes;
                if h == 0 {
                    "off".to_string()
                } else if h > crate::predict::HORIZON_MINUTES {
                    // Beyond the local AR2 projection only an uploader forecast
                    // (Loop / OpenAPS) can reach — say so rather than implying
                    // a warning that far out always exists.
                    format!(
                        "{} min · local forecast {} min",
                        h,
                        crate::predict::HORIZON_MINUTES
                    )
                } else {
                    format!("{h} min")
                }
            }
            // Show where pushes go, but never the full URL — an ntfy topic or
            // a webhook path is a secret, and the settings screen is the pane
            // most likely to end up in a screenshot.
            Field::NotifyContent => if self.alerts.notify_content {
                "value + state"
            } else {
                "generic (no data)"
            }
            .to_string(),
            Field::PushAlerts => match (&self.alerts.push_url, self.alerts.push_enabled) {
                (None, _) => "not configured".to_string(),
                (Some(url), on) => {
                    let state = if on { "on" } else { "off" };
                    let warn = if crate::config::is_insecure_url(url) {
                        " ⚠ unencrypted"
                    } else {
                        ""
                    };
                    format!("{state} · {}{warn}", push_host(url))
                }
            },
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

/// The host part of a push URL, for display without leaking the topic/path.
fn push_host(url: &str) -> &str {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or(url)
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

    /// C1 of the review: an episode is one urgent *state*, not "urgent".
    #[test]
    fn snoozing_a_sensor_gap_does_not_silence_the_low_that_follows() {
        let mut a = app();
        a.last_ok_ms = Some(NOW);
        a.entries.clear(); // a total dropout is a sensor gap
        assert_eq!(a.evaluate_alert(NOW), Alert::Stale);
        a.update_urgent(NOW);
        assert!(a.alarm_active(NOW));

        // 03:00 — silence the gap.
        a.snooze_alarm(NOW);
        assert!(!a.alarm_active(NOW));

        // 03:02 — the sensor comes back at 40 mg/dL. Different emergency.
        let later = NOW + 2 * 60_000;
        a.entries = vec![entry(40.0, later)];
        assert_eq!(a.evaluate_alert(later), Alert::UrgentLow);
        a.update_urgent(later);
        assert!(
            a.alarm_active(later),
            "the gap's snooze silenced the urgent low that followed it"
        );
        // The escalation clock restarts from the low, not from the gap.
        assert_eq!(a.urgent_since(), Some(later));
    }

    #[test]
    fn a_new_urgent_state_pushes_at_its_own_onset() {
        let mut a = app();
        a.alerts.push_url = Some("https://ntfy.sh/topic".into());
        a.alerts.escalate_minutes = 20;

        a.entries = vec![entry(40.0, NOW)];
        a.evaluate_alert(NOW);
        a.update_urgent(NOW);
        assert!(a.take_push(NOW).is_some(), "no push at the low's onset");

        // Recovers, then goes urgent-high two minutes later.
        let t1 = NOW + 60_000;
        a.entries = vec![entry(100.0, t1)];
        a.evaluate_alert(t1);
        a.update_urgent(t1);
        let t2 = NOW + 120_000;
        a.entries = vec![entry(300.0, t2)];
        a.evaluate_alert(t2);
        a.update_urgent(t2);

        // Its own onset push, not an escalation inherited from the low.
        let msg = a.take_push(t2).expect("no push at the high's onset");
        assert!(msg.contains("URGENT HIGH"), "{msg}");
        assert!(!msg.contains("STILL"), "escalated at onset: {msg}");
    }

    #[test]
    fn the_same_urgent_state_keeps_its_episode() {
        // The flip side: consecutive urgent-low readings must NOT re-arm, or
        // the snooze would be useless and every reading would re-push.
        let mut a = app();
        a.alerts.push_url = Some("https://ntfy.sh/topic".into());
        a.entries = vec![entry(40.0, NOW)];
        a.evaluate_alert(NOW);
        a.update_urgent(NOW);
        a.take_push(NOW);
        a.snooze_alarm(NOW);

        let later = NOW + 60_000;
        a.entries = vec![entry(38.0, later)];
        assert_eq!(a.evaluate_alert(later), Alert::UrgentLow);
        a.update_urgent(later);
        assert!(!a.alarm_active(later), "snooze was dropped mid-episode");
        assert_eq!(a.urgent_since(), Some(NOW), "escalation clock restarted");
        assert_eq!(a.take_push(later), None, "pushed twice for one episode");
    }

    #[test]
    fn browsing_history_does_not_silence_the_alarm() {
        let mut a = app();
        // A live urgent low, and the graph pointed at yesterday.
        a.entries = vec![entry(38.0, NOW)];
        a.live_edge = Some(entry(38.0, NOW));
        a.view.shift_day(-1, NOW);
        assert!(!a.view.is_live());

        assert_eq!(
            a.evaluate_alert(NOW),
            Alert::UrgentLow,
            "the alarm followed the viewport instead of the clock"
        );
        assert!(a.alarm_active(NOW));
    }

    #[test]
    fn a_site_that_never_connects_eventually_alarms() {
        let mut a = app();
        // Nothing fetched, ever — a watcher started with a bad token.
        assert_eq!(a.evaluate_alert(NOW), Alert::InRange, "alarmed too early");

        // Past the staleness window with still nothing, silence is wrong.
        let later = NOW + (a.alerts.stale_minutes + 1) * 60_000;
        assert_eq!(a.evaluate_alert(later), Alert::Stale);
        a.update_urgent(later);
        assert!(a.alarm_active(later));
    }

    #[test]
    fn alert_does_not_flap_on_a_threshold() {
        let mut a = app();
        // Cross into Low, then bounce back onto the boundary.
        a.entries = vec![entry(69.0, NOW)];
        assert_eq!(a.evaluate_alert(NOW), Alert::Low);
        a.entries = vec![entry(71.0, NOW)]; // within the hysteresis margin
        assert_eq!(a.evaluate_alert(NOW), Alert::Low);
        // A genuine recovery still clears it.
        a.entries = vec![entry(80.0, NOW)];
        assert_eq!(a.evaluate_alert(NOW), Alert::InRange);
    }

    #[test]
    fn flat_glucose_does_not_predict_a_low() {
        let mut a = app();
        a.entries = vec![entry(75.0, NOW)];
        a.evaluate_alert(NOW);
        // A flat forecast whose cone edge dips below the low threshold but
        // whose centre holds steady must not announce a low.
        a.predictions = (1..=6)
            .map(|i| Prediction {
                at_ms: NOW + i * 5 * 60_000,
                low: 75.0 - 4.0 * i as f64,
                high: 75.0 + 4.0 * i as f64,
            })
            .collect();
        assert_eq!(a.prediction_eta(NOW), None);

        // A forecast actually heading low still fires, timed off the centre.
        a.predictions = (1..=6)
            .map(|i| Prediction {
                at_ms: NOW + i * 5 * 60_000,
                low: 75.0 - 3.0 * i as f64,
                high: 75.0 - i as f64,
            })
            .collect();
        assert_eq!(a.prediction_eta(NOW), Some((false, 15)));
    }

    #[test]
    fn permanent_failures_pause_fetching() {
        let mut a = app();
        for _ in 0..CONFIG_FAIL_LIMIT {
            a.mark_offline(NOW, "authentication failed".into(), true);
        }
        assert!(a.fetch_paused);
        assert!(!a.should_retry(NOW + 60_000));
        assert!(!a.should_auto_refresh());
        // An explicit retry (r) resumes, and a success clears the state.
        a.resume_fetching();
        assert!(a.should_auto_refresh());
        a.mark_offline(NOW, "authentication failed".into(), true);
        a.mark_online(NOW);
        assert!(!a.fetch_paused);
    }

    #[test]
    fn thresholds_cannot_cross() {
        let mut a = app();
        a.settings_sel = Field::ALL.iter().position(|&f| f == Field::Low).unwrap();
        // Drive `low` down past `urgent_low`: it must stop there, not overtake.
        for _ in 0..200 {
            a.settings_adjust(-1);
        }
        assert!(a.alerts.low >= a.alerts.urgent_low);
        // And up past `high`.
        for _ in 0..400 {
            a.settings_adjust(1);
        }
        assert!(a.alerts.low <= a.alerts.high);
        assert!(a.alerts.urgent_low <= a.alerts.low);
        assert!(a.alerts.high <= a.alerts.urgent_high);
    }

    #[test]
    fn push_toggle_needs_a_url_and_round_trips() {
        let mut a = app();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::PushAlerts)
            .unwrap();
        // No URL configured: the row explains itself instead of toggling.
        assert_eq!(a.field_value(Field::PushAlerts), "not configured");
        a.settings_adjust(1);
        assert!(a.alerts.push_enabled);

        a.alerts.push_url = Some("https://ntfy.sh/secret-topic".into());
        assert_eq!(a.field_value(Field::PushAlerts), "on · ntfy.sh"); // no topic leaked
        a.settings_adjust(1);
        assert!(!a.alerts.push_enabled);
        assert!(a.alerts.push_url.is_some()); // disabling keeps the URL
        assert_eq!(a.build_config().alerts.push_enabled, Some(false));

        // Disabled means no push is emitted, even in an urgent episode.
        a.entries = vec![entry(40.0, NOW)];
        a.evaluate_alert(NOW);
        a.update_urgent(NOW);
        assert_eq!(a.take_push(NOW), None);
    }

    #[test]
    fn backoff_reaches_the_documented_ceiling() {
        let mut a = app();
        let expected = [5, 10, 20, 40, 60, 60];
        for (i, secs) in expected.iter().enumerate() {
            a.mark_offline(NOW, "offline".into(), false);
            assert!(
                a.should_retry(NOW + secs * 1000),
                "failure {} should retry after {secs}s",
                i + 1
            );
            assert!(!a.should_retry(NOW + secs * 1000 - 1));
        }
    }

    #[test]
    fn retries_continue_while_browsing_history() {
        let mut a = app();
        a.view.end = Some(NOW - 3_600_000); // pinned into history
        a.mark_offline(NOW, "connection refused".into(), false);
        // The periodic refresh stays off in history, but recovery must not:
        // otherwise returning to live lands on a still-offline dashboard.
        assert!(!a.should_auto_refresh());
        assert!(a.should_retry(NOW + 5_000));
    }

    #[test]
    fn stale_uploader_forecast_does_not_fire() {
        let mut a = app();
        a.entries = vec![entry(100.0, NOW)];
        a.evaluate_alert(NOW);
        // A forecast published 40 minutes ago: every point is already past,
        // and they all sit low. It must not read as "low in ~0 min".
        a.predictions = (1..=6)
            .map(|i| Prediction {
                at_ms: NOW - 40 * 60_000 + i * 5 * 60_000,
                low: 60.0,
                high: 60.0,
            })
            .collect();
        assert_eq!(a.prediction_eta(NOW), None);
        assert_eq!(a.take_predictive(NOW), None);
    }

    #[test]
    fn horizon_beyond_the_local_forecast_says_so() {
        let mut a = app();
        a.alerts.predict_horizon_minutes = crate::predict::HORIZON_MINUTES;
        assert_eq!(a.field_value(Field::PredictHorizon), "30 min");
        a.alerts.predict_horizon_minutes = 45;
        assert_eq!(
            a.field_value(Field::PredictHorizon),
            "45 min · local forecast 30 min"
        );
    }

    #[test]
    fn editing_the_site_url_normalizes_and_reloads() {
        let mut a = app();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteUrl)
            .unwrap();
        assert!(a.begin_field_edit());
        // Pre-filled with the current URL so a typo is corrected, not retyped.
        assert_eq!(a.field_edit.as_ref().unwrap().buffer, a.active_site().url);
        a.field_edit.as_mut().unwrap().buffer = "ns.example.com/api/v1/entries.json".into();
        a.commit_field_edit();
        assert_eq!(a.active_site().url, "https://ns.example.com");
        assert!(a.site_dirty); // triggers a client rebuild + refresh
        assert!(a.field_edit.is_none());
    }

    #[test]
    fn a_bad_url_keeps_the_editor_open() {
        let mut a = app();
        let before = a.active_site().url.clone();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteUrl)
            .unwrap();
        a.begin_field_edit();
        a.field_edit.as_mut().unwrap().buffer = "ftp://nope".into();
        a.commit_field_edit();
        assert_eq!(a.active_site().url, before);
        assert!(a.field_edit.is_some()); // fix it in place
        assert!(a.status.is_some());
    }

    #[test]
    fn token_edit_is_masked_and_never_rendered() {
        let mut a = app();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteToken)
            .unwrap();
        a.begin_field_edit();
        let edit = a.field_edit.as_ref().unwrap();
        assert!(edit.masked);
        assert!(edit.buffer.is_empty()); // never pre-fills the secret
        a.field_edit.as_mut().unwrap().buffer = "s3cret-token".into();
        a.commit_field_edit();
        assert_eq!(a.active_site().token, "s3cret-token");
        assert!(a.site_dirty);
        let shown = a.field_value(Field::SiteToken);
        assert!(
            !shown.contains("s3cret"),
            "token leaked into the row: {shown}"
        );

        // An empty commit leaves the existing token alone.
        a.begin_field_edit();
        a.commit_field_edit();
        assert_eq!(a.active_site().token, "s3cret-token");
    }

    #[test]
    fn edits_mark_settings_unsaved() {
        let mut a = app();
        assert!(!a.settings_dirty);
        a.settings_sel = Field::ALL.iter().position(|&f| f == Field::Low).unwrap();
        a.settings_adjust(1);
        assert!(a.settings_dirty);
        // Rows that only print a hint aren't an edit.
        a.settings_dirty = false;
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteUrl)
            .unwrap();
        a.settings_adjust(1);
        assert!(!a.settings_dirty);
        // Nor is toggling push with no URL configured.
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::PushAlerts)
            .unwrap();
        a.settings_adjust(1);
        assert!(!a.settings_dirty);
        // Editing the site is.
        a.begin_field_edit();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteUrl)
            .unwrap();
        a.begin_field_edit();
        a.field_edit.as_mut().unwrap().buffer = "https://ns.example.com".into();
        a.commit_field_edit();
        assert!(a.settings_dirty);
    }

    #[test]
    fn notify_content_toggles_and_round_trips() {
        let mut a = app();
        assert!(a.alerts.notify_content); // detailed by default
        assert_eq!(a.field_value(Field::NotifyContent), "value + state");
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::NotifyContent)
            .unwrap();
        a.settings_adjust(1);
        assert!(!a.alerts.notify_content);
        assert_eq!(a.field_value(Field::NotifyContent), "generic (no data)");
        assert_eq!(a.build_config().alerts.notify_content, Some(false));
    }

    /// CLAUDE.md's rule: anything user-editable must round-trip through
    /// `build_config`. This asserts it instead of trusting the reviewer to
    /// notice a field that was added to the settings screen but never written.
    #[test]
    fn every_edited_setting_round_trips_through_the_config() {
        let mut a = app();
        // Move every value off its default, in the direction the UI allows.
        a.units = Units::Mgdl;
        a.refresh_secs = 45;
        a.alerts.desktop = false;
        a.alerts.notify_content = false;
        a.alerts.sound = false;
        a.alerts.snooze_minutes = 7;
        a.alerts.quiet_start = Some(23 * 60);
        a.alerts.quiet_end = Some(7 * 60);
        a.alerts.quiet_urgent_low = false;
        a.alerts.escalate_minutes = 20;
        a.alerts.push_url = Some("https://ntfy.sh/topic".into());
        a.alerts.push_enabled = false;
        a.alerts.predict_horizon_minutes = 45;
        a.alerts.urgent_low = 50.0;
        a.alerts.low = 72.0;
        a.alerts.high = 170.0;
        a.alerts.urgent_high = 240.0;
        a.alerts.stale_minutes = 9;
        a.graph_style = GraphStyle::Blocks;
        a.agp_days = 30;
        a.minimap_enabled = false;
        a.minimap_span_ms = 48 * MS_PER_HOUR;
        a.sites[0].url = "https://ns.example.com".into();
        a.sites[0].token = "tok".into();

        // Serialize as `w` does, then read it back the way startup does.
        let written = toml::to_string_pretty(&a.build_config()).unwrap();
        let cfg: Config = toml::from_str(&written).unwrap();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();

        assert_eq!(cfg.units, a.units);
        assert_eq!(cfg.refresh_secs, a.refresh_secs);
        assert_eq!(cfg.graph_style, a.graph_style);
        assert_eq!(cfg.agp_days, a.agp_days);
        assert_eq!(cfg.minimap.enabled, a.minimap_enabled);
        assert_eq!(
            cfg.minimap.span_hours as i64,
            a.minimap_span_ms / MS_PER_HOUR
        );
        assert_eq!(sites[0].url, a.sites[0].url);
        assert_eq!(sites[0].token, a.sites[0].token);

        assert_eq!(alerts.desktop, a.alerts.desktop);
        assert_eq!(alerts.notify_content, a.alerts.notify_content);
        assert_eq!(alerts.sound, a.alerts.sound);
        assert_eq!(alerts.snooze_minutes, a.alerts.snooze_minutes);
        assert_eq!(alerts.quiet_start, a.alerts.quiet_start);
        assert_eq!(alerts.quiet_end, a.alerts.quiet_end);
        assert_eq!(alerts.quiet_urgent_low, a.alerts.quiet_urgent_low);
        assert_eq!(alerts.escalate_minutes, a.alerts.escalate_minutes);
        assert_eq!(alerts.push_url, a.alerts.push_url);
        assert_eq!(alerts.push_enabled, a.alerts.push_enabled);
        assert_eq!(
            alerts.predict_horizon_minutes,
            a.alerts.predict_horizon_minutes
        );
        assert_eq!(alerts.stale_minutes, a.alerts.stale_minutes);
        // Thresholds are stored in mg/dL and written in display units, so they
        // survive the conversion round trip rather than being bit-identical.
        for (got, want) in [
            (alerts.urgent_low, a.alerts.urgent_low),
            (alerts.low, a.alerts.low),
            (alerts.high, a.alerts.high),
            (alerts.urgent_high, a.alerts.urgent_high),
        ] {
            assert!((got - want).abs() < 0.5, "{got} != {want}");
        }
    }

    /// Every settings row must render something and belong to a section — a
    /// row added to `Field::ALL` without a `field_value` arm shows up here.
    #[test]
    fn every_settings_row_renders() {
        let a = app();
        let mut seen: Vec<Field> = Vec::new();
        for f in Field::ALL {
            assert!(!seen.contains(&f), "{f:?} appears twice in Field::ALL");
            seen.push(f);
            assert!(!f.label().is_empty(), "{f:?} has no label");
            assert!(!f.group().is_empty(), "{f:?} has no group");
            assert!(!a.field_value(f).is_empty(), "{f:?} renders nothing");
        }
    }

    /// End to end: a real 401 from a real socket must pause fetching, and a
    /// real 500 must not. This is the join between the client's error
    /// classification and the app's retry policy — each half was tested, the
    /// seam between them wasn't.
    #[tokio::test]
    async fn the_retry_policy_matches_what_the_server_actually_said() {
        use crate::nightscout::{fake, Client};

        let auth = fake::serve(401, "unauthorized").await;
        let client = Client::for_site(&auth).unwrap();
        let mut a = app();
        for _ in 0..3 {
            let err = client.entries_range(0, 1, 1).await.unwrap_err();
            let permanent = err
                .downcast_ref::<crate::nightscout::FetchError>()
                .is_some();
            a.mark_offline(NOW, err.to_string(), permanent);
        }
        assert!(a.fetch_paused, "a rejected token should stop the retries");
        assert!(!a.should_auto_refresh());

        let flaky = fake::serve(500, "boom").await;
        let client = Client::for_site(&flaky).unwrap();
        let mut b = app();
        for _ in 0..5 {
            let err = client.entries_range(0, 1, 1).await.unwrap_err();
            let permanent = err
                .downcast_ref::<crate::nightscout::FetchError>()
                .is_some();
            b.mark_offline(NOW, err.to_string(), permanent);
        }
        assert!(
            !b.fetch_paused,
            "a restarting site should keep being retried"
        );
        assert!(b.should_retry(NOW + 120_000));
    }

    #[test]
    fn the_push_names_the_site_and_honours_the_privacy_switch() {
        let mut a = app();
        a.alerts.push_url = Some("https://ntfy.sh/topic".into());
        a.sites.push(crate::config::Site {
            name: "bob".into(),
            url: "https://ns.example.com".into(),
            token: "t".into(),
        });
        a.entries = vec![entry(40.0, NOW)];
        a.live_edge = Some(entry(40.0, NOW));
        a.evaluate_alert(NOW);
        a.update_urgent(NOW);

        let msg = a.take_push(NOW).expect("no push");
        assert!(msg.contains("URGENT LOW"), "{msg}");
        // Whose low is it? The only channel that reaches a phone must say.
        assert!(msg.contains("[default]"), "no site name: {msg}");
        // Demo config displays mmol/L, so 40 mg/dL reads as 2.2.
        assert!(msg.contains("2.2"), "{msg}");

        // With content off, the reading must not leave the machine.
        let mut b = app();
        b.alerts.push_url = Some("https://ntfy.sh/topic".into());
        b.alerts.notify_content = false;
        b.entries = vec![entry(40.0, NOW)];
        b.live_edge = Some(entry(40.0, NOW));
        b.evaluate_alert(NOW);
        b.update_urgent(NOW);
        let msg = b.take_push(NOW).expect("no push");
        assert!(msg.contains("URGENT LOW"), "{msg}");
        assert!(!msg.contains("2.2"), "the reading leaked: {msg}");
        assert!(!msg.contains("mmol"), "the unit leaked: {msg}");
    }

    #[test]
    fn an_unencrypted_webhook_is_flagged_in_settings() {
        let mut a = app();
        a.alerts.push_url = Some("http://ntfy.example.com/topic".into());
        assert!(a.field_value(Field::PushAlerts).contains("⚠ unencrypted"));
        // The topic path is still never shown.
        assert!(!a.field_value(Field::PushAlerts).contains("topic"));

        a.alerts.push_url = Some("https://ntfy.example.com/topic".into());
        assert!(!a.field_value(Field::PushAlerts).contains("unencrypted"));
    }

    #[test]
    fn exports_are_owner_only_and_report_where_they_went() {
        let dir = std::env::temp_dir().join(format!("sugarrush-exp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = app();
        let entries = vec![entry(100.0, NOW), entry(105.0, NOW - 300_000)];
        let paths = crate::export::write_pair(&dir, &entries, &a.alerts, a.units, 14, NOW).unwrap();

        assert_eq!(paths.len(), 2);
        for p in &paths {
            assert!(p.is_absolute(), "not an absolute path: {}", p.display());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(p).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600, "{} is {mode:o}", p.display());
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transient_failures_keep_retrying() {
        let mut a = app();
        for _ in 0..10 {
            a.mark_offline(NOW, "connection refused".into(), false);
        }
        assert!(!a.fetch_paused);
        assert!(a.should_retry(NOW + 120_000));
    }
}
