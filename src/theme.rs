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

/// The six color roles, in the order used by the settings screen and by
/// [`theme_from_names`] / [`ThemeConfig`].
pub const DEFAULT_NAMES: [&str; 6] = ["red", "green", "yellow", "red", "magenta", "cyan"];

/// A colorblind-safe palette (Okabe–Ito) as hex, same role order as
/// [`DEFAULT_NAMES`]: low / in-range / high / urgent / forecast / graph.
pub const COLORBLIND_NAMES: [&str; 6] = [
    "#0072b2", // low — blue
    "#009e73", // in range — bluish green
    "#e69f00", // high — orange
    "#d55e00", // urgent — vermillion
    "#cc79a7", // forecast — pink
    "#56b4e9", // graph — sky blue
];

/// Named colors the settings screen cycles through.
pub const PALETTE: [&str; 12] = [
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "white",
    "lightred",
    "lightgreen",
    "lightyellow",
    "lightblue",
];

/// Next/previous palette color relative to `current` (wrapping).
pub fn cycle_color(current: &str, dir: i32) -> &'static str {
    let idx = PALETTE.iter().position(|&c| c == current).unwrap_or(0) as i32;
    let n = PALETTE.len() as i32;
    PALETTE[(idx + dir).rem_euclid(n) as usize]
}

/// Build a [`Theme`] from six color names (role order matches `DEFAULT_NAMES`).
pub fn theme_from_names(names: &[String; 6]) -> Theme {
    let d = Theme::default();
    Theme {
        low: parse_color(&names[0]).unwrap_or(d.low),
        in_range: parse_color(&names[1]).unwrap_or(d.in_range),
        high: parse_color(&names[2]).unwrap_or(d.high),
        urgent: parse_color(&names[3]).unwrap_or(d.urgent),
        prediction: parse_color(&names[4]).unwrap_or(d.prediction),
        graph: parse_color(&names[5]).unwrap_or(d.graph),
    }
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
    fn cycle_color_wraps_both_ways() {
        assert_eq!(cycle_color("red", 1), "green");
        assert_eq!(cycle_color("red", -1), *PALETTE.last().unwrap());
        // Unknown color starts from index 0.
        assert_eq!(cycle_color("chartreuse", 1), PALETTE[1]);
    }

    #[test]
    fn theme_from_names_uses_defaults_for_bad_names() {
        let names = [
            "chartreuse".to_string(), // invalid → default low (red)
            "cyan".to_string(),
            "yellow".to_string(),
            "red".to_string(),
            "magenta".to_string(),
            "blue".to_string(),
        ];
        let t = theme_from_names(&names);
        assert_eq!(t.low, Color::Red); // fell back
        assert_eq!(t.in_range, Color::Cyan);
        assert_eq!(t.graph, Color::Blue);
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
