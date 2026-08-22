//! One-shot status-bar output for whatever bar you run.
//!
//! Every bar wants the same three facts — the reading, how it's moving, and how
//! worried to look — and disagrees only about syntax. So this builds one
//! [`Status`] and renders it per format: Waybar's JSON, i3blocks' three-line
//! protocol, polybar's and tmux's inline colour tags, or plain text for
//! everything else (a shell prompt, a macOS menu-bar helper, `watch`).
//!
//! It always prints something. A bar showing a stale dash is a bar you can
//! still read; a bar showing nothing looks like it works.

use anyhow::Result;
use ratatui::style::Color;
use serde_json::json;

use crate::units::Units;

use crate::alert::{self, Alert};
use crate::config::Config;
use crate::nightscout::{Client, Entry};

const HOUR_MS: i64 = 3_600_000;
const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Output syntax for a status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One JSON object, in the shape Waybar's custom modules read. Every bar
    /// that takes JSON reads the same document, which is why the format is
    /// named for the syntax rather than for Waybar.
    Json,
    /// i3blocks: full text, short text, colour — one per line.
    I3blocks,
    /// polybar: inline `%{F#rrggbb}` colour tags.
    Polybar,
    /// tmux: inline `#[fg=#rrggbb]` style tags.
    Tmux,
    /// Plain text, no markup.
    Text,
}

impl Format {
    /// Parse a `--format` value. `None` for anything unrecognised, so the
    /// caller can say which formats exist rather than silently picking one.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            // `waybar` came first and is in every config that predates the
            // rename, so it keeps working; `bar` reads better next to the
            // other bar-shaped names.
            "json" | "waybar" | "bar" => Some(Format::Json),
            "i3blocks" => Some(Format::I3blocks),
            "polybar" => Some(Format::Polybar),
            "tmux" => Some(Format::Tmux),
            "text" | "plain" => Some(Format::Text),
            _ => None,
        }
    }

    pub const NAMES: &'static str = "text, json (waybar, bar), i3blocks, polybar, tmux";
}

/// Everything a status bar might want, resolved once.
#[derive(Debug, Clone)]
pub struct Status {
    /// The reading, in display units — or `—` when there's nothing to show.
    pub value: String,
    /// The display unit the value is in — the one fact that tells a bar what
    /// kind of measurement it is showing.
    pub units: &'static str,
    pub arrow: String,
    pub delta: String,
    pub state: Alert,
    pub tooltip: String,
    /// Position between the urgent bounds, 0–100.
    pub percentage: u8,
    pub color: Color,
    /// The last hour as `(epoch_ms, display_value)`, oldest first, so a bar
    /// can draw the shape of it rather than only the number.
    pub series: Vec<(i64, f64)>,
    /// Where the reading is heading, when predictive alerts are switched on.
    pub forecast: Option<crate::predict::Outlook>,
    /// Set when the forecast — not the reading — is what the colour is saying.
    predicted: Option<Alert>,
}

impl Status {
    /// The CSS-style class a bar styles from. A predicted crossing is named as
    /// one, never as the state itself: the reading has not crossed anything,
    /// and a bar that cannot tell them apart would cry wolf.
    pub fn class(&self) -> String {
        match self.predicted {
            Some(a) => format!("predicted-{}", a.class()),
            None => self.state.class().to_string(),
        }
    }

    /// The tooltip line a predicted crossing earns, if any. A bar that shows
    /// only a colour change would say something is wrong without saying what.
    pub fn forecast_line(&self) -> Option<String> {
        let ahead = self.predicted?;
        let forecast = self.forecast.as_ref()?;
        Some(format!(
            "{} in {} min · {}",
            forecast.value,
            forecast.in_min,
            ahead.label()
        ))
    }

    /// Let a forecast colour a reading that is still in range.
    ///
    /// Only from in-range outwards: an alarm that is already sounding must
    /// never be repainted by a projection that says it is about to stop, and a
    /// projection is not evidence that anything has happened yet.
    pub fn apply_forecast(&mut self, theme: &crate::theme::Theme) {
        if self.state != Alert::InRange {
            return;
        }
        let Some(forecast) = &self.forecast else {
            return;
        };
        let ahead = forecast.alert;
        if ahead == Alert::InRange {
            return;
        }
        self.color = ahead.color(theme);
        self.predicted = Some(ahead);
    }
}

