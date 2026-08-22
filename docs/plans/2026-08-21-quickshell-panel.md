# Quickshell Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the Quickshell bar widget a popup panel showing a chart, time-in-range and pattern insights, fed by a new `sugarrush snapshot` command that prints one JSON document.

**Architecture:** A new `src/snapshot.rs` composes the existing analysis modules (`alert`, `stats`, `agp`, `units`) into a serializable document, split into a pure `build()` that takes entries and a thin `fetch()` that does the network — so everything interesting is unit-testable without a Nightscout. On the QML side, `quickshell/Panel.qml` (new) holds every Omarchy-internal import and is loaded by path from `BarWidget.qml`, which stays a plain `Item`; if the panel fails to load, the pill keeps working.

**Tech Stack:** Rust (serde_json, chrono, chrono_tz, anyhow, tokio), QML (Quickshell 0.3, omarchy-shell 4 `qs.Ui`/`qs.Commons`).

**Spec:** [`docs/specs/2026-08-21-quickshell-panel-design.md`](../specs/2026-08-21-quickshell-panel-design.md)

## Global Constraints

- Rust is pinned via mise. Every cargo command is prefixed: `mise exec -- cargo …`.
- All four gates must pass before any commit is considered done: `mise exec -- cargo fmt --all`, `mise exec -- cargo clippy --all-targets -- -D warnings`, `mise exec -- cargo build`, `mise exec -- cargo test`.
- `scripts/check-process.sh` must pass.
- Commits: conventional-commit style, lowercase imperative (`feat:`, `fix:`, `docs:`). Never commit to `main`. Work happens on branch `feat/quickshell-panel`.
- Tests live inline in `#[cfg(test)] mod tests` at the bottom of the module they cover. This repo has no `tests/` directory; do not create one.
- **Every test must be seen failing on an assertion before its implementation is written.** A compile error (`unresolved import`, `no function named …`) is not proof — write the test against a stub that compiles and returns a wrong-but-typed value if you must.
- Alert thresholds are mg/dL internally; display units only at the serialization edge. Never store display units in a struct that other modules read.
- Any `src/` change needs a `CHANGELOG.md` bullet under `## [Unreleased]` — CI enforces it.
- The JSON document's `schema` value is `1` and appears in every document, including error documents.
- QML changes are only picked up after restarting `omarchy-shell` (it compiles plugin QML once per process). `omarchy plugin disable`/`enable` does **not** reload it, and also erases the widget's per-widget options.
- The per-widget option name may never be `exec`, `source` or `type` — the bar reads those to decide a slot is one of its own built-in modules and the plugin then never loads.

---

### Task 1: The snapshot document types and pure builder

**Files:**
- Create: `src/snapshot.rs`
- Modify: `src/main.rs:1-30` (add `mod snapshot;` among the existing `mod` lines, alphabetical: after `mod service;`, before `mod sound;`)

**Interfaces:**
- Consumes: `crate::nightscout::Entry { sgv: f64, date: i64, direction: Option<String> }` and `Entry::arrow() -> &'static str`; `crate::config::Alerts { urgent_low, low, high, urgent_high, stale_minutes, .. }` (mg/dL); `crate::units::Units` with `format(mgdl) -> String`, `format_delta(mgdl) -> String`, `label() -> &'static str`, `from_mgdl(mgdl) -> f64`; `crate::alert::{evaluate, Alert}` where `Alert::class() -> &'static str`; `crate::stats::{tir, mean_mgdl, gmi, cv_pct, Tir}`; `crate::agp::{profile_in, insights, Insight, Pattern}` where `Insight { kind, from_min, to_min, extreme }`, `Insight::window() -> String`, `Insight::text(units) -> String`; `crate::theme::Theme` via `Alert::color(&theme) -> ratatui::style::Color`.
- Produces: `snapshot::Snapshot` (serializable), `snapshot::build(BuildInput) -> Snapshot`, `snapshot::error_doc(now_ms: i64, message: &str) -> Snapshot`, `snapshot::history_window(days: u32, now_ms: i64) -> Option<(i64, i64, usize)>`, and `snapshot::hex(color) -> String` re-used from a shared helper (see Step 3).

- [ ] **Step 1: Write the failing test for the document shape**

Add to a new `src/snapshot.rs`, at the bottom:

```rust
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
}
```

- [ ] **Step 2: Run the test and watch it fail on an assertion, not on compilation**

First make it compile with a deliberately wrong stub — put this above the test module in `src/snapshot.rs`:

```rust
//! One JSON document describing the current state: reading, recent series,
//! time-in-range and pattern insights.
//!
//! `status.rs` renders a *line*; every format it knows is a string. A panel
//! wants a document, so it lives here rather than as a sixth `Format`.
//!
//! Split in two on purpose: [`build`] is pure and takes entries, so every
//! shape and conversion is unit-testable; [`fetch`] only talks to Nightscout.

use serde::Serialize;

use crate::config::Alerts;
use crate::nightscout::Entry;
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
    pub schema: u8,
}

pub fn build(_input: BuildInput) -> Snapshot {
    Snapshot { schema: 0 }
}
```

Add `mod snapshot;` to `src/main.rs` in the alphabetical `mod` list (after `mod service;`).

Run: `mise exec -- cargo test --quiet snapshot::`
Expected: FAIL with `assertion left == right failed, left: 0, right: 1` on the `schema` assertion — the module compiles and runs, the value is wrong.

