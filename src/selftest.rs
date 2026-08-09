//! The alarm self-test.
//!
//! `docs/alarm-contract.md` lists eight independent ways a night can pass
//! without a sound. Every one of them was silent — nothing in the app could
//! tell you which, or that any of them applied. "Audible alarm: on" is a claim
//! about a config field, not about whether this machine can make a noise.
//!
//! This walks the whole chain and says what it found, loudly enough to be
//! useful at 11pm the night before you rely on it.

use anyhow::Result;
use chrono::{Local, TimeZone, Timelike};

use crate::config::Config;
use crate::nightscout::Client;
use crate::{now_ms, sound, watch};

/// One line of the report.
enum Check {
    /// Working, with what was observed.
    Ok(String, String),
    /// Not working, with what to do about it.
    Bad(String, String),
    /// Deliberately off, or not applicable. Not a failure, but worth saying —
    /// a channel someone thinks is armed and isn't is the whole problem.
    Off(String, String),
}

impl Check {
    fn render(&self) -> String {
        let (mark, label, detail) = match self {
            Check::Ok(l, d) => ("✓", l, d),
            Check::Bad(l, d) => ("✗", l, d),
            Check::Off(l, d) => ("·", l, d),
        };
        format!("{mark} {label:<22} {detail}")
    }
    fn failed(&self) -> bool {
        matches!(self, Check::Bad(..))
    }
}

/// Run every check and print the report. Exits non-zero if any channel that is
/// supposed to work doesn't, so it can be used in a cron or a health check.
pub async fn run(quiet: bool) -> Result<()> {
    let cfg = Config::load()?;
    let (_, mut warnings) = cfg.alerts.resolve_checked(cfg.units);
    let sites = cfg.resolve_sites()?;
    for site in &sites {
        let (_, site_warnings) = site.resolve_alerts(&cfg.alerts, cfg.units);
        warnings.extend(
            site_warnings
                .into_iter()
                .map(|w| format!("{}: {w}", site.name)),
        );
    }
    let profiles: Vec<_> = sites
        .iter()
        .map(|site| {
            (
                site.name.clone(),
                site.resolve_alerts(&cfg.alerts, cfg.units).0,
            )
        })
        .collect();
    let now = now_ms();
    let mut checks: Vec<Check> = Vec::new();

    // 1. Config.
    if warnings.is_empty() {
        checks.push(Check::Ok(
            "config".into(),
            format!("{} site(s), thresholds valid", sites.len()),
        ));
    } else {
        checks.push(Check::Bad("config".into(), warnings.join("; ")));
    }

    // 2. Can we see the data at all? An alarm can only fire on readings that
    //    arrive.
    for site in &sites {
        let site_alerts = site.resolve_alerts(&cfg.alerts, cfg.units).0;
        let label = if sites.len() > 1 {
            format!("site · {}", site.name)
        } else {
            "site".into()
        };
        match Client::for_site(site) {
            Ok(client) => match client.entries_range(now - 3_600_000, now, 12).await {
                Ok(entries) => match entries.first() {
                    Some(e) => {
                        let age = (now - e.date) / 60_000;
                        let detail = format!(
                            "reachable · newest reading {age}m old ({} {})",
                            cfg.units.format(e.sgv),
                            cfg.units.label()
                        );
                        // Older than the staleness threshold means the alarm
                        // would already be reporting a gap.
                        if age > site_alerts.stale_minutes {
                            checks.push(Check::Bad(
                                label,
                                format!("{detail} — already past stale_minutes"),
                            ));
                        } else {
                            checks.push(Check::Ok(label, detail));
                        }
                    }
                    None => checks.push(Check::Bad(
                        label,
                        "reachable, but no readings in the last hour".into(),
                    )),
                },
                Err(e) => checks.push(Check::Bad(label, e.to_string())),
            },
            Err(e) => checks.push(Check::Bad(label, e.to_string())),
        }
    }

    // 3. The audible alarm. This is the one that has to actually make a noise:
    //    a working config field and a working audio path are different claims.
    if !profiles.iter().any(|(_, alerts)| alerts.sound) {
        checks.push(Check::Off(
            "audible alarm".into(),
            "off (sound = false)".into(),
        ));
    } else if quiet {
        checks.push(Check::Off(
            "audible alarm".into(),
            "on — not played (--quiet)".into(),
        ));
    } else {
        match sound::sound_check(sound::Tone::Low) {
            sound::Played::Player(p) => {
                checks.push(Check::Ok("audible alarm".into(), format!("played via {p}")))
            }
            sound::Played::Bell => checks.push(Check::Bad(
                "audible alarm".into(),
                "no audio player worked — fell back to the terminal bell, \
                 which many terminals render silently"
                    .into(),
            )),
            sound::Played::Nothing => checks.push(Check::Bad(
                "audible alarm".into(),
                "couldn't even write the sound file".into(),
            )),
        }
    }

    // 4. Quiet hours — a scheduled, invisible silence.
    for (site, alerts) in &profiles {
        let label = if profiles.len() > 1 {
            format!("quiet hours · {site}")
        } else {
            "quiet hours".into()
        };
        match (alerts.quiet_start, alerts.quiet_end) {
            (Some(start), Some(end)) => {
                let min_of_day = Local
                    .timestamp_millis_opt(now)
                    .single()
                    .map(|d| d.hour() as i32 * 60 + d.minute() as i32)
                    .unwrap_or(0);
                let hhmm = |m: i32| format!("{:02}:{:02}", m / 60, m % 60);
                let window = format!("{}–{}", hhmm(start), hhmm(end));
                if alerts.in_quiet_hours(min_of_day) {
                    let detail = if alerts.quiet_urgent_low {
                        format!("active now ({window}) — only urgent lows will sound")
                    } else {
                        format!("active now ({window}) — nothing will sound")
                    };
                    checks.push(Check::Off(label, detail));
                } else {
                    checks.push(Check::Ok(label, format!("set ({window}), not active now")));
                }
            }
            _ => checks.push(Check::Ok(label, "not set".into())),
        }
    }

    // 5. An active snooze someone forgot about.
    match watch::snoozed_until() {
        Some(t) if t > now => {
            let clock = Local
                .timestamp_millis_opt(t)
                .single()
                .map(|d| d.format("%H:%M").to_string())
                .unwrap_or_default();
            checks.push(Check::Off(
                "snooze".into(),
                format!("active until {clock} — run `sugarrush snooze off`"),
            ));
        }
        _ => checks.push(Check::Ok("snooze".into(), "none active".into())),
    }

    // 6. Desktop notifications: the D-Bus call can fail with no daemon running,
    //    and the result used to be discarded.
    if !profiles.iter().any(|(_, alerts)| alerts.desktop) {
        checks.push(Check::Off(
            "desktop notification".into(),
            "off (desktop = false)".into(),
        ));
    } else if quiet {
        checks.push(Check::Off(
            "desktop notification".into(),
            "on — not sent (--quiet)".into(),
        ));
    } else if crate::notify_text("sugarrush: alarm self-test") {
        checks.push(Check::Ok("desktop notification".into(), "delivered".into()));
    } else {
        checks.push(Check::Bad(
            "desktop notification".into(),
            "the notification daemon rejected it or isn't running".into(),
        ));
    }

    // 7. The push webhook — the channel that reaches a phone, and the only one
    //    escalation has.
    for (site, alerts) in &profiles {
        let label = if profiles.len() > 1 {
            format!("push webhook · {site}")
        } else {
            "push webhook".into()
        };
        match (&alerts.push_url, alerts.push_enabled) {
            (Some(url), true) if !quiet => {
                if crate::push(url, "sugarrush: alarm self-test").await {
                    checks.push(Check::Ok(label, "delivered".into()));
                } else {
                    checks.push(Check::Bad(label, "the POST failed — check push_url".into()));
                }
            }
            (Some(_), true) => {
                checks.push(Check::Off(label, "configured — not sent (--quiet)".into()))
            }
            (Some(_), false) => checks.push(Check::Off(label, "configured but disabled".into())),
            (None, _) => checks.push(Check::Off(label, "not configured".into())),
        }
    }

    // 8. Escalation with nowhere to go — a setting that reads as armed and does
    //    nothing at all.
    for (site, alerts) in &profiles {
        let label = if profiles.len() > 1 {
            format!("escalation · {site}")
        } else {
            "escalation".into()
        };
        if alerts.escalate_minutes > 0 {
            if alerts.push_url.is_some() && alerts.push_enabled {
                checks.push(Check::Ok(
                    label,
                    format!(
                        "after {} min, via the push webhook",
                        alerts.escalate_minutes
                    ),
                ));
            } else {
                checks.push(Check::Bad(
                    label,
                    format!(
                    "set to {} min but the push webhook is its only channel — it will do nothing",
                    alerts.escalate_minutes
                ),
                ));
            }
        } else {
            checks.push(Check::Off(label, "off".into()));
        }
    }

    // 9. Is anything actually watching? Everything above is moot if nothing is
    //    running while you sleep.
    if watch::is_alive(watch::Role::Watch, now) {
        checks.push(Check::Ok("watcher".into(), "running".into()));
    } else {
        checks.push(Check::Bad(
            "watcher".into(),
            "not running — start it with `systemctl --user start sugarrush-watch` \
             (see `sugarrush --install-unit`)"
                .into(),
        ));
    }

    println!("sugarrush alarm self-test\n");
    for c in &checks {
        println!("{}", c.render());
    }

    let failures = checks.iter().filter(|c| c.failed()).count();
    println!();
    if failures == 0 {
        println!("Everything that is switched on is working.");
        println!("Lines marked · are deliberately off — check they're what you meant.");
        Ok(())
    } else {
        println!("{failures} problem(s) above would keep an alarm from reaching you.");
        std::process::exit(1);
    }
}

