//! Shareable exports of a period of readings: raw CSV, and a plain-text
//! clinical summary.
//!
//! The point is handing something to a clinician (or to a spreadsheet) without
//! screenshots of a terminal. Both formats cover the same fixed window — the
//! `AGP days` setting — so the numbers in the summary describe exactly the rows
//! in the CSV.

use chrono::{Local, TimeZone};

use crate::agp;
use crate::config::Alerts;
use crate::nightscout::Entry;
use crate::stats;
use crate::units::Units;

/// Which file an export produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Report,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Report => "txt",
        }
    }
}

/// Write both files into `dir`, owner-only, returning their absolute paths.
///
/// Shared by the `e` key and the `export` subcommand, which had drifted: the
/// CLI honoured `--out` while the in-app export wrote bare filenames into
/// whatever directory the process happened to start in. Both used
/// `std::fs::write`, i.e. 0644 — the config file holding a *read-only token*
/// gets 0600, while a named person's 14-day glucose record did not.
pub fn write_pair(
    dir: &std::path::Path,
    entries: &[Entry],
    alerts: &Alerts,
    units: Units,
    days: u32,
    now_ms: i64,
    timezone: Option<&str>,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    use anyhow::Context;
    let mut written = Vec::new();
    for (format, body) in [
        (Format::Csv, csv_in(entries, units, timezone)),
        (
            Format::Report,
            report_in(entries, alerts, units, days, now_ms, timezone),
        ),
    ] {
        let path = dir.join(filename(format, now_ms));
        crate::config::write_private(&path, &body)
            .with_context(|| format!("failed to write {}", path.display()))?;
        written.push(std::fs::canonicalize(&path).unwrap_or(path));
    }
    Ok(written)
}

/// `sugarrush-YYYYMMDD-HHMM.csv` — sortable, and obvious in a Downloads folder.
pub fn filename(format: Format, now_ms: i64) -> String {
    let stamp = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|dt| dt.format("%Y%m%d-%H%M").to_string())
        .unwrap_or_else(|| "export".into());
    format!("sugarrush-{stamp}.{}", format.extension())
}