- [ ] **Step 3: Move `hex` out of `status.rs` so both callers share one definition**

`status.rs` has a private `fn hex(Color) -> String` with the full named-colour table. Two copies would drift. Move it to `src/theme.rs` as `pub fn hex(color: ratatui::style::Color) -> String` (body unchanged, doc comment unchanged), and in `src/status.rs` delete the private copy plus its `fn color_for`, replacing uses with `crate::theme::hex(...)` and `self.state.color(&theme)`.

Run: `mise exec -- cargo test --quiet status::`
Expected: PASS — `status`'s own tests (`each_bar_gets_its_own_syntax`, `waybar_output_is_json_with_the_state_class`, `named_colours_become_hex_for_bars`) still pass. Move `named_colours_become_hex_for_bars` to `theme.rs`'s test module alongside the function.

- [ ] **Step 4: Implement the real document types and `build`**

Replace the stub in `src/snapshot.rs`:

```rust
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
    let series = recent
        .iter()
        .rev()
        .map(|e| (e.date, scaled(units, e.sgv)))
        .collect();

    let stats_from = now_ms - stats_window_h * 3_600_000;
    let stats_entries: Vec<Entry> = if history.is_empty() {
        recent.iter().filter(|e| e.date >= stats_from).cloned().collect()
    } else {
        history.iter().filter(|e| e.date >= stats_from).cloned().collect()
    };
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

fn stats_for(
    entries: &[Entry],
    units: Units,
    alerts: &Alerts,
    window_h: i64,
) -> Option<Stats> {
    let mean = crate::stats::mean_mgdl(entries)?;
    let tir = crate::stats::tir(
        entries,
        alerts.urgent_low,
        alerts.low,
        alerts.high,
        alerts.urgent_high,
    )?;
    if entries.len() < 2 {
        return None;
    }
    Some(Stats {
        window_h,
        mean: scaled(units, mean),
        gmi: (crate::stats::gmi(mean) * 10.0).round() / 10.0,
        cv: crate::stats::cv_pct(entries).map(|cv| (cv * 10.0).round() / 10.0),
        tir: TirDoc {
            very_low: round1(tir.very_low),
            low: round1(tir.low),
            in_range: round1(tir.in_range),
            high: round1(tir.high),
            very_high: round1(tir.very_high),
        },
    })
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
```

`Entry` already derives `Clone` (`src/nightscout.rs:25`), so the `stats_entries` filter compiles as written.

- [ ] **Step 5: Run the test**

Run: `mise exec -- cargo test --quiet snapshot::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/snapshot.rs src/main.rs src/status.rs src/theme.rs src/nightscout.rs
git commit -m "feat: build a snapshot document from readings"
```

---

### Task 2: Display units, empty data, and the error document

**Files:**
- Modify: `src/snapshot.rs` (tests and `error_doc`)

