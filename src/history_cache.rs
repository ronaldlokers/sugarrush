//! Explicitly opt-in, bounded, owner-only local reading history.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::nightscout::Entry;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Status,
    Clear,
}

fn path(site: &str) -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir);
    let key: String = site
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    base.join("sugarrush/cache").join(format!("{key}.json"))
}

fn state_root() -> PathBuf {
    path("placeholder")
        .parent()
        .and_then(|cache| cache.parent())
        .map(ToOwned::to_owned)
        .unwrap_or_else(std::env::temp_dir)
}

pub fn merge(site: &str, entries: &[Entry], now_ms: i64, retention_days: u32) -> Result<()> {
    let _lock = CacheLock::acquire()?;
    if state_root().join("cache.disabled").exists() {
        anyhow::bail!("history cache was disabled by another process");
    }
    let kept = merge_entries(read_all(site), entries, now_ms, retention_days);
    let body = serde_json::to_string(&kept)?;
    let target = path(site);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::Config::write_atomic(&target, &body)
}

fn merge_entries(
    existing: Vec<Entry>,
    entries: &[Entry],
    now_ms: i64,
    retention_days: u32,
) -> Vec<Entry> {
    let cutoff = now_ms - i64::from(retention_days.clamp(1, 90)) * 86_400_000;
    let mut by_time = BTreeMap::new();
    for entry in existing.into_iter().chain(entries.iter().cloned()) {
        if (cutoff..=now_ms + 300_000).contains(&entry.date) {
            by_time.insert(entry.date, entry);
        }
    }
    let mut kept: Vec<_> = by_time.into_values().collect();
    kept.reverse();
    kept
}

pub fn load(site: &str, start_ms: i64, end_ms: i64) -> Vec<Entry> {
    read_all(site)
        .into_iter()
        .filter(|entry| (start_ms..=end_ms).contains(&entry.date))
        .collect()
}

fn read_all(site: &str) -> Vec<Entry> {
    let target = path(site);
    let Ok(raw) = std::fs::read_to_string(&target) else {
        return Vec::new();
    };
    match serde_json::from_str(&raw) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("sugarrush: ignoring corrupt history cache for {site}: {error}");
            Vec::new()
        }
    }
}

pub fn clear_all() -> Result<()> {
    let _lock = CacheLock::acquire()?;
    std::fs::create_dir_all(state_root())?;
    // Publish the opt-out boundary before deleting. Even if deletion then
    // fails, a stale process cannot recreate or add more health data.
    crate::config::Config::write_atomic(&state_root().join("cache.disabled"), "disabled\n")?;
    let Some(parent) = path("placeholder").parent().map(ToOwned::to_owned) else {
        return Ok(());
    };
    if parent.exists() {
        std::fs::remove_dir_all(&parent)
            .with_context(|| format!("failed to remove {}", parent.display()))?;
    }
    Ok(())
}

pub fn enable() -> Result<()> {
    let _lock = CacheLock::acquire()?;
    let marker = state_root().join("cache.disabled");
    match std::fs::remove_file(marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn describe(site: &str) -> Result<(usize, Option<i64>, Option<i64>, u64)> {
    let target = path(site);
    let entries = read_all(site);
    let bytes = std::fs::metadata(target).map(|m| m.len()).unwrap_or(0);
    Ok((
        entries.len(),
        entries.iter().map(|entry| entry.date).min(),
        entries.iter().map(|entry| entry.date).max(),
        bytes,
    ))
}

pub fn clear_site(site: &str) -> Result<()> {
    let _lock = CacheLock::acquire()?;
    match std::fs::remove_file(path(site)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn purge_all() -> Result<()> {
    let _lock = CacheLock::acquire()?;
    let cache = path("placeholder").parent().map(ToOwned::to_owned);
    if let Some(cache) = cache.filter(|path| path.exists()) {
        std::fs::remove_dir_all(cache)?;
    }
    Ok(())
}

struct CacheLock(PathBuf);

impl CacheLock {
    fn acquire() -> Result<Self> {
        let root = state_root();
        std::fs::create_dir_all(&root)?;
        let path = root.join("cache.lock");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        anyhow::bail!("timed out waiting to update private history cache");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_names_cannot_escape_the_cache_directory() {
        let target = path("../../alice");
        assert_eq!(
            target.extension().and_then(|value| value.to_str()),
            Some("json")
        );
        assert!(!target.file_name().unwrap().to_string_lossy().contains('/'));
    }

    #[test]
    fn retention_and_timestamp_dedup_bound_the_cache() {
        let now = 1_700_000_000_000;
        let entry = |date, sgv| Entry {
            date,
            sgv,
            direction: None,
        };
        let kept = merge_entries(
            vec![
                entry(now - 2 * 86_400_000, 90.0),
                entry(now - 60_000, 100.0),
            ],
            &[entry(now - 60_000, 105.0), entry(now, 110.0)],
            now,
            1,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].date, now);
        assert_eq!(kept[1].sgv, 105.0, "new data wins duplicate timestamps");
    }
}
