//! Headless alarm watcher.
//!
//! The TUI can only alarm while a terminal is open, which is the wrong shape
//! for the job the alarm actually does — waking someone at 3am. `sugarrush
//! watch` runs the same alert pipeline with no UI: fetch, classify, notify,
//! sound, escalate, push.
//!
//! Two things make it safe to leave running:
//!
//! - **It defers to a live TUI.** Both processes write a heartbeat; if the
//!   dashboard is on screen and fresh, the watcher stays quiet rather than
//!   double-alarming.
//! - **It remembers across restarts.** Episode state (what was notified, when
//!   the urgent episode began, whether it escalated, an active snooze) is
//!   persisted, so `systemctl restart` doesn't re-announce a low you already
//!   acknowledged — or reset an escalation timer that was about to fire.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::alert::Alert;
use crate::app::App;
use crate::config::Config;
use crate::nightscout::Client;
use crate::{now_ms, predict, sound};

/// How long a heartbeat stays trustworthy. Longer than the TUI's redraw
/// cadence and the watcher's own tick, short enough that a crashed TUI doesn't
/// silence the watcher for long.
const HEARTBEAT_STALE_MS: i64 = 30_000;

/// Which process wrote a heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tui,
    Watch,
}

impl Role {
    fn file(self) -> &'static str {
        match self {
            Role::Tui => "tui.alive",
            Role::Watch => "watch.alive",
        }
    }
}

/// Directory for liveness files: runtime state that should not survive a
/// reboot. Falls back to the temp dir when there's no `XDG_RUNTIME_DIR`.
fn runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("sugarrush")
}

/// Where episode state is persisted — this one *should* survive a restart.
fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir);
    base.join("sugarrush").join("watch.json")
}

/// Record that this process is alive, right now. Best-effort: a missing
/// heartbeat only means the other side doesn't defer to us.
pub fn heartbeat(role: Role, now_ms: i64) {
    let dir = runtime_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(role.file()), now_ms.to_string());
}

/// Remove this process's heartbeat on a clean exit, so the other side stops
/// deferring immediately instead of waiting for it to go stale.
pub fn clear_heartbeat(role: Role) {
    let _ = std::fs::remove_file(runtime_dir().join(role.file()));
}

/// True when the given role reported in recently enough to be trusted.
pub fn is_alive(role: Role, now_ms: i64) -> bool {
    let Ok(raw) = std::fs::read_to_string(runtime_dir().join(role.file())) else {
        return false;
    };
    let Ok(stamp) = raw.trim().parse::<i64>() else {
        return false;
    };
    is_fresh(stamp, now_ms)
}

/// Heartbeat freshness. A stamp from the future (clock change, another machine
/// sharing the directory) is not evidence of life.
fn is_fresh(stamp_ms: i64, now_ms: i64) -> bool {
    (0..HEARTBEAT_STALE_MS).contains(&(now_ms - stamp_ms))
}

/// Persisted episode state, keyed by site name — a caregiver watching three
/// people has three independent episodes, and a low for one of them must not
/// silence the announcement for another.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    #[serde(default)]
    pub sites: std::collections::BTreeMap<String, Episode>,
}

impl State {
    pub fn load() -> Self {
        std::fs::read_to_string(state_path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = state_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        let body = serde_json::to_string_pretty(self).context("failed to serialize watch state")?;
        std::fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))
    }
}

/// The parts of one site's alert episode worth carrying across a restart.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Episode {
    /// The alert we last notified about, so a restart mid-low is silent.
    pub last_notified: Option<String>,
    /// When the current urgent episode began (epoch ms), for escalation.
    pub urgent_since: Option<i64>,
    pub pushed_episode: bool,
    pub escalated: bool,
    /// An active snooze outlives a restart — otherwise restarting the service
    /// is a way to un-silence an alarm someone deliberately silenced.
    pub snooze_until: Option<i64>,
}

impl Episode {
    /// Read the episode state out of a running `App`.
    pub fn capture(app: &App) -> Self {
        Self {
            last_notified: app.last_notified().map(|a| a.class().to_string()),
            urgent_since: app.urgent_since(),
            pushed_episode: app.pushed_episode(),
            escalated: app.escalated(),
            snooze_until: app.snooze_until(),
        }
    }