impl Status {
    /// The compact one-liner every format is built from.
    ///
    /// Non-in-range states carry a text marker, not just a colour: only the
    /// Waybar format emits a CSS class, so on polybar, tmux and i3blocks the
    /// colour *was* the entire signal — and `--format text`, the one a shell
    /// prompt or a screen reader consumes, had no state at all.
    pub fn text(&self) -> String {
        format!(
            "{}{} {} {}",
            self.marker(),
            self.value,
            self.arrow,
            self.delta
        )
    }

    /// A terse severity prefix: `!!` urgent, `!` out of range, `?` no data.
    pub fn marker(&self) -> &'static str {
        match self.state {
            Alert::UrgentLow | Alert::UrgentHigh => "!! ",
            Alert::Low | Alert::High => "! ",
            Alert::Stale => "? ",
            Alert::InRange => "",
        }
    }

    /// What fits when the bar is short on room: value and arrow only.
    pub fn short_text(&self) -> String {
        format!("{}{} {}", self.marker(), self.value, self.arrow)
    }

    /// Render for a bar.
    pub fn render(&self, format: Format) -> String {
        let hex = crate::theme::hex(self.color);
        match format {
            // `color` is not part of Waybar's schema — Waybar ignores it, and
            // it styles the module from the class via CSS. It is here for bars
            // that have no stylesheet to write, notably the Quickshell widget,
            // so they can follow the configured sugarrush theme.
            Format::Json => json!({
                "text": self.text(),
                "tooltip": self.tooltip,
                "class": self.class(),
                "percentage": self.percentage,
                "color": hex,
                // The parts as well as the line, so a bar can compose its own
                // — the Quickshell widget puts the unit after the value —
                // without parsing `text` back apart.
                "value": self.value,
                "units": self.units,
                "arrow": self.arrow,
                "delta": self.delta,
                // The shape of the last hour, and where it is heading: a bar
                // that draws a sparkline should not have to fetch twice.
                "series": self.series,
                "forecast": self.forecast,
            })
            .to_string(),
            // i3blocks reads three lines: full text, short text, colour.
            Format::I3blocks => format!("{}\n{}\n{hex}", self.text(), self.short_text()),
            Format::Polybar => format!("%{{F{hex}}}{}%{{F-}}", self.text()),
            Format::Tmux => format!("#[fg={hex}]{}#[default]", self.text()),
            // No markup: whoever consumes this can colour it themselves, and a
            // prompt or a log is better off without escape codes in it.
            Format::Text => self.text(),
        }
    }
}

/// Fetch the last hour and build the status. Never fails: an error becomes a
/// stale-looking status carrying the message, so the bar keeps rendering.
pub async fn status(cfg: &Config) -> Status {
    match build(cfg).await {
        Ok(s) => s,
        Err(e) => Status {
            value: "—".into(),
            units: cfg.units.label(),
            arrow: String::new(),
            delta: String::new(),
            state: Alert::Stale,
            tooltip: format!("sugarrush: {e}"),
            percentage: 0,
            color: cfg.theme.resolve().urgent,
            series: Vec::new(),
            forecast: None,
            predicted: None,
        },
    }
}