**Interfaces:**
- Consumes: `snapshot::{build, BuildInput, Snapshot}` from Task 1.
- Produces: `snapshot::error_doc(now_ms: i64, message: &str) -> Snapshot`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/snapshot.rs`:

```rust
    #[test]
    fn every_value_is_in_the_display_unit() {
        let mmol = build(input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]));
        let mmol_json = serde_json::to_value(&mmol).unwrap();
        assert_eq!(mmol_json["series"][0][1], 6.4);
        assert_eq!(mmol_json["range"]["low"], 3.9);
        assert_eq!(mmol_json["range"]["high"], 10.0);

        let mut mgdl_input = input(vec![entry(115.0, NOW - MIN, "Flat")], vec![]);
        mgdl_input.units = Units::Mgdl;
        let mgdl_json = serde_json::to_value(&build(mgdl_input)).unwrap();
        assert_eq!(mgdl_json["units"], "mg/dL");
        assert_eq!(mgdl_json["series"][0][1], 115.0);
        assert_eq!(mgdl_json["range"]["low"], 70.0);
        assert_eq!(mgdl_json["range"]["high"], 180.0);
    }

    #[test]
    fn no_readings_gives_empty_collections_rather_than_a_failure() {
        let json = serde_json::to_value(&build(input(vec![], vec![]))).unwrap();

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
        let json = serde_json::to_value(&error_doc(NOW, "no site configured")).unwrap();

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
```

- [ ] **Step 2: Run the tests and watch them fail on assertions**

Add a stub that compiles but is wrong:

```rust
/// A document that says only what went wrong. Exit code stays 0 and the shape
/// stays parseable, so a bar renders the message instead of a parse failure —
/// the same promise `status` makes.
pub fn error_doc(now_ms: i64, message: &str) -> Snapshot {
    Snapshot {
        schema: 1,
        units: "mg/dL",
        generated_at: now_ms,
        error: None,
        now: None,
        range: None,
        series: None,
        stats: None,
        insights: None,
    }
}
```

Run: `mise exec -- cargo test --quiet snapshot::`
Expected: `an_error_is_still_a_document` FAILS with `left: Null, right: "no site configured"`. The other two may already pass — that is fine, they pin Task 1's behaviour against regression.

- [ ] **Step 3: Implement**

In `error_doc`, set `error: Some(message.to_string())`. Add the `units` argument so the document still tells a consumer which unit it would have used:

```rust
pub fn error_doc(now_ms: i64, message: &str) -> Snapshot {
    Snapshot {
        schema: 1,
        units: crate::units::Units::default().label(),
        generated_at: now_ms,
        error: Some(message.to_string()),
        now: None,
        range: None,
        series: None,
        stats: None,
        insights: None,
    }
}
```

If `Units` has no `Default`, use `Units::Mmol` and add a comment that an error document's unit is nominal.

- [ ] **Step 4: Write the failing test for insight mapping**

An insight needs a *pattern*: readings below target at the same time of day on
at least three separate days, over a run of at least 45 minutes. Build exactly
that, so the test pins the mapping rather than hoping the fixture qualifies:

```rust
    /// Four days of readings: 60 mg/dL between 02:00 and 03:00 local, 110 the
    /// rest of the time. That clears `agp`'s two bars for a pattern — a run of
    /// at least 45 minutes, on at least three separate days.
    fn nightly_lows() -> Vec<Entry> {
        let day = 24 * 3_600_000i64;
        let mut out = Vec::new();
        for d in 1..=4i64 {
            let midnight = NOW - d * day - (NOW - d * day) % day;
            for minute in (0..24 * 60).step_by(5) {
                let at = midnight + minute as i64 * 60_000;
                let sgv = if (120..=180).contains(&minute) { 60.0 } else { 110.0 };
                out.push(entry(sgv, at, "Flat"));
            }
        }
        out.reverse(); // newest first, as Nightscout returns them
        out
    }

    #[test]
    fn a_pattern_becomes_an_insight_row() {
        let json = serde_json::to_value(&build(input(vec![], nightly_lows()))).unwrap();
        let first = &json["insights"][0];

        assert_eq!(first["kind"], "lows");
        assert_eq!(first["window"], "02:00–03:15");
        // Display units here too — the panel prints this without converting.
        assert_eq!(first["extreme"], 3.3);
        assert!(first["text"].as_str().unwrap().starts_with("lows 02:00–03:15"));
    }
```

- [ ] **Step 5: Run it and watch it fail**

Run: `mise exec -- cargo test --quiet a_pattern_becomes_an_insight_row`
Expected: FAIL. If it fails because `insights` is empty rather than on the
`kind` assertion, the fixture does not qualify — widen the low window or add a
day until it does, then re-run. Do not weaken the assertion to match an empty
list; an empty list is the bug this test exists to catch.

The `window` and `extreme` values above are what `agp` should produce for this
fixture (buckets are 15 minutes, so a 02:00–03:00 run reports through 03:15,
and 60 mg/dL is 3.3 mmol/L). If the real values differ, check `agp::window()`
at `src/agp.rs:221` and fix the expectation — once — to the value it computes,
and say so in the commit message.

- [ ] **Step 6: Run the tests**

Run: `mise exec -- cargo test --quiet snapshot::`
Expected: PASS, all five tests.

- [ ] **Step 7: Commit**

```bash
git add src/snapshot.rs
git commit -m "feat: report snapshot failures as a document"
```

---

### Task 3: The fetch layer and the history window

**Files:**
- Modify: `src/snapshot.rs`

**Interfaces:**
- Consumes: `crate::config::{Config, Site}` with `Config::load()`, `cfg.resolve_sites() -> Result<Vec<Site>>`, `site.resolve_alerts(&cfg.alerts, cfg.units) -> (Alerts, Vec<String>)`, `cfg.theme.resolve() -> Theme`, `cfg.units`; `crate::nightscout::Client::for_site(site) -> Result<Client>` and `client.entries_range(start_ms, end_ms, want) -> Result<Vec<Entry>>`.
- Produces: `snapshot::history_window(days: u32, now_ms: i64) -> Option<(i64, i64, usize)>` and `async snapshot::fetch(cfg: &Config, site: &Site, hours: u32, days: u32) -> Snapshot`.

- [ ] **Step 1: Write the failing test for the window helper**

```rust
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
```

- [ ] **Step 2: Run it and watch it fail on an assertion**

Stub:

```rust
/// The range and row count for the multi-day history fetch, or `None` when the
/// caller asked for none. Separate from `fetch` so the "no history means no
/// query" rule is testable without a Nightscout.
pub fn history_window(days: u32, now_ms: i64) -> Option<(i64, i64, usize)> {
    Some((now_ms, now_ms, 0))
}
```

Run: `mise exec -- cargo test --quiet snapshot::asking_for_no_history`
Expected: FAIL with `assertion failed: history_window(0, NOW).is_none()`.

- [ ] **Step 3: Implement the helper and the fetch**

```rust
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
```

Add the imports this needs at the top of the module: `use crate::config::{Config, Site}; use crate::nightscout::Client;`.

- [ ] **Step 4: Run the tests**

Run: `mise exec -- cargo test --quiet snapshot::`
Expected: PASS, five tests.

- [ ] **Step 5: Commit**

```bash
git add src/snapshot.rs
git commit -m "feat: fetch the readings a snapshot needs"
```

---

### Task 4: The `sugarrush snapshot` subcommand

**Files:**
- Modify: `src/main.rs` — the `Mode` enum (~line 55-75), `parse_args` (~line 162-500), the `--site` `reject_flag` `matches!` (~line 420-433), the dispatch `match` (~line 540-560), the help tables (~line 840-900)

**Interfaces:**
- Consumes: `snapshot::{fetch, error_doc, Snapshot}` from Task 3.
- Produces: the CLI surface `sugarrush snapshot [--hours N] [--days N] [--site NAME]`.

- [ ] **Step 1: Write the failing test for argument parsing**

`parse_args` reads `std::env::args`, so test the mode's shape through the existing help-table test instead — this repo already tests `--help` content that way. Add to `mod tests` in `src/main.rs`:

```rust
    #[test]
    fn snapshot_is_documented_in_help() {
        let commands = command_rows();
        let row = commands
            .iter()
            .find(|(usage, _)| usage.starts_with("sugarrush snapshot"))
            .expect("snapshot has a help row");
        assert_eq!(row.0, "sugarrush snapshot [--hours N] [--days N]");
        assert!(row.1.contains("JSON"));
    }
```

Look at how the existing help rows are stored around `src/main.rs:844-860` (`"sugarrush status [--format FORMAT]"`, `"one line for a status bar"`). If they are a `const` array rather than a `command_rows()` function, assert against that array by its real name instead — do not invent a helper.

- [ ] **Step 2: Run it and watch it fail**

Run: `mise exec -- cargo test --quiet snapshot_is_documented_in_help`
Expected: FAIL on `expect("snapshot has a help row")` — the row is absent.

- [ ] **Step 3: Add the mode, the parsing, and the help rows**

In the `Mode` enum:

```rust
    /// Print one JSON document describing the current state, and exit.
    Snapshot { hours: u32, days: u32 },
```

In `parse_args`, alongside `"status"` and `"export"`:

```rust
            "snapshot" => {
                mode = Some(Mode::Snapshot {
                    hours: 6,
                    days: 14,
                })
            }
            "--hours" => {
                i += 1;
                snapshot_hours = args.get(i).and_then(|v| v.parse::<u32>().ok());
                if snapshot_hours.is_none() {
                    eprintln!("sugarrush: --hours needs a whole number of hours");
                    std::process::exit(2);
                }
            }
            "--days" if matches!(mode, Some(Mode::Snapshot { .. })) => {
                i += 1;
                snapshot_days = args.get(i).and_then(|v| v.parse::<u32>().ok());
                if snapshot_days.is_none() {
                    eprintln!("sugarrush: --days needs a whole number of days");
                    std::process::exit(2);
                }
            }
```

`--days` already exists for `export` and `alerts`; guard the new arm with the `matches!` shown so the existing behaviour is untouched, and apply `snapshot_hours` / `snapshot_days` where the function finalizes its mode (the same place `status_format` is applied, ~line 469-491):

```rust
        Some(Mode::Snapshot { .. }) => Mode::Snapshot {
            hours: snapshot_hours.unwrap_or(6).clamp(1, 72),
            days: snapshot_days.unwrap_or(14).clamp(0, 90),
        },
```

Add `Mode::Snapshot { .. }` to the `matches!` list in the `--site` `reject_flag` call, next to `Mode::Export { .. }`, so `snapshot --site NAME` is accepted rather than rejected.

Help rows, next to the `status` pair:

```rust
        "sugarrush snapshot [--hours N] [--days N]",
        "one JSON document: reading, series, stats, insights",
```

And in the flag table, next to `("--format FORMAT", "status-bar syntax")`:

```rust
    ("--hours N", "snapshot chart window (default 6)"),
    ("--days N", "snapshot history for patterns (default 14, 0 = none)"),
```

- [ ] **Step 4: Run the test**

Run: `mise exec -- cargo test --quiet snapshot_is_documented_in_help`
Expected: PASS.

- [ ] **Step 5: Wire the dispatch**

In the mode `match` in `run`, next to `Mode::Status`:

```rust
        Mode::Snapshot { hours, days } => {
            let cfg = Config::load()?;
            let sites = cfg.resolve_sites()?;
            let site = match snapshot_site(&sites, snapshot_site_name.as_deref()) {
                Ok(site) => site,
                Err(message) => {
                    // A panel can render a message; it cannot render a
                    // non-zero exit with nothing on stdout.
                    println!(
                        "{}",
                        serde_json::to_string(&snapshot::error_doc(
                            chrono::Utc::now().timestamp_millis(),
                            &message,
                        ))?
                    );
                    return Ok(());
                }
            };
            println!(
                "{}",
                serde_json::to_string(&snapshot::fetch(&cfg, site, hours, days).await)?
            );
            Ok(())
        }
```

with the site chooser next to `run_export`:

```rust
/// The site a snapshot describes: the only one, or the named one. Returns the
/// message to print rather than an error, because the caller turns it into a
/// document rather than a failure.
fn snapshot_site<'a>(
    sites: &'a [config::Site],
    selected: Option<&str>,
) -> Result<&'a config::Site, String> {
    match selected {
        Some(name) => sites
            .iter()
            .find(|site| site.name == name)
            .ok_or_else(|| format!("no site named '{name}'")),
        None => sites.first().ok_or_else(|| "no site configured".to_string()),
    }
}
```

Reuse the existing `--site` value binding (the variable `snooze_site` currently holds it; rename it to `site_name` in the same commit if it is shared, or read whichever variable `parse_args` already stores `--site` into — check `src/main.rs:285-290` before writing this).

- [ ] **Step 6: Verify by hand against the real site**

Run: `mise exec -- cargo run --quiet -- snapshot --hours 3 --days 0 | jq '{schema, units, now, series: (.series|length), stats, insights}'`
Expected: `schema: 1`, a `now` block, a non-empty `series`, `stats` present, `insights: []`.

Run: `mise exec -- cargo run --quiet -- snapshot --site nope | jq .`
Expected: `{"schema":1,"units":…,"generated_at":…,"error":"no site named 'nope'"}` and exit code 0 (`echo $?`).

- [ ] **Step 7: Run all four gates**

```bash
mise exec -- cargo fmt --all
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo build
mise exec -- cargo test
./scripts/check-process.sh
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat: add the snapshot subcommand"
```

---

### Task 5: `--demo` support for snapshot

**Files:**
- Modify: `src/main.rs` (dispatch arm), `src/snapshot.rs` (a demo assembler)

**Interfaces:**
- Consumes: `crate::demo::entries(start_ms: i64, end_ms: i64) -> Vec<Entry>`.
- Produces: `snapshot::demo(units: Units, alerts: Alerts, theme: Theme, hours: u32, days: u32, now_ms: i64) -> Snapshot`. It takes the three settings rather than a `&Config` because `Config` has no `Default` and a test would have no way to build one; `Alerts` and `Theme` both do (`src/config.rs:537`, used at `src/alert.rs:278`).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn the_demo_document_has_the_same_shape_as_a_real_one() {
        let json =
            serde_json::to_value(&demo(Units::Mmol, alerts(), Theme::default(), 6, 14, NOW))
                .unwrap();

        assert_eq!(json["schema"], 1);
        assert!(json.get("error").is_none());
        assert!(json["now"]["value"].is_string());
        // Six hours of five-minute readings, so a chart has something to draw.
        assert!(json["series"].as_array().unwrap().len() > 50);
        assert!(json["stats"]["tir"]["in_range"].is_number());
    }
```

