//! Config loading from ~/.config/sugarrush/config.toml.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::theme::ThemeConfig;
use crate::units::Units;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the single Nightscout instance (legacy single-site form).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Read-only token for the single-site form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// One or more named sites (multi-site form). Takes precedence over the
    /// top-level `url`/`token` when non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<Site>,
    #[serde(default = "default_units")]
    pub units: Units,
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    #[serde(default)]
    pub alerts: AlertsConfig,
    #[serde(default, skip_serializing_if = "is_default_theme")]
    pub theme: ThemeConfig,
    /// How the graph draws readings.
    #[serde(default)]
    pub graph_style: GraphStyle,
    /// How many days of history the AGP view folds over.
    #[serde(default = "default_agp_days")]
    pub agp_days: u32,
    /// Minimap navigator settings.
    #[serde(default)]
    pub minimap: MinimapConfig,
}

/// The 24h (configurable) overview strip and its mouse navigation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MinimapConfig {
    /// Show the strip and enable mouse capture (drag to pan, click to jump).
    #[serde(default = "minimap_enabled")]
    pub enabled: bool,
    /// Width of the overview window in hours.
    #[serde(default = "minimap_span")]
    pub span_hours: u32,
}

impl Default for MinimapConfig {
    fn default() -> Self {
        Self {
            enabled: minimap_enabled(),
            span_hours: minimap_span(),
        }
    }
}

fn minimap_enabled() -> bool {
    true
}
fn minimap_span() -> u32 {
    24
}

/// Marker style for the graph's readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphStyle {
    /// Thin connected braille line.
    Line,
    /// Discrete medium dots (default).
    #[default]
    Dots,
    /// Discrete chunky blocks.
    Blocks,
}

impl GraphStyle {
    /// Cycle to the next/previous style.
    pub fn cycle(self, dir: i32) -> Self {
        let order = [GraphStyle::Line, GraphStyle::Dots, GraphStyle::Blocks];
        let idx = order.iter().position(|&s| s == self).unwrap_or(0) as i32;
        order[(idx + dir).rem_euclid(order.len() as i32) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            GraphStyle::Line => "line",
            GraphStyle::Dots => "dots",
            GraphStyle::Blocks => "blocks",
        }
    }
}

/// A named Nightscout site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    #[serde(default = "default_site_name")]
    pub name: String,
    pub url: String,
    pub token: String,
}

impl Site {
    /// Trimmed base URL without a trailing slash.
    pub fn base_url(&self) -> &str {
        self.url.trim_end_matches('/')
    }

    /// True when the token travels in clear text: plain `http://` to anything
    /// but the local machine. The token is a query parameter, so anyone on the
    /// path can read it (and the glucose data with it).
    pub fn is_insecure(&self) -> bool {
        is_insecure_url(self.base_url())
    }
}

/// True when a webhook URL would send the alert in clear text.
///
/// `push_url` had none of the scrutiny the site URL gets — no wizard gate, no
/// settings warning, no footer banner — despite being the one channel that
/// leaves the machine. The documented example is a public ntfy topic, where
/// anyone who learns the topic name is subscribed to a stranger's
/// hypoglycaemia; over plain http it's readable by the network too.
pub fn is_insecure_url(url: &str) -> bool {
    let url = url.trim().trim_end_matches('/');
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    !matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Clean up a Nightscout URL as typed: trim it, default the scheme to HTTPS,
/// drop a trailing slash, and strip an API path if one was pasted along.
///
/// People copy the URL out of a browser tab that's showing
/// `…/api/v1/entries.json?count=10`, or type a bare `mysite.herokuapp.com`.
/// Both are unambiguous — accept them rather than failing a live-test with a
/// confusing 404.
pub fn normalize_site_url(input: &str) -> Result<String> {
    let raw = input.trim();
    if raw.is_empty() {
        bail!("the URL is empty");
    }
    // Resolve the scheme first: trimming slashes before this would turn a bare
    // "https://" into the hostname "https:".
    let mut s = match raw.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => {
            format!("https://{rest}")
        }
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => format!("http://{rest}"),
        Some((scheme, _)) => bail!("unsupported scheme '{scheme}://' — use https://"),
        // A bare host: assume HTTPS rather than silently downgrading.
        None => format!("https://{raw}"),
    };
    // Strip a pasted API path: everything from `/api/` onwards belongs to the
    // client, not the base URL.
    if let Some(idx) = s.to_lowercase().find("/api/") {
        s.truncate(idx);
    } else if s.to_lowercase().ends_with("/api") {
        s.truncate(s.len() - 4);
    }
    let s = s.trim_end_matches('/').to_string();
    let host = s
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
        .unwrap_or("");
    if host.is_empty() {
        bail!("no host in '{input}'");
    }
    Ok(s)
}

