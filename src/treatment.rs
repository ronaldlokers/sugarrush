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

#[derive(Debug, Clone, Copy)]
pub enum Format {
    Text,
    Json,
    Csv,
}

impl Format {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }
}

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
    site_id: String,
    operation_id: String,
    intended_at: i64,
    outcome: String,
    carbs: Option<f64>,
    insulin: Option<f64>,
    note_present: bool,
}

pub fn render(days: i64, site: Option<&str>, format: Format) -> Result<String> {
    let days = days.clamp(1, RETAIN_DAYS);
    let cutoff = Utc::now().timestamp_millis() - days * 86_400_000;
    let (records, corrupt) = read_audit()?;
    let records: Vec<_> = records
        .into_iter()
        .filter(|record| record.ts >= cutoff && site.is_none_or(|site| record.site == site))
        .collect();
    if corrupt > 0 {
        eprintln!("sugarrush treatments: {corrupt} corrupt audit line(s) were skipped");
    }
    if let Some(site) = site {
        let cfg = Config::load()?;
        let names: Vec<_> = cfg
            .resolve_sites()?
            .into_iter()
            .map(|site| site.name)
            .collect();
        if !names.iter().any(|name| name == site)
            && !records.iter().any(|record| record.site == site)
        {
            bail!("unknown site '{site}'; available: {}", names.join(", "));
        }
    }
    match format {
        Format::Json => Ok(serde_json::to_string_pretty(&records)? + "\n"),
        Format::Csv => {
            let mut out =
                "attempted_at_ms,site,intended_at_ms,operation_id,outcome,carbs_g,insulin_u,note_present\n"
                    .to_string();
            for record in &records {
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{}\n",
                    record.ts,
                    csv_field(&record.site),
                    record.intended_at,
                    record.operation_id,
                    record.outcome,
                    record.carbs.map(|v| v.to_string()).unwrap_or_default(),
                    record.insulin.map(|v| v.to_string()).unwrap_or_default(),
                    record.note_present
                ));
            }
            Ok(out)
        }
        Format::Text => {
            let mut out = String::from(
                "Local submission audit — not a complete or clinically verified Nightscout record.\n",
            );
            for record in &records {
                let at = DateTime::from_timestamp_millis(record.intended_at)
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| record.intended_at.to_string());
                out.push_str(&format!(
                    "{at}  {:<12} {:<9} {}  carbs={} insulin={} note={}\n",
                    record.site,
                    record.outcome,
                    record.operation_id,
                    record
                        .carbs
                        .map(|v| format!("{v}g"))
                        .unwrap_or_else(|| "—".into()),
                    record
                        .insulin
                        .map(|v| format!("{v}U"))
                        .unwrap_or_else(|| "—".into()),
                    if record.note_present { "yes" } else { "no" }
                ));
            }
            if records.is_empty() {
                out.push_str("No matching submission attempts.\n");
            }
            if corrupt > 0 {
                out.push_str(&format!(
                    "WARNING: {corrupt} corrupt audit line(s) were skipped.\n"
                ));
            }
            Ok(out)
        }
    }
}

fn csv_field(value: &str) -> String {
    let guarded = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", guarded.replace('"', "\"\""))
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

    review(
        &request,
        &site.name,
        carbs,
        insulin,
        created,
        cfg.allow_unattended_writes,
    )?;
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
        site_id: site.stable_id(),
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
    allow_unattended: bool,
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
        // The interactive path is guarded by typing the person's name; this one
        // skips that by construction, so the grant has to exist at rest. A flag
        // combination is available to anything that can compose a command line
        // on this machine; a config key is a decision someone made once, in a
        // file they own, and `sugarrush about` reports it.
        if !allow_unattended {
            bail!(
                "unattended writes are not enabled for this install.\n\
                 Add `allow_unattended_writes = true` to {} to permit \
                 --non-interactive treatment writes.",
                Config::path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "config.toml".into())
            );
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
    Ok(read_audit()?
        .0
        .into_iter()
        .rfind(|record| record.operation_id == operation_id))
}

fn read_audit() -> Result<(Vec<Audit>, usize)> {
    let body = match std::fs::read_to_string(audit_path()) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), 0));
        }
        Err(error) => return Err(error.into()),
    };
    let mut records = Vec::new();
    let mut corrupt = 0;
    for line in body.lines() {
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(_) => corrupt += 1,
        }
    }
    Ok((records, corrupt))
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

const STALE_LOCK: Duration = Duration::from_secs(30);