    /// Put it back, so the restarted process continues the same episode.
    pub fn restore(&self, app: &mut App) {
        let alert = self.last_notified.as_deref().and_then(alert_from_class);
        app.restore_episode(
            alert,
            self.urgent_since,
            self.pushed_episode,
            self.escalated,
            self.snooze_until,
        );
    }
}

/// Alerts are stored by their stable kebab-case name rather than a serde enum,
/// so a future variant rename can't silently resurrect the wrong state.
fn alert_from_class(class: &str) -> Option<Alert> {
    [
        Alert::UrgentLow,
        Alert::Low,
        Alert::InRange,
        Alert::High,
        Alert::UrgentHigh,
        Alert::Stale,
    ]
    .into_iter()
    .find(|a| a.class() == class)
}

/// Run the watcher until killed.
pub async fn run() -> Result<()> {
    let cfg = Config::load()?;
    let (alerts, warnings) = cfg.alerts.resolve_checked(cfg.units);
    crate::warn_about_config(&warnings);
    let sites = cfg.resolve_sites()?;

    // One pipeline per site. A caregiver watching three people needs three
    // independent episodes — hysteresis, escalation, snooze and all — so the
    // simplest correct thing is to run the same machinery per site rather than
    // inventing a second, thinner alert path for the multi-site case.
    let state = State::load();
    let mut watched: Vec<Watched> = sites
        .iter()
        .map(|site| {
            let mut app = App::new(&cfg, alerts.clone(), vec![site.clone()]);
            if let Some(episode) = state.sites.get(&site.name) {
                episode.restore(&mut app);
            }
            Ok(Watched {
                name: site.name.clone(),
                client: Client::for_site(site)?,
                app,
                last_logged: None,
            })
        })
        .collect::<Result<_>>()?;

    println!(
        "sugarrush watch: {} · every {}s",
        watched
            .iter()
            .map(|w| format!("{} ({})", w.name, w.app.active_site().base_url()))
            .collect::<Vec<_>>()
            .join(", "),
        cfg.refresh_secs.max(5)
    );

    // Whether we're following more than one person, which changes how the
    // journal and notifications read.
    let multi = watched.len() > 1;

    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.refresh_secs.max(5)));
    // The alarm re-sounds on its own cadence while an urgent state persists,
    // independent of how often we fetch.
    let mut alarm_ticker = tokio::time::interval(Duration::from_secs(3));
    alarm_ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = now_ms();
                heartbeat(Role::Watch, now);
                let mut state = State::default();
                for w in watched.iter_mut() {
                    if let Err(e) = poll(&mut w.app, &w.client, now).await {
                        // Keep watching: an outage is exactly when the Stale
                        // alarm matters, and it can only fire if we stay alive.
                        eprintln!("sugarrush watch [{}]: {e}", w.name);
                    }
                    react(w, now, multi).await;
                    state.sites.insert(w.name.clone(), Episode::capture(&w.app));
                }
                if let Err(e) = state.save() {
                    eprintln!("sugarrush watch: {e}");
                }
            }
            _ = alarm_ticker.tick() => {
                let now = now_ms();
                // Any site in an urgent state sounds the alarm; the tone comes
                // from the worst of them, so a low is never masked by a high
                // somewhere else.
                let mut worst: Option<&App> = None;
                for w in watched.iter_mut() {
                    w.app.evaluate_alert(now);
                    w.app.update_urgent(now);
                    if w.app.alarm_active(now)
                        && worst.is_none_or(|c| severity(w.app.alert) < severity(c.alert))
                    {
                        worst = Some(&w.app);
                    }
                }
                if let Some(app) = worst {
                    if !deferring(now) {
                        sound::alarm(app.alarm_tone());
                    }
                }
            }
        }
    }
}

/// One site being watched: its own client, its own alert pipeline, its own
/// notion of what it last reported.
struct Watched {
    name: String,
    client: Client,
    app: App,
    /// What the journal last said about this site, so recoveries are logged.
    last_logged: Option<Alert>,
}

