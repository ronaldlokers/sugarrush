//! Config loading from ~/.config/sugarrush/config.toml.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    /// Warn when the forecast predicts a low/high crossing within this many
    /// minutes (0 disables predictive alerts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predict_horizon_minutes: Option<i64>,
}

impl AlertsConfig {
    /// Resolve to concrete mg/dL thresholds, converting any user-supplied
    /// values from `units` and filling gaps with defaults.
    pub fn resolve(&self, units: Units) -> Alerts {
        let d = Alerts::default();
        Alerts {
            urgent_low: self.urgent_low.map_or(d.urgent_low, |v| units.to_mgdl(v)),
            low: self.low.map_or(d.low, |v| units.to_mgdl(v)),
            high: self.high.map_or(d.high, |v| units.to_mgdl(v)),
            urgent_high: self.urgent_high.map_or(d.urgent_high, |v| units.to_mgdl(v)),
            stale_minutes: self.stale_minutes.unwrap_or(d.stale_minutes),
            desktop: self.desktop.unwrap_or(d.desktop),
            sound: self.sound.unwrap_or(d.sound),
            snooze_minutes: self.snooze_minutes.unwrap_or(d.snooze_minutes),
            quiet_start: self.quiet_start.as_deref().and_then(parse_hhmm),
            quiet_end: self.quiet_end.as_deref().and_then(parse_hhmm),
            quiet_urgent_low: self.quiet_urgent_low.unwrap_or(d.quiet_urgent_low),
            escalate_minutes: self.escalate_minutes.unwrap_or(d.escalate_minutes),
            push_url: self.push_url.clone(),
            predict_horizon_minutes: self
                .predict_horizon_minutes
                .unwrap_or(d.predict_horizon_minutes),
        }
    }
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
        if !self.sites.is_empty() {
            return Ok(self.sites.clone());
        }
        match (&self.url, &self.token) {
            (Some(url), Some(token)) => Ok(vec![Site {
                name: default_site_name(),
                url: url.clone(),
                token: token.clone(),
            }]),
            _ => bail!("config needs either url + token, or at least one [[sites]] entry"),
        }
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
}
