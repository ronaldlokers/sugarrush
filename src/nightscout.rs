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
            .map_err(redact)
            .context("can't reach Nightscout (check the URL and your connection)")?;
        let entries: Vec<Entry> = check_status(resp)?
            .json()
            .await
            .map_err(redact)
            .context("failed to parse Nightscout response")?;
        Ok(clean_newest_first(entries))
    }

    /// Fetch the latest `/api/v1/devicestatus` record and read both things we
    /// want from it: the uploader metadata (battery, IOB/COB, last seen) and
    /// any published forecast (Loop's `loop.predicted`, OpenAPS's
    /// `openaps.suggested.predBGs`).
    ///
    /// One request, not two. These used to be separate calls to the same
    /// endpoint on every refresh — double the requests, and a chance of the two
    /// halves coming from different records if the uploader posted in between.
    pub async fn device_status(&self) -> Result<(DeviceStatus, Option<Vec<Prediction>>)> {
        let url = format!("{}/api/v1/devicestatus.json", self.base_url);
        let value: Value = self
            .http
            .get(&url)
            .query(&[("count", "1"), ("token", self.token.as_str())])
            .send()
            .await
            .map_err(redact)
            .context("devicestatus request failed")?
            .error_for_status()
            .map_err(redact)
            .context("Nightscout returned an error status")?
            .json()
            .await
            .map_err(redact)
            .context("failed to parse devicestatus response")?;
        let latest = value.as_array().and_then(|items| items.first());
        Ok((
            latest.map(parse_device_status).unwrap_or_default(),
            latest.and_then(parse_predicted),
        ))
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
            .map_err(redact)
            .context("treatments request failed")?
            .error_for_status()
            .map_err(redact)
            .context("Nightscout returned an error status")?
            .json()
            .await
            .map_err(redact)
            .context("failed to parse treatments response")?;
        Ok(parse_sensor_start(&value))
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
            .map_err(redact)
            .context("treatments request failed")?
            .error_for_status()
            .map_err(redact)
            .context("Nightscout returned an error status")?
            .json()
            .await
            .map_err(redact)
            .context("failed to parse treatments response")?;
        Ok(parse_treatments(&value, start_ms, end_ms))
    }
}

/// The most recent sensor start/change from a `/treatments` payload. Nightscout
/// spells these several ways ("Sensor Start", "Sensor Change"), so match on the
/// shared word and take the newest.
fn parse_sensor_start(value: &Value) -> Option<i64> {
    value.as_array().and_then(|items| {
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
    })
}

/// Carb/insulin treatments from a `/treatments` payload, within the window.
/// Entries with neither carbs nor insulin (site changes, notes, temp basals)
/// carry nothing to draw, so they're dropped.
fn parse_treatments(value: &Value, start_ms: i64, end_ms: i64) -> Vec<Treatment> {
    value
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
        .unwrap_or_default()
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

/// Strip the request URL out of a reqwest error before it goes anywhere.
///
/// Nightscout takes the token as a query parameter, so the URL *is* the
/// credential. reqwest's `Display` includes the full URL ("… for url (…)"),
/// and `sugarrush export` propagates its error to `main`, which prints the
/// whole `Caused by:` chain — landing the token in cron mail, the journal,
/// terminal scrollback, and pasted bug reports.
fn redact(e: reqwest::Error) -> reqwest::Error {
    e.without_url()
}

/// Turn a response status into an actionable error, distinguishing an auth
/// problem (the common "pasted API_SECRET instead of a read-only token" case)
/// from a generic server error — so the UI never mislabels a bad token as a
/// network outage.
fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    use reqwest::StatusCode;
    match resp.status() {
        s if s.is_success() => Ok(resp),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(FetchError::Auth.into()),
        StatusCode::NOT_FOUND => Err(FetchError::NotFound.into()),
        s => anyhow::bail!("Nightscout returned HTTP {}", s.as_u16()),
    }
}