/// Alert thresholds as written in config.toml. Glucose bounds are expressed in
/// the configured display `units`; omitted fields fall back to unit-independent
/// physiological defaults. Call [`AlertsConfig::resolve`] to get mg/dL values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgent_low: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgent_high: Option<f64>,
    /// Warn when the newest reading is older than this many minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_minutes: Option<i64>,
    /// Fire desktop notifications on threshold crossings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<bool>,
    /// Play a looping audible alarm on urgent/stale states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
    /// How long the snooze key silences the audible alarm, in minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snooze_minutes: Option<i64>,
    /// Start of the quiet-hours window, `HH:MM` (empty/absent = disabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_start: Option<String>,
    /// End of the quiet-hours window, `HH:MM`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_end: Option<String>,
    /// Whether urgent-low still sounds during quiet hours (safety override).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_urgent_low: Option<bool>,
    /// Escalate an unacknowledged urgent alert after this many minutes
    /// (0 disables escalation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalate_minutes: Option<i64>,
    /// Optional webhook / ntfy topic URL to POST urgent alerts to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_url: Option<String>,
    /// Whether `push_url` is actually used. Lets the settings screen turn push
    /// alerts off without discarding the configured URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_enabled: Option<bool>,
    /// Whether desktop notifications spell out the alert and the reading.
    /// `false` keeps them content-free for lock screens and shared displays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_content: Option<bool>,
    /// Warn when the forecast predicts a low/high crossing within this many
    /// minutes (0 disables predictive alerts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predict_horizon_minutes: Option<i64>,
}

impl AlertsConfig {
    /// Resolve to concrete mg/dL thresholds, converting any user-supplied
    /// values from `units` and filling gaps with defaults.
    pub fn resolve(&self, units: Units) -> Alerts {
        self.resolve_checked(units).0
    }

