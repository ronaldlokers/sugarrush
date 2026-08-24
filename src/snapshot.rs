//! One JSON document describing the current state: reading, recent series,
//! time-in-range and pattern insights.
//!
//! `status.rs` renders a *line*; every format it knows is a string. A panel
//! wants a document, so it lives here rather than as a sixth `Format`.
//!
//! Split in two on purpose: [`build`] is pure and takes entries, so every
//! shape and conversion is unit-testable; `fetch` only talks to Nightscout.

use chrono::{Local, TimeZone};
use chrono_tz::Tz;
use serde::Serialize;

use crate::config::{Alerts, Config, Site};
use crate::nightscout::{Client, Entry};
use crate::theme::Theme;
use crate::units::Units;

/// Everything [`build`] needs, so its signature does not grow to nine
/// positional arguments that callers can silently transpose.
pub struct BuildInput {
    pub now_ms: i64,
    /// The site this document describes. Named, not implied: a consumer that
    /// wants to act on the reading — log a treatment against it, snooze it —
    /// has to say which site it means, and guessing "the first one" is not
    /// something a health-record write should do on someone's behalf.
    pub site: String,
    /// Whether this site has a treatment write token. The token itself never
    /// leaves the config — a consumer needs to know that a write is possible,
    /// never what it would be made with.
    pub can_write: bool,
    pub units: Units,
    pub alerts: Alerts,
    pub theme: Theme,
    pub timezone: Option<chrono_tz::Tz>,
    /// Newest first, as Nightscout returns them — the last `--hours`.
    pub recent: Vec<Entry>,
    /// Newest first — the last `--days`. Empty when history was not asked for.
    pub history: Vec<Entry>,
    /// Alarm episodes read from the local alert log, oldest first. Empty when
    /// nothing has fired, which is the ordinary case for a good fortnight.
    pub alert_records: Vec<crate::alertlog::Record>,
    /// Carbs and insulin logged inside the `recent` window. Empty when the
    /// site logs none, or when the request for them failed — a document
    /// without them is still a document.
    pub treatments: Vec<crate::nightscout::Treatment>,
    pub stats_window_h: i64,
    /// Epoch ms of the newest sensor start/change, if the site logs them.
    pub sensor_start_ms: Option<i64>,
    /// Expected sensor life in days; `0` means the age is reported without a
    /// countdown, because nothing here knows what "expiring" would mean.
    pub sensor_days: u32,
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
    /// The site the document is about, so a consumer can name it back.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub site: String,
    /// Whether a treatment could be written to it. A UI that offers to log
    /// one without this sends someone to a command that refuses them —
    /// `treatment` rejects a site with no write token, on purpose.
    pub can_write: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// Always present: a consumer has to paint something even when there is
    /// no reading to paint, and an absent palette would mean falling back to
    /// a hard-coded one at exactly the wrong moment.
    pub theme: ThemeDoc,
    /// `[epoch_ms, value]` oldest first, in display units. Empty when there
    /// were no readings in the window — which the panel draws as a message,
    /// not as a flat line at zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<(i64, f64)>>,
    /// `None` with fewer than two readings: no mean worth printing.
    /// Oldest first, inside the `series` window: what was done, so a chart can
    /// say why the line moved rather than only that it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treatments: Option<Vec<TreatmentDoc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
    /// The same figures over the whole history window, as something to read
    /// the recent ones against.
    ///
    /// "76% in range" is a number with no scale. Clinical targets exist, but
    /// the comparison that changes what someone does is against themselves a
    /// fortnight ago, and only this document can make it — a consumer has one
    /// window and no way to compute the other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Stats>,
    /// Time in range per local day, oldest first, over the history window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<Vec<DayDoc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insights: Option<Vec<InsightDoc>>,
    /// Alarm episodes over the history window, newest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Vec<AlertDoc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<BandDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor: Option<SensorDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast: Option<crate::predict::Outlook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<OverviewDoc>,
    /// The ambulatory glucose profile: one composite day, not a time series.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agp: Option<AgpDoc>,
}

/// The whole window folded onto a single 24-hour clock — the standard way a
/// clinic reads a CGM, and the one view that answers "what do my nights look
/// like" rather than "what happened last night".
#[derive(Debug, Clone, Serialize)]
pub struct AgpDoc {
    /// Distinct local dates behind the percentiles. Below `BAND_MIN_DAYS`
    /// there is no profile: with one day p25 and p50 are the same number, and
    /// a chart that cannot tell a habit from a Tuesday should not be drawn.
    pub days: usize,
    /// Minutes per bucket, so a consumer can size its own x-axis.
    pub step_min: i64,
    /// `[minute_of_local_day, p05, p25, p50, p75, p95]`, in display units.
    pub points: Vec<(i64, f64, f64, f64, f64, f64)>,
    /// `[minute_of_local_day, [one value per day]]`, in display units — the
    /// days behind the envelope, so a consumer can show the three that went
    /// low rather than the median that hides them.
    pub samples: Vec<(i64, Vec<f64>)>,
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
    /// When a reading stops counting as current. A consumer that draws an
    /// ageing number needs the same threshold the alarm uses, or it invents
    /// its own idea of "old" and disagrees with the thing making the noise.
    pub stale_minutes: i64,
}

/// The resolved palette, as hex.
///
/// Sent because a consumer that hard-codes a copy of these colours ignores
/// the colourblind preset — the one setting whose entire purpose is that
/// red and green are the wrong pair for some people. The single `now.color`
/// was never enough: a chart, a time-in-range bar and a sensor countdown all
/// paint states the current reading is not in.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeDoc {
    pub low: String,
    pub in_range: String,
    pub high: String,
    pub urgent: String,
    pub prediction: String,
    pub graph: String,
}

impl ThemeDoc {
    fn of(theme: &Theme) -> Self {
        let hex = crate::theme::hex;
        Self {
            low: hex(theme.low),
            in_range: hex(theme.in_range),
            high: hex(theme.high),
            urgent: hex(theme.urgent),
            prediction: hex(theme.prediction),
            graph: hex(theme.graph),
        }
    }
}

