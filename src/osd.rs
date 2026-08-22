//! Omarchy's on-screen display, as a second urgent channel.
//!
//! A desktop notification is not enough on its own. Omarchy's notification
//! service suppresses everything under Do Not Disturb except its own toasts
//! and `notify-send` at critical urgency — sugarrush sends under its own app
//! name, so a night with Do Not Disturb on is a night the toast never appears.
//! The OSD is drawn by the shell itself on the overlay layer: it is above
//! fullscreen windows, and no notification policy can hold it back.

use crate::alert::Alert;
use crate::units::Units;

/// How long an urgent OSD stays up. Long enough to catch someone walking
/// past, bounded so a crashed sugarrush cannot leave the screen occupied.
pub const SECONDS: u64 = 30;

/// The glyph the OSD shows beside the message. Nerd Font, which the shell
/// already requires for its own OSDs.
pub fn glyph(alert: Alert) -> &'static str {
    match alert {
        // A clock, not a warning: a stale alarm says the readings stopped,
        // which is a different problem from the glucose being wrong.
        Alert::Stale => "\u{f0150}",
        _ => "\u{f0026}",
    }
}

/// The alert's name, short enough for one OSD line.
///
/// `Alert::label` is written for a notification body with room to spare;
/// `SENSOR GAP \u{2014} no recent readings` is more than twice what fits here.
fn label(alert: Alert) -> &'static str {
    match alert {
        Alert::Stale => "NO DATA",
        other => other.label(),
    }
}

/// Whether this alert earns an OSD.
///
/// Urgent only, deliberately: the OSD is the channel that cannot be turned
/// down or deferred, so spending it on an in-range crossing would train
/// someone to ignore the one thing that must not be ignored.
pub fn should_show(alerts: &crate::config::Alerts, alert: Alert) -> bool {
    alerts.osd && alert.is_urgent()
}

/// The JSON payload for `osd show`.
///
/// The message is one short line: the OSD elides past its own maximum width,
/// and an elided alarm is worse than a terse one.
pub fn payload(
    alert: Alert,
    sgv: Option<f64>,
    units: Units,
    content: bool,
    seconds: u64,
) -> String {
    // Same privacy rule as the notification: with content off, nothing that a
    // glance across the room shouldn't read. The OSD is the more exposed of
    // the two channels, since no lock screen policy applies to it.
    let message = if !content {
        "sugarrush alert".to_string()
    } else {
        match sgv {
            // No separator between the label and the reading: the OSD elides
            // past its own width, and the pair only fits without one.
            Some(v) => format!("{} {} {}", label(alert), units.format(v), units.label()),
            None => label(alert).to_string(),
        }
    };
    serde_json::json!({
        "icon": glyph(alert),
        "message": message,
        "duration": seconds.saturating_mul(1000),
    })
    .to_string()
}

/// Show the payload on Omarchy's OSD. Best-effort: no shell, no OSD, no error.
///
/// Goes through `omarchy-shell` rather than `qs` directly, which buys three
/// things a direct call would have to reimplement: the `--` that keeps a
/// function named `show` from being parsed as `qs ipc show`, a timeout, and
/// recovery of `WAYLAND_DISPLAY` for a caller that has none — which the
/// systemd-run watcher generally does not.
///
/// Returns whether the OSD accepted the call, so the caller can log a dead
/// channel the same way it logs a dead notification daemon. The exit status
/// cannot answer that: `omarchy-shell` reports IPC-level failures on stdout
/// and still exits zero, so the reply itself is what gets checked.
pub fn show(payload: &str) -> bool {
    let mut cmd = std::process::Command::new("omarchy-shell");
    cmd.args(["osd", "show", payload]);
    // A user unit started outside a graphical session inherits neither, and
    // `omarchy-shell` refuses without the first of them.
    if std::env::var_os("OMARCHY_PATH").is_none() {
        cmd.env("OMARCHY_PATH", "/usr/share/omarchy");
    }
    let Ok(out) = cmd.stdin(std::process::Stdio::null()).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "ok"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_names_the_state_and_the_reading() {
        let json: serde_json::Value = serde_json::from_str(&payload(
            Alert::UrgentLow,
            Some(54.0),
            Units::Mgdl,
            true,
            30,
        ))
        .expect("payload is JSON");
        assert_eq!(json["message"], "URGENT LOW 54 mg/dL");
        assert_eq!(json["duration"], 30_000);
        assert_eq!(json["icon"], glyph(Alert::UrgentLow));
    }

    #[test]
    fn only_urgent_alerts_reach_the_osd() {
        let mut alerts = crate::config::Alerts::default();
        assert!(alerts.osd, "the OSD is on by default");
        assert!(should_show(&alerts, Alert::UrgentLow));
        assert!(should_show(&alerts, Alert::Stale));
        assert!(!should_show(&alerts, Alert::Low), "a low is not urgent");
        assert!(!should_show(&alerts, Alert::High), "a high is not urgent");
        alerts.osd = false;
        assert!(
            !should_show(&alerts, Alert::UrgentLow),
            "the setting is off"
        );
    }

    #[test]
    fn the_worst_case_message_fits_the_osd() {
        // The OSD elides past `maxMessageWidth`, and an elided alarm is worse
        // than a terse one. 21 characters is what fit on the shipped shell at
        // its own font size; the widest thing sugarrush can say is an urgent
        // high with a three-digit mg/dL reading.
        let widest = payload(Alert::UrgentHigh, Some(288.0), Units::Mgdl, true, 30);
        let json: serde_json::Value = serde_json::from_str(&widest).unwrap();
        let message = json["message"].as_str().unwrap();
        assert!(
            message.chars().count() <= 21,
            "{message} is {} characters and will elide",
            message.chars().count()
        );
    }

    #[test]
    fn a_stale_alarm_is_not_dressed_as_a_glucose_alarm() {
        assert_ne!(glyph(Alert::Stale), glyph(Alert::UrgentLow));
        assert_eq!(label(Alert::Stale), "NO DATA");
        assert!(
            label(Alert::Stale).chars().count() <= 21,
            "the notification's own wording does not fit here"
        );
    }

    #[test]
    fn a_content_free_payload_carries_no_reading() {
        let json: serde_json::Value = serde_json::from_str(&payload(
            Alert::UrgentLow,
            Some(54.0),
            Units::Mgdl,
            false,
            30,
        ))
        .expect("payload is JSON");
        let message = json["message"].as_str().unwrap();
        assert!(!message.contains("54"), "leaked the reading: {message}");
        assert_eq!(message, "sugarrush alert");
    }
}
