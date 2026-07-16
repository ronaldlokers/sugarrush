//! Nightscout REST client and data models.
//!
//! Reads sensor glucose values (SGV) from `/api/v1/entries/sgv.json`, authed
//! with a read-only token passed as a query parameter.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::Config;

/// A single sensor glucose reading as returned by Nightscout.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    /// Sensor glucose value in mg/dL.
    pub sgv: f64,
    /// Epoch milliseconds of the reading.
    pub date: i64,
    /// Trend direction, e.g. "Flat", "FortyFiveUp", "SingleDown".
    #[serde(default)]
    pub direction: Option<String>,
}

impl Entry {
    /// Unicode trend arrow for this reading's direction.
    pub fn arrow(&self) -> &'static str {
        match self.direction.as_deref() {
            Some("DoubleUp") => "⇈",
            Some("SingleUp") => "↑",
            Some("FortyFiveUp") => "↗",
            Some("Flat") => "→",
            Some("FortyFiveDown") => "↘",
            Some("SingleDown") => "↓",
            Some("DoubleDown") => "⇊",
            _ => "-",
        }
    }
}

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl Client {
    pub fn new(cfg: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("sugarrush/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            base_url: cfg.base_url().to_string(),
            token: cfg.token.clone(),
        })
    }

    /// Fetch SGV entries whose `date` falls within `[start_ms, end_ms]`, newest
    /// first. `want` bounds the request; Nightscout returns at most what exists
    /// in the range.
    pub async fn entries_range(
        &self,
        start_ms: i64,
        end_ms: i64,
        want: usize,
    ) -> Result<Vec<Entry>> {
        let count = want.max(1);
        let url = format!("{}/api/v1/entries/sgv.json", self.base_url);
        let entries: Vec<Entry> = self
            .http
            .get(&url)
            .query(&[
                ("find[date][$gte]", start_ms.to_string()),
                ("find[date][$lte]", end_ms.to_string()),
                ("count", count.to_string()),
                ("token", self.token.clone()),
            ])
            .send()
            .await
            .context("request to Nightscout failed")?
            .error_for_status()
            .context("Nightscout returned an error status")?
            .json()
            .await
            .context("failed to parse Nightscout response")?;
        Ok(entries)
    }
}