async fn build(cfg: &Config) -> Result<Status> {
    let sites = cfg.resolve_sites()?;
    let site = sites
        .first()
        .ok_or_else(|| anyhow::anyhow!("no site configured"))?;
    let client = Client::for_site(site)?;

    let now = chrono::Utc::now().timestamp_millis();
    let entries = client.entries_range(now - HOUR_MS, now, 100).await?;
    let theme = cfg.theme.resolve();
    let units = cfg.units;
    let alerts = site.resolve_alerts(&cfg.alerts, units).0;

    let Some(latest) = entries.first() else {
        return Ok(Status {
            value: "—".into(),
            units: units.label(),
            arrow: String::new(),
            delta: String::new(),
            state: Alert::Stale,
            tooltip: "sugarrush: no recent readings".into(),
            percentage: 0,
            color: theme.urgent,
            series: Vec::new(),
            forecast: None,
            predicted: None,
        });
    };

    let state = alert::evaluate(latest.sgv, now - latest.date, &alerts);
    let delta = entries
        .get(1)
        .map(|prev| units.format_delta(latest.sgv - prev.sgv))
        .unwrap_or_else(|| "--".into());

    // Oldest → newest for the sparkline.
    let values: Vec<f64> = entries.iter().rev().map(|e: &Entry| e.sgv).collect();
    let age_min = ((now - latest.date) / 60_000).max(0);
    let tooltip = format!(
        "{} {}  {}\nΔ {} · {}m ago\n{}",
        units.format(latest.sgv),
        units.label(),
        latest.direction.as_deref().unwrap_or("?"),
        delta,
        age_min,
        sparkline(&values),
    );

    let span = (alerts.urgent_high - alerts.urgent_low).max(1.0);
    let percentage = (((latest.sgv - alerts.urgent_low) / span * 100.0).clamp(0.0, 100.0)) as u8;

    // Oldest first, in display units: a bar draws it left to right, and every
    // other number in this document is already in the unit someone reads in.
    let series = entries
        .iter()
        .rev()
        .map(|e: &Entry| (e.date, scaled(units, e.sgv)))
        .collect();
    // Gated on the same switch as the predictive notification: someone who
    // turned forecasts off should not have one colouring their bar.
    let forecast = (alerts.predict_horizon_minutes > 0)
        .then(|| crate::predict::outlook(&entries, &alerts, units))
        .flatten();

    let mut status = Status {
        value: units.format(latest.sgv),
        units: units.label(),
        arrow: latest.arrow().to_string(),
        delta,
        state,
        tooltip,
        percentage,
        color: state.color(&theme),
        series,
        forecast,
        predicted: None,
    };
    status.apply_forecast(&theme);
    if let Some(line) = status.forecast_line() {
        status.tooltip = format!("{}\n{line}", status.tooltip);
    }
    Ok(status)
}

