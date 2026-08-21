//! One JSON document describing the current state: reading, recent series,
//! time-in-range and pattern insights.
//!
//! `status.rs` renders a *line*; every format it knows is a string. A panel
//! wants a document, so it lives here rather than as a sixth `Format`.
//!
//! Split in two on purpose: [`build`] is pure and takes entries, so every
//! shape and conversion is unit-testable; `fetch` only talks to Nightscout.

use serde::Serialize;

use crate::config::{Alerts, Config, Site};
use crate::nightscout::{Client, Entry};
use crate::theme::Theme;
use crate::units::Units;

/// Everything [`build`] needs, so its signature does not grow to nine
/// positional arguments that callers can silently transpose.
pub struct BuildInput {
    pub now_ms: i64,
    pub units: Units,
    pub alerts: Alerts,
    pub theme: Theme,
    pub timezone: Option<chrono_tz::Tz>,
    /// Newest first, as Nightscout returns them — the last `--hours`.
    pub recent: Vec<Entry>,
    /// Newest first — the last `--days`. Empty when history was not asked for.
    pub history: Vec<Entry>,
    pub stats_window_h: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    /// Bumped only for a breaking change; adding a field is not one. A newer
    /// consumer against an older binary can then refuse a shape it doesn't
    /// know instead of guessing at it.
    pub schema: u8,
    pub units: &'static str,
    pub generated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// `[epoch_ms, value]` oldest first, in display units. Empty when there
    /// were no readings in the window — which the panel draws as a message,
    /// not as a flat line at zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<(i64, f64)>>,
    /// `None` with fewer than two readings: no mean worth printing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insights: Option<Vec<InsightDoc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reading {
    pub value: String,
    pub arrow: String,
    pub direction: String,
    pub delta: String,
    pub class: &'static str,
    pub color: String,
    pub age_min: i64,
}

/// Alert bounds in display units, so the panel can shade its target band
/// without knowing anything about mg/dL.
#[derive(Debug, Clone, Serialize)]
pub struct Range {
    pub urgent_low: f64,
    pub low: f64,
    pub high: f64,
    pub urgent_high: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub window_h: i64,
    pub mean: f64,
    pub gmi: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cv: Option<f64>,
    pub tir: TirDoc,
}