- [ ] **Step 2: Run it and watch it fail on an assertion**

Stub:

```rust
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
    let _ = (units, alerts, theme, hours, days);
    error_doc(now_ms, "demo")
}
```

Run: `mise exec -- cargo test --quiet the_demo_document`
Expected: FAIL — `json.get("error").is_none()` fails.

- [ ] **Step 3: Implement**

```rust
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
    // demo::entries returns oldest first; the rest of this module expects
    // newest first, as Nightscout returns them.
    let mut recent = crate::demo::entries(recent_from, now_ms);
    recent.reverse();
    let mut history = match history_window(days, now_ms) {
        Some((start, end, _)) => crate::demo::entries(start, end),
        None => Vec::new(),
    };
    history.reverse();

    build(BuildInput {
        now_ms,
        units,
        alerts,
        theme,
        timezone: None,
        recent,
        history,
        stats_window_h: 24,
    })
}
```

Check `demo::entries`' ordering at `src/demo.rs:11` before trusting the `reverse()` calls; drop them if it already returns newest first.

In `src/main.rs`, the dispatch arm takes the existing `demo` flag into account:

```rust
        Mode::Snapshot { hours, days } if demo => {
            // A demo run must work with no config file at all, so the three
            // settings fall back to their own defaults rather than to a
            // Config that cannot be constructed without one.
            let (units, alerts, theme) = match Config::load() {
                Ok(cfg) => (
                    cfg.units,
                    cfg.alerts.resolve_checked(cfg.units).0,
                    cfg.theme.resolve(),
                ),
                Err(_) => (
                    crate::units::Units::Mmol,
                    config::Alerts::default(),
                    theme::Theme::default(),
                ),
            };
            println!(
                "{}",
                serde_json::to_string(&snapshot::demo(
                    units,
                    alerts,
                    theme,
                    hours,
                    days,
                    chrono::Utc::now().timestamp_millis(),
                ))?
            );
            Ok(())
        }
```