/// A reading in display units, rounded the way that unit is written.
fn scaled(units: Units, mgdl: f64) -> f64 {
    let value = units.from_mgdl(mgdl);
    match units {
        Units::Mmol => (value * 10.0).round() / 10.0,
        Units::Mgdl => value.round(),
    }
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
    use crate::theme::Theme;

    fn status() -> Status {
        Status {
            value: "5.6".into(),
            units: "mmol/L",
            arrow: "→".into(),
            delta: "+0.2".into(),
            state: Alert::InRange,
            tooltip: "tip".into(),
            percentage: 42,
            color: Color::Rgb(0x12, 0x34, 0x56),
            series: Vec::new(),
            forecast: None,
            predicted: None,
        }
    }

    #[test]
    fn the_json_carries_the_shape_of_the_last_hour() {
        let mut s = status();
        s.series = vec![(1_000, 5.0), (1_300, 5.6)];
        let json: serde_json::Value = serde_json::from_str(&s.render(Format::Json)).unwrap();
        assert_eq!(json["series"][0][0], 1_000);
        assert_eq!(json["series"][0][1], 5.0);
        assert_eq!(json["series"][1][1], 5.6);
    }

    #[test]
    fn a_forecast_crossing_colours_a_reading_that_is_still_in_range() {
        let theme = Theme::default();
        let mut s = status();
        s.forecast = Some(crate::predict::Outlook {
            in_min: 30,
            value: 3.4,
            class: "urgent-low",
            alert: Alert::UrgentLow,
        });
        s.apply_forecast(&theme);
        // The reading has not crossed anything, so the value and the state are
        // untouched — but the pill says a low is coming.
        assert_eq!(s.value, "5.6");
        assert_eq!(s.state, Alert::InRange);
        assert_eq!(s.class(), "predicted-urgent-low");
        assert_eq!(s.color, Alert::UrgentLow.color(&theme));
        // And it says so in words, not only in colour.
        assert_eq!(
            s.forecast_line().as_deref(),
            Some("3.4 in 30 min · URGENT LOW")
        );
    }

    #[test]
    fn a_forecast_never_talks_over_an_alarm_that_is_already_sounding() {
        let theme = Theme::default();
        let mut s = status();
        s.state = Alert::UrgentLow;
        s.color = Alert::UrgentLow.color(&theme);
        // A rebound after a correction: the projection lands high while the
        // reading is still urgently low. What is happening beats what might.
        s.forecast = Some(crate::predict::Outlook {
            in_min: 30,
            value: 11.2,
            class: "high",
            alert: Alert::High,
        });
        s.apply_forecast(&theme);
        assert_eq!(s.class(), "urgent-low");
        assert_eq!(s.color, Alert::UrgentLow.color(&theme));
        assert_eq!(s.forecast_line(), None, "nothing to add to the tooltip");
    }

    #[test]
    fn format_names_are_forgiving_but_not_guessy() {
        // `json` is the name; `waybar` is what every config written before
        // the rename says, and must keep working.
        assert_eq!(Format::parse("json"), Some(Format::Json));
        assert_eq!(Format::parse("waybar"), Some(Format::Json));
        assert_eq!(Format::parse("bar"), Some(Format::Json));
        assert_eq!(Format::parse("JSON"), Some(Format::Json));
        assert_eq!(Format::parse("Tmux"), Some(Format::Tmux));
        assert_eq!(Format::parse("plain"), Some(Format::Text));
        // An unknown name is rejected rather than silently defaulted.
        assert_eq!(Format::parse("i3"), None);
        assert_eq!(Format::parse(""), None);
    }

    #[test]
    fn each_bar_gets_its_own_syntax() {
        let s = status();
        assert_eq!(s.render(Format::Text), "5.6 → +0.2");
        assert_eq!(s.render(Format::Polybar), "%{F#123456}5.6 → +0.2%{F-}");
        assert_eq!(s.render(Format::Tmux), "#[fg=#123456]5.6 → +0.2#[default]");
        // i3blocks: full text, short text, colour — in that order.
        assert_eq!(s.render(Format::I3blocks), "5.6 → +0.2\n5.6 →\n#123456");
    }

    #[test]
    fn waybar_output_is_json_with_the_state_class() {
        let json: serde_json::Value = serde_json::from_str(&status().render(Format::Json)).unwrap();
        assert_eq!(json["text"], "5.6 → +0.2");
        assert_eq!(json["class"], "in-range");
        assert_eq!(json["percentage"], 42);
        assert_eq!(json["tooltip"], "tip");
        // Waybar ignores the extra key; a QML bar widget uses it to follow the
        // configured sugarrush theme instead of hard-coding its own palette.
        assert_eq!(json["color"], "#123456");
        // The parts, so a bar can compose its own line — putting the unit
        // after the value, say — instead of parsing `text` back apart.
        assert_eq!(json["value"], "5.6");
        assert_eq!(json["units"], "mmol/L");
        assert_eq!(json["arrow"], "→");
        assert_eq!(json["delta"], "+0.2");
    }

    #[test]
    fn urgent_states_use_the_urgent_colour() {
        let theme = Theme::default();
        for state in [Alert::UrgentLow, Alert::UrgentHigh, Alert::Stale] {
            assert_eq!(state.color(&theme), theme.urgent);
        }
        assert_eq!(Alert::InRange.color(&theme), theme.in_range);
        assert_eq!(Alert::High.color(&theme), theme.high);
        assert_eq!(Alert::Low.color(&theme), theme.low);
    }

    #[test]
    fn sparkline_maps_range_to_bars() {
        assert_eq!(sparkline(&[]), "");
        let s = sparkline(&[100.0, 150.0, 200.0]);
        assert_eq!(s.chars().count(), 3);
        assert_eq!(s.chars().next(), Some('▁'));
        assert_eq!(s.chars().last(), Some('█'));
    }
}
