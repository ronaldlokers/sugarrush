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
