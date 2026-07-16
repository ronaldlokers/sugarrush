//! Configurable colors for the display.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Color roles as written in config.toml. Each is an optional color name
/// (`red`, `green`, `cyan`, …) or `#rrggbb` hex; omitted roles use defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

impl ThemeConfig {
    pub fn resolve(&self) -> Theme {
        let d = Theme::default();
        Theme {
            low: pick(&self.low, d.low),
            in_range: pick(&self.in_range, d.in_range),
            high: pick(&self.high, d.high),
            urgent: pick(&self.urgent, d.urgent),
            prediction: pick(&self.prediction, d.prediction),
            graph: pick(&self.graph, d.graph),
        }
    }
}

/// Resolved display colors.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub low: Color,
    pub in_range: Color,
    pub high: Color,
    pub urgent: Color,
    pub prediction: Color,
    pub graph: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            low: Color::Red,
            in_range: Color::Green,
            high: Color::Yellow,
            urgent: Color::Red,
            prediction: Color::Magenta,
            graph: Color::Cyan,
        }
    }
}

fn pick(name: &Option<String>, fallback: Color) -> Color {
    name.as_deref().and_then(parse_color).unwrap_or(fallback)
}

/// Parse a color name or `#rrggbb` hex string.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().to_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    Some(match s.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "white" => Color::White,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_case_insensitively() {
        assert_eq!(parse_color("Red"), Some(Color::Red));
        assert_eq!(parse_color(" cyan "), Some(Color::Cyan));
        assert_eq!(parse_color("grey"), Some(Color::Gray));
    }

    #[test]
    fn parses_hex() {
        assert_eq!(parse_color("#ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("#zzz"), None);
    }

    #[test]
    fn unknown_is_none_and_falls_back() {
        assert_eq!(parse_color("chartreuse"), None);
        let t = ThemeConfig {
            low: Some("chartreuse".into()),
            graph: Some("#000000".into()),
            ..Default::default()
        }
        .resolve();
        assert_eq!(t.low, Color::Red); // fallback default
        assert_eq!(t.graph, Color::Rgb(0, 0, 0));
    }
}
