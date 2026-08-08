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

use crate::alert::{self, Alert};
use crate::config::Config;
use crate::nightscout::{Client, Entry};
use crate::theme::Theme;

const HOUR_MS: i64 = 3_600_000;
const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Output syntax for a status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Waybar custom module: one JSON object.
    Waybar,
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
            "waybar" | "json" => Some(Format::Waybar),
            "i3blocks" => Some(Format::I3blocks),
            "polybar" => Some(Format::Polybar),
            "tmux" => Some(Format::Tmux),
            "text" | "plain" => Some(Format::Text),
            _ => None,
        }
    }

    pub const NAMES: &'static str = "text, waybar (json), i3blocks, polybar, tmux";
}

/// Everything a status bar might want, resolved once.
#[derive(Debug, Clone)]
pub struct Status {
    /// The reading, in display units — or `—` when there's nothing to show.
    pub value: String,
    pub arrow: String,
    pub delta: String,
    pub state: Alert,
    pub tooltip: String,
    /// Position between the urgent bounds, 0–100.
    pub percentage: u8,
    pub color: Color,
}

impl Status {
    /// The compact one-liner every format is built from.
    pub fn text(&self) -> String {
        format!("{} {} {}", self.value, self.arrow, self.delta)
    }

    /// What fits when the bar is short on room: value and arrow only.
    pub fn short_text(&self) -> String {
        format!("{} {}", self.value, self.arrow)
    }

    /// Render for a bar.
    pub fn render(&self, format: Format) -> String {
        let hex = hex(self.color);
        match format {
            Format::Waybar => json!({
                "text": self.text(),
                "tooltip": self.tooltip,
                "class": self.state.class(),
                "percentage": self.percentage,
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
            arrow: String::new(),
            delta: String::new(),
            state: Alert::Stale,
            tooltip: format!("sugarrush: {e}"),
            percentage: 0,
            color: cfg.theme.resolve().urgent,
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
    let alerts = cfg.alerts.resolve(units);

    let Some(latest) = entries.first() else {
        return Ok(Status {
            value: "—".into(),
            arrow: String::new(),
            delta: String::new(),
            state: Alert::Stale,
            tooltip: "sugarrush: no recent readings".into(),
            percentage: 0,
            color: theme.urgent,
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

    Ok(Status {
        value: units.format(latest.sgv),
        arrow: latest.arrow().to_string(),
        delta,
        state,
        tooltip,
        percentage,
        color: color_for(state, &theme),
    })
}

/// The configured colour for an alert state.
fn color_for(state: Alert, theme: &Theme) -> Color {
    match state {
        Alert::UrgentLow | Alert::UrgentHigh | Alert::Stale => theme.urgent,
        Alert::Low => theme.low,
        Alert::High => theme.high,
        Alert::InRange => theme.in_range,
    }
}

/// A `#rrggbb` string for a colour. Bars want hex, and the terminal's own
/// palette isn't available to them — so named colours map to their conventional
/// values rather than to whatever the terminal would have drawn.
fn hex(color: Color) -> String {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (0xcc, 0x24, 0x1d),
        Color::Green => (0x98, 0x97, 0x1a),
        Color::Yellow => (0xd7, 0x99, 0x21),
        Color::Blue => (0x45, 0x85, 0x88),
        Color::Magenta => (0xb1, 0x62, 0x86),
        Color::Cyan => (0x68, 0x9d, 0x6a),
        Color::Gray | Color::DarkGray => (0x92, 0x83, 0x74),
        Color::LightRed => (0xfb, 0x49, 0x34),
        Color::LightGreen => (0xb8, 0xbb, 0x26),
        Color::LightYellow => (0xfa, 0xbd, 0x2f),
        Color::LightBlue => (0x83, 0xa5, 0x98),
        Color::LightMagenta => (0xd3, 0x86, 0x9b),
        Color::LightCyan => (0x8e, 0xc0, 0x7c),
        Color::White => (0xff, 0xff, 0xff),
        // Indexed and Reset carry no colour we can name; white reads on every
        // bar background, which a guess might not.
        _ => (0xff, 0xff, 0xff),
    };
    format!("#{r:02x}{g:02x}{b:02x}")
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

    fn status() -> Status {
        Status {
            value: "5.6".into(),
            arrow: "→".into(),
            delta: "+0.2".into(),
            state: Alert::InRange,
            tooltip: "tip".into(),
            percentage: 42,
            color: Color::Rgb(0x12, 0x34, 0x56),
        }
    }

    #[test]
    fn format_names_are_forgiving_but_not_guessy() {
        assert_eq!(Format::parse("waybar"), Some(Format::Waybar));
        assert_eq!(Format::parse("JSON"), Some(Format::Waybar));
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
        let json: serde_json::Value =
            serde_json::from_str(&status().render(Format::Waybar)).unwrap();
        assert_eq!(json["text"], "5.6 → +0.2");
        assert_eq!(json["class"], "in-range");
        assert_eq!(json["percentage"], 42);
        assert_eq!(json["tooltip"], "tip");
    }

    #[test]
    fn named_colours_become_hex_for_bars() {
        assert_eq!(hex(Color::Rgb(0, 0x80, 0xff)), "#0080ff");
        assert_eq!(hex(Color::Red), "#cc241d");
        // A colour with no nameable value falls back to something readable
        // rather than a guess.
        assert_eq!(hex(Color::Reset), "#ffffff");
    }

    #[test]
    fn urgent_states_use_the_urgent_colour() {
        let theme = Theme::default();
        for state in [Alert::UrgentLow, Alert::UrgentHigh, Alert::Stale] {
            assert_eq!(color_for(state, &theme), theme.urgent);
        }
        assert_eq!(color_for(Alert::InRange, &theme), theme.in_range);
        assert_eq!(color_for(Alert::High, &theme), theme.high);
        assert_eq!(color_for(Alert::Low, &theme), theme.low);
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
