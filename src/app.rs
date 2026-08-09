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

#[path = "settings.rs"]
mod settings;
pub use settings::{Field, FieldEdit, SettingsExit};

#[path = "alert_engine.rs"]
mod alert_engine;
use alert_engine::{AlertEngine, PushDue};

#[path = "fetch_state.rs"]
mod fetch_state;
use fetch_state::FetchState;
#[cfg(test)]
use fetch_state::CONFIG_FAIL_LIMIT;

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
    /// First visible person in the worst-first follower list.
    pub follower_scroll: usize,
    /// Selected follower identity. The list is severity-sorted and can reorder
    /// after every refresh, so an index could act on the wrong person.
    pub follower_selected: Option<String>,
    /// Settings have been changed but not written back to `config.toml`.
    /// Every edit applies live, so without this there's nothing to distinguish
    /// "changed and saved" from "changed and lost on quit".
    pub settings_dirty: bool,
    /// Last successfully loaded/saved configuration, used to make Discard a
    /// real rollback rather than merely hiding the dirty marker.
    pub settings_baseline: Config,
    pub settings_exit: Option<SettingsExit>,
    /// Forecast points `(epoch_ms, mg/dL)`, live mode only.
    pub predictions: Vec<Prediction>,
    /// Uploader/device metadata + IOB/COB (live mode only).
    pub device: DeviceStatus,
    /// Carb/insulin treatments within the current window.
    pub treatments: Vec<Treatment>,
    /// Epoch ms of the latest sensor start/change, if known.
    /// Result of the last in-app alarm self-test, shown on its settings row.
    /// "Audible alarm: on" is a claim about a config field; this is a claim
    /// about whether this machine can make a noise.
    pub alarm_test: Option<String>,
    pub sensor_start_ms: Option<i64>,
    /// When the sensor-start lookup last ran. A sensor lasts ten to fourteen
    /// days, so asking every refresh cost a second `/treatments` request per
    /// cycle for a number that changes twice a month.
    pub sensor_fetched_ms: i64,
    /// Configured alert thresholds and behaviour (mg/dL internally).
    pub alerts: Alerts,
    /// Top-level defaults and optional resolved overrides, parallel to `sites`.
    global_alerts: Alerts,
    site_alerts: Vec<Option<Alerts>>,
    /// Current alert state (only meaningful in live mode).
    pub alert: Alert,
    /// State that spans alert passes: episode identity, debounce, escalation,
    /// predictive notification, and snoozing.
    alert_engine: AlertEngine,

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
    pub site_validated: Vec<bool>,
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
    /// Connection health, errors, and retry/backoff state.
    fetch: FetchState,
    /// Set when a desktop notification could not be delivered — no
    /// notification daemon, or D-Bus refused it.
    pub notify_failed: bool,
    pub should_quit: bool,
    /// Whether the keybinding help overlay is showing.
    pub show_help: bool,
}

/// Whether the alarm can reach you *right now*, in one phrase.
///
/// The review's headline finding was that the app never told you the state of
/// your own safety net: whether it was armed, whether quiet hours or a snooze
/// was suppressing it, whether the watcher was even running, whether escalation
/// had a channel. Four separate silences, each invisible. This is the one
/// answer, and it is drawn in the header where it survives every error state —
/// unlike the footer, which an error used to take over completely.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Armed {
    /// Nothing is configured to announce anything.
    Off,
    /// A snooze someone set, with minutes remaining.
    Snoozed(i64),
    /// Inside quiet hours: minute-of-day it ends, and whether urgent lows
    /// still sound.
    Quiet { until: i32, urgent_low_only: bool },
    /// A watcher was running and isn't any more.
    WatcherStopped,
    /// Armed, and a headless watcher is up.
    Watching,
    /// Armed.
    Ready,
}

impl Armed {
    /// The chip text. Deliberately says "alarm" every time: the word is what
    /// makes the chip findable when someone is looking for exactly this.
    pub fn label(self) -> String {
        let hhmm = |m: i32| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
        match self {
            Armed::Off => " ⚑ alarm off ".into(),
            Armed::Snoozed(mins) => format!(" ⏸ alarm snoozed · {mins}m left "),
            Armed::Quiet {
                until,
                urgent_low_only: true,
            } => format!(" ☾ quiet until {} · urgent lows only ", hhmm(until)),
            Armed::Quiet {
                until,
                urgent_low_only: false,
            } => format!(" ☾ quiet until {} · all alarms silent ", hhmm(until)),
            Armed::WatcherStopped => " ⚠ watcher stopped ".into(),
            Armed::Watching => " ⚑ alarm armed · watcher up ".into(),
            Armed::Ready => " ⚑ alarm armed ".into(),
        }
    }

