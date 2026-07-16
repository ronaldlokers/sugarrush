//! Nightscout REST client and data models.
//!
//! Reads sensor glucose values (SGV) from `/api/v1/entries/sgv.json`, authed
//! with a read-only token passed as a query parameter.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::config::Config;

const PRED_STEP_MS: i64 = 5 * 60_000;

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

    /// Fetch uploader-published forecasts from `/api/v1/devicestatus` (Loop's
    /// `loop.predicted` or OpenAPS's `openaps.suggested.predBGs`). Returns
    /// `(epoch_ms, mg/dL)` points, or `None` when no device predictions exist.
    pub async fn predictions(&self) -> Result<Option<Vec<(i64, f64)>>> {
        let url = format!("{}/api/v1/devicestatus.json", self.base_url);
        let value: Value = self
            .http
            .get(&url)
            .query(&[("count", "1"), ("token", self.token.as_str())])
            .send()
            .await
            .context("devicestatus request failed")?
            .error_for_status()
            .context("Nightscout returned an error status")?
            .json()
            .await
            .context("failed to parse devicestatus response")?;
        Ok(value
            .as_array()
            .and_then(|items| items.first())
            .and_then(parse_predicted))
    }
}

/// Extract a predicted SGV series from one devicestatus record.
fn parse_predicted(item: &Value) -> Option<Vec<(i64, f64)>> {
    // Loop: loop.predicted { startDate, values: [mg/dL, …] }
    if let Some(pred) = item.get("loop").and_then(|l| l.get("predicted")) {
        let start = pred.get("startDate").and_then(Value::as_str).and_then(parse_iso)?;
        let values = pred.get("values")?.as_array()?;
        return Some(space(start, values));
    }
    // OpenAPS: openaps.suggested { timestamp, predBGs: { COB|IOB|ZT: [...] } }
    if let Some(sug) = item.get("openaps").and_then(|o| o.get("suggested")) {
        let start = sug.get("timestamp").and_then(Value::as_str).and_then(parse_iso)?;
        let pred_bgs = sug.get("predBGs")?;
        let arr = ["COB", "IOB", "ZT"]
            .iter()
            .find_map(|k| pred_bgs.get(k).and_then(Value::as_array))?;
        return Some(space(start, arr));
    }
    None
}

/// Turn a start time plus a 5-minute-spaced value array into timed points.
fn space(start_ms: i64, values: &[Value]) -> Vec<(i64, f64)> {
    values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_f64().map(|bg| (start_ms + i as i64 * PRED_STEP_MS, bg)))
        .collect()
}

fn parse_iso(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis())
}