/// One logged carb or insulin entry. Never both zero: Nightscout logs notes,
/// finger sticks and sensor changes in the same collection, and a marker on a
/// chart with no amount behind it is a mark that means nothing.
#[derive(Debug, Clone, Serialize)]
pub struct TreatmentDoc {
    pub at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carbs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insulin: Option<f64>,
}

/// One local day's time in range.
///
/// Per day rather than folded onto one clock like the profile: the profile
/// answers "what do my nights look like", and this answers the other question
/// people actually track — whether it is getting better.
#[derive(Debug, Clone, Serialize)]
pub struct DayDoc {
    /// Local date, `YYYY-MM-DD`, in the site's timezone.
    pub date: String,
    /// How many readings the day is computed from. A day behind a sensor
    /// change has a handful, and a bar drawn from those is a rumour — the
    /// count is here so a consumer can say so instead of drawing it the same
    /// as a full day.
    pub readings: usize,
    pub tir: TirDoc,
}

/// One alarm episode, and what the delivery attempts around it did.
///
/// `alertlog.rs` has recorded every episode and every delivery outcome since
/// it was written, and nothing in any UI reads it. A failed push is the most
/// important line in that file and the least visible: it means an alarm was
/// raised and never arrived.
#[derive(Debug, Clone, Serialize)]
pub struct AlertDoc {
    pub at_ms: i64,
    /// The alert label, e.g. `"URGENT LOW"`.
    pub state: String,
    /// How long it lasted. Absent when it was still open at the end of the
    /// window — not the same as an episode that lasted no time at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i64>,
    /// The reading when it began, in display units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Delivery channels that failed while it was open, e.g. `"push"`. Never
    /// a destination or a token — the log does not hold those either.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub window_h: i64,
    /// How many readings the figures are computed from.
    ///
    /// A percentage over half a day of data is not a smaller truth than one
    /// over a whole day — it is a different claim, and without this a window
    /// with a four-hour sensor gap draws exactly the same confident stripe as
    /// a complete one.
    pub readings: usize,
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

/// The shape of a typical day across the charted window: the median with the
/// interquartile band around it, one point per `agp` bucket.
///
/// Absolute timestamps rather than times of day, because the profile is
/// bucketed in the followed person's timezone and a consumer plotting it
/// should not have to redo that conversion to line the band up with the line.
#[derive(Debug, Clone, Serialize)]
pub struct BandDoc {
    /// Distinct local dates behind the percentiles.
    pub days: usize,
    /// `[epoch_ms, p25, p50, p75]`, oldest first, in display units.
    pub points: Vec<(i64, f64, f64, f64)>,
}

/// The history the band was built from, at a step a panel can scroll across.
///
/// It costs nothing extra: those readings were already fetched for the
/// percentiles and the patterns, and were being thrown away. One point per
/// quarter hour is finer than a strip a few hundred pixels wide can show, and
/// a fourteen-day window is about 1300 of them.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewDoc {
    pub step_min: i64,
    /// `[epoch_ms, value]`, oldest first, in display units.
    pub points: Vec<(i64, f64)>,
}

/// How long the sensor has been running, and — when its expected life is
/// known — how much of that is left.
#[derive(Debug, Clone, Serialize)]
pub struct SensorDoc {
    pub started_at: i64,
    pub age_h: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_h: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired: Option<bool>,
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
        site,
        can_write,
        units,
        alerts,
        theme,
        timezone,
        recent,
        history,
        alert_records,
        treatments,
        stats_window_h,
        sensor_start_ms,
        sensor_days,
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
    // Over everything fetched, not a fixed fortnight: the window is whatever
    // `--days` asked for, and the figure is named by the span it covers.
    let baseline = stats_for(&history, units, &alerts, 24 * 400);

    // Oldest first, to match `series`, and only the ones with an amount: a
    // note or a finger stick logged in the same collection would otherwise
    // become a marker standing for nothing.
    let window_from = series.first().map(|(at, _)| *at).unwrap_or(i64::MIN);
    let mut logged: Vec<TreatmentDoc> = treatments
        .iter()
        .filter(|t| t.at_ms >= window_from && t.at_ms <= now_ms)
        .filter(|t| t.carbs.is_some_and(|c| c > 0.0) || t.insulin.is_some_and(|i| i > 0.0))
        .map(|t| TreatmentDoc {
            at_ms: t.at_ms,
            carbs: t.carbs.filter(|c| *c > 0.0),
            insulin: t.insulin.filter(|i| *i > 0.0),
        })
        .collect();
    logged.sort_by_key(|t| t.at_ms);

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

    let band = band_for(&history, &series, timezone, units);
    let sensor = sensor_for(sensor_start_ms, sensor_days, now_ms);
    // `predict::outlook` returns nothing when the two newest readings are not
    // a normal CGM step apart — a sensor gap extrapolated as if it were five
    // minutes is how a benign drift once became a "heading low" — so an absent
    // forecast here is a deliberate silence, not a failure.
    let forecast = crate::predict::outlook(&recent, &alerts, units);
    let overview = overview_for(&history, units);
    let agp = agp_for(&bands, &crate::agp::samples_in(&history, timezone), units);

    Snapshot {
        schema: 1,
        units: units.label(),
        generated_at: now_ms,
        error: None,
        site,
        can_write,
        now,
        range: Some(Range {
            urgent_low: scaled(units, alerts.urgent_low),
            low: scaled(units, alerts.low),
            high: scaled(units, alerts.high),
            urgent_high: scaled(units, alerts.urgent_high),
            stale_minutes: alerts.stale_minutes,
        }),
        theme: ThemeDoc::of(&theme),
        series: Some(series),
        treatments: Some(logged),
        stats,
        baseline,
        days: days_for(&history, timezone, &alerts),
        insights: Some(insights),
        alerts: Some(alerts_for(&alert_records, units, now_ms)),
        band,
        agp,
        sensor,
        forecast,
        overview,
    }
}