/// The classification the self-test would report for a given state, so the
/// tests can exercise the decision without a network or a sound card.
#[cfg(test)]
pub fn escalation_is_inert(alerts: &crate::config::Alerts) -> bool {
    alerts.escalate_minutes > 0 && !(alerts.push_url.is_some() && alerts.push_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalation_without_a_push_url_is_reported_as_inert() {
        let mut a = crate::config::Alerts {
            escalate_minutes: 10,
            push_url: None,
            push_enabled: false,
            ..crate::config::Alerts::default()
        };

        // The exact combination the review flagged: "Escalate after 10 min"
        // sitting next to "Push alerts: not configured", reading as two
        // independent switches when one is the other's only channel.
        assert!(escalation_is_inert(&a));

        a.push_url = Some("http://example.invalid/hook".into());
        assert!(
            escalation_is_inert(&a),
            "a URL that is switched off is still no channel"
        );

        a.push_enabled = true;
        assert!(!escalation_is_inert(&a));

        a.escalate_minutes = 0;
        a.push_url = None;
        a.push_enabled = false;
        assert!(
            !escalation_is_inert(&a),
            "escalation that is off cannot be inert"
        );
    }

    #[test]
    fn a_check_renders_its_state() {
        assert!(Check::Ok("a".into(), "b".into()).render().starts_with('✓'));
        assert!(Check::Bad("a".into(), "b".into()).render().starts_with('✗'));
        assert!(Check::Off("a".into(), "b".into()).render().starts_with('·'));
        assert!(Check::Bad("a".into(), "b".into()).failed());
        assert!(!Check::Off("a".into(), "b".into()).failed());
    }
}