    /// Resolve, and report anything that had to be coerced.
    ///
    /// A threshold that can't alarm is worse than no threshold at all, and the
    /// realistic way to get one is a unit mismatch: the shipped example is in
    /// mmol/L, so a US user who changes `units` to `mgdl` and nothing else ends
    /// up with `low = 3.9 mg/dL`. Nothing then reads as low — a 40 mg/dL hypo
    /// classifies as *urgent high*, because it is above every threshold.
    ///
    /// So an implausible value is not clamped toward the edge of the range
    /// (`3.9` → `20` still can't alarm); it falls back to the physiological
    /// default and says so. The settings screen has enforced the same
    /// invariants for a while — this brings the config-file path in line.
    pub fn resolve_checked(&self, units: Units) -> (Alerts, Vec<String>) {
        let d = Alerts::default();
        let mut warnings = Vec::new();
        let mut threshold = |raw: Option<f64>, default: f64, name: &str| {
            plausible(raw, default, name, units, &mut warnings)
        };
        let (urgent_low, low, high, urgent_high) = (
            threshold(self.urgent_low, d.urgent_low, "urgent_low"),
            threshold(self.low, d.low, "low"),
            threshold(self.high, d.high, "high"),
            threshold(self.urgent_high, d.urgent_high, "urgent_high"),
        );
        // Crossed thresholds silently empty a band and misclassify every
        // reading in it, so order is restored rather than trusted.
        let (urgent_low, low, high, urgent_high) =
            order_thresholds(urgent_low, low, high, urgent_high, units, &mut warnings);

        let stale_minutes = match self.stale_minutes {
            Some(m) if m < 1 => {
                warnings.push(format!(
                    "stale_minutes = {m} would make every reading stale — using {}",
                    d.stale_minutes
                ));
                d.stale_minutes
            }
            other => other.unwrap_or(d.stale_minutes),
        };

        let alerts = Alerts {
            urgent_low,
            low,
            high,
            urgent_high,
            stale_minutes,
            desktop: self.desktop.unwrap_or(d.desktop),
            sound: self.sound.unwrap_or(d.sound),
            snooze_minutes: self.snooze_minutes.unwrap_or(d.snooze_minutes),
            quiet_start: self.quiet_start.as_deref().and_then(parse_hhmm),
            quiet_end: self.quiet_end.as_deref().and_then(parse_hhmm),
            quiet_urgent_low: self.quiet_urgent_low.unwrap_or(d.quiet_urgent_low),
            escalate_minutes: self.escalate_minutes.unwrap_or(d.escalate_minutes),
            push_url: self.push_url.clone(),
            push_enabled: self.push_enabled.unwrap_or(d.push_enabled),
            notify_content: self.notify_content.unwrap_or(d.notify_content),
            predict_horizon_minutes: self
                .predict_horizon_minutes
                .unwrap_or(d.predict_horizon_minutes),
        };
        (alerts, warnings)
    }
}

/// Lowest and highest glucose a threshold can meaningfully sit at, in mg/dL.
/// A CGM reports roughly 40–400; outside this band a threshold cannot separate
/// real readings from each other, so it can only ever mis-classify them.
const MIN_PLAUSIBLE_MGDL: f64 = 20.0;
const MAX_PLAUSIBLE_MGDL: f64 = 500.0;

/// One threshold, converted and sanity-checked against the physiological range.
fn plausible(
    raw: Option<f64>,
    default: f64,
    name: &str,
    units: Units,
    warnings: &mut Vec<String>,
) -> f64 {
    let Some(v) = raw else { return default };
    let mgdl = units.to_mgdl(v);
    if (MIN_PLAUSIBLE_MGDL..=MAX_PLAUSIBLE_MGDL).contains(&mgdl) {
        return mgdl;
    }
    warnings.push(format!(
        "{name} = {v} {} is outside the physiological range — check `units` — using {} {}",
        units.label(),
        units.format(default),
        units.label()
    ));
    default
}

/// Restore `urgent_low <= low <= high <= urgent_high`, reporting any change.
fn order_thresholds(
    urgent_low: f64,
    low: f64,
    high: f64,
    urgent_high: f64,
    units: Units,
    warnings: &mut Vec<String>,
) -> (f64, f64, f64, f64) {
    if urgent_low <= low && low <= high && high <= urgent_high {
        return (urgent_low, low, high, urgent_high);
    }
    let mut v = [urgent_low, low, high, urgent_high];
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    warnings.push(format!(
        "alert thresholds were out of order — using {} <= {} <= {} <= {} {}",
        units.format(v[0]),
        units.format(v[1]),
        units.format(v[2]),
        units.format(v[3]),
        units.label()
    ));
    (v[0], v[1], v[2], v[3])
}

/// Create (or truncate) a file that only the owner can read, with the mode set
/// at creation time so a secret is never written to a briefly-readable file.
#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<File> {
    File::create(path)
}

/// Write a file only the owner can read. Used for anything holding personal
/// data — the token, and exports of someone's glucose history.
pub fn write_private(path: &Path, body: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
    }
    let mut f =
        create_private(path).with_context(|| format!("failed to create {}", path.display()))?;
    f.write_all(body.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    set_owner_only(path);
    Ok(())
}