/// A lock file that heals itself.
///
/// `create_new` plus `Drop` is only a lock while the process lives to run
/// `Drop`. A `SIGKILL`, an OOM kill or a power cut leaves the file behind, and
/// with no staleness rule that is permanent: every later run waits the timeout
/// and fails, so one hard kill disabled treatment writes until someone found and
/// deleted a file nothing had told them about.
///
/// A lock older than `STALE` is treated as abandoned and removed. The window is
/// far longer than the milliseconds a real holder needs, so a live holder is
/// never stolen from; and stealing is safe here anyway, because the writes it
/// guards are atomic replacements rather than read-modify-write sequences that
/// could interleave.
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
                    if steal_if_stale(path) {
                        continue;
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "timed out waiting to update the treatment audit. If no other \
                             sugarrush is running, remove {} and try again",
                            path.display()
                        );
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// Remove a lock whose holder is plainly gone. Returns whether it did.
fn steal_if_stale(path: &Path) -> bool {
    let Ok(age) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    if age.elapsed().is_ok_and(|elapsed| elapsed > STALE_LOCK) {
        return std::fs::remove_file(path).is_ok();
    }
    false
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unattended(confirm: bool, id: Option<&str>) -> Request {
        Request {
            site: "Alex".into(),
            carbs: Some(15.0),
            insulin: None,
            note: None,
            at: None,
            confirm,
            non_interactive: true,
            operation_id: id.map(str::to_string),
        }
    }

    /// The interactive path is guarded by typing the person's name. The
    /// unattended path skips that by construction, so the grant has to exist at
    /// rest — a flag combination is available to anything that can compose a
    /// command line on this machine.
    #[test]
    fn unattended_writes_need_a_grant_that_lives_in_the_config() {
        let at = Utc::now();
        let request = unattended(true, Some("11111111-2222-3333-4444-555555555555"));

        let denied = review(&request, "Alex", Some(15.0), None, at, false)
            .expect_err("no grant means no unattended write");
        let denied = denied.to_string();
        assert!(denied.contains("allow_unattended_writes"), "got {denied:?}");
        assert!(denied.contains("not enabled"), "got {denied:?}");

        review(&request, "Alex", Some(15.0), None, at, true)
            .expect("with the grant, an unattended write proceeds");
    }

    /// The grant does not replace the other two requirements — it is added to
    /// them, so a script still has to be explicit and still has to carry a
    /// retry identity.
    #[test]
    fn a_grant_does_not_excuse_confirm_or_an_operation_id() {
        let at = Utc::now();
        for request in [
            unattended(false, Some("11111111-2222-3333-4444-555555555555")),
            unattended(true, None),
        ] {
            assert!(
                review(&request, "Alex", Some(15.0), None, at, true).is_err(),
                "the grant must not stand in for --confirm or --operation-id"
            );
        }
    }

    /// `create_new` plus `Drop` is only a lock while the process lives to run
    /// `Drop`. A SIGKILL or a power cut used to leave the file behind
    /// permanently, so one hard kill disabled treatment writes until someone
    /// found and deleted a file nothing had told them about.
    #[test]
    fn an_abandoned_lock_is_reclaimed() {
        let dir = std::env::temp_dir().join(format!("sugarrush-locktest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.lock");

        // A lock a live holder owns is never stolen.
        std::fs::write(&path, "").unwrap();
        assert!(!steal_if_stale(&path), "a fresh lock must be respected");
        assert!(path.exists());

        // One older than the staleness window is abandoned, and reclaimed.
        let old = std::time::SystemTime::now() - STALE_LOCK - Duration::from_secs(5);
        set_mtime(&path, old);
        assert!(steal_if_stale(&path), "an abandoned lock must be reclaimed");
        assert!(!path.exists());

        // And acquiring now succeeds rather than timing out.
        let lock = FileLock::acquire(&path).expect("should acquire after reclaiming");
        drop(lock);
        assert!(!path.exists(), "Drop releases the lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The error a caller sees when a lock is genuinely held has to say what to
    /// do about it; the old one named neither the file nor the remedy.
    #[test]
    fn a_held_lock_reports_the_path_to_remove() {
        let dir =
            std::env::temp_dir().join(format!("sugarrush-locktest-held-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("held.lock");
        std::fs::write(&path, "").unwrap();

        let error = match FileLock::acquire(&path) {
            Ok(_) => panic!("a held lock must not be acquired"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(&path.display().to_string()), "got {error:?}");
        assert!(error.contains("remove"), "got {error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn set_mtime(path: &Path, when: std::time::SystemTime) {
        // No filetime crate here; go through the file's own handle.
        // `File::set_modified` is cross-platform, and the test that calls this
        // is not gated — a `#[cfg(unix)]` here failed the Windows build with
        // "cannot find function `set_mtime`" rather than skipping anything.
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn note_removes_terminal_and_bidi_controls() {
        assert_eq!(sanitize_note(" meal\n\u{202e}ok ").unwrap(), "mealok");
    }
    #[test]
    fn bounds_are_enforced() {
        assert!(checked_amount("carbs", Some(0.0), 0.1, 300.0).is_err());
    }
    #[test]
    fn audit_csv_cannot_execute_a_person_name() {
        assert_eq!(csv_field("=cmd()"), "\"'=cmd()\"");
        assert_eq!(csv_field("Alice \"A\""), "\"Alice \"\"A\"\"\"");
    }
}