/// Ranking used to pick which site's tone to sound when several are urgent.
fn severity(alert: Alert) -> u8 {
    match alert {
        Alert::UrgentLow => 0,
        Alert::UrgentHigh => 1,
        Alert::Stale => 2,
        Alert::Low => 3,
        Alert::High => 4,
        Alert::InRange => 5,
    }
}

/// True when the dashboard is up: it is already showing and sounding this, so
/// the watcher stays out of the way.
fn deferring(now_ms: i64) -> bool {
    is_alive(Role::Tui, now_ms)
}

/// One fetch cycle: readings, and the uploader forecast that predictive alerts
/// read. Deliberately narrower than the TUI's refresh — no treatments, no AGP
/// history, no minimap. Nothing here is drawn.
async fn poll(app: &mut App, client: &Client, now_ms: i64) -> Result<()> {
    let (start, end) = app.view.bounds(now_ms);
    match client
        .entries_range(start, end, app.view.span.fetch_count())
        .await
    {
        Ok(entries) => {
            app.entries = entries;
            app.mark_online(now_ms);
        }
        Err(e) => {
            let permanent = e.downcast_ref::<crate::nightscout::FetchError>().is_some();
            app.mark_offline(now_ms, e.to_string(), permanent);
            return Err(e);
        }
    }
    let published = client.device_status().await.ok().and_then(|(status, p)| {
        app.device = status;
        p
    });
    app.predictions = published.unwrap_or_else(|| predict::ar2(&app.entries));
    Ok(())
}

/// Classify and act: notify, push, and warn about a forecast crossing.
async fn react(w: &mut Watched, now_ms: i64, multi: bool) {
    // Whose reading this is only matters when there's more than one person
    // being watched; prefixing every line with "default" otherwise is noise.
    let who = if multi {
        format!("[{}] ", w.name)
    } else {
        String::new()
    };
    let app = &mut w.app;
    let last_logged = &mut w.last_logged;
    let state = app.evaluate_alert(now_ms);
    app.update_urgent(now_ms);

    if deferring(now_ms) {
        // Still classify (so episode state stays correct and a later handover
        // is seamless), but let the dashboard do the announcing. Track the
        // state anyway, so a low that cleared while the dashboard was up isn't
        // reported as a recovery the moment it closes.
        *last_logged = Some(state);
        return;
    }

    // Log every transition, then notify if desktop notifications are on. The
    // logging is deliberately outside that check: someone running the watcher
    // with `desktop = false` (push only, or just the audible alarm) still needs
    // a journal that says what happened and when.
    if let Some(a) = app.take_notification() {
        println!("{} · {who}{}", stamp(now_ms), a.label());
        if app.alerts.desktop {
            // The notification names the site, or a caregiver gets "URGENT
            // LOW" with no idea whose it is.
            if multi && app.alerts.notify_content {
                crate::notify_text(&format!("{}: {}", w.name, a.label()));
            } else {
                crate::notify(
                    a,
                    app.latest().map(|e| e.sgv),
                    app.units,
                    app.alerts.notify_content,
                );
            }
        }
    }
    if let Some(msg) = app.take_predictive(now_ms) {
        println!("{} · {who}{msg}", stamp(now_ms));
        if app.alerts.desktop {
            if app.alerts.notify_content {
                crate::notify_text(&msg);
            } else {
                crate::notify_text("alert — open sugarrush");
            }
        }
    }
    // An alarm that ends is news too: a journal with "URGENT LOW" and nothing
    // after it doesn't say whether it lasted two minutes or all night.
    if last_logged.is_some_and(Alert::is_alerting) && !state.is_alerting() {
        println!("{} · {who}recovered · {}", stamp(now_ms), state.label());
    }
    *last_logged = Some(state);

    if let Some(msg) = app.take_push(now_ms) {
        if let Some(url) = app.alerts.push_url.clone() {
            if !crate::push(&url, &msg).await {
                eprintln!("sugarrush watch: push failed — check push_url");
            }
        }
    }
    if state == Alert::Stale && !app.online {
        println!("{} · {who}offline", stamp(now_ms));
    }
}

