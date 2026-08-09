//! A durable record of what the alarm actually did.
//!
//! Nothing persisted this. "Did it alarm last night, and for how long?" was
//! unanswerable — the journal only exists if the daemon runs under systemd, is
//! rotated by someone else's policy, and says nothing at all about alarms the
//! dashboard handled. For a tool people rely on overnight, "I think it went
//! off around 3" is not a good enough answer, and neither is silence.
//!
//! Append-only JSONL at `$XDG_STATE_HOME/sugarrush/alerts.jsonl`, owner-only:
//! in follower mode this is a third party's alert history.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::alert::Alert;
use crate::units::Units;

#[derive(Debug, Clone, Copy)]
pub enum Format {
    Text,
    Json,
    Csv,
}

impl Format {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "text" | "plain" => Some(Self::Text),
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }
}

/// Entries older than this are dropped when the file is next compacted. Three
/// months covers a quarterly clinic appointment, which is what someone would
/// actually bring this to.
const RETAIN_DAYS: i64 = 90;

/// Compact when the file passes this. Each line is ~90 bytes, so this is
/// several months of a bad sensor before any rewriting happens.
const COMPACT_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    /// Epoch ms.
    pub ts: i64,
    pub site: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<String>,
    /// `"alert"` when an episode began, `"recovered"` when it ended.
    pub event: String,
    /// The alert label, e.g. `"URGENT LOW"`.
    pub state: String,
    /// The reading at the time, mg/dL. Absent when there wasn't one — which is
    /// itself the story for a sensor gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sgv: Option<f64>,
    /// Delivery channel and outcome for `event = "delivery"`. These contain
    /// no destination URL, token, reading, or message body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir);
    base.join("sugarrush").join("alerts.jsonl")
}

/// Append one record. Best-effort: failing to write history must never stop an
/// alarm from being delivered.
pub fn record(site: &str, event: &str, state: Alert, sgv: Option<f64>) {
    let entry = Record {
        ts: crate::now_ms(),
        site: site.to_string(),
        site_id: None,
        event: event.to_string(),
        state: state.label().to_string(),
        sgv,
        channel: None,
        outcome: None,
    };
    let _ = append(&entry);
}

/// Persist a privacy-safe channel outcome. "accepted" means only that the
/// local API or remote endpoint accepted the request; it never means a person
/// saw, read, or heard it.
pub fn record_delivery(
    site: &str,
    site_id: Option<&str>,
    channel: &str,
    outcome: &str,
    state: Alert,
) {
    let entry = Record {
        ts: crate::now_ms(),
        site: site.to_string(),
        site_id: site_id.map(str::to_string),
        event: "delivery".into(),
        state: state.label().to_string(),
        sgv: None,
        channel: Some(channel.to_string()),
        outcome: Some(outcome.to_string()),
    };
    let _ = append(&entry);
}

pub fn latest_delivery(site_id: &str, legacy_name: &str) -> Option<Record> {
    read(crate::now_ms() - RETAIN_DAYS * 86_400_000)
        .into_iter()
        .rev()
        .find(|record| delivery_matches(record, site_id, legacy_name))
}

fn delivery_matches(record: &Record, site_id: &str, legacy_name: &str) -> bool {
    record.event == "delivery"
        && record
            .site_id
            .as_deref()
            .map_or(record.site == legacy_name, |id| id == site_id)
}

fn append(entry: &Record) -> Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _lock = AlertLogLock::acquire(&path.with_extension("lock"))?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > COMPACT_BYTES) {
        compact(&path)?;
    }
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');

    // O_APPEND, so the dashboard and the daemon can both write without a lock:
    // one line under a page is written atomically, and neither can truncate
    // what the other wrote.
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // In follower mode this names a third party's alert history.
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

struct AlertLogLock(PathBuf);

