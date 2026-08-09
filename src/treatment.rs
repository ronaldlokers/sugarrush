//! Explicit, audited Nightscout CarePortal writes.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Config;
use crate::nightscout::Client;

#[derive(Debug, Clone)]
pub struct Request {
    pub site: String,
    pub carbs: Option<f64>,
    pub insulin: Option<f64>,
    pub note: Option<String>,
    pub at: Option<String>,
    pub confirm: bool,
}

#[derive(Serialize)]
struct Audit<'a> {
    ts: i64,
    site: &'a str,
    identifier: &'a str,
    outcome: &'a str,
    carbs: Option<f64>,
    insulin: Option<f64>,
    note_present: bool,
}

pub async fn run(request: Request) -> Result<()> {
    if !request.confirm {
        bail!("treatment writes require --confirm; review the person, amounts, and time first");
    }
    let carbs = checked_amount("carbs", request.carbs, 0.1, 300.0)?;
    let insulin = checked_amount("insulin", request.insulin, 0.01, 50.0)?;
    if carbs.is_none() && insulin.is_none() {
        bail!("provide --carbs G and/or --insulin U");
    }
    let note = request.note.as_deref().map(sanitize_note).transpose()?;
    let created = match request.at.as_deref() {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .context("--at must be RFC3339, for example 2026-08-09T14:30:00+02:00")?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    if (created.timestamp_millis() - Utc::now().timestamp_millis()).abs() > 7 * 86_400_000 {
        bail!("treatment time must be within seven days of now");
    }
    let cfg = Config::load()?;
    let sites = cfg.resolve_sites()?;
    let site = sites
        .iter()
        .find(|site| site.name == request.site)
        .with_context(|| {
            format!(
                "unknown site '{}'; available: {}",
                request.site,
                sites
                    .iter()
                    .map(|site| site.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    if site.is_insecure() {
        bail!("refusing a health-data write over unencrypted HTTP");
    }
    // Nightscout's UUID handling preserves this as `identifier`, which its
    // sync layer uses to deduplicate a replayed request.
    let identifier = uuid::Uuid::new_v4().to_string();
    let event_type = match (carbs, insulin) {
        (Some(_), Some(_)) => "Meal Bolus",
        (Some(_), None) => "Carb Correction",
        (None, Some(_)) => "Correction Bolus",
        (None, None) => unreachable!(),
    };
    let mut body = serde_json::json!({
        "_id": identifier,
        "identifier": identifier,
        "eventType": event_type,
        "created_at": created.to_rfc3339(),
        "mills": created.timestamp_millis(),
        "enteredBy": "sugarrush"
    });
    if let Some(value) = carbs {
        body["carbs"] = value.into();
    }
    if let Some(value) = insulin {
        body["insulin"] = value.into();
    }
    if let Some(value) = &note {
        body["notes"] = value.clone().into();
    }

    let result = Client::create_treatment(site, &body).await;
    audit(Audit {
        ts: Utc::now().timestamp_millis(),
        site: &site.name,
        identifier: &identifier,
        outcome: if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        },
        carbs,
        insulin,
        note_present: note.is_some(),
    })?;
    result?;
    println!(
        "Nightscout accepted treatment for {} at {} (id {identifier})",
        site.name,
        created.to_rfc3339()
    );
    println!("Accepted does not mean clinically verified; confirm it in Nightscout.");
    Ok(())
}

fn checked_amount(name: &str, value: Option<f64>, min: f64, max: f64) -> Result<Option<f64>> {
    if let Some(value) = value {
        if !value.is_finite() || !(min..=max).contains(&value) {
            bail!("{name} must be between {min} and {max}");
        }
    }
    Ok(value)
}

fn sanitize_note(value: &str) -> Result<String> {
    let value: String = value
        .trim()
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    if value.chars().count() > 200 {
        bail!("note must be 200 characters or fewer");
    }
    if value.is_empty() {
        bail!("note is empty after unsafe control characters were removed");
    }
    Ok(value)
}

fn audit_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("sugarrush")
        .join("treatments.jsonl")
}

fn audit(entry: Audit<'_>) -> Result<()> {
    let path = audit_path();
    std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_removes_terminal_and_bidi_controls() {
        assert_eq!(
            sanitize_note(" meal\n\u{202e}ok ").unwrap(),
            " mealok ".trim()
        );
    }

    #[test]
    fn bounds_are_enforced() {
        assert!(checked_amount("carbs", Some(0.0), 0.1, 300.0).is_err());
        assert_eq!(
            checked_amount("carbs", Some(15.0), 0.1, 300.0).unwrap(),
            Some(15.0)
        );
    }
}
