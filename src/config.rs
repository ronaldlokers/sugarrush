//! Config loading from ~/.config/sugarrush/config.toml.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::units::Units;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the Nightscout instance, no trailing slash.
    pub url: String,
    /// Read-only access token (Nightscout Subject with `readable` role).
    pub token: String,
    #[serde(default = "default_units")]
    pub units: Units,
    #[serde(default = "default_refresh")]
    pub refresh_secs: u64,
    #[serde(default)]
    pub alerts: Alerts,
}

/// Alert thresholds and behaviour. Glucose bounds are in mg/dL regardless of
/// the display unit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Alerts {
    pub urgent_low: f64,
    pub low: f64,
    pub high: f64,
    pub urgent_high: f64,
    /// Warn when the newest reading is older than this many minutes.
    pub stale_minutes: i64,
    /// Fire desktop notifications via `notify-send` on threshold crossings.
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
        let cfg: Config =
            toml::from_str(&raw).with_context(|| format!("invalid config at {}", path.display()))?;
        Ok(cfg)
    }

    /// Trimmed base URL without a trailing slash.
    pub fn base_url(&self) -> &str {
        self.url.trim_end_matches('/')
    }
}