/// One point per quarter hour across the history, newest value in each bucket.
///
/// The newest rather than the mean: a mean smooths away the peak that made
/// someone scroll back to look at it in the first place.
const OVERVIEW_STEP_MIN: i64 = 15;

fn overview_for(history: &[Entry], units: Units) -> Option<OverviewDoc> {
    if history.is_empty() {
        return None;
    }
    let step = OVERVIEW_STEP_MIN * 60_000;
    let mut points: Vec<(i64, f64)> = Vec::new();
    let mut last_bucket = i64::MIN;
    // History arrives newest first; walking it in reverse keeps the output
    // oldest first without a sort.
    for entry in history.iter().rev() {
        let bucket = entry.date / step * step;
        if bucket == last_bucket {
            if let Some(point) = points.last_mut() {
                point.1 = scaled(units, entry.sgv);
            }
            continue;
        }
        last_bucket = bucket;
        points.push((bucket, scaled(units, entry.sgv)));
    }
    (!points.is_empty()).then_some(OverviewDoc {
        step_min: OVERVIEW_STEP_MIN,
        points,
    })
}

/// The sensor's age, and its remaining life when one is configured.
fn sensor_for(started_at: Option<i64>, days: u32, now_ms: i64) -> Option<SensorDoc> {
    let started_at = started_at?;
    let age_h = ((now_ms - started_at) / 3_600_000).max(0);
    if days == 0 {
        // The age is a fact; "expiring" would be a guess, so it is left unsaid
        // rather than assumed from someone else's sensor.
        return Some(SensorDoc {
            started_at,
            age_h,
            expires_in_h: None,
            expired: None,
        });
    }
    let life_h = i64::from(days) * 24;
    Some(SensorDoc {
        started_at,
        age_h,
        expires_in_h: Some((life_h - age_h).max(0)),
        expired: Some(age_h >= life_h),
    })
}

/// A band is a claim about a typical day, so it needs several days behind it.
/// Same bar `agp::insights` sets before it will name a pattern: below this the
/// percentiles describe one or two nights, not a habit.
const BAND_MIN_DAYS: usize = 3;

/// The percentile profile as one composite day.
fn agp_for(
    bands: &[crate::agp::Band],
    samples: &[(i64, Vec<f64>)],
    units: Units,
) -> Option<AgpDoc> {
    let days = bands.iter().map(|b| b.days).max().unwrap_or(0);
    if bands.is_empty() || days < BAND_MIN_DAYS {
        return None;
    }
    Some(AgpDoc {
        days,
        step_min: crate::agp::BUCKET_MIN,
        points: bands
            .iter()
            .map(|b| {
                (
                    b.minute,
                    scaled(units, b.p05),
                    scaled(units, b.p25),
                    scaled(units, b.p50),
                    scaled(units, b.p75),
                    scaled(units, b.p95),
                )
            })
            .collect(),
        samples: samples
            .iter()
            .map(|(minute, values)| {
                (
                    *minute,
                    values.iter().map(|value| scaled(units, *value)).collect(),
                )
            })
            .collect(),
    })
}

/// The profile sampled across the window `series` covers, one point per bucket.
fn band_for(
    history: &[Entry],
    series: &[(i64, f64)],
    timezone: Option<chrono_tz::Tz>,
    units: Units,
) -> Option<BandDoc> {
    let (first, last) = (series.first()?.0, series.last()?.0);
    let bands = crate::agp::profile_in(history, timezone);
    if bands.is_empty() {
        return None;
    }
    let days = bands.iter().map(|b| b.days).max().unwrap_or(0);
    if days < BAND_MIN_DAYS {
        return None;
    }

    let step = crate::agp::BUCKET_MIN * 60_000;
    let mut points = Vec::new();
    let mut at = first - first.rem_euclid(step);
    while at <= last {
        if let Some(band) = bucket_at(&bands, at, timezone) {
            points.push((
                at,
                scaled(units, band.p25),
                scaled(units, band.p50),
                scaled(units, band.p75),
            ));
        }
        at += step;
    }
    (!points.is_empty()).then_some(BandDoc { days, points })
}

/// The profile bucket covering `at`, by local time of day.
fn bucket_at(
    bands: &[crate::agp::Band],
    at: i64,
    timezone: Option<chrono_tz::Tz>,
) -> Option<&crate::agp::Band> {
    use chrono::{TimeZone, Timelike};
    let minute = match timezone {
        Some(tz) => {
            let local = tz.timestamp_millis_opt(at).single()?;
            i64::from(local.hour() * 60 + local.minute())
        }
        None => {
            let local = chrono::Local.timestamp_millis_opt(at).single()?;
            i64::from(local.hour() * 60 + local.minute())
        }
    };
    // Buckets carry their centre minute, and `profile_in` omits empty ones, so
    // this matches on the bucket index rather than assuming a dense slice.
    let index = minute / crate::agp::BUCKET_MIN;
    bands
        .iter()
        .find(|b| b.minute / crate::agp::BUCKET_MIN == index)
}