/// A failure that repeating the request cannot fix — the site config is wrong.
/// Callers downcast to this (`err.downcast_ref::<FetchError>()`) to stop
/// retrying instead of hammering the server forever with a bad token or URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// 401/403 — the token is missing, wrong, or lacks the `readable` role.
    Auth,
    /// 404 — the base URL doesn't point at a Nightscout API.
    NotFound,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Auth => write!(
                f,
                "authentication failed — check your read-only token (a Nightscout \
                 Subject token with the 'readable' role, not API_SECRET)"
            ),
            FetchError::NotFound => write!(
                f,
                "no Nightscout API at this URL (HTTP 404) — check the site URL"
            ),
        }
    }
}

impl std::error::Error for FetchError {}

/// Filter sensor-error codes and force newest-first ordering.
///
/// Drop non-physiological values: Nightscout stores sensor error/noise states
/// as small SGV codes (0–12), which must never be read as a real (urgent-low)
/// reading or fed to the forecast (ln of a tiny value).
///
/// The order is ours to enforce, not the server's: everything downstream —
/// the current value, the delta, the staleness check, the AR2 forecast — reads
/// `entries[0]` as *the* latest reading. Nightscout normally sorts descending
/// by `date`, but a mirror, proxy, or `find[]` query variation that returns
/// another order would silently make an old reading the displayed one.
fn clean_newest_first(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = entries
        .into_iter()
        .filter(|e| e.sgv >= MIN_PHYSIOLOGICAL_SGV)
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.date));
    out
}

