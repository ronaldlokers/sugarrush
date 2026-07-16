//! One-shot Waybar custom-module output: a single JSON line for the status bar.
//!
//! Prints `{text, tooltip, class, percentage}` and exits — no TUI. `text` is
//! the current value + trend arrow + delta; `tooltip` adds detail and a block
//! sparkline of the last hour; `class` is the alert state for CSS styling.

use anyhow::Result;
use serde_json::json;

use crate::alert;
use crate::config::Config;
use crate::nightscout::Client;

const HOUR_MS: i64 = 3_600_000;
const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Fetch the last hour and render the Waybar JSON line. Always returns valid
/// JSON, even on error, so Waybar has something to show.
pub async fn line(cfg: &Config) -> String {
    match build(cfg).await {
        Ok(s) => s,
        Err(e) => json!({
            "text": "—",
            "tooltip": format!("sugarrush: {e}"),
            "class": "stale",
        })
        .to_string(),
    }
}

async fn build(cfg: &Config) -> Result<String> {
    let sites = cfg.resolve_sites()?;
    let site = sites
        .first()
        .ok_or_else(|| anyhow::anyhow!("no site configured"))?;
    let client = Client::for_site(site)?;

    let now = chrono::Utc::now().timestamp_millis();
    let entries = client.entries_range(now - HOUR_MS, now, 100).await?;

    let latest = match entries.first() {
        Some(e) => e,
        None => {
            return Ok(json!({
                "text": "—",
                "tooltip": "sugarrush: no recent readings",
                "class": "stale",
            })
            .to_string())
        }
    };

    let units = cfg.units;
    let alerts = cfg.alerts.resolve(units);
    let state = alert::evaluate(latest.sgv, now - latest.date, &alerts);

    let delta = entries
        .get(1)
        .map(|prev| latest.sgv - prev.sgv)
        .map(|d| {
            format!(
                "{}{}",
                if d >= 0.0 { "+" } else { "-" },
                units.format(d.abs())
            )
        })
        .unwrap_or_else(|| "--".into());

    let text = format!("{} {} {}", units.format(latest.sgv), latest.arrow(), delta);

    // Oldest → newest values for the sparkline.
    let values: Vec<f64> = entries.iter().rev().map(|e| e.sgv).collect();
    let spark = sparkline(&values);
    let age_min = ((now - latest.date) / 60_000).max(0);
    let tooltip = format!(
        "{} {}  {}\nΔ {} · {}m ago\n{}",
        units.format(latest.sgv),
        units.label(),
        latest.direction.as_deref().unwrap_or("?"),
        delta,
        age_min,
        spark,
    );

    // Position within the alerting range, as a rough 0–100 for Waybar.
    let span = (alerts.urgent_high - alerts.urgent_low).max(1.0);
    let percentage = (((latest.sgv - alerts.urgent_low) / span * 100.0).clamp(0.0, 100.0)) as u8;

    Ok(json!({
        "text": text,
        "tooltip": tooltip,
        "class": state.class(),
        "percentage": percentage,
    })
    .to_string())
}

/// An 8-level block sparkline over the values (min→max normalized).
fn sparkline(values: &[f64]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let (min, max) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = (max - min).max(1.0);
    values
        .iter()
        .map(|&v| {
            let level = ((v - min) / range * (BARS.len() - 1) as f64).round() as usize;
            BARS[level.min(BARS.len() - 1)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_maps_range_to_bars() {
        assert_eq!(sparkline(&[]), "");
        // Min → lowest bar, max → highest bar.
        let s: Vec<char> = sparkline(&[1.0, 2.0, 3.0]).chars().collect();
        assert_eq!(s.first(), Some(&'▁'));
        assert_eq!(s.last(), Some(&'█'));
        // A flat series is all the same (lowest) bar.
        assert_eq!(sparkline(&[5.0, 5.0, 5.0]), "▁▁▁");
    }
}