/// Stats over `entries`, labelled with the window they actually cover.
///
/// `window_h` is what the caller asked for, not what the readings span: with
/// `--days 0` the only entries are the chart's few hours, and reporting
/// "100% in range" over three hours as a 24-hour figure is a claim the data
/// does not support. So the label is the covered span, capped at the request.
/// Episodes and the deliveries that failed during them, newest first.
///
/// Failures are attached to the episode they happened in rather than listed
/// separately: "a push failed at 04:55" means nothing on its own, and "the
/// 04:55 low never reached your phone" means everything.
fn alerts_for(records: &[crate::alertlog::Record], units: Units, now_ms: i64) -> Vec<AlertDoc> {
    let mut out: Vec<AlertDoc> = crate::alertlog::episodes(records)
        .into_iter()
        .map(|episode| {
            let ends = episode.end.unwrap_or(now_ms);
            let failed: Vec<String> = records
                .iter()
                .filter(|r| r.event == "delivery" && r.ts >= episode.start && r.ts <= ends)
                .filter(|r| r.outcome.as_deref().is_some_and(|o| o != "accepted"))
                .filter_map(|r| r.channel.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            AlertDoc {
                at_ms: episode.start,
                state: episode.state.clone(),
                minutes: episode.minutes(),
                value: episode.sgv.map(|sgv| scaled(units, sgv)),
                failed,
            }
        })
        .collect();
    // Newest first: a list of alarms is read from the most recent one back.
    out.sort_by_key(|episode| std::cmp::Reverse(episode.at_ms));
    out
}

/// Time in range for each local day in `entries`, oldest first.
///
/// Grouped in the site's timezone, like the profile: a day boundary is a local
/// midnight, and someone in Auckland reading a UTC split would see every night
/// cut in half.
fn days_for(entries: &[Entry], timezone: Option<Tz>, alerts: &Alerts) -> Option<Vec<DayDoc>> {
    if entries.is_empty() {
        return None;
    }
    let mut by_date: std::collections::BTreeMap<String, Vec<Entry>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        let date = match timezone {
            Some(tz) => tz
                .timestamp_millis_opt(entry.date)
                .single()
                .map(|dt| dt.date_naive().to_string()),
            None => Local
                .timestamp_millis_opt(entry.date)
                .single()
                .map(|dt| dt.date_naive().to_string()),
        };
        if let Some(date) = date {
            by_date.entry(date).or_default().push(entry.clone());
        }
    }

    // BTreeMap keys are `YYYY-MM-DD` strings, which sort chronologically.
    let days: Vec<DayDoc> = by_date
        .into_iter()
        .filter_map(|(date, entries)| {
            let tir = crate::stats::tir(
                &entries,
                alerts.urgent_low,
                alerts.low,
                alerts.high,
                alerts.urgent_high,
            )?;
            Some(DayDoc {
                date,
                readings: entries.len(),
                tir: TirDoc {
                    very_low: round1(tir.very_low),
                    low: round1(tir.low),
                    in_range: round1(tir.in_range),
                    high: round1(tir.high),
                    very_high: round1(tir.very_high),
                },
            })
        })
        .collect();
    (!days.is_empty()).then_some(days)
}

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
        readings: entries.len(),
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
pub fn error_doc(now_ms: i64, theme: &Theme, message: &str) -> Snapshot {
    Snapshot {
        schema: 1,
        // Nominal: there is no reading to express in any unit, and a consumer
        // that got this far reads `error` before anything else.
        units: Units::Mmol.label(),
        generated_at: now_ms,
        error: Some(message.to_string()),
        // Nothing was read, so there is no site to name.
        site: String::new(),
        can_write: false,
        now: None,
        range: None,
        treatments: None,
        // Even a failure is drawn in the user's colours: the message and the
        // frame around it are the only things on screen, and falling back to
        // a built-in palette here would ignore the colourblind preset at the
        // one moment the panel has nothing else to say.
        theme: ThemeDoc::of(theme),
        series: None,
        stats: None,
        baseline: None,
        days: None,
        insights: None,
        alerts: None,
        band: None,
        sensor: None,
        forecast: None,
        overview: None,
        agp: None,
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
        Err(e) => error_doc(now_ms, &cfg.theme.resolve(), &e.to_string()),
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

    // Best-effort: a site whose uploader logs no sensor changes still gets a
    // document, just without the sensor block.
    let sensor_start_ms = client.sensor_start().await.ok().flatten();

    // Also best-effort. Plenty of people log nothing, and a site that answers
    // the entries query but not this one should still get a chart.
    let treatments = client
        .treatments(recent_from, now_ms)
        .await
        .unwrap_or_default();

    // Local, not fetched: the alert log is what this machine's alarm did, and
    // it is the only record of a delivery that never arrived.
    let alert_records = crate::alertlog::read(now_ms - i64::from(days.max(1)) * 24 * 3_600_000);

    Ok(build(BuildInput {
        now_ms,
        site: site.name.clone(),
        can_write: site
            .write_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty()),
        units: cfg.units,
        alerts,
        theme: cfg.theme.resolve(),
        timezone: site
            .timezone
            .as_deref()
            .and_then(|name| name.parse::<chrono_tz::Tz>().ok()),
        recent,
        history,
        alert_records,
        treatments,
        stats_window_h: 24,
        sensor_start_ms,
        sensor_days: cfg.sensor_days.min(30),
    }))
}