    /// Whether this state means something is suppressing or breaking the alarm.
    pub fn is_suppressed(self) -> bool {
        !matches!(self, Armed::Ready | Armed::Watching)
    }
}

/// Everything one pass of the alarm machine decided to announce.
///
/// The reaction sequence used to be written out twice — once in the TUI's
/// `apply`, once in `watch::react` — and a third partial copy on the TUI's
/// 3-second alarm ticker. They had already drifted: the daemon called
/// `update_urgent` before consuming the notification and the TUI called it
/// after, and the TUI only consumed a notification when desktop notifications
/// were *enabled*, so switching them on could fire a queued announcement for an
/// episode that ended hours earlier.
///
/// `App::react` is now the only path. It decides *what* happened; each front
/// end decides how to deliver it (journal line, desktop toast, webhook, sound).
#[derive(Debug, Clone, PartialEq)]
pub struct Reaction {
    /// The state after this pass.
    pub state: Alert,
    /// A desktop notification is due for this alert.
    pub notification: Option<Alert>,
    /// A predictive ("heading low") warning is due.
    pub predictive: Option<String>,
    /// A webhook POST is due: the URL and the message.
    pub push: Option<(String, String)>,
    /// An alerting episode ended on this pass. A journal with "URGENT LOW" and
    /// nothing after it doesn't say whether it lasted two minutes or all night.
    pub recovered: bool,
    /// The audible alarm should be sounding right now.
    pub sound: bool,
}

