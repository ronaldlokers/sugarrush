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

/// How often the watcher says it is alive and well, even when nothing
/// changed. Without this the journal only ever records transitions, so the
/// morning after a missed alarm an empty journal means either "glucose was
/// flat all night" or "the daemon was dead" — and there is no way to tell.
const LIVENESS_INTERVAL_MS: i64 = 15 * 60_000;

/// Slowest sensible poll for a background watcher. Readings arrive every five
/// minutes; polling faster only burns someone else's server and your battery.
const WATCH_MIN_INTERVAL_SECS: u64 = 60;

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
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("sugarrush"),
        // Without a per-user runtime dir the fallback was a shared
        // /tmp/sugarrush, where any local user could create tui.alive and keep
        // it fresh to silence someone else's alarm daemon indefinitely. Scope
        // it per-uid, and see `is_alive` for the ownership check.
        None => std::env::temp_dir().join(format!("sugarrush-{}", current_uid())),
    }
}

/// Our own user id, read from a file we definitionally own rather than via a
/// libc call — this crate has no `unsafe` and a uid lookup is not worth
/// starting.
#[cfg(unix)]
fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    dirs::home_dir()
        .and_then(|h| std::fs::metadata(h).ok())
        .map(|m| m.uid())
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// True when `path` is owned by us. A heartbeat is a claim that another
/// process is handling the alarm; one we can't vouch for is not a claim to act
/// on, and acting on it means going silent.
#[cfg(unix)]
fn owned_by_us(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|m| m.uid() == current_uid())
}

#[cfg(not(unix))]
fn owned_by_us(_path: &std::path::Path) -> bool {
    true
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
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
    let path = runtime_dir().join(role.file());
    // Fail open: anything we can't vouch for means we keep alarming.
    if !owned_by_us(&path) {
        return false;
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
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
        // Atomic and owner-only, like the config file next door. A torn write
        // loads as Default, which silently cancels an active snooze and
        // restarts an escalation timer — the two things this file exists to
        // prevent. It also names a third party's alert history in follower
        // mode, so it is not world-readable.
        crate::config::Config::write_atomic(&path, &body)
    }
}

/// Take on a snooze that was set from outside this process.
///
/// The daemon writes this file every poll, so on-disk and in-memory normally
/// agree. The daemon never sets a snooze itself, which means any difference
/// came from `sugarrush snooze` — including a cancellation — and wins. Without
/// this the next poll would quietly overwrite the snooze someone just set.
fn adopt_external_snooze(app: &mut App, on_disk: Option<&Episode>) {
    if let Some(e) = on_disk {
        if e.snooze_until != app.snooze_until() {
            app.set_snooze(e.snooze_until);
        }
    }
}

/// Set (or clear) the snooze on every watched site, by writing the state file
/// the daemon already reads.
///
/// This is how `sugarrush snooze` reaches a running `watch`. There is no IPC:
/// the daemon re-reads this file every poll, so the snooze lands within one
/// interval — and, more importantly, it works when *no* daemon is running,
/// arming the next one instead of failing. A 3am snooze that only works if the
/// service happens to be up is not a snooze.
///
/// Returns how many sites it applied to.
pub fn set_snooze(until: Option<i64>) -> Result<usize> {
    let mut state = State::load();
    if state.sites.is_empty() {
        // No episode file yet (a daemon that has never run, or never alarmed).
        // Record it anyway under the configured site names, so the snooze is
        // in place before the first alarm rather than after it.
        let cfg = crate::config::Config::load()?;
        for site in cfg.resolve_sites()? {
            state.sites.entry(site.name).or_default();
        }
    }
    for episode in state.sites.values_mut() {
        episode.snooze_until = until;
    }
    let n = state.sites.len();
    state.save()?;
    Ok(n)
}