placed **above** the non-demo `Mode::Snapshot` arm. Check how `demo` is threaded into `run` — it is a `parse_args` local at `src/main.rs:165`; follow whatever the dashboard path already does rather than adding a second channel.

- [ ] **Step 4: Run the test and the command**

Run: `mise exec -- cargo test --quiet the_demo_document`
Expected: PASS.

Run: `mise exec -- cargo run --quiet -- snapshot --demo | jq '{schema, series: (.series|length), insights: (.insights|length)}'`
Expected: a full document with a non-empty series.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/snapshot.rs
git commit -m "feat: render a demo snapshot without a site"
```

---

### Task 6: The panel QML

**Files:**
- Create: `quickshell/Panel.qml`
- Modify: `quickshell/BarWidget.qml`

**Interfaces:**
- Consumes: from the shell — `qs.Ui` (`Panel`, `KeyboardPanel`, `PanelKeyCatcher`, `PanelHero`, `PanelSectionHeader`, `Button`), `qs.Commons` (`Style`, `Color`, `Border`); from the bar — `bar.requestPopout`, `bar.barForeground`, `bar.run`; from Task 4 — `sugarrush snapshot --hours N --days M` on stdout.
- Produces: `Panel.qml` exposing `bar`, `settings`, `anchorItem`, `hostWidget`, `opened`, `open()`, `close()`, `toggle()`, `closeForPopoutSwitch()`, `popoutSwitchClosing`, `refresh()`.

- [ ] **Step 1: Write `quickshell/Panel.qml`**

```qml
// The sugarrush popup panel for the Omarchy bar.
//
// Every shell-internal import in this plugin lives in this file. BarWidget.qml
// reaches it through a Loader by path and checks the Loader's status, so a
// shell release that moves its internals costs the panel and nothing else —
// the pill keeps showing the reading.

