//! Synthetic data for `--demo` mode: a plausible glucose trace and treatments,
//! generated deterministically so the app runs with no config or network (used
//! for the README recording and for trying sugarrush before setup).

use crate::nightscout::{DeviceStatus, Entry, Treatment};

const STEP_MS: i64 = 5 * 60_000;

/// A smooth, deterministic glucose trace over `[start_ms, end_ms]`, newest
/// first (as Nightscout returns entries). Values in mg/dL.
pub fn entries(start_ms: i64, end_ms: i64) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut t = end_ms - end_ms.rem_euclid(STEP_MS); // align to 5-min grid
    while t >= start_ms {
        let sgv = curve(t);
        let prev = curve(t - STEP_MS);
        out.push(Entry {
            sgv,
            date: t,
            direction: Some(direction(sgv - prev).to_string()),
        });
        t -= STEP_MS;
    }
    out
}

/// mg/dL at time `t` — two sine waves for a natural-looking wander.
fn curve(t: i64) -> f64 {
    let m = t as f64 / 60_000.0; // minutes
    let v = 135.0 + 60.0 * (m / 47.0).sin() + 18.0 * (m / 11.0).cos();
    v.clamp(45.0, 320.0)
}

fn direction(delta: f64) -> &'static str {
    match delta {
        d if d > 8.0 => "SingleUp",
        d if d > 2.0 => "FortyFiveUp",
        d if d < -8.0 => "SingleDown",
        d if d < -2.0 => "FortyFiveDown",
        _ => "Flat",
    }
}

/// Synthetic IOB/COB + battery.
pub fn device() -> DeviceStatus {
    DeviceStatus {
        battery: Some(76),
        device: Some("demo".into()),
        last_ms: None,
        iob: Some(1.4),
        cob: Some(18.0),
    }
}

/// A couple of treatment markers within the last few hours.
pub fn treatments(now_ms: i64) -> Vec<Treatment> {
    vec![
        Treatment {
            at_ms: now_ms - 95 * 60_000,
            carbs: Some(40.0),
            insulin: Some(4.5),
        },
        Treatment {
            at_ms: now_ms - 40 * 60_000,
            carbs: None,
            insulin: Some(1.2),
        },
    ]
}
