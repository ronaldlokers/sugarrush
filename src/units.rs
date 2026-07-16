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
}