/// The snooze currently recorded on disk, if every site shares one.
///
/// Used by the alarm self-test: a forgotten snooze is one of the ways a night
/// passes without a sound, and nothing in the app said so.
pub fn snoozed_until() -> Option<i64> {
    let state = State::load();
    let mut values = state.sites.values().map(|e| e.snooze_until);
    let first = values.next()??;
    values.all(|v| v == Some(first)).then_some(first)
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
    /// Which urgent state this episode belongs to, so a restart mid-episode
    /// isn't mistaken for a change of emergency.
    #[serde(default)]
    pub episode_kind: Option<String>,
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
            episode_kind: app.episode_kind().map(|a| a.class().to_string()),
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
            self.episode_kind.as_deref().and_then(alert_from_class),
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
    // Report liveness immediately on start, then on the interval.
    let mut last_liveness: Option<i64> = None;

    // CGM data arrives once every five minutes, so the TUI's default of a few
    // seconds — chosen for a responsive dashboard — meant roughly 17,000
    // requests a day per site against someone's self-hosted Nightscout, and
    // kept a laptop's radio awake continuously. The daemon polls at its own,
    // slower cadence unless the user has deliberately set a slower one.
    let period = cfg.refresh_secs.max(WATCH_MIN_INTERVAL_SECS);
    let mut ticker = tokio::time::interval(Duration::from_secs(period));
    // Catching up on missed ticks all at once would fire a burst of fetches
    // (and alarms) after a suspend or a slow request.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The alarm re-sounds on its own cadence while an urgent state persists,
    // independent of how often we fetch.
    let mut alarm_ticker = tokio::time::interval(Duration::from_secs(3));
    alarm_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    alarm_ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = now_ms();
                heartbeat(Role::Watch, now);
                // Re-read before writing: `sugarrush snooze` sets the snooze by
                // writing this file, and without this the daemon would overwrite
                // it with its own in-memory copy on the very next poll. The
                // daemon never sets a snooze itself, so anything that differs
                // from what we last wrote came from outside and wins.
                let on_disk = State::load();
                let mut state = State::default();
                for w in watched.iter_mut() {
                    adopt_external_snooze(&mut w.app, on_disk.sites.get(&w.name));
                    // The retry policy already exists on App and the TUI obeys
                    // it; the daemon was hammering a failing site at full rate
                    // and ignoring a paused-after-auth-failure state entirely.
                    if !w.app.online && !w.app.should_retry(now) {
                        continue;
                    }
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
                if last_liveness.is_none_or(|t| now - t >= LIVENESS_INTERVAL_MS) {
                    println!("{} · {}", stamp(now), liveness(&watched, now));
                    last_liveness = Some(now);
                }
            }
            _ = alarm_ticker.tick() => {
                let now = now_ms();
                // The same reaction the poll runs, not a partial copy of it:
                // re-classifying here is what makes a sensor gap start sounding
                // within seconds instead of waiting out the poll interval, and
                // routing it through `react` means the announcement that comes
                // with it goes out on the same schedule.
                //
                // Any site in an urgent state sounds the alarm; the tone comes
                // from the worst of them, so a low is never masked by a high
                // somewhere else.
                let mut worst: Option<(Alert, sound::Tone)> = None;
                for w in watched.iter_mut() {
                    let r = react(w, now, multi).await;
                    let alert = w.app.alert;
                    if r.sound && worst.is_none_or(|(a, _)| alert.severity() < a.severity()) {
                        worst = Some((alert, w.app.alarm_tone()));
                    }
                }
                if let Some((_, tone)) = worst {
                    if !deferring(now) {
                        sound::alarm(tone);
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

/// Deliver the alarm machine's reaction to the journal, the desktop and the
/// webhook.
///
/// The sequence itself lives in `App::react` — this used to reimplement it,
/// and had already drifted from the dashboard's copy over where
/// `update_urgent` sat relative to the announcements.
async fn react(w: &mut Watched, now_ms: i64, multi: bool) -> crate::app::Reaction {
    // Whose reading this is only matters when there's more than one person
    // being watched; prefixing every line with "default" otherwise is noise.
    let who = if multi {
        format!("[{}] ", w.name)
    } else {
        String::new()
    };
    let app = &mut w.app;
    let r = app.react(now_ms);

    if deferring(now_ms) {
        // The dashboard is up and doing the announcing. Everything above still
        // ran, so episode state stays correct and a later handover is seamless;
        // the announcements are dropped rather than held, or they would all
        // arrive at once the moment the dashboard closed.
        w.last_logged = Some(r.state);
        return r;
    }

    // Log every transition, then notify if desktop notifications are on. The
    // logging is deliberately outside that check: someone running the watcher
    // with `desktop = false` (push only, or just the audible alarm) still needs
    // a journal that says what happened and when.
    if let Some(a) = r.notification {
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
    if let Some(msg) = r.predictive.clone() {
        println!("{} · {who}{msg}", stamp(now_ms));
        if app.alerts.desktop {
            if app.alerts.notify_content {
                let _ = crate::notify_text(&msg);
            } else {
                let _ = crate::notify_text("alert — open sugarrush");
            }
        }
    }
    // An alarm that ends is news too: a journal with "URGENT LOW" and nothing
    // after it doesn't say whether it lasted two minutes or all night.
    if r.recovered {
        println!("{} · {who}recovered · {}", stamp(now_ms), r.state.label());
    }
    w.last_logged = Some(r.state);

    if let Some((url, msg)) = r.push.clone() {
        if !crate::push(&url, &msg).await {
            eprintln!("sugarrush watch: push failed — check push_url");
        }
    }
    if r.state == Alert::Stale && !w.app.online {
        println!("{} · {who}offline", stamp(now_ms));
    }
    r
}

/// A one-line "still here, and here's what I can see" summary, so an otherwise
/// quiet journal proves the watcher was running rather than merely silent.
fn liveness(watched: &[Watched], now_ms: i64) -> String {
    let parts: Vec<String> = watched
        .iter()
        .map(|w| {
            let who = if watched.len() > 1 {
                format!("{}: ", w.name)
            } else {
                String::new()
            };
            match w.app.latest() {
                Some(e) => format!(
                    "{who}{} {} · {} · {}m ago",
                    w.app.units.format(e.sgv),
                    w.app.units.label(),
                    w.app.alert.label(),
                    ((now_ms - e.date) / 60_000).max(0)
                ),
                None if w.app.online => format!("{who}no readings"),
                None => format!("{who}offline"),
            }
        })
        .collect();
    format!("ok · {}", parts.join(" · "))
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

    /// Before `sugarrush snooze`, the only way to stop a 3am alarm from the
    /// daemon was `systemctl --user stop` — which also disarms the *next* one.
    /// The snooze arrives by way of this file, and the daemon rewrites the file
    /// every poll, so it has to take on what it finds rather than overwrite it.
    #[test]
    fn an_externally_set_snooze_is_adopted_not_clobbered() {
        let cfg = crate::config::Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites);
        assert_eq!(app.snooze_until(), None);

        let until = NOW + 15 * 60_000;
        adopt_external_snooze(
            &mut app,
            Some(&Episode {
                snooze_until: Some(until),
                ..Episode::default()
            }),
        );
        assert_eq!(app.snooze_until(), Some(until), "the snooze must land");

        // `sugarrush snooze off` clears it the same way.
        adopt_external_snooze(&mut app, Some(&Episode::default()));
        assert_eq!(app.snooze_until(), None, "a cancellation must land too");

        // No entry on disk yet (a site the daemon has never alarmed on) leaves
        // whatever is in memory alone.
        app.set_snooze(Some(until));
        adopt_external_snooze(&mut app, None);
        assert_eq!(app.snooze_until(), Some(until));
    }

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
            episode_kind: Some(Alert::UrgentLow.class().to_string()),
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
            episode_kind: Some(Alert::UrgentLow.class().to_string()),
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
    fn liveness_reports_what_the_watcher_can_see() {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites.clone());
        app.entries = vec![crate::nightscout::Entry {
            sgv: 100.0,
            date: NOW - 120_000,
            direction: None,
        }];
        app.mark_online(NOW);
        app.evaluate_alert(NOW);
        let watched = vec![Watched {
            name: "default".into(),
            client: Client::for_site(&sites[0]).unwrap(),
            app,
            last_logged: None,
        }];

        let line = liveness(&watched, NOW);
        // The point of the line is that it proves the watcher is alive AND
        // that it is seeing fresh data — an "ok" with a two-hour-old reading
        // would be a different kind of lie.
        assert!(line.starts_with("ok · "), "{line}");
        assert!(line.contains("in range"), "{line}");
        assert!(line.contains("2m ago"), "{line}");
        // Single site: no name prefix cluttering every line.
        assert!(!line.contains("default:"), "{line}");
    }

    #[test]
    fn liveness_says_offline_rather_than_ok_when_it_has_nothing() {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites.clone());
        app.mark_offline(NOW, "connection refused".into(), false);
        let watched = vec![Watched {
            name: "default".into(),
            client: Client::for_site(&sites[0]).unwrap(),
            app,
            last_logged: None,
        }];
        assert!(liveness(&watched, NOW).contains("offline"));
    }
}
