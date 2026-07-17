//! Nightscout REST client and data models.
//!
//! Reads sensor glucose values (SGV) from `/api/v1/entries/sgv.json`, authed
//! with a read-only token passed as a query parameter.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::config::Site;

const PRED_STEP_MS: i64 = 5 * 60_000;

/// Lowest SGV treated as a real glucose reading (mg/dL). Nightscout encodes
/// sensor errors as small codes (0–12); anything below a physiological floor is
/// noise, not a hypo. CGMs themselves don't report below ~39.
const MIN_PHYSIOLOGICAL_SGV: f64 = 39.0;

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
    pub fn for_site(site: &Site) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("sugarrush/", env!("CARGO_PKG_VERSION")))
            // Bound every request so a half-open connection can't freeze the
            // run loop (and with it keyboard input and the audible alarm).
            .timeout(std::time::Duration::from_secs(12))
            .connect_timeout(std::time::Duration::from_secs(6))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            http,
            base_url: site.base_url().to_string(),
            token: site.token.clone(),
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
        let resp = self
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
            .context("can't reach Nightscout (check the URL and your connection)")?;
        let entries: Vec<Entry> = check_status(resp)?
            .json()
            .await
            .context("failed to parse Nightscout response")?;
        // Drop non-physiological values: Nightscout stores sensor error/noise
        // states as small SGV codes (0–12), which must never be read as a real
        // (urgent-low) reading or fed to the forecast (ln of a tiny value).
        Ok(entries
            .into_iter()
            .filter(|e| e.sgv >= MIN_PHYSIOLOGICAL_SGV)
            .collect())
    }

    /// Fetch uploader-published forecasts from `/api/v1/devicestatus` (Loop's
    /// `loop.predicted` or OpenAPS's `openaps.suggested.predBGs`). Returns
    /// `(epoch_ms, mg/dL)` points, or `None` when no device predictions exist.
    pub async fn predictions(&self) -> Result<Option<Vec<Prediction>>> {
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

    /// Fetch uploader/device metadata from `/api/v1/devicestatus`.
    pub async fn device_status(&self) -> Result<DeviceStatus> {
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
            .map(parse_device_status)
            .unwrap_or_default())
    }

    /// Epoch ms of the most recent sensor start/change treatment, if any.
    pub async fn sensor_start(&self) -> Result<Option<i64>> {
        let url = format!("{}/api/v1/treatments.json", self.base_url);
        let value: Value = self
            .http
            .get(&url)
            .query(&[("count", "50"), ("token", self.token.as_str())])
            .send()
            .await
            .context("treatments request failed")?
            .error_for_status()
            .context("Nightscout returned an error status")?
            .json()
            .await
            .context("failed to parse treatments response")?;
        Ok(value.as_array().and_then(|items| {
            items
                .iter()
                .filter_map(|t| {
                    let event = t.get("eventType")?.as_str()?;
                    event
                        .contains("Sensor")
                        .then(|| {
                            t.get("created_at")
                                .and_then(Value::as_str)
                                .and_then(parse_iso)
                        })
                        .flatten()
                })
                .max()
        }))
    }

    /// Fetch carb/insulin treatments whose `created_at` falls within
    /// `[start_ms, end_ms]`.
    pub async fn treatments(&self, start_ms: i64, end_ms: i64) -> Result<Vec<Treatment>> {
        let since = DateTime::from_timestamp_millis(start_ms)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        let url = format!("{}/api/v1/treatments.json", self.base_url);
        let value: Value = self
            .http
            .get(&url)
            .query(&[
                ("find[created_at][$gte]", since.as_str()),
                ("count", "300"),
                ("token", self.token.as_str()),
            ])
            .send()
            .await
            .context("treatments request failed")?
            .error_for_status()
            .context("Nightscout returned an error status")?
            .json()
            .await
            .context("failed to parse treatments response")?;
        Ok(value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|t| {
                        let at = t.get("mills").and_then(Value::as_i64).or_else(|| {
                            t.get("created_at")
                                .and_then(Value::as_str)
                                .and_then(parse_iso)
                        })?;
                        if at < start_ms || at > end_ms {
                            return None;
                        }
                        let carbs = t.get("carbs").and_then(Value::as_f64).filter(|c| *c > 0.0);
                        let insulin = t
                            .get("insulin")
                            .and_then(Value::as_f64)
                            .filter(|i| *i > 0.0);
                        (carbs.is_some() || insulin.is_some()).then_some(Treatment {
                            at_ms: at,
                            carbs,
                            insulin,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// A carb and/or insulin treatment.
#[derive(Debug, Clone)]
pub struct Treatment {
    pub at_ms: i64,
    pub carbs: Option<f64>,
    pub insulin: Option<f64>,
}

/// Uploader / device metadata, best-effort.
#[derive(Debug, Clone, Default)]
pub struct DeviceStatus {
    /// Uploader battery percentage.
    pub battery: Option<i64>,
    /// Device identifier string.
    pub device: Option<String>,
    /// When this status was recorded (epoch ms).
    pub last_ms: Option<i64>,
    /// Insulin on board, units.
    pub iob: Option<f64>,
    /// Carbs on board, grams.
    pub cob: Option<f64>,
}

fn parse_device_status(item: &Value) -> DeviceStatus {
    let battery = item
        .get("uploader")
        .and_then(|u| u.get("battery"))
        .or_else(|| item.get("uploaderBattery"))
        .and_then(Value::as_i64);
    let device = item
        .get("device")
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_ms = item.get("mills").and_then(Value::as_i64).or_else(|| {
        item.get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_iso)
    });
    // Loop: loop.iob.iob / loop.cob.cob. OpenAPS: openaps.suggested.IOB / .COB.
    let l = item.get("loop");
    let s = item.get("openaps").and_then(|o| o.get("suggested"));
    let iob = l
        .and_then(|l| l.get("iob"))
        .and_then(|i| i.get("iob"))
        .or_else(|| s.and_then(|s| s.get("IOB")))
        .and_then(Value::as_f64);
    let cob = l
        .and_then(|l| l.get("cob"))
        .and_then(|c| c.get("cob"))
        .or_else(|| s.and_then(|s| s.get("COB")))
        .and_then(Value::as_f64);
    DeviceStatus {
        battery,
        device,
        last_ms,
        iob,
        cob,
    }
}

/// Extract a predicted SGV series from one devicestatus record.
/// One forecast step: a low–high band (mg/dL) at a time.
#[derive(Debug, Clone, Copy)]
pub struct Prediction {
    pub at_ms: i64,
    pub low: f64,
    pub high: f64,
}

fn parse_predicted(item: &Value) -> Option<Vec<Prediction>> {
    // Loop: loop.predicted { startDate, values } — a single curve.
    if let Some(pred) = item.get("loop").and_then(|l| l.get("predicted")) {
        let start = pred
            .get("startDate")
            .and_then(Value::as_str)
            .and_then(parse_iso)?;
        let values = pred.get("values")?.as_array()?;
        return Some(envelope(start, &[values]));
    }
    // OpenAPS: openaps.suggested.predBGs { IOB, ZT, COB, UAM, … } — the cone is
    // the min/max envelope across all published curves.
    if let Some(sug) = item.get("openaps").and_then(|o| o.get("suggested")) {
        let start = sug
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso)?;
        let curves: Vec<&Vec<Value>> = sug
            .get("predBGs")?
            .as_object()?
            .values()
            .filter_map(Value::as_array)
            .collect();
        if curves.is_empty() {
            return None;
        }
        return Some(envelope(start, &curves));
    }
    None
}

/// Per-timestep min/max across the given 5-min-spaced curves.
fn envelope(start_ms: i64, curves: &[&Vec<Value>]) -> Vec<Prediction> {
    let max_len = curves.iter().map(|c| c.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for c in curves {
            if let Some(v) = c.get(i).and_then(Value::as_f64) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if lo <= hi {
            out.push(Prediction {
                at_ms: start_ms + i as i64 * PRED_STEP_MS,
                low: lo,
                high: hi,
            });
        }
    }
    out
}

/// Turn a response status into an actionable error, distinguishing an auth
/// problem (the common "pasted API_SECRET instead of a read-only token" case)
/// from a generic server error — so the UI never mislabels a bad token as a
/// network outage.
fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    use reqwest::StatusCode;
    match resp.status() {
        s if s.is_success() => Ok(resp),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => anyhow::bail!(
            "authentication failed — check your read-only token (a Nightscout \
             Subject token with the 'readable' role, not API_SECRET)"
        ),
        s => anyhow::bail!("Nightscout returned HTTP {}", s.as_u16()),
    }
}

fn parse_iso(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}
