//! Explicit, durable, audited Nightscout CarePortal writes.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::nightscout::{Client, TreatmentWriteError};

const RETAIN_DAYS: i64 = 90;

#[derive(Debug, Clone)]
pub struct Request {
    pub site: String,
    pub carbs: Option<f64>,
    pub insulin: Option<f64>,
    pub note: Option<String>,
    pub at: Option<String>,
    pub confirm: bool,
    pub non_interactive: bool,
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Audit {
    schema: u8,
    ts: i64,
    site: String,
    operation_id: String,
    intended_at: i64,
    outcome: String,
    carbs: Option<f64>,
    insulin: Option<f64>,
    note_present: bool,
}

pub async fn run(request: Request) -> Result<()> {
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
    let now = Utc::now().timestamp_millis();
    if created.timestamp_millis() > now + 5 * 60_000 {
        bail!("treatment time cannot be in the future (five minutes of clock skew is allowed)");
    }
    if created.timestamp_millis() < now - 7 * 86_400_000 {
        bail!("treatment time must be within the last seven days");
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
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    if site.is_insecure() {
        bail!("refusing a health-data write over unencrypted HTTP");
    }

    review(&request, &site.name, carbs, insulin, created)?;
    let operation_id = match request.operation_id.as_deref() {
        Some(value) => uuid::Uuid::parse_str(value)
            .context("--operation-id must be a UUID")?
            .to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    if let Some(previous) = latest(&operation_id)? {
        match previous.outcome.as_str() {
            "accepted" => {
                println!("Treatment operation {operation_id} was already accepted; nothing sent.");
                return Ok(());
            }
            "rejected" => bail!(
                "operation {operation_id} was definitively rejected; use a new operation ID only after correcting it"
            ),
            _ => println!(
                "Retrying unresolved operation {operation_id} with the same Nightscout identifier."
            ),
        }
    }

    let base = Audit {
        schema: 1,
        ts: now,
        site: site.name.clone(),
        operation_id: operation_id.clone(),
        intended_at: created.timestamp_millis(),
        outcome: "intent".into(),
        carbs,
        insulin,
        note_present: note.is_some(),
    };
    append(&base).context("could not persist treatment intent; nothing was sent")?;

    let event_type = match (carbs, insulin) {
        (Some(_), Some(_)) => "Meal Bolus",
        (Some(_), None) => "Carb Correction",
        (None, Some(_)) => "Correction Bolus",
        (None, None) => unreachable!(),
    };
    let mut body = serde_json::json!({
        "_id": operation_id,
        "identifier": operation_id,
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

    let remote = Client::create_treatment(site, &body).await;
    let outcome = match &remote {
        Ok(()) => "accepted",
        Err(TreatmentWriteError::Definitive(_)) => "rejected",
        Err(TreatmentWriteError::Unknown(_)) => "unknown",
    };
    let mut final_record = base;
    final_record.ts = Utc::now().timestamp_millis();
    final_record.outcome = outcome.into();
    let audit_result = append(&final_record);

    match remote {
        Ok(()) => {
            println!(
                "Nightscout accepted treatment for {} at {} (operation {operation_id}).",
                site.name,
                created.to_rfc3339()
            );
            if let Err(error) = audit_result {
                eprintln!("WARNING: remote write succeeded, but the local audit failed: {error}");
                eprintln!("Do not retry: Nightscout already accepted operation {operation_id}.");
            }
            println!("Accepted does not mean clinically verified; confirm it in Nightscout.");
            Ok(())
        }
        Err(error @ TreatmentWriteError::Unknown(_)) => {
            if let Err(audit_error) = audit_result {
                eprintln!("WARNING: local audit also failed: {audit_error}");
            }
            bail!(
                "{error}. Do not retry blindly; check Nightscout for operation {operation_id}. If absent, retry with --operation-id {operation_id}"
            )
        }
        Err(error) => {
            if let Err(audit_error) = audit_result {
                eprintln!("WARNING: local audit also failed: {audit_error}");
            }
            Err(error.into())
        }
    }
}

fn review(
    request: &Request,
    site: &str,
    carbs: Option<f64>,
    insulin: Option<f64>,
    at: DateTime<Utc>,
) -> Result<()> {
    println!("Review treatment write:");
    println!("  person:  {site}");
    println!("  time:    {}", at.to_rfc3339());
    println!(
        "  carbs:   {}",
        carbs
            .map(|v| format!("{v} g"))
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "  insulin: {}",
        insulin
            .map(|v| format!("{v} U"))
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "  note:    {}",
        if request.note.is_some() {
            "present (hidden)"
        } else {
            "—"
        }
    );
    if request.non_interactive {
        if !request.confirm || request.operation_id.is_none() {
            bail!("--non-interactive requires both --confirm and a stable --operation-id UUID");
        }
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!("interactive confirmation needs a terminal; automation must use --non-interactive --confirm --operation-id UUID");
    }
    print!("Type the person name '{site}' to write: ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if answer.trim() != site {
        bail!("confirmation cancelled; nothing was sent");
    }
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

fn latest(operation_id: &str) -> Result<Option<Audit>> {
    let path = audit_path();
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(body
        .lines()
        .filter_map(|line| serde_json::from_str::<Audit>(line).ok())
        .rfind(|record| record.operation_id == operation_id))
}

fn append(entry: &Audit) -> Result<()> {
    let path = audit_path();
    std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
    let _lock = FileLock::acquire(&path.with_extension("lock"))?;
    compact(&path)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    file.sync_data()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn compact(path: &Path) -> Result<()> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let cutoff = Utc::now().timestamp_millis() - RETAIN_DAYS * 86_400_000;
    let kept: Vec<&str> = body
        .lines()
        .filter(|line| serde_json::from_str::<Audit>(line).is_ok_and(|record| record.ts >= cutoff))
        .collect();
    if kept.len() == body.lines().count() {
        return Ok(());
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    crate::config::Config::write_atomic(path, &out)
}

struct FileLock(PathBuf);
impl FileLock {
    fn acquire(path: &Path) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(_) => return Ok(Self(path.to_path_buf())),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        bail!("timed out waiting to update treatment audit");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_removes_terminal_and_bidi_controls() {
        assert_eq!(sanitize_note(" meal\n\u{202e}ok ").unwrap(), "mealok");
    }
    #[test]
    fn bounds_are_enforced() {
        assert!(checked_amount("carbs", Some(0.0), 0.1, 300.0).is_err());
    }
}