/// Synthetic data in the real document's shape: a panel to look at with no
/// site configured, and the screenshots the README needs.
pub fn demo(
    units: Units,
    alerts: Alerts,
    theme: Theme,
    hours: u32,
    days: u32,
    now_ms: i64,
) -> Snapshot {
    let hours = hours.max(1);
    let recent_from = now_ms - i64::from(hours) * 3_600_000;
    // demo::entries already returns newest first, as Nightscout does, so the
    // rest of this module reads it unchanged.
    let recent = crate::demo::entries(recent_from, now_ms);
    let history = match history_window(days, now_ms) {
        Some((start, end, _)) => crate::demo::entries(start, end),
        None => Vec::new(),
    };

    build(BuildInput {
        now_ms,
        site: "demo".to_string(),
        // Demo mode writes nothing anywhere; offering to would be a lie.
        can_write: false,
        units,
        alerts,
        theme,
        timezone: None,
        recent,
        history,
        // Two of them, spaced the way a meal and its bolus actually land, so
        // the demo chart shows the markers rather than an empty lane.
        // Demo mode has no alarm history, and inventing one would put words
        // in the mouth of the thing people trust overnight.
        alert_records: Vec::new(),
        treatments: crate::demo::treatments(now_ms),
        stats_window_h: 24,
        // Demo data has no sensor history to speak of, and inventing a
        // countdown would be the one number on screen that means nothing.
        sensor_start_ms: None,
        sensor_days: 0,
    })
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
            site: "Alex".to_string(),
            can_write: true,
            units: Units::Mmol,
            alerts: alerts(),
            theme: Theme::default(),
            // Fixed, not the machine's: anything that groups by local day —
            // `days_for`, the profile — otherwise splits `NOW` differently in
            // every offset. That is not hypothetical; the day-strip test
            // passed at +02:00 and failed in CI's UTC for exactly this.
            timezone: Some(chrono_tz::UTC),
            recent,
            history,
            alert_records: Vec::new(),
            treatments: Vec::new(),
            stats_window_h: 24,
            sensor_start_ms: None,
            sensor_days: 0,
        }
    }

    /// A colourblind palette, the one preset whose whole point is that the
    /// consumer must not be painting in red and green.
    fn colorblind() -> Theme {
        crate::theme::theme_from_names(&crate::theme::COLORBLIND_NAMES.map(|name| name.to_string()))
    }

    #[test]
    fn the_document_carries_the_palette_it_was_built_with() {
        let snap = build(BuildInput {
            theme: colorblind(),
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let json = serde_json::to_value(&snap).unwrap();

        // Blue for low and white for in-range, not the default red and green:
        // a consumer that hard-codes a palette would draw the wrong two.
        let expect = colorblind();
        assert_eq!(json["theme"]["low"], crate::theme::hex(expect.low));
        assert_eq!(
            json["theme"]["in_range"],
            crate::theme::hex(expect.in_range)
        );
        assert_eq!(json["theme"]["high"], crate::theme::hex(expect.high));
        assert_eq!(json["theme"]["urgent"], crate::theme::hex(expect.urgent));
        assert_eq!(
            json["theme"]["prediction"],
            crate::theme::hex(expect.prediction)
        );
        assert_eq!(json["theme"]["graph"], crate::theme::hex(expect.graph));
        assert_ne!(
            json["theme"]["low"],
            crate::theme::hex(Theme::default().low),
            "the default palette reached the document instead of the configured one"
        );
    }

    #[test]
    fn even_a_failure_is_drawn_in_the_configured_palette() {
        let json = serde_json::to_value(error_doc(NOW, &colorblind(), "site unreachable")).unwrap();
        assert_eq!(json["error"], "site unreachable");
        assert_eq!(
            json["theme"]["urgent"],
            crate::theme::hex(colorblind().urgent)
        );
    }

    #[test]
    fn the_document_names_the_site_it_describes() {
        // A consumer that acts on the reading has to name the site back —
        // `sugarrush treatment` refuses to write without one, on purpose.
        let snap = build(BuildInput {
            site: "Alex's Libre".to_string(),
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["site"], "Alex's Libre");
    }

    #[test]
    fn logged_carbs_and_insulin_reach_the_document_oldest_first() {
        use crate::nightscout::Treatment;
        let snap = build(BuildInput {
            treatments: vec![
                Treatment {
                    at_ms: NOW - 20 * MIN,
                    carbs: None,
                    insulin: Some(1.5),
                },
                Treatment {
                    at_ms: NOW - 50 * MIN,
                    carbs: Some(40.0),
                    insulin: Some(4.5),
                },
            ],
            ..input(
                vec![
                    entry(115.0, NOW - 4 * MIN, "Flat"),
                    entry(113.0, NOW - 60 * MIN, "Flat"),
                ],
                vec![],
            )
        });
        let json = serde_json::to_value(&snap).unwrap();

        // Oldest first, like `series`: a chart draws left to right and should
        // not have to reverse one of the two.
        assert_eq!(json["treatments"][0]["at_ms"], NOW - 50 * MIN);
        assert_eq!(json["treatments"][0]["carbs"], 40.0);
        assert_eq!(json["treatments"][0]["insulin"], 4.5);
        assert_eq!(json["treatments"][1]["at_ms"], NOW - 20 * MIN);
        assert_eq!(json["treatments"][1]["insulin"], 1.5);
        // Absent rather than zero: the marker for it is not drawn at all.
        assert!(json["treatments"][1].get("carbs").is_none());
    }

    #[test]
    fn a_treatment_with_no_amount_is_not_a_marker() {
        use crate::nightscout::Treatment;
        // Nightscout keeps notes, finger sticks and sensor changes in the same
        // collection. A mark on the chart standing for none of the two things
        // this draws would mean nothing.
        let snap = build(BuildInput {
            treatments: vec![
                Treatment {
                    at_ms: NOW - 10 * MIN,
                    carbs: None,
                    insulin: None,
                },
                Treatment {
                    at_ms: NOW - 11 * MIN,
                    carbs: Some(0.0),
                    insulin: Some(0.0),
                },
            ],
            ..input(
                vec![
                    entry(115.0, NOW - 4 * MIN, "Flat"),
                    entry(113.0, NOW - 60 * MIN, "Flat"),
                ],
                vec![],
            )
        });
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(
            json["treatments"].as_array().map(Vec::len),
            Some(0),
            "an amountless treatment became a marker"
        );
    }

    #[test]
    fn a_treatment_from_before_the_window_is_left_out() {
        use crate::nightscout::Treatment;
        // Nightscout's own `count` default can hand back more history than was
        // asked for; a marker outside the plotted window would be drawn at the
        // wrong end of the chart or off it.
        let snap = build(BuildInput {
            treatments: vec![Treatment {
                at_ms: NOW - 400 * MIN,
                carbs: Some(30.0),
                insulin: None,
            }],
            ..input(
                vec![
                    entry(115.0, NOW - 4 * MIN, "Flat"),
                    entry(113.0, NOW - 60 * MIN, "Flat"),
                ],
                vec![],
            )
        });
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["treatments"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn the_document_says_whether_a_treatment_could_be_written() {
        // A UI that offers to log one against a site with no write token
        // sends someone to a command that refuses them.
        let yes = build(BuildInput {
            can_write: true,
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let no = build(BuildInput {
            can_write: false,
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        assert_eq!(serde_json::to_value(&yes).unwrap()["can_write"], true);
        assert_eq!(serde_json::to_value(&no).unwrap()["can_write"], false);

        // And never the token itself, whatever else the document grows.
        let text = serde_json::to_string(&yes).unwrap();
        assert!(!text.contains("token"), "the document mentions a token");
    }

    #[test]
    fn the_baseline_covers_the_history_the_recent_window_is_a_slice_of() {
        // Two days of history, the newest of which is also the stats window:
        // the two figures must be computed over different sets, or the
        // comparison is a number against itself.
        let mut history = Vec::new();
        // A steady 200 across the older day, a steady 100 across today.
        for step in 0..24 {
            history.push(entry(200.0, NOW - (30 + step) * 60 * MIN, "Flat"));
        }
        for step in 0..12 {
            history.push(entry(100.0, NOW - step * 60 * MIN, "Flat"));
        }
        let snap = build(input(
            vec![history[24].clone(), history[25].clone()],
            history,
        ));
        let json = serde_json::to_value(&snap).unwrap();

        let recent = json["stats"]["mean"].as_f64().unwrap();
        let baseline = json["baseline"]["mean"].as_f64().unwrap();
        assert!(
            recent < baseline,
            "today ({recent}) should read lower than the fortnight ({baseline})"
        );
        // And the baseline names the span it actually covers, not the recent
        // window's, or the label would claim a day of history for a fortnight.
        assert!(
            json["baseline"]["window_h"].as_i64().unwrap()
                > json["stats"]["window_h"].as_i64().unwrap(),
            "the baseline covers no more ground than the recent window"
        );
    }

    #[test]
    fn each_local_day_gets_its_own_time_in_range() {
        // Two days, told apart by their readings: a low day and a good one.
        //
        // Half-hourly rather than hourly, and 15 hours rather than 24: `NOW`
        // is 17:00 UTC, so a full day back from it straddles midnight and the
        // newest day would hold only the 18 readings after it.
        let day_ms = 24 * 60 * MIN;
        let mut history = Vec::new();
        for step in 0..30 {
            history.push(entry(60.0, NOW - day_ms - step * 30 * MIN, "Flat"));
        }
        for step in 0..30 {
            history.push(entry(110.0, NOW - step * 30 * MIN, "Flat"));
        }
        let snap = build(input(
            vec![history[24].clone(), history[25].clone()],
            history,
        ));
        let json = serde_json::to_value(&snap).unwrap();
        let days = json["days"].as_array().unwrap();

        assert!(days.len() >= 2, "expected a day each, got {}", days.len());
        // Oldest first, and the older day is the one below range.
        let first = &days[0];
        let last = &days[days.len() - 1];
        assert!(
            first["date"].as_str() < last["date"].as_str(),
            "not oldest first"
        );
        // 60 mg/dL is below `low` (70) but above `urgent_low` (55), so it
        // lands in the low band rather than the very-low one.
        let below =
            first["tir"]["very_low"].as_f64().unwrap() + first["tir"]["low"].as_f64().unwrap();
        assert!(below > 0.0, "the low day did not read as low, got {below}");
        assert_eq!(last["tir"]["in_range"], 100.0);
        // The count is what lets a consumer tell a full day from a rumour.
        assert!(last["readings"].as_u64().unwrap() >= 20);
    }

    #[test]
    fn days_split_at_local_midnight_not_utc() {
        // 22:30 and 23:30 UTC. In Auckland (+12) both are already tomorrow,
        // so a UTC split would cut the night in half and report two days
        // where the person had one.
        let utc_evening = chrono::DateTime::parse_from_rfc3339("2026-08-21T22:30:00Z")
            .unwrap()
            .timestamp_millis();
        let history = vec![
            entry(110.0, utc_evening, "Flat"),
            entry(112.0, utc_evening + 60 * MIN, "Flat"),
        ];
        let snap = build(BuildInput {
            timezone: Some("Pacific/Auckland".parse().unwrap()),
            ..input(history.clone(), history)
        });
        let json = serde_json::to_value(&snap).unwrap();
        let days = json["days"].as_array().unwrap();
        assert_eq!(days.len(), 1, "one local day, not two");
        assert_eq!(days[0]["date"], "2026-08-22", "grouped in UTC, not locally");
    }

    #[test]
    fn the_stats_say_how_many_readings_they_stand_on() {
        // A window with a sensor gap draws the same confident bar as a whole
        // one unless the count comes with it.
        let sparse: Vec<Entry> = (0..6)
            .map(|step| entry(110.0, NOW - step * 60 * MIN, "Flat"))
            .collect();
        let snap = build(input(sparse.clone(), sparse));
        let json = serde_json::to_value(&snap).unwrap();

        assert_eq!(json["stats"]["readings"], 6);
        // `window_h` is the span actually covered — first reading to last —
        // not the span asked for. Six hourly readings cover five hours, and
        // six readings across five hours is 8% of the ~60 a CGM would produce
        // in them. A consumer can only say that if it is told both numbers.
        assert_eq!(json["stats"]["window_h"], 5);
    }

    fn alert_record(ts: i64, event: &str, state: &str) -> crate::alertlog::Record {
        crate::alertlog::Record {
            ts,
            site: "Alex".into(),
            site_id: None,
            event: event.into(),
            state: state.into(),
            sgv: Some(58.0),
            channel: None,
            outcome: None,
        }
    }

    #[test]
    fn an_alarm_carries_the_delivery_that_failed_during_it() {
        // "A push failed at 04:55" means nothing on its own. "The 04:55 low
        // never reached your phone" means everything, so the failure belongs
        // to the episode rather than to a list of its own.
        let start = NOW - 60 * MIN;
        let mut push = alert_record(start + 10 * MIN, "delivery", "URGENT LOW");
        push.channel = Some("push".into());
        push.outcome = Some("failed".into());
        let mut desktop = alert_record(start + 11 * MIN, "delivery", "URGENT LOW");
        desktop.channel = Some("desktop".into());
        desktop.outcome = Some("accepted".into());

        let records = vec![
            alert_record(start, "alert", "URGENT LOW"),
            push,
            desktop,
            alert_record(start + 20 * MIN, "recovered", "URGENT LOW"),
        ];
        let snap = build(BuildInput {
            alert_records: records,
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let json = serde_json::to_value(&snap).unwrap();

        assert_eq!(json["alerts"][0]["state"], "URGENT LOW");
        assert_eq!(json["alerts"][0]["minutes"], 20);
        // Only the one that failed — an accepted delivery is not news.
        assert_eq!(json["alerts"][0]["failed"][0], "push");
        assert_eq!(
            json["alerts"][0]["failed"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn alarms_are_newest_first() {
        let records = vec![
            alert_record(NOW - 300 * MIN, "alert", "LOW"),
            alert_record(NOW - 290 * MIN, "recovered", "LOW"),
            alert_record(NOW - 60 * MIN, "alert", "HIGH"),
            alert_record(NOW - 50 * MIN, "recovered", "HIGH"),
        ];
        let snap = build(BuildInput {
            alert_records: records,
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let json = serde_json::to_value(&snap).unwrap();
        // A list of alarms is read from the most recent one back.
        assert_eq!(json["alerts"][0]["state"], "HIGH");
        assert_eq!(json["alerts"][1]["state"], "LOW");
    }

    #[test]
    fn the_range_carries_the_threshold_a_reading_goes_stale_at() {
        // A consumer that draws an ageing reading needs the number the alarm
        // uses, not one of its own.
        let snap = build(BuildInput {
            alerts: Alerts {
                stale_minutes: 9,
                ..alerts()
            },
            ..input(vec![entry(115.0, NOW - 4 * MIN, "Flat")], vec![])
        });
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["range"]["stale_minutes"], 9);
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
        let json =
            serde_json::to_value(error_doc(NOW, &Theme::default(), "no site configured")).unwrap();

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

    #[test]
    fn the_demo_document_has_the_same_shape_as_a_real_one() {
        let json = serde_json::to_value(demo(Units::Mmol, alerts(), Theme::default(), 6, 14, NOW))
            .unwrap();

        assert_eq!(json["schema"], 1);
        assert!(json.get("error").is_none());
        assert!(json["now"]["value"].is_string());
        // Six hours of five-minute readings, so a chart has something to draw.
        assert!(json["series"].as_array().unwrap().len() > 50);
        assert!(json["stats"]["tir"]["in_range"].is_number());
    }

    #[test]
    fn the_band_describes_a_typical_day_across_the_chart_window() {
        let mut with_history = input(
            vec![
                entry(115.0, NOW - 5 * MIN, "Flat"),
                entry(113.0, NOW - 65 * MIN, "Flat"),
            ],
            nightly_lows(),
        );
        with_history.timezone = Some(chrono_tz::UTC);
        let json = serde_json::to_value(build(with_history)).unwrap();

        assert_eq!(json["band"]["days"], 4);
        let points = json["band"]["points"].as_array().unwrap();
        // One point per 15-minute bucket across the charted window, so the
        // band lines up with the line drawn over it.
        assert!(points.len() >= 4, "got {} points", points.len());
        let first = points[0].as_array().unwrap();
        assert_eq!(first.len(), 4, "each point is [ts, p25, p50, p75]");
        // Display units, like every other value in the document.
        let p50 = first[2].as_f64().unwrap();
        assert!((2.0..=12.0).contains(&p50), "p50 {p50} is not mmol/L");
        // Ordered percentiles, ordered timestamps.
        assert!(first[1].as_f64().unwrap() <= p50);
        assert!(p50 <= first[3].as_f64().unwrap());
        assert!(
            points[0].as_array().unwrap()[0].as_i64().unwrap()
                < points[1].as_array().unwrap()[0].as_i64().unwrap()
        );
    }

    #[test]
    fn one_days_history_is_not_a_typical_day() {
        // A band drawn from a day or two describes those days, not a habit —
        // the same bar `agp` sets before it will name a pattern.
        let day = 24 * 3_600_000i64;
        let mut one_day: Vec<Entry> = (0..288)
            .map(|i| entry(110.0, NOW - day - i * 5 * MIN, "Flat"))
            .collect();
        one_day.reverse();
        let mut short = input(vec![entry(115.0, NOW - 5 * MIN, "Flat")], one_day);
        short.timezone = Some(chrono_tz::UTC);
        let json = serde_json::to_value(build(short)).unwrap();

        assert!(json.get("band").is_none());
    }

    #[test]
    fn the_profile_is_a_whole_day_whatever_the_chart_shows() {
        let mut with_history = input(
            vec![
                entry(115.0, NOW - 5 * MIN, "Flat"),
                entry(113.0, NOW - 65 * MIN, "Flat"),
            ],
            nightly_lows(),
        );
        with_history.timezone = Some(chrono_tz::UTC);
        let json = serde_json::to_value(build(with_history)).unwrap();

        assert_eq!(json["agp"]["days"], 4);
        assert_eq!(json["agp"]["step_min"], crate::agp::BUCKET_MIN);
        let points = json["agp"]["points"].as_array().unwrap();
        // The whole clock, not the charted window: this is the view that says
        // "this happens at 3am", so it cannot stop where the chart does.
        assert_eq!(points.len(), (24 * 60 / crate::agp::BUCKET_MIN) as usize);

        let first = points[0].as_array().unwrap();
        assert_eq!(first.len(), 6, "[minute, p05, p25, p50, p75, p95]");
        assert_eq!(first[0].as_i64().unwrap(), crate::agp::BUCKET_MIN / 2);
        // Minutes of a local day, ordered, ending inside the last bucket.
        let last = points[points.len() - 1].as_array().unwrap();
        assert!(last[0].as_i64().unwrap() < 1440);

        // Display units and ordered percentiles, like everything else here.
        let p50 = first[3].as_f64().unwrap();
        assert!((2.0..=12.0).contains(&p50), "p50 {p50} is not mmol/L");
        assert!(first[1].as_f64().unwrap() <= first[2].as_f64().unwrap());
        assert!(first[2].as_f64().unwrap() <= p50);
        assert!(p50 <= first[4].as_f64().unwrap());
        assert!(first[4].as_f64().unwrap() <= first[5].as_f64().unwrap());
    }

    #[test]
    fn one_days_history_is_not_a_profile() {
        // Same bar as the band: with one day the median and the quartiles are
        // the same number, and drawing that invites reading a Tuesday as a
        // habit.
        let day = 24 * 3_600_000i64;
        let mut one_day: Vec<Entry> = (0..288)
            .map(|i| entry(110.0, NOW - day - i * 5 * MIN, "Flat"))
            .collect();
        one_day.reverse();
        let mut short = input(vec![entry(115.0, NOW - 5 * MIN, "Flat")], one_day);
        short.timezone = Some(chrono_tz::UTC);
        let json = serde_json::to_value(build(short)).unwrap();

        assert!(json.get("agp").is_none());
    }

    #[test]
    fn the_sensor_block_counts_down_from_its_expected_life() {
        let day = 24 * 3_600_000i64;
        let mut input = input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]);
        input.sensor_start_ms = Some(NOW - 6 * day - 4 * 3_600_000);
        input.sensor_days = 10;
        let json = serde_json::to_value(build(input)).unwrap();

        assert_eq!(json["sensor"]["age_h"], 148);
        // Six days four hours in, a ten-day sensor has three days and change.
        assert_eq!(json["sensor"]["expires_in_h"], 92);
        assert_eq!(json["sensor"]["expired"], false);
    }

    #[test]
    fn a_sensor_past_its_life_says_so_rather_than_counting_backwards() {
        let day = 24 * 3_600_000i64;
        let mut input = input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]);
        input.sensor_start_ms = Some(NOW - 12 * day);
        input.sensor_days = 10;
        let json = serde_json::to_value(build(input)).unwrap();

        assert_eq!(json["sensor"]["expired"], true);
        assert_eq!(json["sensor"]["expires_in_h"], 0);
    }

    #[test]
    fn no_expected_life_reports_the_age_and_claims_nothing_else() {
        let day = 24 * 3_600_000i64;
        let mut input = input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]);
        input.sensor_start_ms = Some(NOW - 3 * day);
        input.sensor_days = 0;
        let json = serde_json::to_value(build(input)).unwrap();

        assert_eq!(json["sensor"]["age_h"], 72);
        // Nothing knows when it runs out, so nothing says.
        assert!(json["sensor"].get("expires_in_h").is_none());
        assert!(json["sensor"].get("expired").is_none());
    }

    #[test]
    fn no_sensor_event_means_no_sensor_block() {
        let json = serde_json::to_value(build(input(vec![], vec![]))).unwrap();
        assert!(json.get("sensor").is_none());
    }

    #[test]
    fn the_forecast_projects_the_reading_forward() {
        // Two readings five minutes apart, climbing: AR2 has what it needs.
        let json = serde_json::to_value(build(input(
            vec![
                entry(150.0, NOW - MIN, "SingleUp"),
                entry(140.0, NOW - 6 * MIN, "SingleUp"),
            ],
            vec![],
        )))
        .unwrap();

        assert_eq!(json["forecast"]["in_min"], 30);
        // The horizon, not the next step: from 8.3 rising, the 5-minute
        // projection is 8.8 and the 30-minute one is 9.6. Anything at or below
        // 9.2 means the wrong end of the cone was reported.
        let value = json["forecast"]["value"].as_f64().unwrap();
        assert!((9.2..14.0).contains(&value), "got {value}");
        // Where it lands decides the colour the panel gives it.
        assert!(json["forecast"]["class"].is_string());
    }

    #[test]
    fn a_gap_in_the_readings_forecasts_nothing() {
        // Half an hour between readings: the model would extrapolate a benign
        // drift into a fabricated low. No forecast beats a made-up one.
        let json = serde_json::to_value(build(input(
            vec![
                entry(150.0, NOW - MIN, "Flat"),
                entry(140.0, NOW - 31 * MIN, "Flat"),
            ],
            vec![],
        )))
        .unwrap();

        assert!(json.get("forecast").is_none());
    }

    #[test]
    fn one_reading_forecasts_nothing() {
        let json =
            serde_json::to_value(build(input(vec![entry(150.0, NOW - MIN, "Flat")], vec![])))
                .unwrap();
        assert!(json.get("forecast").is_none());
    }

    #[test]
    fn the_overview_covers_the_history_at_a_coarser_step() {
        // Two days of five-minute readings: 576 points fine, 192 at a quarter
        // of an hour. The panel scrolls across the coarse ones and never pays
        // for a second fetch.
        let day = 24 * 3_600_000i64;
        // Newest first, as Nightscout returns them: the loop counts backwards
        // from now, so it is already in that order.
        let history: Vec<Entry> = (0..576)
            .map(|i| entry(110.0 + (i % 40) as f64, NOW - i * 5 * MIN, "Flat"))
            .collect();
        let json =
            serde_json::to_value(build(input(vec![entry(115.0, NOW - MIN, "Flat")], history)))
                .unwrap();

        assert_eq!(json["overview"]["step_min"], 15);
        let points = json["overview"]["points"].as_array().unwrap();
        assert!(
            (180..=200).contains(&points.len()),
            "two days at 15 minutes is about 192 points, got {}",
            points.len()
        );
        // Oldest first, in display units, like every other series here.
        let first = points[0].as_array().unwrap();
        let second = points[1].as_array().unwrap();
        assert!(first[0].as_i64().unwrap() < second[0].as_i64().unwrap());
        let value = first[1].as_f64().unwrap();
        assert!((5.0..9.0).contains(&value), "{value} is not mmol/L");
        // It reaches back further than the chart's own window.
        let span_h = (points[points.len() - 1].as_array().unwrap()[0]
            .as_i64()
            .unwrap()
            - first[0].as_i64().unwrap())
            / 3_600_000;
        assert!(span_h >= 47, "got {span_h}h");
        let _ = day;
    }

    #[test]
    fn no_history_means_no_overview() {
        let json =
            serde_json::to_value(build(input(vec![entry(115.0, NOW - MIN, "Flat")], vec![])))
                .unwrap();
        assert!(json.get("overview").is_none());
    }
}