fn parse_iso(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// A minimal Nightscout stand-in, served over a real socket.
///
/// The parser tests below cover shapes; this covers the wire — status codes,
/// timeouts, and the `FetchError` classification the retry logic depends on.
/// It is deliberately hand-rolled over `TcpListener` rather than pulling in a
/// test HTTP server: the crate ships no dev-dependencies, and a dependency in
/// the alarm path's test harness is a dependency in the alarm path.
#[cfg(test)]
pub mod fake {
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use crate::config::Site;

    /// Serve `status` and `body` to every request, then return a `Site`
    /// pointing at it. The server lives as long as the test.
    pub async fn serve(status: u16, body: &'static str) -> Site {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let response = Arc::new(format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let response = Arc::clone(&response);
                tokio::spawn(async move {
                    // Read the request line so the client isn't writing into a
                    // closed socket, then answer.
                    let mut buf = [0u8; 2048];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.flush().await;
                });
            }
        });
        Site {
            name: "fake".into(),
            url: format!("http://127.0.0.1:{port}"),
            token: "test-token".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sgv: f64, date: i64) -> Entry {
        Entry {
            sgv,
            date,
            direction: None,
        }
    }

    #[test]
    fn newest_first_regardless_of_server_order() {
        let out = clean_newest_first(vec![
            entry(100.0, 1_000),
            entry(120.0, 3_000),
            entry(110.0, 2_000),
        ]);
        assert_eq!(
            out.iter().map(|e| e.date).collect::<Vec<_>>(),
            vec![3_000, 2_000, 1_000]
        );
        assert_eq!(out[0].sgv, 120.0);
    }

    /// The token is a query parameter, so any error carrying the request URL
    /// carries the credential. `sugarrush export` prints the whole cause chain
    /// on failure, so this must hold for every error the client can produce.
    #[tokio::test]
    async fn errors_never_carry_the_token() {
        // Port 1 refuses instantly — no DNS, no network, no flakiness.
        let site = Site {
            name: "default".into(),
            url: "http://127.0.0.1:1".into(),
            token: "SEKRIT-TOKEN-9Q7X".into(),
        };
        let client = Client::for_site(&site).unwrap();
        let err = client.entries_range(0, 1, 1).await.unwrap_err();

        // Check the whole chain the way anyhow's Termination prints it, not
        // just the outermost context — that was exactly the hole.
        let chain = format!("{err:#}\n{err:?}");
        assert!(
            !chain.contains("SEKRIT-TOKEN-9Q7X"),
            "token leaked into the error chain:\n{chain}"
        );
        assert!(
            !chain.contains("token="),
            "query string leaked into the error chain:\n{chain}"
        );
    }

    #[tokio::test]
    async fn a_real_response_round_trips_over_the_wire() {
        let body = r#"[{"sgv":142,"date":1700000000000,"direction":"Flat"},
                       {"sgv":138,"date":1699999700000,"direction":"FortyFiveDown"}]"#;
        let site = fake::serve(200, body).await;
        let client = Client::for_site(&site).unwrap();
        let entries = client
            .entries_range(0, 1_800_000_000_000, 10)
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sgv, 142.0); // newest first
    }

    #[tokio::test]
    async fn a_401_is_permanent_so_the_client_stops_retrying() {
        let site = fake::serve(401, "unauthorized").await;
        let client = Client::for_site(&site).unwrap();
        let err = client.entries_range(0, 1, 1).await.unwrap_err();
        assert_eq!(err.downcast_ref::<FetchError>(), Some(&FetchError::Auth));
    }

    #[tokio::test]
    async fn a_404_says_this_is_not_a_nightscout() {
        let site = fake::serve(404, "nope").await;
        let client = Client::for_site(&site).unwrap();
        let err = client.entries_range(0, 1, 1).await.unwrap_err();
        assert_eq!(
            err.downcast_ref::<FetchError>(),
            Some(&FetchError::NotFound)
        );
    }

    #[tokio::test]
    async fn a_500_is_transient_so_the_backoff_keeps_trying() {
        let site = fake::serve(500, "boom").await;
        let client = Client::for_site(&site).unwrap();
        let err = client.entries_range(0, 1, 1).await.unwrap_err();
        // Not a FetchError: the site may simply be restarting.
        assert!(err.downcast_ref::<FetchError>().is_none());
        assert!(err.to_string().contains("500"), "{err}");
    }

    #[tokio::test]
    async fn a_proxy_serving_html_is_reported_as_a_parse_failure() {
        // The classic Cloudflare-Access / nginx-auth Nightscout setup: 200 OK
        // with a login page instead of JSON.
        let site = fake::serve(200, "<!doctype html><title>Sign in</title>").await;
        let client = Client::for_site(&site).unwrap();
        let err = client.entries_range(0, 1, 1).await.unwrap_err();
        assert!(err.to_string().contains("parse"), "{err}");
    }

    #[tokio::test]
    async fn an_empty_site_is_not_an_error() {
        // A site with no readings yet is a legitimate state, not a failure —
        // the Stale alarm is what covers it, not a fetch error.
        let site = fake::serve(200, "[]").await;
        let client = Client::for_site(&site).unwrap();
        assert!(client.entries_range(0, 1, 1).await.unwrap().is_empty());
    }

    #[test]
    fn entries_deserialize_from_a_nightscout_payload() {
        // Shape as served by /api/v1/entries/sgv.json — extra fields and all.
        let raw = r#"[
          {"_id":"a","sgv":142,"date":1700000000000,"dateString":"2023-11-14T22:13:20.000Z",
           "trend":4,"direction":"Flat","device":"share2","type":"sgv"},
          {"_id":"b","sgv":138,"date":1699999700000,"direction":"FortyFiveDown","type":"sgv"}
        ]"#;
        let entries: Vec<Entry> = serde_json::from_str(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sgv, 142.0);
        assert_eq!(entries[0].date, 1_700_000_000_000);
        assert_eq!(entries[0].arrow(), "→");
        assert_eq!(entries[1].arrow(), "↘");
    }

    #[test]
    fn entry_survives_a_missing_direction() {
        let e: Entry = serde_json::from_str(r#"{"sgv":95,"date":1}"#).unwrap();
        assert!(e.direction.is_none());
        assert_eq!(e.arrow(), "-");
    }

    #[test]
    fn parses_a_loop_forecast() {
        let raw = r#"{
          "device":"loop://iPhone","created_at":"2023-11-14T22:10:00.000Z",
          "loop":{"iob":{"iob":1.35},"cob":{"cob":18},
                  "predicted":{"startDate":"2023-11-14T22:10:00.000Z",
                               "values":[120,118,115,110]}}
        }"#;
        let item: Value = serde_json::from_str(raw).unwrap();
        let preds = parse_predicted(&item).unwrap();
        assert_eq!(preds.len(), 4);
        // A single curve: the band collapses onto the curve itself.
        assert_eq!(preds[0].low, 120.0);
        assert_eq!(preds[0].high, 120.0);
        assert_eq!(preds[3].low, 110.0);
        // Steps are 5 minutes apart, starting at the published time.
        assert_eq!(preds[1].at_ms - preds[0].at_ms, PRED_STEP_MS);

        let status = parse_device_status(&item);
        assert_eq!(status.iob, Some(1.35));
        assert_eq!(status.cob, Some(18.0));
        assert_eq!(status.device.as_deref(), Some("loop://iPhone"));
    }

    #[test]
    fn parses_an_openaps_forecast_as_an_envelope() {
        let raw = r#"{
          "device":"openaps://rig","mills":1700000000000,
          "openaps":{"suggested":{"timestamp":"2023-11-14T22:10:00.000Z","IOB":0.8,"COB":24,
                                  "predBGs":{"IOB":[120,118,116],"ZT":[120,125,130],"UAM":[120,110,100]}}},
          "uploader":{"battery":76}
        }"#;
        let item: Value = serde_json::from_str(raw).unwrap();
        let preds = parse_predicted(&item).unwrap();
        assert_eq!(preds.len(), 3);
        // The cone is the min/max across every published curve at each step.
        assert_eq!(preds[0].low, 120.0);
        assert_eq!(preds[0].high, 120.0);
        assert_eq!(preds[2].low, 100.0);
        assert_eq!(preds[2].high, 130.0);

        let status = parse_device_status(&item);
        assert_eq!(status.iob, Some(0.8));
        assert_eq!(status.cob, Some(24.0));
        assert_eq!(status.battery, Some(76));
        assert_eq!(status.last_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn device_status_falls_back_to_uploader_battery_field() {
        let item: Value = serde_json::from_str(
            r#"{"uploaderBattery":42,"created_at":"2023-11-14T22:10:00.000Z"}"#,
        )
        .unwrap();
        let status = parse_device_status(&item);
        assert_eq!(status.battery, Some(42));
        assert!(status.last_ms.is_some()); // parsed from created_at
        assert!(status.iob.is_none());
    }

    #[test]
    fn no_forecast_when_the_uploader_publishes_none() {
        let item: Value =
            serde_json::from_str(r#"{"device":"share2","uploader":{"battery":90}}"#).unwrap();
        assert!(parse_predicted(&item).is_none());
    }

    #[test]
    fn treatments_are_filtered_to_the_window_and_to_real_doses() {
        let raw = r#"[
          {"eventType":"Meal Bolus","mills":1500,"carbs":45,"insulin":4.2},
          {"eventType":"Correction Bolus","created_at":"1970-01-01T00:00:02.000Z","insulin":1.1},
          {"eventType":"Note","mills":1600,"notes":"felt low"},
          {"eventType":"Temp Basal","mills":1700,"carbs":0,"insulin":0},
          {"eventType":"Meal Bolus","mills":99999,"carbs":30}
        ]"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let t = parse_treatments(&value, 1000, 5000);
        // The note and the all-zero temp basal carry nothing to draw; the last
        // one is outside the window.
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].carbs, Some(45.0));
        assert_eq!(t[0].insulin, Some(4.2));
        assert_eq!(t[1].at_ms, 2000); // created_at parsed when mills is absent
        assert_eq!(t[1].carbs, None);
    }

    #[test]
    fn sensor_start_takes_the_newest_sensor_event() {
        let raw = r#"[
          {"eventType":"Sensor Start","created_at":"2023-11-01T08:00:00.000Z"},
          {"eventType":"Site Change","created_at":"2023-11-12T08:00:00.000Z"},
          {"eventType":"Sensor Change","created_at":"2023-11-11T07:30:00.000Z"}
        ]"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let start = parse_sensor_start(&value).unwrap();
        // The 11 Nov sensor change, not the newer (but unrelated) site change.
        assert_eq!(start, parse_iso("2023-11-11T07:30:00.000Z").unwrap());
        // Nothing sensor-related at all.
        let none: Value = serde_json::from_str(
            r#"[{"eventType":"Site Change","created_at":"2023-11-12T08:00:00.000Z"}]"#,
        )
        .unwrap();
        assert!(parse_sensor_start(&none).is_none());
    }

    #[test]
    fn drops_sensor_error_codes() {
        let out = clean_newest_first(vec![entry(5.0, 2_000), entry(90.0, 1_000)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sgv, 90.0);
    }
}