import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "sugarrush.glucose"
  ipcTarget: "sugarrush.glucose"

  property var anchorItem: null
  // The bar tracks the widget in its slot, not this nested panel, so the
  // popout coordinator and the open-panel dot have to be handed that widget.
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property int panelHours: setting("panelHours", 6)
  readonly property int insightDays: setting("insightDays", 14)
  readonly property int cacheMinutes: setting("panelCacheMinutes", 5)
  readonly property string snapshotCommand: setting("snapshotCommand", "sugarrush snapshot")

  property var doc: null
  property string loadError: ""
  property double fetchedAt: 0

  function stale() {
    return !doc || (Date.now() - fetchedAt) > cacheMinutes * 60000
  }

  function refresh(force) {
    if (!force && !stale()) return
    if (snapProc.running) return
    snapProc.running = true
  }

  onOpenedChanged: if (opened) refresh(false)

  function apply(out) {
    try {
      var parsed = JSON.parse(String(out || ""))
      if (parsed.schema !== 1) {
        root.loadError = "this sugarrush speaks snapshot schema " + parsed.schema + ", the panel speaks 1"
        return
      }
      root.doc = parsed
      root.loadError = parsed.error ? String(parsed.error) : ""
      root.fetchedAt = Date.now()
    } catch (e) {
      root.loadError = "could not read snapshot output — update sugarrush"
    }
  }

  Process {
    id: snapProc
    command: ["bash", "-lc", root.snapshotCommand + " --hours " + root.panelHours + " --days " + root.insightDays]
    stdout: StdioCollector {
      id: snapOut
      waitForEnd: true
      onStreamFinished: root.apply(snapOut.text)
    }
    stderr: StdioCollector {
      id: snapErr
      waitForEnd: true
      onStreamFinished: if (snapErr.text.trim() !== "") console.warn("sugarrush panel", snapErr.text.trim())
    }
  }

  KeyboardPanel {
    id: surface
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    focusTarget: keys
    contentWidth: surface.fittedContentWidth(Style.space(420))
    contentHeight: surface.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keys
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function (direction) { root.switchPanel(direction) }

      Column {
        id: column
        width: parent.width
        spacing: Style.space(12)

        PanelHero {
          title: root.doc && root.doc.now ? root.doc.now.value + " " + root.doc.units : "—"
          meta: root.doc && root.doc.now ? root.doc.now.arrow + "  " + root.doc.now.direction : ""
          detail: root.doc && root.doc.now
            ? "Δ " + root.doc.now.delta + " · " + root.doc.now.age_min + "m ago"
            : root.loadError
          foreground: root.doc && root.doc.now ? root.doc.now.color : root.barForeground
        }

        Chart {
          width: parent.width
          height: Style.space(120)
          doc: root.doc
        }

        PanelSectionHeader { text: "Time in range" }

        TirBar {
          width: parent.width
          height: Style.space(14)
          stats: root.doc ? root.doc.stats : null
        }

        Text {
          visible: root.doc && root.doc.stats
          width: parent.width
          color: root.barForeground
          font.family: Style.font.family
          text: root.doc && root.doc.stats
            ? "mean " + root.doc.stats.mean + " · GMI " + root.doc.stats.gmi + " · CV " + (root.doc.stats.cv === undefined ? "—" : root.doc.stats.cv + "%")
            : ""
        }

        PanelSectionHeader { text: "Patterns"; visible: root.insightDays > 0 }

        Column {
          width: parent.width
          spacing: Style.space(4)
          visible: root.insightDays > 0

          Repeater {
            model: root.doc && root.doc.insights ? root.doc.insights : []
            Text {
              required property var modelData
              width: parent.width
              wrapMode: Text.WordWrap
              color: root.barForeground
              font.family: Style.font.family
              text: modelData.text
            }
          }

          Text {
            visible: !root.doc || !root.doc.insights || root.doc.insights.length === 0
            color: Qt.darker(root.barForeground, 1.4)
            font.family: Style.font.family
            text: "not enough history yet for patterns"
          }
        }

        Row {
          spacing: Style.space(8)

          // Ui.Button, not PanelActionButton: the latter is an icon button
          // (`iconText`) with no text label.
          Button {
            text: "Refresh"
            onClicked: root.refresh(true)
          }

          Button {
            text: "Open dashboard"
            onClicked: {
              root.close()
              if (root.bar) root.bar.run(root.setting("onClick", "omarchy-launch-floating-terminal-with-presentation sugarrush"))
            }
          }
        }
      }
    }
  }
}
```

`Chart` and `TirBar` are the next two steps; write them as sibling files so this one stays readable.

- [ ] **Step 2: Write `quickshell/Chart.qml`**

```qml
// The last N hours as a line over a shaded target band. Canvas rather than
// Shapes: one paint over an array, and new data is a single requestPaint().

import QtQuick
import qs.Commons