#[derive(Debug, Clone, Serialize)]
pub struct TirDoc {
    pub very_low: f64,
    pub low: f64,
    pub in_range: f64,
    pub high: f64,
    pub very_high: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightDoc {
    pub kind: &'static str,
    pub window: String,
    /// The worst value in the run, in display units.
    pub extreme: f64,
    pub text: String,
}

/// Round to one decimal for mmol/L, to whole numbers for mg/dL — matching how
/// `Units::format` renders, so a chart's axis and the hero never disagree.
fn scaled(units: Units, mgdl: f64) -> f64 {
    let value = units.from_mgdl(mgdl);
    match units {
        Units::Mmol => (value * 10.0).round() / 10.0,
        Units::Mgdl => value.round(),
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub fn build(input: BuildInput) -> Snapshot {
    let BuildInput {
        now_ms,
        units,
        alerts,
        theme,
        timezone,
        recent,
        history,
        stats_window_h,
    } = input;

    let now = recent.first().map(|latest| {
        let state = crate::alert::evaluate(latest.sgv, now_ms - latest.date, &alerts);
        let delta = recent
            .get(1)
            .map(|prev| units.format_delta(latest.sgv - prev.sgv))
            .unwrap_or_else(|| "--".into());
        Reading {
            value: units.format(latest.sgv),
            arrow: latest.arrow().to_string(),
            direction: latest.direction.clone().unwrap_or_else(|| "?".into()),
            delta,
            class: state.class(),
            color: crate::theme::hex(state.color(&theme)),
            age_min: ((now_ms - latest.date) / 60_000).max(0),
        }
    });

    // Oldest first: a chart draws left to right, and every consumer would
    // otherwise reverse it themselves.
    let series: Vec<(i64, f64)> = recent
        .iter()
        .rev()
        .map(|e| (e.date, scaled(units, e.sgv)))
        .collect();

    // Stats prefer the multi-day history when it was fetched: one query beats
    // two, and the 24h window is a slice of it.
    let stats_from = now_ms - stats_window_h * 3_600_000;
    let source = if history.is_empty() {
        &recent
    } else {
        &history
    };
    let stats_entries: Vec<Entry> = source
        .iter()
        .filter(|e| e.date >= stats_from)
        .cloned()
        .collect();
    let stats = stats_for(&stats_entries, units, &alerts, stats_window_h);

    let bands = crate::agp::profile_in(&history, timezone);
    let insights = crate::agp::insights(&bands, alerts.low, alerts.high)
        .iter()
        .map(|insight| InsightDoc {
            kind: match insight.kind {
                crate::agp::Pattern::Lows => "lows",
                crate::agp::Pattern::Highs => "highs",
            },
            window: insight.window(),
            extreme: scaled(units, insight.extreme),
            text: insight.text(units),
        })
        .collect();

    Snapshot {
        schema: 1,
        units: units.label(),
        generated_at: now_ms,
        error: None,
        now,
        range: Some(Range {
            urgent_low: scaled(units, alerts.urgent_low),
            low: scaled(units, alerts.low),
            high: scaled(units, alerts.high),
            urgent_high: scaled(units, alerts.urgent_high),
        }),
        series: Some(series),
        stats,
        insights: Some(insights),
    }
}

/// Stats over `entries`, labelled with the window they actually cover.
///
/// `window_h` is what the caller asked for, not what the readings span: with
/// `--days 0` the only entries are the chart's few hours, and reporting
/// "100% in range" over three hours as a 24-hour figure is a claim the data
/// does not support. So the label is the covered span, capped at the request.
fn stats_for(entries: &[Entry], units: Units, alerts: &Alerts, window_h: i64) -> Option<Stats> {
    if entries.len() < 2 {
        return None;
    }
    let newest = entries.iter().map(|e| e.date).max()?;
    let oldest = entries.iter().map(|e| e.date).min()?;
    let covered_h = ((newest - oldest) as f64 / 3_600_000.0).ceil() as i64;
    let window_h = covered_h.clamp(1, window_h);
    let mean = crate::stats::mean_mgdl(entries)?;
    let tir = crate::stats::tir(
        entries,
        alerts.urgent_low,
        alerts.low,
        alerts.high,
        alerts.urgent_high,
    )?;
    Some(Stats {
        window_h,
        mean: scaled(units, mean),
        gmi: round1(crate::stats::gmi(mean)),
        cv: crate::stats::cv_pct(entries).map(round1),
        tir: TirDoc {
            very_low: round1(tir.very_low),
            low: round1(tir.low),
            in_range: round1(tir.in_range),
            high: round1(tir.high),
            very_high: round1(tir.very_high),
        },
    })
}

/// A document that says only what went wrong. Exit code stays 0 and the shape
/// stays parseable, so a bar renders the message instead of a parse failure —
/// the same promise `status` makes.
pub fn error_doc(now_ms: i64, message: &str) -> Snapshot {
    Snapshot {
        schema: 1,
        // Nominal: there is no reading to express in any unit, and a consumer
        // that got this far reads `error` before anything else.
        units: Units::Mmol.label(),
        generated_at: now_ms,
        error: Some(message.to_string()),
        now: None,
        range: None,
        series: None,
        stats: None,
        insights: None,
    }
}

/// The range and row count for the multi-day history fetch, or `None` when the
/// caller asked for none. Separate from `fetch` so the "no history means no
/// query" rule is testable without a Nightscout.
pub fn history_window(days: u32, now_ms: i64) -> Option<(i64, i64, usize)> {
    if days == 0 {
        return None;
    }
    let start = now_ms - i64::from(days) * 24 * 3_600_000;
    let want = days as usize * 288 + 288;
    Some((start, now_ms, want))
}

/// Fetch and assemble. Never returns an error: a failure becomes a document
/// carrying the message, so the caller always has something to print.
pub async fn fetch(cfg: &Config, site: &Site, hours: u32, days: u32) -> Snapshot {
    let now_ms = chrono::Utc::now().timestamp_millis();
    match collect(cfg, site, hours, days, now_ms).await {
        Ok(snapshot) => snapshot,
        Err(e) => error_doc(now_ms, &e.to_string()),
    }
}

async fn collect(
    cfg: &Config,
    site: &Site,
    hours: u32,
    days: u32,
    now_ms: i64,
) -> anyhow::Result<Snapshot> {
    let client = Client::for_site(site)?;
    let (alerts, _warnings) = site.resolve_alerts(&cfg.alerts, cfg.units);

    let hours = hours.max(1);
    let recent_from = now_ms - i64::from(hours) * 3_600_000;
    let recent = client
        .entries_range(recent_from, now_ms, hours as usize * 12 + 12)
        .await?;

    let history = match history_window(days, now_ms) {
        Some((start, end, want)) => client.entries_range(start, end, want).await?,
        None => Vec::new(),
    };

    Ok(build(BuildInput {
        now_ms,
        units: cfg.units,
        alerts,
        theme: cfg.theme.resolve(),
        timezone: site
            .timezone
            .as_deref()
            .and_then(|name| name.parse::<chrono_tz::Tz>().ok()),
        recent,
        history,
        stats_window_h: 24,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Alerts;
    use crate::nightscout::Entry;
    use crate::theme::Theme;
    use crate::units::Units;

    const MIN: i64 = 60_000;

    fn entry(sgv: f64, date: i64, direction: &str) -> Entry {
        Entry {
            sgv,
            date,
            direction: Some(direction.into()),
        }
    }

    fn alerts() -> Alerts {
        Alerts {
            urgent_low: 55.0,
            low: 70.0,
            high: 180.0,
            urgent_high: 250.0,
            ..Alerts::default()
        }
    }

    /// now = 2026-08-21T17:00:00Z in epoch ms, so the test reads in absolutes.
    const NOW: i64 = 1_755_795_600_000;

    fn input(recent: Vec<Entry>, history: Vec<Entry>) -> BuildInput {
        BuildInput {
            now_ms: NOW,
            units: Units::Mmol,
            alerts: alerts(),
            theme: Theme::default(),
            timezone: None,
            recent,
            history,
            stats_window_h: 24,
        }
    }

    #[test]
    fn a_normal_document_carries_schema_units_and_the_current_reading() {
        let snap = build(input(
            vec![
                entry(115.0, NOW - 4 * MIN, "Flat"),
                entry(113.0, NOW - 9 * MIN, "Flat"),
            ],
            vec![],
        ));
        let json = serde_json::to_value(&snap).unwrap();

        assert_eq!(json["schema"], 1);
        assert_eq!(json["units"], "mmol/L");
        assert_eq!(json["generated_at"], NOW);
        assert_eq!(json["now"]["value"], "6.4");
        assert_eq!(json["now"]["arrow"], "→");
        assert_eq!(json["now"]["direction"], "Flat");
        assert_eq!(json["now"]["class"], "in-range");
        assert_eq!(json["now"]["age_min"], 4);
        // Colour comes from the configured theme, as a bar-ready hex string.
        assert_eq!(json["now"]["color"].as_str().unwrap().len(), 7);
        assert!(json["now"]["color"].as_str().unwrap().starts_with('#'));
        // No error key on a healthy document — consumers check for it first.
        assert!(json.get("error").is_none());
    }

    #[test]
    fn every_value_is_in_the_display_unit() {
        let mmol = build(input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]));
        let mmol_json = serde_json::to_value(&mmol).unwrap();
        assert_eq!(mmol_json["series"][0][1], 6.4);
        assert_eq!(mmol_json["range"]["low"], 3.9);
        assert_eq!(mmol_json["range"]["high"], 10.0);

        let mut mgdl_input = input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]);
        mgdl_input.units = Units::Mgdl;
        let mgdl_json = serde_json::to_value(build(mgdl_input)).unwrap();
        assert_eq!(mgdl_json["units"], "mg/dL");
        assert_eq!(mgdl_json["series"][0][1], 115.0);
        assert_eq!(mgdl_json["range"]["low"], 70.0);
        assert_eq!(mgdl_json["range"]["high"], 180.0);
    }

    #[test]
    fn no_readings_gives_empty_collections_rather_than_a_failure() {
        let json = serde_json::to_value(build(input(vec![], vec![]))).unwrap();

        assert_eq!(json["schema"], 1);
        assert_eq!(json["series"].as_array().unwrap().len(), 0);
        assert_eq!(json["insights"].as_array().unwrap().len(), 0);
        // Fewer than two readings: no mean worth printing, and the panel hides
        // the row rather than drawing zeroes.
        assert!(json.get("stats").is_none());
        assert!(json.get("now").is_none());
        assert!(json.get("error").is_none());
    }

    #[test]
    fn an_error_is_still_a_document() {
        let json = serde_json::to_value(error_doc(NOW, "no site configured")).unwrap();

        assert_eq!(json["schema"], 1);
        assert_eq!(json["generated_at"], NOW);
        assert_eq!(json["error"], "no site configured");
        // Absent, not null: a consumer checks for `error` first and never has
        // to tell "missing" from "null" to know there is nothing to draw.
        assert!(json.get("now").is_none());
        assert!(json.get("series").is_none());
        assert!(json.get("stats").is_none());
        assert!(json.get("insights").is_none());
    }

    /// Four days of readings: 60 mg/dL between 02:00 and 03:00 local, 110 the
    /// rest of the time. That clears `agp`'s two bars for a pattern — a run of
    /// at least 45 minutes, on at least three separate days.
    fn nightly_lows() -> Vec<Entry> {
        let day = 24 * 3_600_000i64;
        let mut out = Vec::new();
        for d in 1..=4i64 {
            let start = NOW - d * day;
            let midnight = start - start.rem_euclid(day);
            for minute in (0..24 * 60).step_by(5) {
                let at = midnight + minute as i64 * 60_000;
                let sgv = if (120..=180).contains(&minute) {
                    60.0
                } else {
                    110.0
                };
                out.push(entry(sgv, at, "Flat"));
            }
        }
        out.reverse(); // newest first, as Nightscout returns them
        out
    }

    #[test]
    fn a_pattern_becomes_an_insight_row() {
        let mut with_history = input(vec![], nightly_lows());
        // The fixture's clock times are UTC, and `agp` buckets by local time —
        // so without pinning this the window would move with the machine's
        // timezone and the assertion would only hold in one place.
        with_history.timezone = Some(chrono_tz::UTC);
        let json = serde_json::to_value(build(with_history)).unwrap();
        let first = &json["insights"][0];

        assert_eq!(first["kind"], "lows");
        assert_eq!(first["window"], "02:00–03:15");
        // Display units here too — the panel prints this without converting.
        assert_eq!(first["extreme"], 3.3);
        assert!(first["text"]
            .as_str()
            .unwrap()
            .starts_with("lows 02:00–03:15"));
    }

    #[test]
    fn asking_for_no_history_means_no_history_query() {
        // `--days 0` is how a caller says "chart only" — it must not turn into
        // a four-thousand-entry query with a zero-length range.
        assert!(history_window(0, NOW).is_none());

        let (start, end, want) = history_window(14, NOW).unwrap();
        assert_eq!(end, NOW);
        assert_eq!(start, NOW - 14 * 24 * 3_600_000);
        // 288 readings a day at five minutes apart, plus a day of slack so a
        // dense sensor doesn't get truncated at the far end.
        assert_eq!(want, 14 * 288 + 288);
    }

    #[test]
    fn stats_are_labelled_with_the_window_they_actually_cover() {
        // Three hours of readings and no history: the figures describe three
        // hours, so they may not be labelled as a day.
        let three_hours: Vec<Entry> = (0..36)
            .map(|i| entry(110.0, NOW - i * 5 * MIN, "Flat"))
            .collect();
        let json = serde_json::to_value(build(input(three_hours, vec![]))).unwrap();
        assert_eq!(json["stats"]["window_h"], 3);

        // A full day of history is labelled as the 24 hours it covers.
        let full_day: Vec<Entry> = (0..288)
            .map(|i| entry(110.0, NOW - i * 5 * MIN, "Flat"))
            .collect();
        let json = serde_json::to_value(build(input(vec![], full_day))).unwrap();
        assert_eq!(json["stats"]["window_h"], 24);
    }
}
