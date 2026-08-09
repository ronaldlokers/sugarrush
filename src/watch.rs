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
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::alert::Alert;
use crate::app::App;
use crate::config::Config;
use crate::nightscout::{Client, DeviceStatus, Entry, Prediction};
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

fn state_lock_path() -> PathBuf {
    state_path().with_extension("lock")
}

/// Cross-process guard for the watch state read/modify/write cycle.
///
/// `sugarrush snooze` and the daemon are separate processes. Atomic rename
/// keeps the JSON intact, but without a lock either process can still replace
/// a newer logical update with an older snapshot.
struct StateLock {
    path: PathBuf,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_state_lock() -> Result<StateLock> {
    let path = state_lock_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(_) => return Ok(StateLock { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // The lock is only held around local file I/O. A much older
                // file was left by a killed process, not a legitimately slow
                // update, and must not disable snooze forever.
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .and_then(|t| t.elapsed().map_err(std::io::Error::other))
                    .is_ok_and(|age| age > Duration::from_secs(30));
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                if Instant::now() >= deadline {
                    anyhow::bail!("timed out waiting to update watcher state");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e).with_context(|| format!("failed to lock {}", path.display())),
        }
    }
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

/// Persisted episode state, keyed by immutable site ID — a caregiver watching three
/// people has three independent episodes, and a low for one of them must not
/// silence the announcement for another.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    #[serde(default)]
    pub sites: std::collections::BTreeMap<String, Episode>,
}

impl State {
    pub fn load() -> Self {
        let path = state_path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(state) => state,
                Err(e) => {
                    eprintln!(
                        "sugarrush: ignored corrupt watcher state at {}: {e}",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!(
                    "sugarrush: could not read watcher state at {}: {e}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    fn save_unlocked(&self) -> Result<()> {
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

    /// Save a daemon snapshot without overwriting a snooze command that
    /// landed while network requests were in flight.
    fn save_daemon_snapshot(&mut self) -> Result<()> {
        let _lock = acquire_state_lock()?;
        let latest = Self::load();
        merge_latest_snoozes(self, &latest);
        self.save_unlocked()
    }
}

fn merge_latest_snoozes(snapshot: &mut State, latest: &State) {
    for (name, episode) in &mut snapshot.sites {
        if let Some(on_disk) = latest.sites.get(name) {
            episode.snooze_until = on_disk.snooze_until;
        }
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

/// Which watched sites an external snooze command targets.
#[derive(Debug, Clone, Copy)]
pub enum SnoozeTarget<'a> {
    Site(&'a str),
    All,
}

/// Set (or clear) the snooze on selected watched sites, by writing the state
/// file the daemon already reads.
///
/// This is how `sugarrush snooze` reaches a running `watch`. There is no IPC:
/// the daemon re-reads this file every poll, so the snooze lands within one
/// interval — and, more importantly, it works when *no* daemon is running,
/// arming the next one instead of failing. A 3am snooze that only works if the
/// service happens to be up is not a snooze.
///
/// Returns how many sites it applied to.
pub fn set_snooze(until: Option<i64>, target: SnoozeTarget<'_>) -> Result<usize> {
    let _lock = acquire_state_lock()?;
    let mut state = State::load();
    let cfg = crate::config::Config::load()?;
    let sites = cfg.resolve_sites()?;
    // One-time migration from the legacy display-name keys. Only an exact
    // current-name match is eligible; orphaned state is never assigned to a
    // different person by guesswork.
    for site in &sites {
        if !state.sites.contains_key(&site.stable_id()) {
            if let Some(episode) = state.sites.remove(&site.name) {
                state.sites.insert(site.stable_id(), episode);
            }
        }
        state.sites.entry(site.stable_id()).or_default();
    }
    let resolved = match target {
        SnoozeTarget::All => SnoozeTarget::All,
        SnoozeTarget::Site(name) => {
            let id = sites
                .iter()
                .find(|site| site.name == name)
                .with_context(|| format!("unknown site '{name}'"))?
                .stable_id();
            let n = apply_snooze(&mut state, until, SnoozeTarget::Site(&id))?;
            state.save_unlocked()?;
            return Ok(n);
        }
    };
    let n = apply_snooze(&mut state, until, resolved)?;
    state.save_unlocked()?;
    Ok(n)
}

fn apply_snooze(state: &mut State, until: Option<i64>, target: SnoozeTarget<'_>) -> Result<usize> {
    let n = match target {
        SnoozeTarget::All => {
            for episode in state.sites.values_mut() {
                episode.snooze_until = until;
            }
            state.sites.len()
        }
        SnoozeTarget::Site(name) => {
            let episode = state
                .sites
                .get_mut(name)
                .with_context(|| format!("unknown site '{name}'"))?;
            episode.snooze_until = until;
            1
        }
    };
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

pub fn snoozes() -> std::collections::HashMap<String, Option<i64>> {
    State::load()
        .sites
        .into_iter()
        .map(|(id, episode)| (id, episode.snooze_until))
        .collect()
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
    let sites = cfg.resolve_sites()?;
    for site in &sites {
        let (alerts, warnings) = site.resolve_alerts(&cfg.alerts, cfg.units);
        crate::warn_about_config(&warnings);
        if insecure_push(&alerts) {
            eprintln!(
                "sugarrush watch [{}]: ⚠ push_url uses unencrypted http://; alert content may be readable in transit",
                site.name
            );
        }
    }

    // One pipeline per site. A caregiver watching three people needs three
    // independent episodes — hysteresis, escalation, snooze and all — so the
    // simplest correct thing is to run the same machinery per site rather than
    // inventing a second, thinner alert path for the multi-site case.
    let mut state = State::load();
    for site in &sites {
        if !state.sites.contains_key(&site.stable_id()) {
            if let Some(episode) = state.sites.remove(&site.name) {
                state.sites.insert(site.stable_id(), episode);
            }
        }
    }
    let mut watched: Vec<Watched> = sites
        .iter()
        .map(|site| {
            let alerts = site.resolve_alerts(&cfg.alerts, cfg.units).0;
            let mut app = App::new(&cfg, alerts, vec![site.clone()]);
            if let Some(episode) = state.sites.get(&site.stable_id()) {
                episode.restore(&mut app);
            }
            Ok(Watched {
                name: site.name.clone(),
                id: site.stable_id(),
                client: Client::for_site(site)?,
                app,
                last_logged: None,
                polling: false,
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
    let (poll_tx, mut poll_rx) = mpsc::unbounded_channel::<PollResult>();

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
                for (index, w) in watched.iter_mut().enumerate() {
                    adopt_external_snooze(&mut w.app, on_disk.sites.get(&w.id));
                    // The retry policy already exists on App and the TUI obeys
                    // it; the daemon was hammering a failing site at full rate
                    // and ignoring a paused-after-auth-failure state entirely.
                    if w.polling || (!w.app.online() && !w.app.should_retry(now)) {
                        continue;
                    }
                    let (start, end) = w.app.view.bounds(now);
                    let count = w.app.view.span.fetch_count();
                    w.polling = true;
                    spawn_poll(index, w.client.clone(), start, end, count, poll_tx.clone());
                }
                // Snapshot every site, including ones skipped during retry
                // backoff. Omitting a skipped site used to delete its episode
                // and snooze state from disk on the next save.
                let mut state = snapshot(&watched);
                if let Err(e) = state.save_daemon_snapshot() {
                    eprintln!("sugarrush watch: {e}");
                }
                if last_liveness.is_none_or(|t| now - t >= LIVENESS_INTERVAL_MS) {
                    println!("{} · {}", stamp(now), liveness(&watched, now));
                    last_liveness = Some(now);
                }
            }
            Some(result) = poll_rx.recv() => {
                let now = now_ms();
                let w = &mut watched[result.index];
                w.polling = false;
                if let Some(error) = apply_poll(&mut w.app, result, now) {
                    // Keep watching: an outage is exactly when the Stale alarm
                    // matters, and it can only fire if we stay alive.
                    eprintln!("sugarrush watch [{}]: {error}", w.name);
                }
                react(w, now, multi);
                let mut state = snapshot(&watched);
                if let Err(e) = state.save_daemon_snapshot() {
                    eprintln!("sugarrush watch: {e}");
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
                    let r = react(w, now, multi);
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

fn insecure_push(alerts: &crate::config::Alerts) -> bool {
    alerts.push_enabled
        && alerts
            .push_url
            .as_deref()
            .is_some_and(|url| url.starts_with("http://"))
}

fn snapshot(watched: &[Watched]) -> State {
    State {
        sites: watched
            .iter()
            .map(|w| (w.id.clone(), Episode::capture(&w.app)))
            .collect(),
    }
}

/// One site being watched: its own client, its own alert pipeline, its own
/// notion of what it last reported.
struct Watched {
    name: String,
    id: String,
    client: Client,
    app: App,
    /// What the journal last said about this site, so recoveries are logged.
    last_logged: Option<Alert>,
    /// At most one network poll per site. Alarm ticks remain independent while
    /// the request is in flight.
    polling: bool,
}

/// True when the dashboard is up: it is already showing and sounding this, so
/// the watcher stays out of the way.
fn deferring(now_ms: i64) -> bool {
    is_alive(Role::Tui, now_ms)
}

struct PollResult {
    index: usize,
    entries: crate::nightscout::Result<Vec<Entry>>,
    device: crate::nightscout::Result<(DeviceStatus, Option<Vec<Prediction>>)>,
}

fn spawn_poll(
    index: usize,
    client: Client,
    start: i64,
    end: i64,
    count: usize,
    tx: mpsc::UnboundedSender<PollResult>,
) {
    tokio::spawn(async move {
        // Entries and device status have independent endpoints. Running both
        // together bounds a site's poll by the slower request rather than the
        // sum, while this task keeps all network waits outside the alarm loop.
        let (entries, device) = tokio::join!(
            client.entries_range(start, end, count),
            client.device_status()
        );
        let _ = tx.send(PollResult {
            index,
            entries,
            device,
        });
    });
}

/// Apply completed network work on the watcher loop. Nothing in here awaits,
/// so the three-second alarm cadence cannot be held hostage by a slow site.
fn apply_poll(app: &mut App, result: PollResult, now_ms: i64) -> Option<String> {
    match result.entries {
        Ok(entries) => {
            app.entries = entries;
            app.mark_online(now_ms);
        }
        Err(e) => {
            let permanent = e.is_permanent();
            app.mark_offline(now_ms, e.to_string(), permanent);
            return Some(e.to_string());
        }
    }
    let published = result.device.ok().and_then(|(status, p)| {
        app.device = status;
        p
    });
    app.predictions = published.unwrap_or_else(|| predict::ar2(&app.entries));
    None
}

/// Deliver the alarm machine's reaction to the journal, the desktop and the
/// webhook.
///
/// The sequence itself lives in `App::react` — this used to reimplement it,
/// and had already drifted from the dashboard's copy over where
/// `update_urgent` sat relative to the announcements.
fn react(w: &mut Watched, now_ms: i64, multi: bool) -> crate::app::Reaction {
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
        crate::alertlog::record(&w.name, "alert", a, app.latest().map(|e| e.sgv));
        println!("{} · {who}{}", stamp(now_ms), a.label());
        if app.alerts.desktop {
            // The notification names the site, or a caregiver gets "URGENT
            // LOW" with no idea whose it is.
            let accepted = if multi && app.alerts.notify_content {
                crate::notify_text(&format!("{}: {}", w.name, a.label()))
            } else {
                crate::notify(
                    a,
                    app.latest().map(|e| e.sgv),
                    app.units,
                    app.alerts.notify_content,
                )
            };
            crate::alertlog::record_delivery(
                &w.name,
                "desktop",
                if accepted { "accepted" } else { "rejected" },
                a,
            );
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
        crate::alertlog::record(&w.name, "recovered", r.state, app.latest().map(|e| e.sgv));
        println!("{} · {who}recovered · {}", stamp(now_ms), r.state.label());
    }
    w.last_logged = Some(r.state);

    if let Some((url, msg)) = r.push.clone() {
        // Delivery has its own ten-second network timeout. Never await it in
        // the reaction loop: simultaneous caregiver escalations must not turn
        // the three-second alarm cadence into N × 10 seconds.
        let site = w.name.clone();
        let state = r.state;
        tokio::spawn(async move {
            let accepted = crate::push(&url, &msg).await;
            crate::alertlog::record_delivery(
                &site,
                "webhook",
                if accepted { "accepted" } else { "rejected" },
                state,
            );
            if !accepted {
                eprintln!("sugarrush watch [{site}]: push failed — check push_url");
            }
        });
    }
    if r.state == Alert::Stale && !w.app.online() {
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
                None if w.app.online() => format!("{who}no readings"),
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
    fn cleartext_push_is_detected_for_headless_warning() {
        let cfg = Config::demo();
        let mut alerts = cfg.alerts.resolve_checked(cfg.units).0;
        alerts.push_enabled = true;
        alerts.push_url = Some("http://example.test/topic".into());
        assert!(insecure_push(&alerts));
        alerts.push_url = Some("https://example.test/topic".into());
        assert!(!insecure_push(&alerts));
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
    fn a_targeted_snooze_never_silences_another_site() {
        let mut state = State::default();
        state.sites.insert("alice".into(), Episode::default());
        state.sites.insert("bob".into(), Episode::default());
        let until = NOW + 900_000;

        assert_eq!(
            apply_snooze(&mut state, Some(until), SnoozeTarget::Site("alice")).unwrap(),
            1
        );
        assert_eq!(state.sites["alice"].snooze_until, Some(until));
        assert_eq!(state.sites["bob"].snooze_until, None);
        assert!(apply_snooze(&mut state, Some(until), SnoozeTarget::Site("nobody")).is_err());
    }

    #[test]
    fn corrupt_state_is_not_mistaken_for_valid_state() {
        assert!(serde_json::from_str::<State>("not json").is_err());
    }

    #[test]
    fn skipped_sites_remain_in_the_daemon_snapshot() {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut app = App::new(&cfg, alerts, sites.clone());
        let episode = Episode {
            last_notified: Some(Alert::UrgentLow.class().to_string()),
            urgent_since: Some(NOW - 600_000),
            pushed_episode: true,
            snooze_until: Some(NOW + 300_000),
            episode_kind: Some(Alert::UrgentLow.class().to_string()),
            ..Episode::default()
        };
        episode.restore(&mut app);
        app.mark_offline(NOW, "offline".into(), false);
        assert!(!app.should_retry(NOW), "ordinary backoff skips this poll");

        let watched = vec![Watched {
            name: "default".into(),
            id: "default".into(),
            client: Client::for_site(&sites[0]).unwrap(),
            app,
            last_logged: None,
            polling: false,
        }];
        assert_eq!(snapshot(&watched).sites["default"], episode);
    }

    #[test]
    fn the_latest_external_snooze_wins_the_daemon_save() {
        let mut daemon = State::default();
        daemon.sites.insert(
            "alice".into(),
            Episode {
                last_notified: Some(Alert::UrgentLow.class().into()),
                snooze_until: None,
                ..Episode::default()
            },
        );
        let mut latest = daemon.clone();
        latest.sites.get_mut("alice").unwrap().snooze_until = Some(NOW + 900_000);

        merge_latest_snoozes(&mut daemon, &latest);
        assert_eq!(daemon.sites["alice"].snooze_until, Some(NOW + 900_000));
        assert_eq!(
            daemon.sites["alice"].last_notified.as_deref(),
            Some("urgent-low"),
            "merging the command must not discard current episode state"
        );

        // A concurrent `sugarrush snooze off` is an update too.
        latest.sites.get_mut("alice").unwrap().snooze_until = None;
        merge_latest_snoozes(&mut daemon, &latest);
        assert_eq!(daemon.sites["alice"].snooze_until, None);
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

    #[tokio::test]
    async fn a_stalled_poll_runs_outside_the_alarm_cadence() {
        let site = crate::nightscout::fake::serve_stalled().await;
        let client = Client::for_site(&site).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_poll(0, client, 0, NOW, 10, tx);

        // The endpoint deliberately never answers. Scheduling it must still
        // return immediately so the watcher can service its alarm ticker.
        let mut alarm = tokio::time::interval(Duration::from_millis(10));
        alarm.tick().await;
        tokio::time::timeout(Duration::from_millis(100), alarm.tick())
            .await
            .expect("a stalled Nightscout request blocked the alarm ticker");
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
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
            id: "default".into(),
            client: Client::for_site(&sites[0]).unwrap(),
            app,
            last_logged: None,
            polling: false,
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
            id: "default".into(),
            client: Client::for_site(&sites[0]).unwrap(),
            app,
            last_logged: None,
            polling: false,
        }];
        assert!(liveness(&watched, NOW).contains("offline"));
    }
}