/// Restrict `path` to owner read/write (no-op off Unix).
pub fn set_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Parse `HH:MM` into minutes-of-day (0..1440).
pub fn parse_hhmm(s: &str) -> Option<i32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: i32 = h.parse().ok()?;
    let m: i32 = m.parse().ok()?;
    if (0..24).contains(&h) && (0..60).contains(&m) {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Format minutes-of-day as `HH:MM`.
pub fn fmt_hhmm(min: i32) -> String {
    let m = min.rem_euclid(1440);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Resolved alert thresholds and behaviour, always in mg/dL.
#[derive(Debug, Clone)]
pub struct Alerts {
    pub urgent_low: f64,
    pub low: f64,
    pub high: f64,
    pub urgent_high: f64,
    pub stale_minutes: i64,
    pub desktop: bool,
    pub sound: bool,
    pub snooze_minutes: i64,
    /// Quiet-hours window as minutes-of-day; `None` when disabled.
    pub quiet_start: Option<i32>,
    pub quiet_end: Option<i32>,
    pub quiet_urgent_low: bool,
    pub escalate_minutes: i64,
    pub push_url: Option<String>,
    pub push_enabled: bool,
    pub notify_content: bool,
    pub predict_horizon_minutes: i64,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            urgent_low: 55.0,
            low: 70.0,
            high: 180.0,
            urgent_high: 250.0,
            stale_minutes: 15,
            desktop: true,
            sound: true,
            snooze_minutes: 15,
            quiet_start: None,
            quiet_end: None,
            quiet_urgent_low: true,
            escalate_minutes: 0,
            push_url: None,
            push_enabled: true,
            notify_content: true,
            predict_horizon_minutes: 30,
        }
    }
}

impl Alerts {
    /// True when `min_of_day` falls inside the quiet-hours window (handles
    /// windows that cross midnight). Always false when quiet hours are unset.
    pub fn in_quiet_hours(&self, min_of_day: i32) -> bool {
        match (self.quiet_start, self.quiet_end) {
            (Some(s), Some(e)) if s <= e => min_of_day >= s && min_of_day < e,
            (Some(s), Some(e)) => min_of_day >= s || min_of_day < e,
            _ => false,
        }
    }
}