Item {
  id: root
  property var doc: null

  readonly property var series: doc && doc.series ? doc.series : []
  readonly property var range: doc && doc.range ? doc.range : null

  onDocChanged: canvas.requestPaint()

  Text {
    anchors.centerIn: parent
    visible: root.series.length === 0
    color: Qt.darker(Color.foreground, 1.4)
    font.family: Style.font.family
    text: "no readings in this window"
  }

  Canvas {
    id: canvas
    anchors.fill: parent
    visible: root.series.length > 0

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      if (root.series.length === 0 || !root.range) return

      var lo = root.range.urgent_low
      var hi = root.range.urgent_high
      for (var i = 0; i < root.series.length; i++) {
        lo = Math.min(lo, root.series[i][1])
        hi = Math.max(hi, root.series[i][1])
      }
      var pad = (hi - lo) * 0.1 || 1
      lo -= pad
      hi += pad

      var t0 = root.series[0][0]
      var t1 = root.series[root.series.length - 1][0]
      var span = Math.max(1, t1 - t0)
      function x(t) { return (t - t0) / span * width }
      function y(v) { return height - (v - lo) / (hi - lo) * height }

      // Target band first, so the line draws over it.
      ctx.fillStyle = Qt.rgba(1, 1, 1, 0.07)
      ctx.fillRect(0, y(root.range.high), width, y(root.range.low) - y(root.range.high))

      ctx.strokeStyle = Qt.rgba(1, 1, 1, 0.18)
      ctx.lineWidth = 1
      var bounds = [root.range.urgent_low, root.range.urgent_high]
      for (var b = 0; b < bounds.length; b++) {
        ctx.beginPath()
        ctx.moveTo(0, y(bounds[b]))
        ctx.lineTo(width, y(bounds[b]))
        ctx.stroke()
      }

      ctx.strokeStyle = root.doc && root.doc.now ? root.doc.now.color : Color.foreground
      ctx.lineWidth = 2
      ctx.beginPath()
      ctx.moveTo(x(root.series[0][0]), y(root.series[0][1]))
      for (var j = 1; j < root.series.length; j++) ctx.lineTo(x(root.series[j][0]), y(root.series[j][1]))
      ctx.stroke()
    }
  }
}
```

- [ ] **Step 3: Write `quickshell/TirBar.qml`**

```qml
// Five bands, widths proportional to their share of the window. Colours are
// the conventional CGM ones rather than the theme's, because the bands mean
// the same thing on every theme.

import QtQuick
import qs.Commons

Item {
  id: root
  property var stats: null

  readonly property var bands: stats && stats.tir
    ? [
      { share: stats.tir.very_low, color: "#cc241d" },
      { share: stats.tir.low, color: "#d79921" },
      { share: stats.tir.in_range, color: "#98971a" },
      { share: stats.tir.high, color: "#d79921" },
      { share: stats.tir.very_high, color: "#cc241d" }
    ]
    : []

  visible: bands.length > 0

  Row {
    anchors.fill: parent
    spacing: 0

    Repeater {
      model: root.bands

      Rectangle {
        required property var modelData
        width: root.width * Math.max(0, modelData.share) / 100
        height: root.height
        color: modelData.color
      }
    }
  }
}
```

- [ ] **Step 4: Give `BarWidget.qml` the host contract**

Add to `quickshell/BarWidget.qml`, inside the root `Item`:

```qml
  // The panel is loaded by path, and its failure is survivable: every
  // Omarchy-internal import lives in Panel.qml, so a shell that moves them
  // costs the popup while the pill goes on working.
  readonly property bool panelReady: panelLoader.status === Loader.Ready && panelLoader.item
  readonly property bool opened: panelReady ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelReady ? panelLoader.item.popoutSwitchClosing === true : false

  function injectPanel() {
    if (!panelReady) return
    var target = panelLoader.item
    if ("bar" in target) target.bar = root.bar
    if ("settings" in target) target.settings = root.settings
    if ("anchorItem" in target) target.anchorItem = root
    if ("hostWidget" in target) target.hostWidget = root
  }

  function open() { if (panelReady) panelLoader.item.open() }
  function close() { if (panelReady) panelLoader.item.close() }
  function closeForPopoutSwitch() { if (panelReady) panelLoader.item.closeForPopoutSwitch() }

  onBarChanged: injectPanel()
  onSettingsChanged: injectPanel()

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      root.injectPanel()
      Qt.callLater(root.injectPanel)
    }
    onStatusChanged: if (status === Loader.Error) console.warn("sugarrush: panel failed to load; the pill still works")
  }
```

and change the click handling so left click toggles the panel, falling back to the TUI when the panel is unavailable:

```qml
    onClicked: function (mouse) {
      if (!root.bar) return
      if (mouse.button === Qt.MiddleButton) {
        root.refresh()
      } else if (mouse.button === Qt.RightButton) {
        if (root.onRightClick !== "") root.bar.run(root.onRightClick)
      } else if (root.panelReady) {
        panelLoader.item.toggle()
      } else if (root.onClick !== "") {
        root.bar.run(root.onClick)
      }
    }