/// Readings as CSV, oldest first — the order a human reads and a spreadsheet
/// charts. Both units are written out: mg/dL because that's what's stored and
/// what every analysis tool expects, and the display unit because that's what
/// the person exporting recognises.
/// Quote a CSV field, and defuse anything a spreadsheet would execute.
///
/// `direction` is a free string from the Nightscout JSON — which, since
/// follower mode, may be *someone else's* server. A comma or newline breaks
/// the file structurally; a leading `=`, `+`, `-`, `@`, tab or CR is run as a
/// formula by Excel, LibreOffice and Sheets. This file exists to be opened in
/// a spreadsheet, so neither is theoretical.
fn field(value: &str) -> String {
    let needs_guard = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let escaped = value.replace('"', "\"\"");
    if needs_guard {
        format!("\"'{escaped}\"")
    } else if value.contains([',', '"', '\n', '\r']) {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

fn timestamp(ms: i64, timezone: Option<&str>, format: &str) -> String {
    if let Some(tz) = timezone.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
        return tz
            .timestamp_millis_opt(ms)
            .single()
            .map(|dt| dt.format(format).to_string())
            .unwrap_or_else(|| "—".into());
    }
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format(format).to_string())
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
pub fn csv(entries: &[Entry], units: Units) -> String {
    csv_in(entries, units, None)
}

pub fn csv_in(entries: &[Entry], units: Units, timezone: Option<&str>) -> String {
    let mut out = String::from("timestamp,epoch_ms,mgdl,");
    out.push_str(match units {
        Units::Mmol => "mmol_l",
        Units::Mgdl => "mgdl_display",
    });
    out.push_str(",direction\n");
    for e in entries.iter().rev() {
        let ts = timestamp(e.date, timezone, "%+");
        out.push_str(&format!(
            "{},{},{:.0},{},{}\n",
            ts,
            e.date,
            e.sgv,
            units.format(e.sgv),
            field(e.direction.as_deref().unwrap_or(""))
        ));
    }
    out
}

/// A plain-text clinical summary: the standard metrics over the window, then
/// the time-of-day profile that shows *when* things go wrong. Deliberately
/// fixed-width text — it survives email, a chat message, and a printer.
#[cfg(test)]
pub fn report(entries: &[Entry], alerts: &Alerts, units: Units, days: u32, now_ms: i64) -> String {
    report_in(entries, alerts, units, days, now_ms, None)
}

pub fn report_in(
    entries: &[Entry],
    alerts: &Alerts,
    units: Units,
    days: u32,
    now_ms: i64,
    timezone: Option<&str>,
) -> String {
    let mut out = String::new();
    let u = units.label();
    let stamp = |ms: i64| timestamp(ms, timezone, "%Y-%m-%d %H:%M");

    out.push_str("sugarrush glucose summary\n");
    out.push_str("=========================\n\n");
    out.push_str(&format!(
        "Period      {} days, to {}\n",
        days,
        stamp(now_ms)
    ));
    out.push_str(&format!(
        "Target      {}–{} {u}\n",
        units.format(alerts.low),
        units.format(alerts.high)
    ));
    out.push_str(&format!(
        "Timezone    {}\n",
        timezone.unwrap_or("local (viewer)")
    ));

    if entries.is_empty() {
        out.push_str("\nNo readings in this period.\n");
        return out;
    }

    // Sensor coverage: every metric below is only as good as this, so it goes
    // above them rather than in a footnote. A CGM posts every 5 minutes.
    let expected = (days as f64 * 24.0 * 12.0).max(1.0);
    let coverage = (entries.len() as f64 / expected * 100.0).min(100.0);
    let (oldest, newest) = (
        entries.last().map(|e| e.date).unwrap_or(now_ms),
        entries.first().map(|e| e.date).unwrap_or(now_ms),
    );
    out.push_str(&format!(
        "Readings    {} ({:.0}% sensor coverage), {} → {}\n",
        entries.len(),
        coverage,
        stamp(oldest),
        stamp(newest)
    ));

    out.push_str("\nTime in range\n-------------\n");
    if let Some(t) = stats::tir(
        entries,
        alerts.urgent_low,
        alerts.low,
        alerts.high,
        alerts.urgent_high,
    ) {
        let row = |label: &str, pct: f64, range: String| {
            // 5% per cell, so a full 100% bar is 20 cells and stays inside its
            // column — the range labels have to line up to be scannable.
            let cells = ((pct / 5.0).round() as usize).min(20);
            format!(
                "{label:<11}{pct:>5.1}%  {:<21} {}\n",
                "█".repeat(cells),
                range
            )
        };
        out.push_str(&row(
            "Very high",
            t.very_high,
            format!("≥ {} {u}", units.format(alerts.urgent_high)),
        ));
        out.push_str(&row(
            "High",
            t.high,
            format!("> {} {u}", units.format(alerts.high)),
        ));
        out.push_str(&row(
            "In range",
            t.in_range,
            format!(
                "{}–{} {u}",
                units.format(alerts.low),
                units.format(alerts.high)
            ),
        ));
        out.push_str(&row(
            "Low",
            t.low,
            format!("< {} {u}", units.format(alerts.low)),
        ));
        out.push_str(&row(
            "Very low",
            t.very_low,
            format!("≤ {} {u}", units.format(alerts.urgent_low)),
        ));
        out.push_str(&format!(
            "\nTime below range {:.1}%  (consensus goal: < 4%, of which < 1% very low) [1]\n",
            t.below()
        ));
    }

    out.push_str("\nAverages\n--------\n");
    if let Some(mean) = stats::mean_mgdl(entries) {
        out.push_str(&format!("Mean        {} {u}\n", units.format(mean)));
        out.push_str(&format!(
            "GMI         {:.1}%   (estimated A1c) [2]\n",
            stats::gmi(mean)
        ));
    }
    if let Some(cv) = stats::cv_pct(entries) {
        out.push_str(&format!(
            "CV          {cv:.1}%   (variability; consensus goal: ≤ 36%) [1]\n"
        ));
    }

    // Hourly profile: the median plus the 5–95 spread per hour of the day, so
    // an overnight low or a post-meal spike is visible as a pattern rather than
    // averaged away.
    out.push_str("\nTime of day (median and 5–95% spread)\n");
    out.push_str("-------------------------------------\n");
    let tz = timezone.and_then(|name| name.parse::<chrono_tz::Tz>().ok());
    let bands = agp::profile_in(entries, tz);
    for hour in 0..24 {
        let in_hour: Vec<&agp::Band> = bands
            .iter()
            .filter(|b| b.minute / 60 == hour as i64)
            .collect();
        if in_hour.is_empty() {
            continue;
        }
        let avg = |f: fn(&agp::Band) -> f64| -> f64 {
            in_hour.iter().map(|b| f(b)).sum::<f64>() / in_hour.len() as f64
        };
        let (p05, p50, p95) = (avg(|b| b.p05), avg(|b| b.p50), avg(|b| b.p95));
        // Position the median on a 40-cell track spanning the reporting range,
        // so the shape of the day is legible in plain text.
        let lo = alerts.urgent_low.min(p05);
        let hi = alerts.urgent_high.max(p95);
        let pos = ((p50 - lo) / (hi - lo).max(1.0) * 39.0).round() as usize;
        let mut track = [' '; 40];
        track[pos.min(39)] = '●';
        out.push_str(&format!(
            "{hour:02}:00  {:>5} {:>5} {:>5}  |{}|\n",
            units.format(p05),
            units.format(p50),
            units.format(p95),
            track.iter().collect::<String>()
        ));
    }

    // Patterns last: the numbers above are what a clinician checks first, and
    // this is the "so when does it actually go wrong" follow-up.
    let found = agp::insights(&bands, alerts.low, alerts.high);
    if !found.is_empty() {
        out.push_str("\nPatterns\n--------\n");
        for i in &found {
            out.push_str(&format!("{}\n", i.text(units)));
        }
        out.push_str(
            "\n(A \"lows\" window is a time of day when a quarter of readings or\n\
             more sit below target; \"highs\" is when the typical reading is above\n\
             it. Runs shorter than 45 minutes aren't reported.)\n",
        );
    }

    // Where the goals come from. A clinician reading "consensus goal: < 4%"
    // is entitled to know whose consensus, and the reader should be able to
    // tell that the percentages above are computed against *this user's*
    // configured thresholds — which may not be the ones the goals assume.
    out.push_str("\nReferences\n----------\n");
    out.push_str(
        "[1] Battelino T, et al. Clinical targets for continuous glucose\n\
         \x20   monitoring data interpretation: recommendations from the\n\
         \x20   international consensus on time in range. Diabetes Care\n\
         \x20   2019;42(8):1593-1603.\n",
    );
    out.push_str(
        "[2] Bergenstal RM, et al. Glucose Management Indicator (GMI): a new\n\
         \x20   term for estimating A1C from continuous glucose monitoring.\n\
         \x20   Diabetes Care 2018;41(11):2275-2280.\n",
    );
    // 70-180 mg/dL is the range the consensus targets are stated against.
    out.push_str(&format!(
        "\nThe consensus targets are stated for a {}–{} {u} range; the percentages\n",
        units.format(70.0),
        units.format(180.0)
    ));
    out.push_str("above are computed against the thresholds configured in sugarrush,\n");
    out.push_str("shown under Target at the top. Compare like with like.\n");

    out.push_str("\nsugarrush is not a medical device. These figures come from CGM\n");
    out.push_str("data as published by Nightscout and are for discussion, not\n");
    out.push_str("treatment decisions.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sgv: f64, date: i64) -> Entry {
        Entry {
            sgv,
            date,
            direction: Some("Flat".into()),
        }
    }

    const NOW: i64 = 1_700_000_000_000;

    fn window() -> Vec<Entry> {
        // Newest first, as everywhere else in the app.
        (0..12)
            .map(|i| entry(100.0 + i as f64 * 5.0, NOW - i * 5 * 60_000))
            .collect()
    }

    /// A clinician reading "consensus goal: < 4%" is entitled to know whose
    /// consensus, and to see that the percentages are computed against the
    /// user's own thresholds rather than the ones the goals assume.
    /// A clinician reading "consensus goal: < 4%" is entitled to know whose
    /// consensus, and to see that the percentages are computed against the
    /// user's own thresholds rather than the ones the goals assume.
    #[test]
    fn dump_refs() {
        let alerts = Alerts::default();
        let out = report(&window(), &alerts, Units::Mgdl, 14, NOW);
        println!("{}", &out[out.len().saturating_sub(900)..]);
    }

    #[test]
    fn the_summary_attributes_its_clinical_figures() {
        let alerts = Alerts::default();
        let out = report(&window(), &alerts, Units::Mgdl, 14, NOW);

        assert!(
            out.contains("Battelino"),
            "the TIR/CV targets need a source"
        );
        // The journal name wraps onto the previous line; the locator does not.
        assert!(out.contains("2019;42(8):1593-1603"));
        assert!(out.contains("Bergenstal"), "GMI needs a source");
        assert!(out.contains("2018;41(11):2275-2280"));

        // Every cited figure carries its marker.
        for (figure, marker) in [("Time below range", "[1]"), ("CV ", "[1]"), ("GMI ", "[2]")] {
            let line = out
                .lines()
                .find(|l| l.contains(figure))
                .unwrap_or_else(|| panic!("no {figure:?} line in the summary"));
            assert!(line.contains(marker), "{figure:?} cites nothing: {line:?}");
        }

        // And the caveat that makes the comparison honest.
        assert!(out.contains("Compare like with like."));
    }

    #[test]
    fn csv_has_a_header_and_is_oldest_first() {
        let out = csv(&window(), Units::Mmol);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("timestamp,epoch_ms,mgdl,mmol_l,direction"));
        assert_eq!(lines.len(), 13); // header + 12 readings
                                     // Oldest row first: it carries the oldest epoch.
        let first_epoch: i64 = lines[1].split(',').nth(1).unwrap().parse().unwrap();
        let last_epoch: i64 = lines[12].split(',').nth(1).unwrap().parse().unwrap();
        assert!(first_epoch < last_epoch);
        assert_eq!(last_epoch, NOW);
        // mg/dL is always written, alongside the display unit.
        assert!(lines[12].contains(",100,5.6,Flat") || lines[12].contains(",100,5.5,Flat"));
    }

    #[test]
    fn configured_timezone_is_declared_and_written_with_an_offset() {
        let csv = csv_in(&window(), Units::Mgdl, Some("America/New_York"));
        let timestamp = csv.lines().nth(1).unwrap().split(',').next().unwrap();
        assert!(timestamp.ends_with("-05:00") || timestamp.ends_with("-04:00"));
        let report = report_in(
            &window(),
            &Alerts::default(),
            Units::Mgdl,
            14,
            NOW,
            Some("America/New_York"),
        );
        assert!(report.contains("Timezone    America/New_York"));
    }

    #[test]
    fn csv_of_nothing_is_still_a_valid_file() {
        let out = csv(&[], Units::Mgdl);
        assert_eq!(out.lines().count(), 1); // header only
    }

    #[test]
    fn report_covers_the_clinical_metrics() {
        let out = report(&window(), &Alerts::default(), Units::Mgdl, 14, NOW);
        for expected in [
            "sugarrush glucose summary",
            "Period      14 days",
            "Time in range",
            "Very low",
            "Time below range",
            "GMI",
            "CV",
            "Time of day",
            "not a medical device",
        ] {
            assert!(out.contains(expected), "report is missing {expected:?}");
        }
        // Coverage is honest: 12 readings is nothing like 14 days of data.
        assert!(out.contains("0% sensor coverage"), "{out}");
    }

    #[test]
    fn report_says_so_when_there_is_nothing_to_report() {
        let out = report(&[], &Alerts::default(), Units::Mmol, 14, NOW);
        assert!(out.contains("No readings in this period."));
        // And doesn't invent statistics for an empty window.
        assert!(!out.contains("GMI"));
    }

    #[test]
    fn filenames_are_sortable_and_typed() {
        let csv_name = filename(Format::Csv, NOW);
        let txt_name = filename(Format::Report, NOW);
        assert!(csv_name.starts_with("sugarrush-"));
        assert!(csv_name.ends_with(".csv"));
        assert!(txt_name.ends_with(".txt"));
        // Same instant → same stem, so the pair stays together in a listing.
        assert_eq!(
            csv_name.trim_end_matches(".csv"),
            txt_name.trim_end_matches(".txt")
        );
    }

    #[test]
    fn csv_fields_cannot_break_the_file_or_run_in_a_spreadsheet() {
        // A hostile or merely odd `direction` from the server.
        let entries = vec![Entry {
            sgv: 100.0,
            date: NOW,
            direction: Some("=cmd|'/c calc'!A1".into()),
        }];
        let out = csv(&entries, Units::Mgdl);
        let row = out.lines().nth(1).unwrap();
        assert!(row.ends_with("\"'=cmd|'/c calc'!A1\""), "{row}");

        // A comma must not add a column.
        let entries = vec![Entry {
            sgv: 100.0,
            date: NOW,
            direction: Some("Flat,Extra".into()),
        }];
        let out = csv(&entries, Units::Mgdl);
        let row = out.lines().nth(1).unwrap();
        assert_eq!(row.matches(',').count(), 5, "field count changed: {row}");
        assert!(row.ends_with("\"Flat,Extra\""), "{row}");

        // The ordinary case stays unquoted and readable.
        let entries = vec![Entry {
            sgv: 100.0,
            date: NOW,
            direction: Some("Flat".into()),
        }];
        assert!(csv(&entries, Units::Mgdl)
            .lines()
            .nth(1)
            .unwrap()
            .ends_with(",Flat"));
    }
}