fn stamp(now_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn a_heartbeat_goes_stale() {
        assert!(is_fresh(NOW, NOW));
        assert!(is_fresh(NOW - 29_000, NOW));
        assert!(!is_fresh(NOW - 31_000, NOW));
        // A stamp from the future is a clock problem, not proof of life.
        assert!(!is_fresh(NOW + 5_000, NOW));
    }

    #[test]
    fn alert_classes_round_trip() {
        for a in [
            Alert::UrgentLow,
            Alert::Low,
            Alert::InRange,
            Alert::High,
            Alert::UrgentHigh,
            Alert::Stale,
        ] {
            assert_eq!(alert_from_class(a.class()), Some(a));
        }
        // An unknown name is dropped rather than guessed at.
        assert_eq!(alert_from_class("not-a-state"), None);
    }

    #[test]
    fn state_survives_a_round_trip_through_json() {
        let episode = Episode {
            last_notified: Some(Alert::UrgentLow.class().to_string()),
            urgent_since: Some(NOW - 600_000),
            pushed_episode: true,
            escalated: false,
            snooze_until: Some(NOW + 300_000),
        };
        // Stored per site, so two people's episodes never collide.
        let mut state = State::default();
        state.sites.insert("alice".into(), episode.clone());
        state.sites.insert("bob".into(), Episode::default());
        let raw = serde_json::to_string(&state).unwrap();
        let back: State = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, state);
        assert_eq!(back.sites["alice"], episode);
        assert_eq!(back.sites["bob"], Episode::default());
    }

    #[test]
    fn a_restart_does_not_re_announce_or_un_snooze() {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites);

        Episode {
            last_notified: Some(Alert::UrgentLow.class().to_string()),
            urgent_since: Some(NOW - 600_000),
            pushed_episode: true,
            escalated: false,
            snooze_until: Some(NOW + 300_000),
        }
        .restore(&mut app);

        // Same urgent state as before the restart: nothing new to announce,
        // nothing new to push, and the snooze still holds.
        app.entries = vec![crate::nightscout::Entry {
            sgv: 45.0,
            date: NOW,
            direction: None,
        }];
        assert_eq!(app.evaluate_alert(NOW), Alert::UrgentLow);
        app.update_urgent(NOW);
        assert_eq!(app.take_notification(), None);
        assert_eq!(app.take_push(NOW), None);
        assert!(!app.alarm_active(NOW));
        // And the escalation timer continues from the original onset rather
        // than restarting — 10 minutes in, not 0.
        assert_eq!(Episode::capture(&app).urgent_since, Some(NOW - 600_000));
    }

    #[test]
    fn a_fresh_episode_still_announces_after_a_restart() {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites);
        // Restored state says the last thing announced was an in-range reading.
        Episode {
            last_notified: Some(Alert::InRange.class().to_string()),
            ..Default::default()
        }
        .restore(&mut app);

        app.entries = vec![crate::nightscout::Entry {
            sgv: 45.0,
            date: NOW,
            direction: None,
        }];
        app.evaluate_alert(NOW);
        assert_eq!(app.take_notification(), Some(Alert::UrgentLow));
    }

    #[test]
    fn two_sites_keep_independent_episodes() {
        // The bug this guards: one map for everyone means announcing a low for
        // alice marks it announced for bob too, and bob's low is never said.
        let mut state = State::default();
        state.sites.insert(
            "alice".into(),
            Episode {
                last_notified: Some(Alert::UrgentLow.class().to_string()),
                urgent_since: Some(NOW - 600_000),
                ..Default::default()
            },
        );

        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut bob = App::new(&cfg, alerts, sites);
        // Nothing stored for bob: his low is still news.
        if let Some(e) = state.sites.get("bob") {
            e.restore(&mut bob);
        }
        bob.entries = vec![crate::nightscout::Entry {
            sgv: 45.0,
            date: NOW,
            direction: None,
        }];
        bob.evaluate_alert(NOW);
        assert_eq!(bob.take_notification(), Some(Alert::UrgentLow));
    }

    #[test]
    fn the_worst_site_picks_the_alarm_tone() {
        // A high somewhere must not mask a low: lows rank first.
        assert!(severity(Alert::UrgentLow) < severity(Alert::UrgentHigh));
        assert!(severity(Alert::UrgentHigh) < severity(Alert::Stale));
        assert!(severity(Alert::Stale) < severity(Alert::Low));
        assert!(severity(Alert::InRange) > severity(Alert::High));
    }
}