```

- [ ] **Step 5: Install and restart the shell**

```bash
cp quickshell/*.qml quickshell/manifest.json ~/.config/omarchy/plugins/sugarrush/
pkill -f 'quickshell -n -p /usr/share/omarchy/shell'
```

Wait for the shell to come back (`pgrep -f 'quickshell -n -p'`), then:

```bash
journalctl --user --since "-30 s" --no-pager | grep -i sugarrush
```

Expected: no `TypeError`, no `module … is not installed`. A warning about `snapshotCommand` not being found is expected until the release carrying Task 4 is installed — point the option at the build under test:

```bash
omarchy bar set sugarrush.glucose snapshotCommand "$PWD/target/release/sugarrush snapshot"
```

- [ ] **Step 6: Verify the panel visually**

Click the pill. Capture it:

```bash
grim -g "1300,0 700x500" /tmp/panel.png
```

Check, by looking at the image: hero shows the reading and age; the chart draws a line over a shaded band; the TIR bar is five proportional segments; the patterns section lists insights or says there are none; both buttons are present.

- [ ] **Step 7: Commit**

```bash
git add quickshell/
git commit -m "feat: a popup panel for the quickshell widget"
```

---

### Task 7: The panel's empty, error and stale states

**Files:**
- Modify: `quickshell/Panel.qml`, `quickshell/Chart.qml`

- [ ] **Step 1: Force and check each state**

Run each command, click the pill, capture, and confirm the panel says the right thing rather than drawing nothing:

```bash
# 1. An error document.
omarchy bar set sugarrush.glucose snapshotCommand "$PWD/target/release/sugarrush snapshot --site nope"
# expect: hero detail shows "no site named 'nope'"; no chart; buttons still work

# 2. No readings in the window.
omarchy bar set sugarrush.glucose snapshotCommand "printf '%s' '{\"schema\":1,\"units\":\"mmol/L\",\"generated_at\":0,\"series\":[],\"insights\":[]}'"
# expect: "no readings in this window"; no stats row; "not enough history yet"

# 3. A binary with no snapshot command at all.
omarchy bar set sugarrush.glucose snapshotCommand "false"
# expect: "could not read snapshot output — update sugarrush"

# 4. A future schema.
omarchy bar set sugarrush.glucose snapshotCommand "printf '%s' '{\"schema\":9}'"
# expect: "this sugarrush speaks snapshot schema 9, the panel speaks 1"

# Restore.
omarchy bar set sugarrush.glucose snapshotCommand "sugarrush snapshot"
```

Each of these needs a shell restart only if `Panel.qml` changed; option changes are picked up live.

- [ ] **Step 2: Fix whatever the states reveal, then re-check**

Typical fixes: a `visible:` binding that leaves an empty `PanelSectionHeader` behind; a hero that shows `undefined` because it read `doc.now` on an error document; a chart that paints a flat line from an empty array.

- [ ] **Step 3: Check the popout handshake and a vertical bar**

```bash
# open the panel, then click another bar widget's popup
# expect: the sugarrush panel closes, the open-panel dot moves to the other widget

omarchy bar position left
# expect: the pill goes compact (value + arrow), the panel opens to the right
omarchy bar position top
```

- [ ] **Step 4: Commit**

```bash
git add quickshell/
git commit -m "fix: draw the panel's empty and error states honestly"
```

---

### Task 8: Documentation

**Files:**
- Modify: `quickshell/README.md`, `README.md`, `CHANGELOG.md`

- [ ] **Step 1: `quickshell/README.md`**

Add a panel section documenting: what the popup shows; that **left click now toggles the panel** and right click opens the TUI; the four new options (`panelHours` 6, `insightDays` 14, `panelCacheMinutes` 5, `snapshotCommand` `sugarrush snapshot`); that `insightDays: 0` turns the patterns section off and skips the heavy query; and that the panel needs a sugarrush with the `snapshot` command while the pill does not.

- [ ] **Step 2: root `README.md`**

- Command table: add `sugarrush snapshot [--hours N] [--days N]` — "one JSON document for a bar panel".
- Status-bars section: mention that the Quickshell widget has a panel with chart, time-in-range and patterns.

- [ ] **Step 3: `CHANGELOG.md`**

Under `## [Unreleased]`, `### Added`:

```markdown
- **The Quickshell widget has a popup panel.** Clicking the bar pill now opens
  a panel with the last hours as a chart, the five time-in-range bands, and the
  pattern insights — the parts of the dashboard worth a glance without opening
  the TUI. Right click still opens the full app.
- **`sugarrush snapshot` prints the whole picture as JSON.** One document with
  the current reading, a series for a chart, time-in-range and patterns, in your
  display units. Built for the panel; useful to anything that wants sugarrush's
  numbers without scraping a bar line.
```

- [ ] **Step 4: Run the gates and commit**

```bash
mise exec -- cargo fmt --all
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test
./scripts/check-process.sh
git add README.md CHANGELOG.md quickshell/README.md
git commit -m "docs: document the panel and the snapshot command"
```

- [ ] **Step 5: Open the PR**

```bash
git push -u origin feat/quickshell-panel
gh pr create --title "feat: a popup panel for the quickshell widget [skip-demo]" --body "…"
```

The body should carry: what shipped, the `snapshot` document shape, the deliberate left-click change, the states table from Task 7 with what was observed for each, and the note that `[skip-demo]` applies because none of this appears in `--demo`'s TUI.

---

## Notes for the executor

- **The spec is the argument; this plan is the sequence.** Read `docs/specs/2026-08-21-quickshell-panel-design.md` first — it explains why `snapshot` is a subcommand rather than a `--format`, why every value is in display units, and why the panel's coupling is confined to one file.
- **Tasks 1-5 are Rust and testable in isolation.** Tasks 6-7 are QML and can only be checked live on Omarchy; there is no harness, because `qs.Ui` resolves only inside the shell.
- **QML edits need a shell restart.** `omarchy plugin disable`/`enable` does not reload QML and erases the widget's options.
- If a step's code does not match what is actually in the file (line numbers drift), follow the surrounding code's pattern rather than forcing the snippet in.
- **One deliberate deviation from the spec.** The spec says `--days 0` should be
  proven by asserting on a fake client's calls. `nightscout::Client` is concrete
  and mocking it would mean a trait seam that nothing else needs, so the rule
  lives in `history_window`, which is pure and tested directly (Task 3); `fetch`
  then has a single `match` on its result. Same guarantee, no seam.
