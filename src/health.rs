//! Machine-readable alarm health for external monitoring.

use serde::Serialize;

use crate::config::Config;
use crate::{alertlog, follow, now_ms, watch};

#[derive(Debug, Serialize)]
pub struct Report {
    pub generated_at_ms: i64,
    pub watcher_alive: bool,
    pub healthy: bool,
    pub sites: Vec<SiteHealth>,
}

#[derive(Debug, Serialize)]
pub struct SiteHealth {
    pub site: String,
    pub endpoint_reachable: bool,
    pub data_fresh: bool,
    pub reading_age_minutes: Option<i64>,
    pub alarm_state: String,
    pub snoozed_until_ms: Option<i64>,
    pub last_delivery: Option<DeliveryHealth>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeliveryHealth {
    pub attempted_at_ms: i64,
    pub channel: String,
    pub outcome: String,
}

pub async fn inspect(cfg: &Config) -> anyhow::Result<Report> {
    let now = now_ms();
    let sites = cfg.resolve_sites()?;
    let profiles: Vec<_> = sites
        .iter()
        .enumerate()
        .map(|(index, site)| {
            (
                site.clone(),
                site.resolve_alerts(&cfg.alerts, cfg.units).0,
                index,
            )
        })
        .collect();
    let poll_profiles: Vec<_> = profiles
        .iter()
        .map(|(site, alerts, _)| (site.clone(), alerts.clone()))
        .collect();
    let statuses = follow::poll(&poll_profiles, now).await;
    let snoozes = watch::snoozes();
    let mut health = Vec::with_capacity(profiles.len());

    // Restore configured order: health output is an API and must not reshuffle
    // whenever severity changes.
    for (site, alerts, _) in profiles {
        let status = statuses.iter().find(|status| status.name == site.name);
        let age = status.and_then(|status| status.age_min(now));
        let reachable = status.is_some_and(|status| status.error.is_none());
        let fresh = reachable && age.is_some_and(|minutes| minutes <= alerts.stale_minutes);
        let delivery = alertlog::latest_delivery(&site.name).and_then(|record| {
            Some(DeliveryHealth {
                attempted_at_ms: record.ts,
                channel: record.channel?,
                outcome: record.outcome?,
            })
        });
        health.push(SiteHealth {
            site: site.name.clone(),
            endpoint_reachable: reachable,
            data_fresh: fresh,
            reading_age_minutes: age,
            alarm_state: status
                .map(|status| status.alert.class().to_string())
                .unwrap_or_else(|| "stale".into()),
            snoozed_until_ms: snoozes.get(&site.name).copied().flatten(),
            last_delivery: delivery,
            error: status.and_then(|status| status.error.clone()),
        });
    }
    let watcher_alive = watch::is_alive(watch::Role::Watch, now);
    let healthy = watcher_alive && health.iter().all(|site| site.data_fresh);
    Ok(Report {
        generated_at_ms: now,
        watcher_alive,
        healthy,
        sites: health,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_json_distinguishes_process_data_and_delivery() {
        let report = Report {
            generated_at_ms: 1,
            watcher_alive: true,
            healthy: false,
            sites: vec![SiteHealth {
                site: "alice".into(),
                endpoint_reachable: true,
                data_fresh: false,
                reading_age_minutes: Some(20),
                alarm_state: "stale".into(),
                snoozed_until_ms: None,
                last_delivery: Some(DeliveryHealth {
                    attempted_at_ms: 1,
                    channel: "webhook".into(),
                    outcome: "accepted".into(),
                }),
                error: None,
            }],
        };
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["watcher_alive"], true);
        assert_eq!(json["sites"][0]["data_fresh"], false);
        assert_eq!(json["sites"][0]["last_delivery"]["outcome"], "accepted");
        assert!(json.get("safe").is_none());
    }
}