impl AlertLogLock {
    fn acquire(path: &std::path::Path) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(_) => return Ok(Self(path.to_path_buf())),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .and_then(|t| t.elapsed().map_err(std::io::Error::other))
                        .is_ok_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = std::fs::remove_file(path);
                    } else if Instant::now() >= deadline {
                        anyhow::bail!("timed out waiting to update alert history");
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for AlertLogLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Rewrite the file keeping only the recent tail.
fn compact(path: &std::path::Path) -> Result<()> {
    let cutoff = crate::now_ms() - RETAIN_DAYS * 86_400_000;
    let kept: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|l| {
            serde_json::from_str::<Record>(l)
                .map(|r| r.ts >= cutoff)
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect();
    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    crate::config::Config::write_atomic(path, &body)
}

/// Every record at or after `since_ms`, oldest first.
pub fn read(since_ms: i64) -> Vec<Record> {
    let Ok(raw) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    let mut out: Vec<Record> = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        .filter(|r| r.ts >= since_ms)
        .collect();
    out.sort_by_key(|r| r.ts);
    out
}

/// One alarm episode, paired from its start and end records.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    pub site: String,
    pub state: String,
    pub start: i64,
    /// `None` if it was still going at the end of the window — an episode with
    /// no recorded end is not the same as one that lasted zero minutes.
    pub end: Option<i64>,
    pub sgv: Option<f64>,
}

impl Episode {
    pub fn minutes(&self) -> Option<i64> {
        self.end.map(|e| ((e - self.start) / 60_000).max(0))
    }
}

/// Pair `alert` records with the `recovered` that followed them, per site.
///
/// A restart mid-episode, or a log that starts inside one, leaves an unpaired
/// end; those are dropped rather than invented, because a duration measured
/// from a start we never saw would be a fabrication.
pub fn episodes(records: &[Record]) -> Vec<Episode> {
    use std::collections::HashMap;
    let mut open: HashMap<&str, &Record> = HashMap::new();
    let mut out = Vec::new();
    for r in records {
        match r.event.as_str() {
            "alert" => {
                // A new alert while one is open means the episode changed
                // variant (a low that became a high); close the old one at the
                // moment the new one started.
                if let Some(prev) = open.insert(&r.site, r) {
                    out.push(Episode {
                        site: prev.site.clone(),
                        state: prev.state.clone(),
                        start: prev.ts,
                        end: Some(r.ts),
                        sgv: prev.sgv,
                    });
                }
            }
            "recovered" => {
                if let Some(prev) = open.remove(r.site.as_str()) {
                    out.push(Episode {
                        site: prev.site.clone(),
                        state: prev.state.clone(),
                        start: prev.ts,
                        end: Some(r.ts),
                        sgv: prev.sgv,
                    });
                }
            }
            _ => {}
        }
    }
    for r in open.into_values() {
        out.push(Episode {
            site: r.site.clone(),
            state: r.state.clone(),
            start: r.ts,
            end: None,
            sgv: r.sgv,
        });
    }
    out.sort_by_key(|e| e.start);
    out
}

/// `sugarrush alerts --days N` — what the alarm did, and for how long.
#[cfg(test)]
pub fn report(days: i64, units: Units) -> String {
    report_site(days, units, None)
}

pub fn render(days: i64, units: Units, site: Option<&str>, format: Format) -> Result<String> {
    let since = crate::now_ms() - days.max(1) * 86_400_000;
    let records = filtered_records(since, site)?;
    Ok(match format {
        Format::Text => report_records(days, units, site, records),
        Format::Json => format!("{}\n", serde_json::to_string_pretty(&records)?),
        Format::Csv => records_csv(&records),
    })
}

#[cfg(test)]
pub fn report_site(days: i64, units: Units, site: Option<&str>) -> String {
    let since = crate::now_ms() - days.max(1) * 86_400_000;
    let records = filtered_records(since, site).unwrap_or_default();
    report_records(days, units, site, records)
}

fn filtered_records(since: i64, site: Option<&str>) -> Result<Vec<Record>> {
    let all = read(since);
    if let Some(name) = site {
        anyhow::ensure!(
            all.iter().any(|record| record.site == name),
            "no alert history for site '{name}' in this window"
        );
        Ok(all
            .into_iter()
            .filter(|record| record.site == name)
            .collect())
    } else {
        Ok(all)
    }
}

fn report_records(days: i64, units: Units, site: Option<&str>, records: Vec<Record>) -> String {
    let episodes = episodes(&records);
    let suffix = site.map(|name| format!(" · {name}")).unwrap_or_default();
    let mut out = format!("sugarrush alerts · last {days} day(s){suffix}\n\n");

    if records.is_empty() {
        out.push_str("Nothing recorded in this window.\n\n");
        // An empty log has two very different causes and the difference
        // matters: a quiet week is good news, a watcher that never ran is not.
        out.push_str(
            "That means either nothing alarmed, or nothing was running to notice.\n\
             `sugarrush watch --test` says which.\n",
        );
        return out;
    }

    let multi = episodes.iter().any(|e| e.site != episodes[0].site);
    for e in &episodes {
        let when = Local
            .timestamp_millis_opt(e.start)
            .single()
            .map(|t| t.format("%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "--".into());
        let dur = match e.minutes() {
            Some(m) => format!("{m:>4}m"),
            None => "   —".into(),
        };
        let value = e
            .sgv
            .map(|v| format!("  {} {}", units.format(v), units.label()))
            .unwrap_or_default();
        let who = if multi {
            format!("[{}] ", e.site)
        } else {
            String::new()
        };
        out.push_str(&format!("{when}  {dur}  {who}{}{value}\n", e.state));
    }

    let ongoing = episodes.iter().filter(|e| e.end.is_none()).count();
    let total: i64 = episodes.iter().filter_map(Episode::minutes).sum();
    out.push_str(&format!(
        "\n{} episode(s), {total} minutes alarming",
        episodes.len()
    ));
    if ongoing > 0 {
        // Not counted in the total: a duration for an episode we haven't seen
        // the end of would be a guess.
        out.push_str(&format!(" ({ongoing} still open, not counted)"));
    }
    out.push('\n');
    let deliveries: Vec<_> = records
        .iter()
        .filter(|record| record.event == "delivery")
        .collect();
    if !deliveries.is_empty() {
        out.push_str("\nDelivery attempts (accepted does not mean seen or heard)\n");
        out.push_str("---------------------------------------------------------\n");
        for record in deliveries {
            let when = Local
                .timestamp_millis_opt(record.ts)
                .single()
                .map(|time| time.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "--".into());
            out.push_str(&format!(
                "{when}  [{}] {} {}\n",
                record.site,
                record.channel.as_deref().unwrap_or("unknown"),
                record.outcome.as_deref().unwrap_or("unknown")
            ));
        }
    }
    out
}

fn records_csv(records: &[Record]) -> String {
    let mut out = String::from("timestamp_ms,site,event,state,sgv_mgdl,channel,outcome\n");
    for record in records {
        let safe = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            record.ts,
            safe(&record.site),
            safe(&record.event),
            safe(&record.state),
            record
                .sgv
                .map(|value| value.to_string())
                .unwrap_or_default(),
            safe(record.channel.as_deref().unwrap_or("")),
            safe(record.outcome.as_deref().unwrap_or(""))
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: i64, site: &str, event: &str, state: &str) -> Record {
        Record {
            ts,
            site: site.into(),
            site_id: None,
            event: event.into(),
            state: state.into(),
            sgv: Some(45.0),
            channel: None,
            outcome: None,
        }
    }

    #[test]
    fn delivery_records_carry_no_health_value_or_destination() {
        let record = Record {
            ts: T,
            site: "alice".into(),
            site_id: Some("site-alice".into()),
            event: "delivery".into(),
            state: "URGENT LOW".into(),
            sgv: None,
            channel: Some("webhook".into()),
            outcome: Some("accepted".into()),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("https://"));
        assert!(!json.contains("token"));
        assert!(!json.contains("sgv"));
        assert_eq!(episodes(&[record]), Vec::new());
    }

    #[test]
    fn immutable_delivery_identity_beats_a_reused_name() {
        let mut record = rec(T, "Alex", "delivery", "LOW");
        record.site_id = Some("old-person".into());
        assert!(!delivery_matches(&record, "new-person", "Alex"));
        assert!(delivery_matches(&record, "old-person", "Renamed Alex"));

        record.site_id = None;
        assert!(delivery_matches(&record, "new-person", "Alex"));
    }

    const T: i64 = 1_700_000_000_000;

    #[test]
    fn an_episode_is_paired_from_its_start_and_end() {
        let records = vec![
            rec(T, "a", "alert", "URGENT LOW"),
            rec(T + 12 * 60_000, "a", "recovered", "in range"),
        ];
        let eps = episodes(&records);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].minutes(), Some(12));
        assert_eq!(eps[0].state, "URGENT LOW");
    }

    /// An episode still running is not an episode that lasted zero minutes, and
    /// must not be totalled as if its duration were known.
    #[test]
    fn an_unfinished_episode_has_no_duration() {
        let eps = episodes(&[rec(T, "a", "alert", "URGENT LOW")]);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].minutes(), None);
        assert_eq!(eps[0].end, None);
    }

    /// A low that becomes a high is a new emergency, not a continuation — the
    /// alarm machine treats it as a new episode and so must the log.
    #[test]
    fn a_change_of_variant_closes_the_previous_episode() {
        let records = vec![
            rec(T, "a", "alert", "URGENT LOW"),
            rec(T + 5 * 60_000, "a", "alert", "URGENT HIGH"),
            rec(T + 20 * 60_000, "a", "recovered", "in range"),
        ];
        let eps = episodes(&records);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].state, "URGENT LOW");
        assert_eq!(eps[0].minutes(), Some(5));
        assert_eq!(eps[1].state, "URGENT HIGH");
        assert_eq!(eps[1].minutes(), Some(15));
    }

    /// Two people being followed have independent episodes; pairing one's
    /// recovery with the other's alert would invent a duration.
    #[test]
    fn sites_do_not_pair_with_each_other() {
        let records = vec![
            rec(T, "alice", "alert", "URGENT LOW"),
            rec(T + 60_000, "bob", "alert", "HIGH"),
            rec(T + 10 * 60_000, "alice", "recovered", "in range"),
        ];
        let eps = episodes(&records);
        assert_eq!(eps.len(), 2);
        let alice = eps.iter().find(|e| e.site == "alice").unwrap();
        let bob = eps.iter().find(|e| e.site == "bob").unwrap();
        assert_eq!(alice.minutes(), Some(10));
        assert_eq!(bob.minutes(), None, "bob's episode is still open");
    }

    /// An empty log has two very different causes, and reporting "no alerts"
    /// for a watcher that never ran would be the most dangerous possible lie.
    #[test]
    fn an_empty_log_says_it_might_mean_nothing_was_watching() {
        let out = report(7, Units::Mgdl);
        assert!(out.contains("Nothing recorded"));
        assert!(out.contains("nothing was running to notice"));
        assert!(out.contains("watch --test"));
    }
}
