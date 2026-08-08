//! Glucose unit handling. Nightscout stores sensor values (sgv) in mg/dL.

use serde::{Deserialize, Serialize};

pub const MMOL_PER_MGDL: f64 = 1.0 / 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Mgdl,
    Mmol,
}

impl Units {
    pub fn toggle(self) -> Self {
        match self {
            Units::Mgdl => Units::Mmol,
            Units::Mmol => Units::Mgdl,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Units::Mgdl => "mg/dL",
            Units::Mmol => "mmol/L",
        }
    }

    /// Convert a raw mg/dL value into this unit.
    #[allow(clippy::wrong_self_convention)]
    pub fn from_mgdl(self, mgdl: f64) -> f64 {
        match self {
            Units::Mgdl => mgdl,
            Units::Mmol => mgdl * MMOL_PER_MGDL,
        }
    }

    /// Convert a value expressed in this unit back into raw mg/dL.
    pub fn to_mgdl(self, value: f64) -> f64 {
        match self {
            Units::Mgdl => value,
            Units::Mmol => value / MMOL_PER_MGDL,
        }
    }

    /// Format a raw mg/dL value for display in this unit.
    pub fn format(self, mgdl: f64) -> String {
        match self {
            Units::Mgdl => format!("{:.0}", mgdl),
            Units::Mmol => format!("{:.1}", self.from_mgdl(mgdl)),
        }
    }

    /// Format a difference with its sign. The sign follows the value *as
    /// displayed*, so a change too small to show doesn't render as `-0.0`
    /// (or a rise of 0.4 mg/dL as a bare `+0`) — it reads as flat.
    pub fn format_delta(self, mgdl: f64) -> String {
        let body = self.format(mgdl.abs());
        if body.chars().all(|c| c == '0' || c == '.') {
            return body;
        }
        format!("{}{}", if mgdl >= 0.0 { "+" } else { "-" }, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_sign_follows_the_displayed_value() {
        // A change too small to render must not carry a sign.
        assert_eq!(Units::Mgdl.format_delta(-0.4), "0");
        assert_eq!(Units::Mgdl.format_delta(0.4), "0");
        assert_eq!(Units::Mmol.format_delta(-0.4), "0.0");
        // Real changes keep theirs.
        assert_eq!(Units::Mgdl.format_delta(5.0), "+5");
        assert_eq!(Units::Mgdl.format_delta(-12.0), "-12");
        assert_eq!(Units::Mmol.format_delta(-18.0), "-1.0");
    }
}