impl App {
    pub fn new(cfg: &Config, alerts: Alerts, sites: Vec<Site>) -> Self {
        let site_count = sites.len();
        let site_alerts: Vec<Option<Alerts>> = sites
            .iter()
            .map(|site| {
                site.alerts
                    .as_ref()
                    .map(|_| site.resolve_alerts(&cfg.alerts, cfg.units).0)
            })
            .collect();
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
            follower_scroll: 0,
            follower_selected: None,
            settings_dirty: false,
            settings_baseline: cfg.clone(),
            settings_exit: None,
            predictions: Vec::new(),
            device: DeviceStatus::default(),
            treatments: Vec::new(),
            alarm_test: None,
            sensor_start_ms: None,
            sensor_fetched_ms: 0,
            alerts: site_alerts
                .first()
                .and_then(Clone::clone)
                .unwrap_or_else(|| alerts.clone()),
            global_alerts: alerts,
            site_alerts,
            alert: Alert::InRange,
            alert_engine: AlertEngine::default(),
            screen: Screen::Dashboard,
            settings_sel: 0,
            refresh_secs: cfg.refresh_secs,
            refresh_dirty: false,
            status: None,
            theme: cfg.theme.resolve(),
            theme_names: names_from_config(&cfg.theme),
            sites,
            site_validated: vec![true; site_count],
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
            fetch: FetchState::default(),
            notify_failed: false,
            should_quit: false,
            show_help: false,
        }
    }

    /// Note which supplementary fetches failed on this refresh (empty clears).
    pub fn set_partial(&mut self, missing: &[&str]) {
        self.fetch.set_partial(missing);
    }

    /// Record a successful fetch.
    pub fn mark_online(&mut self, now_ms: i64) {
        self.fetch.mark_online(now_ms);
    }

    /// Record a failed fetch and schedule a backoff retry (5s → 60s).
    ///
    /// `permanent` marks a failure that repeating the request can't fix (a bad
    /// token or URL). After [`CONFIG_FAIL_LIMIT`] of those in a row, automatic
    /// fetching pauses: retrying a rejected token every few seconds only risks
    /// tripping server-side rate limits, and the fix is in the user's hands.
    pub fn mark_offline(&mut self, now_ms: i64, err: String, permanent: bool) {
        self.fetch.mark_offline(now_ms, err, permanent);
    }

    /// Resume fetching after a pause — on an explicit refresh, or when the site
    /// config changes (a new site, or an edited URL/token).
    pub fn resume_fetching(&mut self) {
        self.fetch.resume();
    }

    /// True when an offline connection is due for a backoff retry.
    ///
    /// Unlike the periodic refresh this doesn't require the live edge: reading
    /// history while the connection is down otherwise left it down, so
    /// returning to live meant an offline dashboard until the next manual
    /// refresh. A retry re-fetches whatever window is shown.
    pub fn should_retry(&self, now_ms: i64) -> bool {
        self.fetch.should_retry(now_ms)
    }

    /// True when the periodic refresh should run (paused after a config error).
    pub fn should_auto_refresh(&self) -> bool {
        self.view.is_live() && !self.fetch.fetch_paused()
    }

    pub fn online(&self) -> bool {
        self.fetch.online()
    }

    pub fn last_ok_ms(&self) -> Option<i64> {
        self.fetch.last_ok_ms()
    }

    pub fn fetch_paused(&self) -> bool {
        self.fetch.fetch_paused()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.fetch.last_error()
    }

    pub fn set_last_error(&mut self, error: String) {
        self.fetch.set_last_error(error);
    }

    pub fn partial(&self) -> Option<&str> {
        self.fetch.partial()
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

    pub fn site_alert_override(&self) -> bool {
        self.site_alerts
            .get(self.site_idx)
            .is_some_and(Option::is_some)
    }

    pub fn set_site_alert_override(&mut self, enabled: bool) {
        let idx = self.site_idx.min(self.sites.len().saturating_sub(1));
        if enabled {
            self.site_alerts[idx] = Some(self.alerts.clone());
        } else {
            self.site_alerts[idx] = None;
            self.alerts = self.global_alerts.clone();
        }
    }

    pub fn sync_active_alerts(&mut self) {
        let idx = self.site_idx.min(self.sites.len().saturating_sub(1));
        if self.site_alerts[idx].is_some() {
            self.site_alerts[idx] = Some(self.alerts.clone());
        } else {
            self.global_alerts = self.alerts.clone();
        }
    }

    fn load_active_alerts(&mut self) {
        self.alerts = self
            .site_alerts
            .get(self.site_idx)
            .and_then(Clone::clone)
            .unwrap_or_else(|| self.global_alerts.clone());
    }

    pub fn alerts_for_site(&self, idx: usize) -> Alerts {
        self.site_alerts
            .get(idx)
            .and_then(Clone::clone)
            .unwrap_or_else(|| self.global_alerts.clone())
    }

    /// Switch to the next configured site (no-op with a single site).
    pub fn next_site(&mut self) {
        if self.sites.len() > 1 {
            self.sync_active_alerts();
            self.site_idx = (self.site_idx + 1) % self.sites.len();
            self.load_active_alerts();
            self.site_dirty = true;
            self.view.follow();
        }
    }

    /// Activate a site by stable configured identity.
    pub fn activate_site(&mut self, name: &str) -> bool {
        let Some(index) = self.sites.iter().position(|site| site.name == name) else {
            return false;
        };
        self.sync_active_alerts();
        self.site_idx = index;
        self.load_active_alerts();
        self.site_dirty = true;
        self.view.follow();
        true
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
        self.follower_scroll = 0;
        self.follower_selected = self.followers.first().map(|site| site.name.clone());
    }

    pub fn scroll_followers(&mut self, delta: isize) {
        if self.followers.is_empty() {
            self.follower_selected = None;
            return;
        }
        let current = self
            .follower_selected
            .as_ref()
            .and_then(|name| self.followers.iter().position(|site| &site.name == name))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.followers.len().saturating_sub(1));
        self.follower_selected = Some(self.followers[next].name.clone());
        self.follower_scroll = next;
    }

    pub fn selected_follower(&self) -> Option<&str> {
        self.follower_selected
            .as_deref()
            .or_else(|| self.followers.first().map(|site| site.name.as_str()))
    }

    pub fn select_follower_edge(&mut self, last: bool) {
        let selected = if last {
            self.followers.last()
        } else {
            self.followers.first()
        };
        self.follower_selected = selected.map(|site| site.name.clone());
        self.follower_scroll = if last {
            self.followers.len().saturating_sub(1)
        } else {
            0
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
                None if self.last_ok_ms().is_some() => Alert::Stale,
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
        self.alert_engine.transition_to(self.alert);
        self.alert
    }

    /// True when the audible alarm should currently sound.
    pub fn alarm_active(&self, now_ms: i64) -> bool {
        if !(self.alerts.sound && self.alert.is_urgent()) {
            return false;
        }
        if self.alert_engine.snooze_until().is_some_and(|t| now_ms < t) {
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
                if self.alert_engine.predictive_was_notified() {
                    return None;
                }
                self.alert_engine.set_predictive_notified(true);
                let dir = if rising { "high" } else { "low" };
                Some(format!("heading {dir} in ~{mins} min"))
            }
            _ => {
                self.alert_engine.set_predictive_notified(false);
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
        self.alert_engine
            .snooze_until()
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
            self.alert_engine.snooze(now_ms + mins * 60_000);
            self.status = Some(format!("alarm snoozed {mins}m"));
        }
    }

    /// The one-phrase answer to "is my safety net armed?".
    ///
    /// Ordered by what actually stops an alarm reaching someone, worst first,
    /// so the chip always names the most suppressing condition rather than the
    /// first one that happens to be true.
    pub fn armed_state(&self, now_ms: i64) -> Armed {
        // Nothing switched on to announce with. Escalation doesn't count: its
        // only channel is the push webhook, which is already in this list.
        let has_channel = self.alerts.sound
            || self.alerts.desktop
            || (self.alerts.push_url.is_some() && self.alerts.push_enabled);
        if !has_channel {
            return Armed::Off;
        }
        if let Some(mins) = self.snooze_remaining_min(now_ms) {
            return Armed::Snoozed(mins);
        }
        if let Some(dt) = Local.timestamp_millis_opt(now_ms).single() {
            let min_of_day = dt.hour() as i32 * 60 + dt.minute() as i32;
            if self.alerts.in_quiet_hours(min_of_day) {
                return Armed::Quiet {
                    until: self.alerts.quiet_end.unwrap_or(min_of_day),
                    urgent_low_only: self.alerts.quiet_urgent_low,
                };
            }
        }
        if self.watcher_seen && !self.watcher_alive {
            return Armed::WatcherStopped;
        }
        if self.watcher_alive {
            return Armed::Watching;
        }
        Armed::Ready
    }

    /// Run one pass of the alarm machine and report what to announce.
    ///
    /// Every consumer of a pending announcement lives here, and consumption is
    /// unconditional — a front end that can't deliver something drops it rather
    /// than leaving it queued for later. This is called on every refresh *and*
    /// on the TUI's 3-second tick, so a transition is announced within seconds
    /// rather than waiting out the refresh interval.
    pub fn react(&mut self, now_ms: i64) -> Reaction {
        let state = self.evaluate_alert(now_ms);
        // Before the announcements: `take_push` reads the episode timers this
        // maintains, so escalation can fire on the pass that earns it.
        self.update_urgent(now_ms);

        let recovered = self.alert_engine.record_state(state);

        Reaction {
            state,
            notification: self.take_notification(),
            predictive: self.take_predictive(now_ms),
            push: self
                .take_push(now_ms)
                .zip(self.alerts.push_url.clone())
                .map(|(msg, url)| (url, msg)),
            recovered,
            sound: self.alarm_active(now_ms),
        }
    }

    /// Track the urgent-episode lifecycle used for escalation and push.
    pub fn update_urgent(&mut self, now_ms: i64) {
        self.alert_engine.update_urgent(self.alert, now_ms);
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
        match self
            .alert_engine
            .take_push(self.alert, now_ms, self.alerts.escalate_minutes)?
        {
            PushDue::Onset => Some(format!("sugarrush: {who}{}{value}", self.alert.label())),
            PushDue::Escalation => Some(format!(
                "sugarrush: {who}STILL {} after {} min{value}",
                self.alert.label(),
                self.alerts.escalate_minutes,
            )),
        }
    }

    /// If the alert level changed into an alerting state since the last desktop
    /// notification, return it (once) and record it. Returning to range or
    /// staying at the same level yields `None`, debouncing repeats.
    pub fn take_notification(&mut self) -> Option<Alert> {
        self.alert_engine.take_notification(self.alert)
    }

    // ---- Episode state, for the headless watcher ----
    //
    // `sugarrush watch` persists these across restarts so a restarted service
    // doesn't re-announce an ongoing low, restart an escalation timer, or
    // cancel a snooze someone deliberately set. They're read-only accessors
    // plus one restore hook rather than public fields, so the engine keeps the
    // invariants together.

    pub fn last_notified(&self) -> Option<Alert> {
        self.alert_engine.last_notified()
    }

    pub fn urgent_since(&self) -> Option<i64> {
        self.alert_engine.urgent_since()
    }

    pub fn episode_kind(&self) -> Option<Alert> {
        self.alert_engine.episode_kind()
    }

    pub fn pushed_episode(&self) -> bool {
        self.alert_engine.pushed_episode()
    }

    pub fn escalated(&self) -> bool {
        self.alert_engine.escalated()
    }

    pub fn snooze_until(&self) -> Option<i64> {
        self.alert_engine.snooze_until()
    }

    /// Adopt a snooze set from outside this process — `sugarrush snooze`
    /// writing the daemon's state file, or the dashboard handing one over.
    pub fn set_snooze(&mut self, until: Option<i64>) {
        self.alert_engine.set_snooze(until);
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
        self.alert_engine.restore(
            last_notified,
            urgent_since,
            pushed_episode,
            escalated,
            snooze_until,
            episode_kind,
        );
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

    /// A snooze set from outside the process — `sugarrush snooze` writing the
    /// daemon's state file — has to actually silence the alarm, not just be
    /// recorded.
    #[test]
    fn an_externally_set_snooze_silences_the_alarm() {
        let mut a = app();
        a.alerts.sound = true;
        a.entries = vec![entry(40.0, NOW)];
        a.react(NOW);
        assert!(a.alarm_active(NOW), "an urgent low should sound");

        a.set_snooze(Some(NOW + 15 * 60_000));
        assert!(!a.alarm_active(NOW), "…and be silenced by a snooze");

        // …and come back when the snooze runs out, not stay silent.
        assert!(
            a.alarm_active(NOW + 16 * 60_000),
            "the alarm must re-arm when the snooze expires"
        );
    }

    /// The review's headline: the app never told you the state of your own
    /// safety net. Four separate silences, each invisible — quiet hours, a
    /// snooze, a stopped watcher, and an alarm with nothing switched on to
    /// announce with. One phrase answers all four, and it names the most
    /// suppressing condition rather than the first that happens to be true.
    #[test]
    fn the_armed_chip_names_the_worst_thing_suppressing_the_alarm() {
        let mut a = app();
        a.alerts.sound = true;
        a.alerts.desktop = false;
        a.alerts.push_url = None;
        a.alerts.quiet_start = None;
        a.alerts.quiet_end = None;

        assert_eq!(a.armed_state(NOW), Armed::Ready);
        assert!(!a.armed_state(NOW).is_suppressed());

        // A watcher that was up and went away outranks "armed".
        a.watcher_alive = true;
        assert_eq!(a.armed_state(NOW), Armed::Watching);
        a.watcher_seen = true;
        a.watcher_alive = false;
        assert_eq!(a.armed_state(NOW), Armed::WatcherStopped);

        // A snooze outranks a stopped watcher: it's the nearer cause.
        a.set_snooze(Some(NOW + 12 * 60_000));
        assert!(matches!(a.armed_state(NOW), Armed::Snoozed(_)));

        // And nothing switched on at all outranks everything.
        a.alerts.sound = false;
        assert_eq!(a.armed_state(NOW), Armed::Off);
        assert!(a.armed_state(NOW).is_suppressed());
    }

    /// Quiet hours mute the alarm on a schedule with no on-screen evidence —
    /// and the chip has to distinguish "urgent lows still sound" from "nothing
    /// sounds", because those are very different nights.
    #[test]
    fn quiet_hours_say_whether_anything_still_sounds() {
        let mut a = app();
        a.alerts.sound = true;
        // A window anchored to NOW's own local time, so the test doesn't
        // depend on the machine's timezone.
        let min_of_day = Local
            .timestamp_millis_opt(NOW)
            .single()
            .map(|d| d.hour() as i32 * 60 + d.minute() as i32)
            .unwrap();
        a.alerts.quiet_start = Some(min_of_day.saturating_sub(60));
        a.alerts.quiet_end = Some((min_of_day + 60) % (24 * 60));
        a.alerts.quiet_urgent_low = true;

        let state = a.armed_state(NOW);
        let Armed::Quiet {
            until,
            urgent_low_only,
        } = state
        else {
            panic!("expected quiet hours, got {state:?}");
        };
        assert_eq!(until, (min_of_day + 60) % (24 * 60));
        assert!(urgent_low_only);
        assert!(
            state.label().contains("urgent lows only"),
            "{}",
            state.label()
        );
        let clock = format!("{:02}:{:02}", (until / 60) % 24, until % 60);
        assert!(state.label().contains(&clock), "{}", state.label());

        a.alerts.quiet_urgent_low = false;
        assert!(a.armed_state(NOW).label().contains("all alarms silent"));
    }

    /// An episode that spans a restart still has an end. The daemon restores
    /// what it last announced, and that is what makes the recovery reportable —
    /// without it the journal showed a low beginning and nothing after it.
    #[test]
    fn an_episode_that_survives_a_restart_still_reports_its_recovery() {
        let mut a = app();
        a.restore_episode(
            Some(Alert::UrgentLow),
            Some(NOW),
            true,
            false,
            None,
            Some(Alert::UrgentLow),
        );

        // Back in range after the restart.
        a.entries = vec![entry(100.0, NOW + 60_000)];
        let r = a.react(NOW + 60_000);
        assert_eq!(r.state, Alert::InRange);
        assert!(
            r.recovered,
            "the low that was running before the restart has ended"
        );
    }

    /// The audible alarm and the notification channels used to run on separate
    /// clocks. The dashboard's 3-second ticker classified and sounded but never
    /// consumed a notification or a push, so a sensor gap that crossed into
    /// Stale between refreshes started beeping immediately while the desktop
    /// notification and the escalation webhook waited out the whole refresh
    /// interval — 60 seconds by default, and as long as the user configured.
    ///
    /// One pass, one decision: whatever sounds also announces.
    #[test]
    fn a_gap_announces_on_the_same_pass_that_sounds_it() {
        let mut a = app();
        a.alerts.push_url = Some("http://example.invalid/hook".into());
        a.alerts.push_enabled = true;
        // A reading that was fine when it arrived.
        a.entries = vec![entry(100.0, NOW)];
        let r = a.react(NOW);
        assert_eq!(r.state, Alert::InRange);
        assert!(!r.sound);

        // Nothing new arrives. Well past the staleness threshold, the *same*
        // pass that starts the alarm must also produce the announcements —
        // no fetch has happened, and none is needed to know this.
        let later = NOW + (a.alerts.stale_minutes + 5) * 60_000;
        let r = a.react(later);
        assert_eq!(r.state, Alert::Stale);
        assert!(r.sound, "a sensor gap should sound");
        assert_eq!(
            r.notification,
            Some(Alert::Stale),
            "…and announce, on the same pass"
        );
        assert!(r.push.is_some(), "…and escalate, on the same pass");
    }

    /// Consumption is unconditional: the machine advances whether or not the
    /// front end can deliver. The dashboard used to call `take_notification`
    /// only inside `if app.alerts.desktop`, so the debounce marker went stale
    /// whenever notifications were off and the two front ends disagreed about
    /// what had already been announced.
    #[test]
    fn a_notification_is_consumed_even_when_it_cannot_be_delivered() {
        let mut a = app();
        a.alerts.desktop = false;
        a.entries = vec![entry(40.0, NOW)];

        let r = a.react(NOW);
        assert_eq!(r.notification, Some(Alert::UrgentLow));

        // Switching desktop notifications on must not resurrect it.
        a.alerts.desktop = true;
        assert_eq!(
            a.react(NOW).notification,
            None,
            "an announcement already consumed must not fire again later"
        );
    }

    /// `take_push` reads the episode timers `update_urgent` maintains, so the
    /// order between them is load-bearing and now has exactly one definition.
    #[test]
    fn escalation_fires_on_the_pass_that_earns_it() {
        let mut a = app();
        a.alerts.push_url = Some("http://example.invalid/hook".into());
        a.alerts.push_enabled = true;
        a.alerts.escalate_minutes = 10;
        a.entries = vec![entry(40.0, NOW)];

        // Onset: the first push.
        let r = a.react(NOW);
        assert!(r.push.is_some(), "urgent onset should push");

        // Still urgent, nothing new to say.
        a.entries = vec![entry(40.0, NOW + 60_000)];
        assert!(a.react(NOW + 60_000).push.is_none());

        // Ten minutes in, unacknowledged: the escalation, on this pass.
        a.entries = vec![entry(40.0, NOW + 600_000)];
        let r = a.react(NOW + 600_000);
        let (_, msg) = r.push.expect("escalation should push");
        assert!(msg.contains("STILL"), "got {msg:?}");
    }

    /// An episode ending is reportable exactly once, and only if one was
    /// actually running.
    #[test]
    fn recovery_is_reported_once_and_only_after_an_alarm() {
        let mut a = app();

        // In range from the start: nothing recovered.
        a.entries = vec![entry(100.0, NOW)];
        assert!(!a.react(NOW).recovered);

        a.entries = vec![entry(40.0, NOW + 60_000)];
        assert!(!a.react(NOW + 60_000).recovered);

        a.entries = vec![entry(100.0, NOW + 120_000)];
        assert!(a.react(NOW + 120_000).recovered, "the low ended");

        a.entries = vec![entry(100.0, NOW + 180_000)];
        assert!(
            !a.react(NOW + 180_000).recovered,
            "recovery is news once, not every pass afterwards"
        );
    }

    /// Two settings rows answer ←/→ with a hint instead of a change. The tail
    /// of `settings_adjust` used to clear the status unconditionally, so both
    /// hints were written and then erased before any frame drew them — dead
    /// code that read as a dead key.
    #[test]
    fn a_row_that_cannot_be_adjusted_says_so() {
        let mut a = app();
        a.screen = Screen::Settings;

        for (field, expect) in [
            (Field::SiteUrl, "press enter to edit"),
            (Field::PushAlerts, "set push_url"),
        ] {
            a.status = None;
            a.settings_sel = Field::ALL.iter().position(|f| *f == field).unwrap();
            a.settings_adjust(1);
            let status = a.status.clone().unwrap_or_default();
            assert!(
                status.contains(expect),
                "{field:?} should explain itself, got {status:?}"
            );
        }

        // And a row that *does* change still clears the previous message.
        a.status = Some("stale".into());
        a.settings_sel = Field::ALL.iter().position(|f| *f == Field::Snooze).unwrap();
        a.settings_adjust(1);
        assert_eq!(a.status, None);
    }

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
        a.mark_online(NOW);
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
        a.mark_online(NOW);
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
        assert!(a.fetch_paused());
        assert!(!a.should_retry(NOW + 60_000));
        assert!(!a.should_auto_refresh());
        // An explicit retry (r) resumes, and a success clears the state.
        a.resume_fetching();
        assert!(a.should_auto_refresh());
        a.mark_offline(NOW, "authentication failed".into(), true);
        a.mark_online(NOW);
        assert!(!a.fetch_paused());
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
    fn push_url_edit_is_masked_replaceable_and_clearable() {
        let mut a = app();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::PushUrl)
            .unwrap();
        assert!(a.begin_field_edit());
        assert!(a.field_edit.as_ref().unwrap().masked);
        assert!(a.field_edit.as_ref().unwrap().buffer.is_empty());
        a.field_edit.as_mut().unwrap().buffer = "https://ntfy.sh/private-topic".into();
        a.commit_field_edit();
        assert_eq!(a.field_value(Field::PushUrl), "set · hidden");
        assert!(!a.field_value(Field::PushUrl).contains("private-topic"));
        assert!(a.alerts.push_enabled);

        assert!(a.begin_field_edit());
        a.field_edit.as_mut().unwrap().buffer = "off".into();
        a.commit_field_edit();
        assert!(a.alerts.push_url.is_none());
        assert!(!a.alerts.push_enabled);
    }

    #[test]
    fn edited_sites_must_pass_a_fresh_reading_test_before_save() {
        let mut a = app();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteToken)
            .unwrap();
        a.begin_field_edit();
        a.field_edit.as_mut().unwrap().buffer = "replacement".into();
        a.commit_field_edit();
        assert!(!a.site_validated[a.site_idx]);
        assert!(
            !a.save_config(),
            "an untested credential must not be persisted"
        );
        assert!(a.status.as_deref().unwrap().contains("test every"));
    }

    #[test]
    fn dirty_settings_require_a_decision_and_discard_really_rolls_back() {
        let mut a = app();
        a.screen = Screen::Settings;
        let before = a.alerts.low;
        a.settings_sel = Field::ALL.iter().position(|&f| f == Field::Low).unwrap();
        a.settings_adjust(1);
        assert_ne!(a.alerts.low, before);

        a.request_settings_exit(SettingsExit::Back);
        assert_eq!(a.settings_exit, Some(SettingsExit::Back));
        assert_eq!(
            a.screen,
            Screen::Settings,
            "the dialog must keep settings open"
        );
        a.cancel_settings_exit();
        assert!(a.settings_exit.is_none());

        a.request_settings_exit(SettingsExit::Back);
        a.discard_settings();
        a.finish_settings_exit(SettingsExit::Back);
        assert_eq!(a.alerts.low, before);
        assert!(!a.settings_dirty);
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

    #[test]
    fn sites_can_be_added_renamed_removed_and_round_trip() {
        let mut a = app();
        let original_url = a.active_site().url.clone();

        a.add_site();
        assert_eq!(a.sites.len(), 2);
        assert_eq!(a.site_idx, 1);
        assert_eq!(a.active_site().url, original_url);
        assert!(a.active_site().token.is_empty(), "a token was copied");

        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteName)
            .unwrap();
        assert!(a.begin_field_edit());
        a.field_edit.as_mut().unwrap().buffer = "alice".into();
        a.commit_field_edit();
        assert_eq!(a.active_site().name, "alice");

        let sites = a.build_config().resolve_sites().unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[1].name, "alice");

        a.remove_site();
        assert_eq!(a.sites.len(), 1);
        a.remove_site();
        assert_eq!(a.sites.len(), 1, "the final site was removed");
    }

    #[test]
    fn follower_activation_uses_stable_site_identity() {
        let mut a = app();
        let mut second = a.sites[0].clone();
        second.name = "bob".into();
        a.sites.push(second);
        a.site_alerts.push(None);
        a.site_validated.push(true);

        assert!(a.activate_site("bob"));
        assert_eq!(a.active_site().name, "bob");
        assert!(!a.activate_site("missing"));
        assert_eq!(a.active_site().name, "bob");
    }

    #[test]
    fn switching_sites_switches_alert_thresholds_and_persists_them() {
        let mut cfg = Config::demo();
        cfg.url = None;
        cfg.token = None;
        cfg.sites = vec![
            Site {
                name: "alice".into(),
                url: "https://alice.example".into(),
                token: "a".into(),
                alerts: None,
            },
            Site {
                name: "bob".into(),
                url: "https://bob.example".into(),
                token: "b".into(),
                alerts: Some(AlertsConfig {
                    low: Some(4.5),
                    ..AlertsConfig::default()
                }),
            },
        ];
        let global = cfg.alerts.resolve(cfg.units);
        let mut a = App::new(&cfg, global.clone(), cfg.resolve_sites().unwrap());
        assert_eq!(a.alerts.low, global.low);
        a.next_site();
        assert!((a.alerts.low - cfg.units.to_mgdl(4.5)).abs() < 0.1);
        a.alerts.low = cfg.units.to_mgdl(4.6);
        a.sync_active_alerts();

        let written = a.build_config();
        let sites = written.resolve_sites().unwrap();
        let bob = sites[1].resolve_alerts(&written.alerts, written.units).0;
        assert!((bob.low - cfg.units.to_mgdl(4.6)).abs() < 0.1);
        let alice = sites[0].resolve_alerts(&written.alerts, written.units).0;
        assert_eq!(alice.low, global.low);
    }

    #[test]
    fn duplicate_site_names_are_rejected() {
        let mut a = app();
        a.add_site();
        a.settings_sel = Field::ALL
            .iter()
            .position(|&f| f == Field::SiteName)
            .unwrap();
        assert!(a.begin_field_edit());
        a.field_edit.as_mut().unwrap().buffer = "default".into();
        a.commit_field_edit();
        assert!(a.field_edit.is_some());
        assert_ne!(a.active_site().name, "default");
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
            let permanent = err.is_permanent();
            a.mark_offline(NOW, err.to_string(), permanent);
        }
        assert!(a.fetch_paused(), "a rejected token should stop the retries");
        assert!(!a.should_auto_refresh());

        let flaky = fake::serve(500, "boom").await;
        let client = Client::for_site(&flaky).unwrap();
        let mut b = app();
        for _ in 0..5 {
            let err = client.entries_range(0, 1, 1).await.unwrap_err();
            let permanent = err.is_permanent();
            b.mark_offline(NOW, err.to_string(), permanent);
        }
        assert!(
            !b.fetch_paused(),
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
            alerts: None,
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
        assert!(!a.fetch_paused());
        assert!(a.should_retry(NOW + 120_000));
    }
}
