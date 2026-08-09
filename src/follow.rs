//! Watching more than one person at once.
//!
//! The dashboard follows one site at a time, which is right for watching your
//! own glucose and wrong for a parent watching a child, or anyone responsible
//! for someone else's readings. This fetches every configured site and reduces
//! each to the handful of facts a caregiver actually scans for: the value, how
//! old it is, and how worried to be.
//!
//! Two rules shape everything here:
//!
//! - **Worst first.** A list sorted by config order buries the person who needs
//!   attention behind the two who are fine.
//! - **A site that fails to load is not a site that's fine.** No reading is
//!   shown as its own state, never as a blank row that reads like "in range".

use crate::alert::{self, Alert};
use crate::config::{Alerts, Site};
use crate::nightscout::{Client, Entry};
use crate::units::Units;

/// One followed site, reduced to what a caregiver scans for.
#[derive(Debug, Clone)]
pub struct SiteStatus {
    pub name: String,
    /// The latest reading, or `None` when the site couldn't be read.
    pub latest: Option<Entry>,
    /// Change from the previous reading, mg/dL.
    pub delta: Option<f64>,
    pub alert: Alert,
    /// Why there's no reading, when there isn't one.
    pub error: Option<String>,
}

impl SiteStatus {
    /// How stale the reading is, in minutes. `None` without a reading.
    pub fn age_min(&self, now_ms: i64) -> Option<i64> {
        self.latest
            .as_ref()
            .map(|e| ((now_ms - e.date) / 60_000).max(0))
    }

    /// The value in display units, or a dash.
    pub fn value(&self, units: Units) -> String {
        self.latest
            .as_ref()
            .map(|e| units.format(e.sgv))
            .unwrap_or_else(|| "—".into())
    }

    pub fn arrow(&self) -> &'static str {
        self.latest.as_ref().map_or("", |e| e.arrow())
    }

    pub fn delta_text(&self, units: Units) -> String {
        self.delta
            .map(|d| units.format_delta(d))
            .unwrap_or_else(|| "--".into())
    }

    /// Sort key: the more attention this site needs, the lower the number.
    /// Unreadable sites rank with the urgent ones — silence from a site you're
    /// responsible for is exactly as informative as a bad reading.
    fn severity(&self) -> u8 {
        self.alert.severity()
    }
}

/// Fetch the latest reading for every site, concurrently.
///
/// One slow or dead site must not delay the others — a caregiver's list is
/// only useful if it arrives — so these run together rather than in sequence.
pub async fn poll(sites: &[(Site, Alerts)], now_ms: i64) -> Vec<SiteStatus> {
    let mut tasks = tokio::task::JoinSet::new();
    for (site, alerts) in sites {
        let (site, alerts) = (site.clone(), alerts.clone());
        tasks.spawn(async move { poll_one(&site, &alerts, now_ms).await });
    }
    let mut out = Vec::with_capacity(sites.len());
    while let Some(res) = tasks.join_next().await {
        // A panicking fetch shouldn't take the whole list with it; the site
        // simply doesn't appear, and the ones that worked still render.
        if let Ok(status) = res {
            out.push(status);
        }
    }
    sort_worst_first(&mut out);
    out
}

async fn poll_one(site: &Site, alerts: &Alerts, now_ms: i64) -> SiteStatus {
    let name = site.name.clone();
    let client = match Client::for_site(site) {
        Ok(c) => c,
        Err(e) => return unreadable(name, e.to_string()),
    };
    // An hour is enough for the value, the delta, and the staleness check.
    match client.entries_range(now_ms - 3_600_000, now_ms, 24).await {
        Ok(entries) => {
            let latest = entries.first().cloned();
            let delta = match (entries.first(), entries.get(1)) {
                (Some(a), Some(b)) => Some(a.sgv - b.sgv),
                _ => None,
            };
            let alert = match &latest {
                Some(e) => alert::evaluate(e.sgv, now_ms - e.date, alerts),
                // Reachable but empty is a sensor gap, not a healthy silence.
                None => Alert::Stale,
            };
            SiteStatus {
                name,
                latest,
                delta,
                alert,
                error: None,
            }
        }
        Err(e) => unreadable(name, e.to_string()),
    }
}

fn unreadable(name: String, error: String) -> SiteStatus {
    SiteStatus {
        name,
        latest: None,
        delta: None,
        alert: Alert::Stale,
        error: Some(error),
    }
}

/// Worst first, then by name so the order is stable between refreshes — a list
/// that reshuffles under the cursor is a list you can't read.
fn sort_worst_first(sites: &mut [SiteStatus]) {
    sites.sort_by(|a, b| a.severity().cmp(&b.severity()).then(a.name.cmp(&b.name)));
}

/// The state to alarm on across every followed site: the worst one.
pub fn worst(sites: &[SiteStatus]) -> Option<&SiteStatus> {
    sites.iter().min_by_key(|s| s.severity())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sgv: f64) -> Entry {
        Entry {
            sgv,
            date: 0,
            direction: Some("Flat".into()),
        }
    }

    fn status(name: &str, alert: Alert) -> SiteStatus {
        SiteStatus {
            name: name.into(),
            latest: Some(entry(100.0)),
            delta: Some(2.0),
            alert,
            error: None,
        }
    }

    #[test]
    fn the_person_who_needs_attention_is_first() {
        let mut sites = vec![
            status("alice", Alert::InRange),
            status("bob", Alert::High),
            status("carol", Alert::UrgentLow),
        ];
        sort_worst_first(&mut sites);
        let order: Vec<&str> = sites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, ["carol", "bob", "alice"]);
        assert_eq!(worst(&sites).unwrap().name, "carol");
    }

    #[test]
    fn a_site_that_cannot_be_read_ranks_with_the_urgent_ones() {
        let mut sites = vec![
            status("alice", Alert::InRange),
            status("bob", Alert::Low),
            unreadable("carol".into(), "connection refused".into()),
        ];
        sort_worst_first(&mut sites);
        // Silence from someone you're responsible for outranks a mild low.
        assert_eq!(sites[0].name, "carol");
        assert_eq!(sites[0].alert, Alert::Stale);
        assert!(sites[0].error.is_some());
        // And it renders as a dash, not as a blank that reads like "fine".
        assert_eq!(sites[0].value(Units::Mgdl), "—");
        assert_eq!(sites[0].age_min(1_000).unwrap_or(-1), -1);
    }

    #[test]
    fn equal_severity_keeps_a_stable_order() {
        let mut sites = vec![
            status("carol", Alert::InRange),
            status("alice", Alert::InRange),
            status("bob", Alert::InRange),
        ];
        sort_worst_first(&mut sites);
        let order: Vec<&str> = sites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, ["alice", "bob", "carol"]);
    }

    #[test]
    fn readings_render_in_display_units() {
        let s = status("alice", Alert::InRange);
        assert_eq!(s.value(Units::Mgdl), "100");
        assert_eq!(s.value(Units::Mmol), "5.6");
        assert_eq!(s.delta_text(Units::Mgdl), "+2");
        assert_eq!(s.arrow(), "→");
        assert_eq!(s.age_min(600_000), Some(10));
    }

    #[test]
    fn nothing_to_follow_has_no_worst() {
        assert!(worst(&[]).is_none());
    }
}