fn default_units() -> Units {
    Units::Mmol
}
fn default_agp_days() -> u32 {
    14
}
fn default_refresh() -> u64 {
    30
}
fn default_site_name() -> String {
    "default".to_string()
}
fn is_default_theme(t: &ThemeConfig) -> bool {
    toml::Value::try_from(t)
        .map(|v| v.as_table().map(|tbl| tbl.is_empty()).unwrap_or(true))
        .unwrap_or(false)
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("could not resolve user config dir")?;
        Ok(dir.join("sugarrush").join("config.toml"))
    }

    /// A self-contained config for `--demo` mode (no real site; the client is
    /// built but never used — demo data is generated locally).
    pub fn demo() -> Self {
        Self {
            url: Some("http://demo.invalid".to_string()),
            token: Some("demo".to_string()),
            sites: Vec::new(),
            units: default_units(),
            refresh_secs: 5,
            alerts: AlertsConfig::default(),
            theme: ThemeConfig::default(),
            graph_style: GraphStyle::default(),
            agp_days: default_agp_days(),
            minimap: MinimapConfig::default(),
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        let raw = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "could not read config at {}. Copy config.example.toml there to get started.",
                path.display()
            )
        })?;
        let cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("invalid config at {}", path.display()))?;
        Ok(cfg)
    }

    /// Write `body` to `path` atomically, owner-only.
    ///
    /// The config file holds the only copy of the Nightscout token, so it must
    /// never be truncated in place: a write that dies part-way (full disk,
    /// crash, power loss) would leave an empty or half-written file and take
    /// the token with it. Write a sibling temp file created with mode 0600 —
    /// so the token is never briefly world-readable — flush it to disk, then
    /// rename over the target. Rename within a filesystem is atomic: readers
    /// see either the old config or the new one, never a torn one.
    pub fn write_atomic(path: &Path, body: &str) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config.toml");
        let tmp = path.with_file_name(format!(".{}.{}.tmp", name, std::process::id()));

        let write = |tmp: &Path| -> Result<()> {
            let mut f = create_private(tmp)
                .with_context(|| format!("failed to create {}", tmp.display()))?;
            f.write_all(body.as_bytes())
                .with_context(|| format!("failed to write {}", tmp.display()))?;
            // Get the bytes on disk before the rename publishes the new file.
            f.sync_all()
                .with_context(|| format!("failed to flush {}", tmp.display()))?;
            Ok(())
        };
        if let Err(e) = write(&tmp) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(
                anyhow::Error::new(e).context(format!("failed to replace {}", path.display()))
            );
        }
        // Re-assert the mode: an existing file's permissions survive a rename
        // on some platforms, and the config may predate this code.
        set_owner_only(path);
        Ok(())
    }

    /// True when the config file is group- or world-readable (Unix only) —
    /// the token lives there in plaintext, so it should be `chmod 600`.
    pub fn perms_too_open() -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(path) = Self::path() {
                if let Ok(meta) = std::fs::metadata(path) {
                    return meta.permissions().mode() & 0o077 != 0;
                }
            }
        }
        false
    }

    /// The configured sites: the `[[sites]]` list if present, otherwise the
    /// legacy top-level `url`/`token` as a single "default" site.
    pub fn resolve_sites(&self) -> Result<Vec<Site>> {
        let mut sites = if !self.sites.is_empty() {
            self.sites.clone()
        } else {
            match (&self.url, &self.token) {
                (Some(url), Some(token)) => vec![Site {
                    name: default_site_name(),
                    url: url.clone(),
                    token: token.clone(),
                }],
                _ => bail!("config needs either url + token, or at least one [[sites]] entry"),
            }
        };
        // Episode state in the watcher is keyed by site name, so two sites
        // sharing one would share an alert episode: announcing a low for one
        // person would mark it announced for the other.
        let mut seen = std::collections::BTreeSet::new();
        for site in &sites {
            if !seen.insert(site.name.clone()) {
                bail!(
                    "two [[sites]] are both named '{}' — names identify people \
                     in alerts and alarm state, so they must be unique",
                    site.name
                );
            }
        }

        // A hand-written config gets the same tidy-up as the wizard's input —
        // a missing scheme or a pasted `/api/v1/…` path shouldn't be a silent
        // 404. An unparseable URL is left alone so the fetch error names it.
        for site in &mut sites {
            if let Ok(url) = normalize_site_url(&site.url) {
                site.url = url;
            }
        }
        Ok(sites)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmol_thresholds_convert_to_mgdl() {
        let raw = AlertsConfig {
            low: Some(3.9),
            urgent_high: Some(13.9),
            ..Default::default()
        };
        let a = raw.resolve(Units::Mmol);
        assert!((a.low - 70.2).abs() < 0.1); // 3.9 * 18
        assert!((a.urgent_high - 250.2).abs() < 0.1);
        // Unset fields keep mg/dL defaults, not converted.
        assert_eq!(a.urgent_low, 55.0);
        assert_eq!(a.high, 180.0);
    }

    #[test]
    fn mgdl_thresholds_pass_through() {
        let raw = AlertsConfig {
            low: Some(70.0),
            ..Default::default()
        };
        assert_eq!(raw.resolve(Units::Mgdl).low, 70.0);
    }

    #[test]
    fn hhmm_round_trips() {
        assert_eq!(parse_hhmm("23:00"), Some(1380));
        assert_eq!(parse_hhmm("07:30"), Some(450));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("nope"), None);
        assert_eq!(fmt_hhmm(1380), "23:00");
        assert_eq!(fmt_hhmm(450), "07:30");
    }

    #[test]
    fn quiet_hours_handles_midnight_wrap() {
        let a = Alerts {
            quiet_start: Some(1380), // 23:00
            quiet_end: Some(420),    // 07:00
            ..Alerts::default()
        };
        assert!(a.in_quiet_hours(1440 - 1)); // 23:59 in window
        assert!(a.in_quiet_hours(0)); // 00:00 in window
        assert!(a.in_quiet_hours(419)); // 06:59 in window
        assert!(!a.in_quiet_hours(420)); // 07:00 out
        assert!(!a.in_quiet_hours(720)); // noon out
                                         // Disabled window is never quiet.
        assert!(!Alerts::default().in_quiet_hours(0));
    }

    #[test]
    fn graph_style_cycles() {
        assert_eq!(GraphStyle::Line.cycle(1), GraphStyle::Dots);
        assert_eq!(GraphStyle::Dots.cycle(1), GraphStyle::Blocks);
        assert_eq!(GraphStyle::Blocks.cycle(1), GraphStyle::Line); // wraps
        assert_eq!(GraphStyle::Line.cycle(-1), GraphStyle::Blocks); // wraps back
    }

    #[test]
    fn empty_config_is_all_defaults() {
        let a = AlertsConfig::default().resolve(Units::Mmol);
        assert_eq!(a.low, 70.0);
        assert!(a.desktop);
        assert_eq!(a.stale_minutes, 15);
    }

    #[test]
    fn write_atomic_replaces_and_stays_owner_only() {
        let dir = std::env::temp_dir().join(format!("sugarrush-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        Config::write_atomic(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        // Replacing leaves no temp file behind and keeps the new content.
        Config::write_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_defaults_to_https_and_trims() {
        assert_eq!(
            normalize_site_url("  ns.example.com/  ").unwrap(),
            "https://ns.example.com"
        );
        assert_eq!(
            normalize_site_url("https://ns.example.com").unwrap(),
            "https://ns.example.com"
        );
        // A pasted API path belongs to the client, not the base URL.
        assert_eq!(
            normalize_site_url("https://ns.example.com/api/v1/entries.json?count=10").unwrap(),
            "https://ns.example.com"
        );
        assert_eq!(
            normalize_site_url("https://ns.example.com/api").unwrap(),
            "https://ns.example.com"
        );
        // http is preserved (not silently upgraded) — it's flagged elsewhere.
        assert_eq!(
            normalize_site_url("http://192.168.1.5:1337").unwrap(),
            "http://192.168.1.5:1337"
        );
    }

    #[test]
    fn normalize_rejects_junk() {
        assert!(normalize_site_url("").is_err());
        assert!(normalize_site_url("   ").is_err());
        assert!(normalize_site_url("ftp://ns.example.com").is_err());
        assert!(normalize_site_url("https://").is_err());
    }

    #[test]
    fn insecure_only_for_remote_http() {
        let site = |url: &str| Site {
            name: "default".into(),
            url: url.into(),
            token: "t".into(),
        };
        assert!(site("http://ns.example.com").is_insecure());
        assert!(site("http://192.168.1.5:1337").is_insecure());
        assert!(!site("https://ns.example.com").is_insecure());
        // Loopback never leaves the machine.
        assert!(!site("http://localhost:1337").is_insecure());
        assert!(!site("http://127.0.0.1:1337").is_insecure());
    }

    /// The exact failure C1 describes: the shipped example is mmol, the user
    /// switches `units` to mgdl and changes nothing else.
    #[test]
    fn a_unit_mismatch_cannot_disable_the_low_alarm() {
        let raw = AlertsConfig {
            urgent_low: Some(3.0),
            low: Some(3.9),
            high: Some(10.0),
            urgent_high: Some(13.9),
            ..Default::default()
        };
        let (a, warnings) = raw.resolve_checked(Units::Mgdl);

        // Before the fix these resolved verbatim, and a 40 mg/dL hypo — a
        // medical emergency — classified as UrgentHigh because it sat above
        // every threshold.
        assert_eq!(
            crate::alert::evaluate(40.0, 0, &a),
            crate::alert::Alert::UrgentLow
        );
        assert_eq!(
            crate::alert::evaluate(300.0, 0, &a),
            crate::alert::Alert::UrgentHigh
        );
        // All four were implausible as mg/dL, so all four are reported.
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].contains("urgent_low"));
        assert!(warnings.iter().all(|w| w.contains("check `units`")));
    }

    #[test]
    fn the_mirror_mistake_is_caught_too() {
        // mg/dL numbers left in an mmol config: 70 mmol/L is ~1260 mg/dL.
        let raw = AlertsConfig {
            low: Some(70.0),
            urgent_low: Some(55.0),
            ..Default::default()
        };
        let (a, warnings) = raw.resolve_checked(Units::Mmol);
        assert_eq!(a.low, 70.0); // fell back to the mg/dL default
        assert_eq!(a.urgent_low, 55.0);
        assert_eq!(warnings.len(), 2);
        // And an in-range reading is not screaming urgent-low.
        assert_eq!(
            crate::alert::evaluate(100.0, 0, &a),
            crate::alert::Alert::InRange
        );
    }

    #[test]
    fn plausible_values_pass_through_untouched_and_silently() {
        let raw = AlertsConfig {
            urgent_low: Some(3.0),
            low: Some(3.9),
            high: Some(10.0),
            urgent_high: Some(13.9),
            ..Default::default()
        };
        let (a, warnings) = raw.resolve_checked(Units::Mmol);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!((a.low - 70.2).abs() < 0.1);
        assert!((a.urgent_high - 250.2).abs() < 0.1);
    }

    #[test]
    fn crossed_thresholds_are_reordered_not_obeyed() {
        let raw = AlertsConfig {
            urgent_low: Some(200.0),
            low: Some(180.0),
            high: Some(70.0),
            urgent_high: Some(55.0),
            ..Default::default()
        };
        let (a, warnings) = raw.resolve_checked(Units::Mgdl);
        assert!(a.urgent_low <= a.low && a.low <= a.high && a.high <= a.urgent_high);
        assert!(warnings.iter().any(|w| w.contains("out of order")));
    }

    #[test]
    fn a_zero_stale_window_would_make_everything_stale() {
        let raw = AlertsConfig {
            stale_minutes: Some(0),
            ..Default::default()
        };
        let (a, warnings) = raw.resolve_checked(Units::Mgdl);
        assert_eq!(a.stale_minutes, 15);
        assert!(warnings.iter().any(|w| w.contains("stale_minutes")));
        // A fresh reading is not immediately Stale.
        assert_eq!(
            crate::alert::evaluate(100.0, 0, &a),
            crate::alert::Alert::InRange
        );
    }

    /// The shipped example must parse to the values it appears to set.
    /// `check-process.sh` greps for key *names*, which passed happily while
    /// `refresh_secs` sat inside `[minimap]` and was silently ignored.
    #[test]
    fn the_example_config_means_what_it_says() {
        let raw = include_str!("../config.example.toml");
        let cfg: Config = toml::from_str(raw).expect("config.example.toml must parse");

        // Keys a user is most likely to change, at the level they think.
        assert_eq!(cfg.refresh_secs, 30, "refresh_secs is not a root-level key");
        assert_eq!(cfg.units, Units::Mmol);
        assert_eq!(cfg.agp_days, 14);
        assert!(cfg.minimap.enabled);
        assert_eq!(cfg.minimap.span_hours, 24);

        // And the thresholds it ships resolve to something that can alarm.
        let (alerts, warnings) = cfg.alerts.resolve_checked(cfg.units);
        assert!(
            warnings.is_empty(),
            "the shipped example warns: {warnings:?}"
        );
        assert_eq!(
            crate::alert::evaluate(40.0, 0, &alerts),
            crate::alert::Alert::UrgentLow
        );
    }

    #[test]
    fn two_sites_cannot_share_a_name() {
        let cfg: Config = toml::from_str(
            r#"
units = "mmol"
[[sites]]
name = "kid"
url = "https://a.example.com"
token = "a"
[[sites]]
name = "kid"
url = "https://b.example.com"
token = "b"
"#,
        )
        .unwrap();
        // Watcher episode state is keyed by name, so a duplicate would make
        // one person's announced low mark the other's as announced.
        let err = cfg.resolve_sites().unwrap_err().to_string();
        assert!(err.contains("both named 'kid'"), "{err}");
    }
}
