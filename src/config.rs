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
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
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
    /// Fire desktop notifications via `notify-send` on threshold crossings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<bool>,
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
        }
    }
}

/// Resolved alert thresholds and behaviour, always in mg/dL.
#[derive(Debug, Clone, Copy)]
pub struct Alerts {
    pub urgent_low: f64,
    pub low: f64,
    pub high: f64,
    pub urgent_high: f64,
    pub stale_minutes: i64,
    pub desktop: bool,
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
        }
    }
}

fn default_units() -> Units {
    Units::Mmol
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
    fn empty_config_is_all_defaults() {
        let a = AlertsConfig::default().resolve(Units::Mmol);
        assert_eq!(a.low, 70.0);
        assert!(a.desktop);
        assert_eq!(a.stale_minutes, 15);
    }
}
