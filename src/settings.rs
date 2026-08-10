//! Settings-screen rows, editing, persistence, and display.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsExit {
    Back,
    Quit,
}

/// Editable rows on the settings screen, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    SiteName,
    SiteUrl,
    SiteToken,
    SiteWriteToken,
    SiteTimezone,
    TestSite,
    AddSite,
    RemoveSite,
    SiteAlerts,
    Units,
    Refresh,
    Desktop,
    NotifyContent,
    Sound,
    /// Not a setting — an action. Runs the alarm self-test.
    TestAlarm,
    Snooze,
    QuietHours,
    QuietStart,
    QuietEnd,
    QuietUrgentLow,
    Escalate,
    PushAlerts,
    PushUrl,
    PredictHorizon,
    UrgentLow,
    Low,
    High,
    UrgentHigh,
    Stale,
    GraphStyle,
    AgpDays,
    CacheEnabled,
    CacheDays,
    MinimapEnabled,
    MinimapSpan,
    ThemeLow,
    ThemeInRange,
    ThemeHigh,
    ThemeUrgent,
    ThemePrediction,
    ThemeGraph,
    Colorblind,
}

impl Field {
    pub const ALL: [Field; 42] = [
        Field::SiteName,
        Field::SiteUrl,
        Field::SiteToken,
        Field::SiteWriteToken,
        Field::SiteTimezone,
        Field::TestSite,
        Field::AddSite,
        Field::RemoveSite,
        Field::SiteAlerts,
        Field::Units,
        Field::Refresh,
        Field::Desktop,
        Field::NotifyContent,
        Field::Sound,
        Field::TestAlarm,
        Field::Snooze,
        Field::QuietHours,
        Field::QuietStart,
        Field::QuietEnd,
        Field::QuietUrgentLow,
        Field::Escalate,
        Field::PushAlerts,
        Field::PushUrl,
        Field::PredictHorizon,
        Field::UrgentLow,
        Field::Low,
        Field::High,
        Field::UrgentHigh,
        Field::Stale,
        Field::GraphStyle,
        Field::AgpDays,
        Field::CacheEnabled,
        Field::CacheDays,
        Field::MinimapEnabled,
        Field::MinimapSpan,
        Field::ThemeLow,
        Field::ThemeInRange,
        Field::ThemeHigh,
        Field::ThemeUrgent,
        Field::ThemePrediction,
        Field::ThemeGraph,
        Field::Colorblind,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::SiteName => "Site name",
            Field::SiteUrl => "Site URL",
            Field::SiteToken => "Read-only token",
            Field::SiteWriteToken => "Treatment write token",
            Field::SiteTimezone => "Person's timezone",
            Field::TestSite => "Test this site",
            Field::AddSite => "Add site",
            Field::RemoveSite => "Remove site",
            Field::SiteAlerts => "Alert settings",
            Field::Units => "Units",
            Field::Refresh => "Refresh interval",
            Field::Desktop => "Desktop notifications",
            Field::NotifyContent => "Notification detail",
            Field::Sound => "Audible alarm",
            Field::TestAlarm => "Test the alarm",
            Field::Snooze => "Snooze",
            Field::QuietHours => "Quiet hours",
            Field::QuietStart => "Quiet start",
            Field::QuietEnd => "Quiet end",
            Field::QuietUrgentLow => "Quiet: urgent-low sounds",
            Field::Escalate => "Escalate after",
            Field::PushAlerts => "Push alerts",
            Field::PushUrl => "Push URL",
            Field::PredictHorizon => "Predict horizon",
            Field::UrgentLow => "Urgent low",
            Field::Low => "Low",
            Field::High => "High",
            Field::UrgentHigh => "Urgent high",
            Field::Stale => "Stale after",
            Field::GraphStyle => "Graph style",
            Field::AgpDays => "AGP days",
            Field::CacheEnabled => "Offline history cache",
            Field::CacheDays => "Cache retention",
            Field::MinimapEnabled => "Minimap",
            Field::MinimapSpan => "Minimap span",
            Field::ThemeLow => "Color: low",
            Field::ThemeInRange => "Color: in range",
            Field::ThemeHigh => "Color: high",
            Field::ThemeUrgent => "Color: urgent",
            Field::ThemePrediction => "Color: forecast",
            Field::ThemeGraph => "Color: graph",
            Field::Colorblind => "Colorblind palette",
        }
    }

    /// The section this field belongs to. Fields are grouped contiguously in
    /// `ALL`, so a header is drawn whenever this changes between rows.
    pub fn group(self) -> &'static str {
        match self {
            Field::SiteName
            | Field::SiteUrl
            | Field::SiteToken
            | Field::SiteWriteToken
            | Field::SiteTimezone
            | Field::TestSite
            | Field::AddSite
            | Field::RemoveSite
            | Field::SiteAlerts => "Site",
            Field::Units | Field::Refresh | Field::CacheEnabled | Field::CacheDays => "General",
            Field::Desktop
            | Field::NotifyContent
            | Field::Sound
            | Field::TestAlarm
            | Field::Snooze
            | Field::QuietHours
            | Field::QuietStart
            | Field::QuietEnd
            | Field::QuietUrgentLow
            | Field::Escalate
            | Field::PushAlerts
            | Field::PushUrl => "Alarm",
            Field::PredictHorizon => "Predictions",
            Field::UrgentLow | Field::Low | Field::High | Field::UrgentHigh | Field::Stale => {
                "Thresholds"
            }
            Field::GraphStyle | Field::AgpDays | Field::MinimapEnabled | Field::MinimapSpan => {
                "Graph"
            }
            Field::ThemeLow
            | Field::ThemeInRange
            | Field::ThemeHigh
            | Field::ThemeUrgent
            | Field::ThemePrediction
            | Field::ThemeGraph
            | Field::Colorblind => "Theme",
        }
    }

    /// For theme rows, the palette index (0..6) into the color roles.
    fn theme_index(self) -> Option<usize> {
        match self {
            Field::ThemeLow => Some(0),
            Field::ThemeInRange => Some(1),
            Field::ThemeHigh => Some(2),
            Field::ThemeUrgent => Some(3),
            Field::ThemePrediction => Some(4),
            Field::ThemeGraph => Some(5),
            _ => None,
        }
    }
}

/// A settings row being edited as free text.
pub struct FieldEdit {
    pub field: Field,
    pub buffer: String,
    /// Render as dots rather than the typed characters.
    pub masked: bool,
}

impl App {
    /// Run the audible half of the alarm test in-app and record what happened.
    ///
    /// The full test is `sugarrush watch --test`; this is the part someone
    /// actually wants at the moment they're looking at the setting — does this
    /// machine make a noise — and the row says so rather than silently doing
    /// nothing.
    pub fn run_alarm_test(&mut self) {
        let result = match crate::sound::sound_check(self.alarm_tone()) {
            crate::sound::Played::Player(p) => format!("sounded via {p}"),
            crate::sound::Played::Bell => "no audio player — terminal bell only".to_string(),
            crate::sound::Played::Nothing => "couldn't write the sound file".to_string(),
        };
        self.status = Some(format!("{result} · full check: sugarrush watch --test"));
        self.alarm_test = Some(result);
    }

    /// Open the text editor for the selected settings row, if it takes text.
    /// Returns true when an editor opened.
    pub fn begin_field_edit(&mut self) -> bool {
        let field = Field::ALL[self.settings_sel.min(Field::ALL.len() - 1)];
        let edit = match field {
            Field::SiteName => FieldEdit {
                field,
                buffer: self.active_site().name.clone(),
                masked: false,
            },
            Field::SiteUrl => FieldEdit {
                field,
                // Pre-fill: a URL is usually being corrected, not replaced.
                buffer: self.active_site().url.clone(),
                masked: false,
            },
            // Start empty and masked — a token is replaced wholesale, and
            // pre-filling it would put the secret back on screen.
            Field::SiteToken | Field::SiteWriteToken => FieldEdit {
                field,
                buffer: String::new(),
                masked: true,
            },
            Field::SiteTimezone => FieldEdit {
                field,
                buffer: self.active_site().timezone.clone().unwrap_or_default(),
                masked: false,
            },
            // Webhook paths commonly contain a private topic or token. Replace
            // wholesale without ever rendering the existing destination.
            Field::PushUrl => FieldEdit {
                field,
                buffer: String::new(),
                masked: true,
            },
            _ => return false,
        };
        self.field_edit = Some(edit);
        true
    }

    pub fn field_edit_push(&mut self, c: char) {
        if let Some(e) = self.field_edit.as_mut() {
            e.buffer.push(c);
        }
    }

    pub fn field_edit_backspace(&mut self) {
        if let Some(e) = self.field_edit.as_mut() {
            e.buffer.pop();
        }
    }

    pub fn cancel_field_edit(&mut self) {
        self.field_edit = None;
    }

    /// Apply the open editor. On a bad URL the editor stays open with the text
    /// intact so it can be fixed rather than retyped.
    pub fn commit_field_edit(&mut self) {
        let Some(edit) = self.field_edit.take() else {
            return;
        };
        let idx = self.site_idx.min(self.sites.len().saturating_sub(1));
        match edit.field {
            Field::SiteName => {
                let name = edit.buffer.trim();
                if name.is_empty() {
                    self.status = Some("site name cannot be empty".to_string());
                    self.field_edit = Some(edit);
                } else if self
                    .sites
                    .iter()
                    .enumerate()
                    .any(|(i, site)| i != idx && site.name == name)
                {
                    self.status = Some(format!("a site named '{name}' already exists"));
                    self.field_edit = Some(edit);
                } else {
                    self.sites[idx].name = name.to_string();
                    self.settings_dirty = true;
                    self.status = Some("site renamed · press w to save".to_string());
                }
            }
            Field::SiteUrl => match crate::config::normalize_site_url(&edit.buffer) {
                Ok(url) => {
                    self.sites[idx].url = url;
                    self.site_validated[idx] = false;
                    self.site_dirty = true;
                    self.settings_dirty = true;
                    self.status = Some(if self.sites[idx].is_insecure() {
                        "site updated · ⚠ unencrypted http, the token is sent in clear".to_string()
                    } else {
                        format!("site set to {}", self.sites[idx].base_url())
                    });
                }
                Err(e) => {
                    self.status = Some(e.to_string());
                    self.field_edit = Some(edit);
                }
            },
            Field::SiteToken => {
                if edit.buffer.trim().is_empty() {
                    self.status = Some("token unchanged".to_string());
                } else {
                    self.sites[idx].token = edit.buffer.trim().to_string();
                    self.site_validated[idx] = false;
                    self.site_dirty = true;
                    self.settings_dirty = true;
                    self.status = Some("token updated · press w to save".to_string());
                }
            }
            Field::SiteWriteToken => {
                let value = edit.buffer.trim();
                if value.is_empty() {
                    self.status = Some("write token unchanged · type 'off' to clear".into());
                } else if value.eq_ignore_ascii_case("off") {
                    self.sites[idx].write_token = None;
                    self.settings_dirty = true;
                    self.status = Some("write access removed · press w to save".into());
                } else {
                    self.sites[idx].write_token = Some(value.to_string());
                    self.settings_dirty = true;
                    self.status = Some(
                        "write token set · capability is checked before every treatment write"
                            .into(),
                    );
                }
            }
            Field::SiteTimezone => {
                let value = edit.buffer.trim();
                if value.is_empty() || value.eq_ignore_ascii_case("local") {
                    self.sites[idx].timezone = None;
                    self.settings_dirty = true;
                    self.status = Some("timezone uses this computer's local time".into());
                } else if value.parse::<chrono_tz::Tz>().is_err() {
                    self.status = Some("use an IANA timezone such as Europe/Amsterdam".into());
                    self.field_edit = Some(edit);
                } else {
                    self.sites[idx].timezone = Some(value.to_string());
                    self.settings_dirty = true;
                    self.status = Some(format!("timezone set to {value} · press w to save"));
                }
            }
            Field::PushUrl => {
                let value = edit.buffer.trim();
                if value.is_empty() {
                    self.status = Some("push URL unchanged · type 'off' to clear".to_string());
                } else if value.eq_ignore_ascii_case("off") {
                    self.alerts.push_url = None;
                    self.alerts.push_enabled = false;
                    self.settings_dirty = true;
                    self.status = Some("push URL cleared · press w to save".to_string());
                } else if reqwest::Url::parse(value)
                    .ok()
                    .is_none_or(|url| !matches!(url.scheme(), "http" | "https"))
                {
                    self.status =
                        Some("push URL must be a complete http:// or https:// URL".into());
                    self.field_edit = Some(edit);
                } else {
                    self.alerts.push_url = Some(value.to_string());
                    self.alerts.push_enabled = true;
                    self.settings_dirty = true;
                    self.status = Some("push URL updated and enabled · press w to save".into());
                }
            }
            _ => {}
        }
    }

    pub fn add_site(&mut self) {
        let mut n = self.sites.len() + 1;
        let name = loop {
            let candidate = format!("site {n}");
            if self.sites.iter().all(|site| site.name != candidate) {
                break candidate;
            }
            n += 1;
        };
        let url = self.active_site().url.clone();
        self.sites.push(Site {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            url,
            token: String::new(),
            write_token: None,
            timezone: None,
            alerts: None,
        });
        self.site_alerts.push(None);
        self.site_validated.push(false);
        self.site_idx = self.sites.len() - 1;
        self.load_active_alerts();
        self.site_dirty = true;
        self.settings_dirty = true;
        self.status = Some("site added · edit its name, URL and token, then press w".to_string());
    }

    pub fn remove_site(&mut self) {
        if self.sites.len() == 1 {
            self.status = Some("the last site cannot be removed".to_string());
            return;
        }
        let idx = self.site_idx.min(self.sites.len() - 1);
        let removed = self.sites.remove(idx).name;
        self.site_alerts.remove(idx);
        self.site_validated.remove(idx);
        self.site_idx = idx.min(self.sites.len() - 1);
        self.load_active_alerts();
        self.site_dirty = true;
        self.settings_dirty = true;
        self.status = Some(format!("removed {removed} · press w to save"));
    }

    // ---- Settings screen ----

    /// Toggle between the dashboard and settings screens.
    pub fn toggle_settings(&mut self) {
        self.screen = match self.screen {
            Screen::Settings => Screen::Dashboard,
            // From the followers list, settings is still one keypress away.
            Screen::Dashboard | Screen::Followers => Screen::Settings,
        };
        self.status = None;
    }

    pub fn selected_field(&self) -> Field {
        Field::ALL[self.settings_sel.min(Field::ALL.len() - 1)]
    }

    /// Move the settings selection, wrapping at the ends.
    pub fn settings_move(&mut self, delta: isize) {
        let n = Field::ALL.len() as isize;
        let cur = self.settings_sel as isize;
        self.settings_sel = ((cur + delta).rem_euclid(n)) as usize;
        self.status = None;
    }

    /// True when the current theme colors match the colorblind-safe palette.
    fn is_colorblind(&self) -> bool {
        self.theme_names
            .iter()
            .zip(theme::COLORBLIND_NAMES)
            .all(|(a, b)| a == b)
    }

    /// Enable quiet hours with a sensible default window if currently unset.
    fn ensure_quiet_hours(&mut self) {
        if self.alerts.quiet_start.is_none() {
            self.alerts.quiet_start = Some(23 * 60);
            self.alerts.quiet_end = Some(7 * 60);
        }
    }

    /// Adjust the selected field by `dir` (-1 / +1), applied live.
    pub fn settings_adjust(&mut self, dir: i32) {
        // Every branch below either changes a persisted value or sets a status
        // message; marking here keeps the flag from being forgotten as rows are
        // added. A no-op row (an unset push URL) clears it again at the end.
        let was_dirty = self.settings_dirty;
        self.settings_dirty = true;
        // Cleared here, not at the end: the tail used to run `self.status =
        // None` unconditionally, which wiped the message the branch below had
        // just set. Both hints — "press enter to edit" and the push-URL note —
        // were dead code that could never reach the screen.
        self.status = None;
        // Threshold step: 0.1 mmol/L or 1 mg/dL, expressed in mg/dL.
        let step_mgdl = self.units.to_mgdl(match self.units {
            Units::Mmol => 0.1,
            Units::Mgdl => 1.0,
        });
        let d = dir as f64;
        match self.selected_field() {
            Field::SiteAlerts => self.set_site_alert_override(!self.site_alert_override()),
            Field::Units => self.toggle_units(),
            Field::Desktop => self.alerts.desktop = !self.alerts.desktop,
            Field::Sound => self.alerts.sound = !self.alerts.sound,
            Field::TestAlarm => {
                self.status = Some("press enter to run the alarm test".to_string());
                self.settings_dirty = was_dirty;
            }
            Field::AddSite | Field::RemoveSite => {
                self.status = Some("press enter".to_string());
                self.settings_dirty = was_dirty;
            }
            Field::Snooze => {
                let next = self.alerts.snooze_minutes + dir as i64 * 5;
                self.alerts.snooze_minutes = next.clamp(1, 120);
            }
            Field::QuietHours => {
                if self.alerts.quiet_start.is_some() {
                    self.alerts.quiet_start = None;
                    self.alerts.quiet_end = None;
                } else {
                    self.alerts.quiet_start = Some(23 * 60); // 23:00
                    self.alerts.quiet_end = Some(7 * 60); // 07:00
                }
            }
            Field::QuietStart => {
                self.ensure_quiet_hours();
                if let Some(s) = self.alerts.quiet_start.as_mut() {
                    *s = (*s + dir * 30).rem_euclid(1440);
                }
            }
            Field::QuietEnd => {
                self.ensure_quiet_hours();
                if let Some(e) = self.alerts.quiet_end.as_mut() {
                    *e = (*e + dir * 30).rem_euclid(1440);
                }
            }
            Field::QuietUrgentLow => self.alerts.quiet_urgent_low = !self.alerts.quiet_urgent_low,
            Field::Escalate => {
                let next = self.alerts.escalate_minutes + dir as i64 * 5;
                self.alerts.escalate_minutes = next.clamp(0, 120);
            }
            Field::PredictHorizon => {
                let next = self.alerts.predict_horizon_minutes + dir as i64 * 5;
                self.alerts.predict_horizon_minutes = next.clamp(0, 60);
            }
            Field::Refresh => {
                let next = self.refresh_secs as i64 + dir as i64 * 5;
                self.refresh_secs = next.max(5) as u64;
                self.refresh_dirty = true;
            }
            Field::Stale => {
                let next = self.alerts.stale_minutes + dir as i64;
                self.alerts.stale_minutes = next.max(1);
            }
            // Thresholds are clamped against their neighbours as well as the
            // physiological range, so the four can never cross: a `low` above
            // `high` (or below `urgent_low`) would silently disable a band and
            // misclassify every reading in it.
            Field::UrgentLow => {
                self.alerts.urgent_low =
                    clamp_bg(self.alerts.urgent_low + d * step_mgdl).min(self.alerts.low)
            }
            Field::Low => {
                self.alerts.low = clamp_bg(self.alerts.low + d * step_mgdl)
                    .clamp(self.alerts.urgent_low, self.alerts.high)
            }
            Field::High => {
                self.alerts.high = clamp_bg(self.alerts.high + d * step_mgdl)
                    .clamp(self.alerts.low, self.alerts.urgent_high)
            }
            Field::UrgentHigh => {
                self.alerts.urgent_high =
                    clamp_bg(self.alerts.urgent_high + d * step_mgdl).max(self.alerts.high)
            }
            Field::SiteName
            | Field::SiteUrl
            | Field::SiteToken
            | Field::SiteWriteToken
            | Field::SiteTimezone
            | Field::PushUrl => {
                self.status = Some("press enter to edit".to_string());
                self.settings_dirty = was_dirty;
            }
            Field::TestSite => {
                self.status = Some("press enter to test this site".into());
                self.settings_dirty = was_dirty;
            }
            Field::NotifyContent => self.alerts.notify_content = !self.alerts.notify_content,
            Field::PushAlerts => {
                // Only meaningful with a URL configured; say so rather than
                // toggling a flag that can't do anything.
                if self.alerts.push_url.is_some() {
                    self.alerts.push_enabled = !self.alerts.push_enabled;
                } else {
                    self.status =
                        Some("set push_url in config.toml to enable push alerts".to_string());
                    self.settings_dirty = was_dirty;
                }
            }
            Field::GraphStyle => self.graph_style = self.graph_style.cycle(dir),
            Field::AgpDays => {
                let next = self.agp_days as i64 + dir as i64;
                self.agp_days = next.clamp(1, 90) as u32;
                // The window changed, so the shared AGP/stats history buffer
                // must be refetched on the next refresh.
                self.agp_fetched_ms = 0;
            }
            Field::CacheEnabled => {
                self.cache_enabled = !self.cache_enabled;
                if !self.cache_enabled {
                    self.status = Some("cache will be deleted when settings are saved".into());
                }
            }
            Field::CacheDays => {
                self.cache_days = (self.cache_days as i32 + dir * 7).clamp(1, 90) as u32;
            }
            Field::MinimapEnabled => self.minimap_enabled = !self.minimap_enabled,
            Field::MinimapSpan => {
                let next = self.minimap_span_ms / MS_PER_HOUR + dir as i64 * 6;
                self.minimap_span_ms = next.clamp(6, 72) * MS_PER_HOUR;
            }
            Field::Colorblind => {
                let names = if self.is_colorblind() {
                    theme::DEFAULT_NAMES
                } else {
                    theme::COLORBLIND_NAMES
                };
                self.theme_names = names.map(String::from);
                self.theme = theme::theme_from_names(&self.theme_names);
            }
            f => {
                if let Some(i) = f.theme_index() {
                    self.theme_names[i] = theme::cycle_color(&self.theme_names[i], dir).to_string();
                    self.theme = theme::theme_from_names(&self.theme_names);
                }
            }
        }
        self.sync_active_alerts();
    }

    /// Persist current settings back to config.toml. Sites and theme are
    /// preserved; thresholds are written in the active display unit.
    pub fn save_config(&mut self) -> bool {
        if self.site_validated.iter().any(|valid| !valid) {
            self.status = Some("test every new or edited site before saving".into());
            return false;
        }
        let persisted = self.build_config();
        let result = Config::path().and_then(|p| {
            let body = toml::to_string_pretty(&persisted).context("failed to serialize config")?;
            // Atomic + owner-only: this file holds the only copy of the token.
            Config::write_atomic(&p, &body)?;
            Ok(p)
        });
        self.status = Some(match result {
            Ok(p) => {
                let delete_cache = self.settings_baseline.history_cache.enabled
                    && !persisted.history_cache.enabled;
                if delete_cache {
                    if let Err(e) = crate::history_cache::clear_all() {
                        return {
                            self.status = Some(format!("saved; cache deletion failed: {e}"));
                            false
                        };
                    }
                } else if persisted.history_cache.enabled {
                    if let Err(e) = crate::history_cache::enable() {
                        return {
                            self.status = Some(format!("saved; cache enabling failed: {e}"));
                            false
                        };
                    }
                }
                self.settings_dirty = false;
                self.settings_baseline = persisted;
                format!("saved to {}", p.display())
            }
            Err(e) => format!("save failed: {e}"),
        });
        !self.settings_dirty
    }

    pub fn request_settings_exit(&mut self, action: SettingsExit) {
        if self.settings_dirty {
            self.settings_exit = Some(action);
            self.status = Some("unsaved changes · w save · d discard · esc cancel".into());
        } else {
            self.finish_settings_exit(action);
        }
    }

    pub fn cancel_settings_exit(&mut self) {
        self.settings_exit = None;
        self.status = Some("kept editing".into());
    }

    pub fn discard_settings(&mut self) {
        let cfg = self.settings_baseline.clone();
        let sites = cfg.resolve_sites().unwrap_or_else(|_| self.sites.clone());
        let alerts = cfg.alerts.resolve_checked(cfg.units).0;
        let fresh = App::new(&cfg, alerts, sites);
        self.units = fresh.units;
        self.sites = fresh.sites;
        self.site_validated = vec![true; self.sites.len()];
        self.site_idx = 0;
        self.alerts = fresh.alerts;
        self.global_alerts = fresh.global_alerts;
        self.site_alerts = fresh.site_alerts;
        self.refresh_secs = fresh.refresh_secs;
        self.allow_unattended_writes = fresh.allow_unattended_writes;
        self.graph_style = fresh.graph_style;
        self.agp_days = fresh.agp_days;
        self.cache_enabled = fresh.cache_enabled;
        self.cache_days = fresh.cache_days;
        self.minimap_enabled = fresh.minimap_enabled;
        self.minimap_span_ms = fresh.minimap_span_ms;
        self.theme = fresh.theme;
        self.theme_names = fresh.theme_names;
        self.settings_dirty = false;
        self.site_dirty = true;
        self.refresh_dirty = true;
        self.status = Some("changes discarded".into());
    }

    pub fn finish_settings_exit(&mut self, action: SettingsExit) {
        self.settings_exit = None;
        match action {
            SettingsExit::Back => self.toggle_settings(),
            SettingsExit::Quit => self.should_quit = true,
        }
    }

    /// Reconstruct a `Config` from current settings for persistence. A lone
    /// "default" site is written back in the legacy top-level form.
    pub(super) fn build_config(&self) -> Config {
        let mut persisted_sites = self.sites.clone();
        for (idx, (site, override_alerts)) in persisted_sites
            .iter_mut()
            .zip(&self.site_alerts)
            .enumerate()
        {
            let current = (idx == self.site_idx && override_alerts.is_some())
                .then_some(&self.alerts)
                .or(override_alerts.as_ref());
            site.alerts = current.map(|alerts| AlertsConfig::from_resolved(alerts, self.units));
        }
        let single_default = persisted_sites.len() == 1
            && persisted_sites[0].name == "default"
            && persisted_sites[0].write_token.is_none()
            && persisted_sites[0].timezone.is_none()
            && persisted_sites[0].alerts.is_none();
        let (url, token, sites) = if single_default {
            (
                Some(persisted_sites[0].url.clone()),
                Some(persisted_sites[0].token.clone()),
                Vec::new(),
            )
        } else {
            (None, None, persisted_sites)
        };
        let u = self.units;
        Config {
            url,
            token,
            sites,
            // Carried through rather than defaulted: `w` in the settings screen
            // must not silently revoke a grant made in the config file.
            allow_unattended_writes: self.allow_unattended_writes,
            units: u,
            refresh_secs: self.refresh_secs,
            alerts: AlertsConfig::from_resolved(
                if self.site_alert_override() {
                    &self.global_alerts
                } else {
                    &self.alerts
                },
                u,
            ),
            theme: ThemeConfig {
                low: Some(self.theme_names[0].clone()),
                in_range: Some(self.theme_names[1].clone()),
                high: Some(self.theme_names[2].clone()),
                urgent: Some(self.theme_names[3].clone()),
                prediction: Some(self.theme_names[4].clone()),
                graph: Some(self.theme_names[5].clone()),
            },
            graph_style: self.graph_style,
            agp_days: self.agp_days,
            minimap: MinimapConfig {
                enabled: self.minimap_enabled,
                span_hours: (self.minimap_span_ms / MS_PER_HOUR) as u32,
            },
            history_cache: crate::config::HistoryCacheConfig {
                enabled: self.cache_enabled,
                retention_days: self.cache_days,
            },
        }
    }

    /// Formatted value of a field for display on the settings screen.
    pub fn field_value(&self, field: Field) -> String {
        match field {
            Field::SiteName => self.active_site().name.clone(),
            Field::SiteAlerts => if self.site_alert_override() {
                "custom for this site"
            } else {
                "use global defaults"
            }
            .to_string(),
            Field::TestSite => if self.site_validated[self.site_idx] {
                "passed"
            } else {
                "required before save"
            }
            .to_string(),
            Field::Units => self.units.label().to_string(),
            Field::Refresh => format!("{}s", self.refresh_secs),
            Field::Desktop => if self.alerts.desktop { "on" } else { "off" }.to_string(),
            Field::Sound => if self.alerts.sound { "on" } else { "off" }.to_string(),
            Field::TestAlarm => self
                .alarm_test
                .clone()
                .unwrap_or_else(|| "press enter".to_string()),
            Field::Snooze => format!("{} min", self.alerts.snooze_minutes),
            Field::QuietHours => if self.alerts.quiet_start.is_some() {
                "on"
            } else {
                "off"
            }
            .to_string(),
            Field::QuietStart => self
                .alerts
                .quiet_start
                .map(crate::config::fmt_hhmm)
                .unwrap_or_else(|| "—".into()),
            Field::QuietEnd => self
                .alerts
                .quiet_end
                .map(crate::config::fmt_hhmm)
                .unwrap_or_else(|| "—".into()),
            Field::QuietUrgentLow => if self.alerts.quiet_urgent_low {
                "on"
            } else {
                "off"
            }
            .to_string(),
            Field::Escalate => {
                if self.alerts.escalate_minutes == 0 {
                    "off".to_string()
                } else {
                    format!("{} min", self.alerts.escalate_minutes)
                }
            }
            Field::SiteUrl => {
                let site = self.active_site();
                // The demo's placeholder URL is not a real site; flagging it
                // would put a warning in every screenshot for nothing.
                if site.is_insecure() && !self.demo {
                    format!("{} ⚠ unencrypted", site.base_url())
                } else {
                    site.base_url().to_string()
                }
            }
            // Never render the token, not even partially: the settings screen
            // is the pane most likely to be screenshotted or screen-shared.
            Field::SiteToken => if self.active_site().token.is_empty() {
                "not set"
            } else {
                "set · ••••••"
            }
            .to_string(),
            Field::SiteWriteToken => if self.active_site().write_token.is_some() {
                "set · hidden · can modify Nightscout"
            } else {
                "off · read-only"
            }
            .to_string(),
            Field::SiteTimezone => self
                .active_site()
                .timezone
                .clone()
                .unwrap_or_else(|| "local (viewer)".into()),
            Field::PushUrl => if self.alerts.push_url.is_some() {
                "set · hidden"
            } else {
                "not configured"
            }
            .to_string(),
            Field::AddSite => "press enter".to_string(),
            Field::RemoveSite => {
                if self.sites.len() == 1 {
                    "unavailable · one site required".to_string()
                } else {
                    format!("press enter · {} selected", self.active_site().name)
                }
            }
            Field::PredictHorizon => {
                let h = self.alerts.predict_horizon_minutes;
                if h == 0 {
                    "off".to_string()
                } else if h > crate::predict::HORIZON_MINUTES {
                    // Beyond the local AR2 projection only an uploader forecast
                    // (Loop / OpenAPS) can reach — say so rather than implying
                    // a warning that far out always exists.
                    format!(
                        "{} min · local forecast {} min",
                        h,
                        crate::predict::HORIZON_MINUTES
                    )
                } else {
                    format!("{h} min")
                }
            }
            // Show where pushes go, but never the full URL — an ntfy topic or
            // a webhook path is a secret, and the settings screen is the pane
            // most likely to end up in a screenshot.
            Field::NotifyContent => if self.alerts.notify_content {
                "value + state"
            } else {
                "generic (no data)"
            }
            .to_string(),
            Field::PushAlerts => match (&self.alerts.push_url, self.alerts.push_enabled) {
                (None, _) => "not configured".to_string(),
                (Some(url), on) => {
                    let state = if on { "on" } else { "off" };
                    let warn = if crate::config::is_insecure_url(url) {
                        " ⚠ unencrypted"
                    } else {
                        ""
                    };
                    format!("{state} · {}{warn}", push_host(url))
                }
            },
            Field::Stale => format!("{} min", self.alerts.stale_minutes),
            Field::UrgentLow => self.threshold(self.alerts.urgent_low),
            Field::Low => self.threshold(self.alerts.low),
            Field::High => self.threshold(self.alerts.high),
            Field::UrgentHigh => self.threshold(self.alerts.urgent_high),
            Field::GraphStyle => self.graph_style.label().to_string(),
            Field::AgpDays => format!("{} days", self.agp_days),
            Field::CacheEnabled => if self.cache_enabled {
                "on · stores health data locally"
            } else {
                "off"
            }
            .into(),
            Field::CacheDays => format!("{} days", self.cache_days),
            Field::MinimapEnabled => if self.minimap_enabled { "on" } else { "off" }.to_string(),
            Field::MinimapSpan => format!("{}h", self.minimap_span_ms / MS_PER_HOUR),
            Field::Colorblind => if self.is_colorblind() { "on" } else { "off" }.to_string(),
            f => f
                .theme_index()
                .map(|i| self.theme_names[i].clone())
                .unwrap_or_default(),
        }
    }

    fn threshold(&self, mgdl: f64) -> String {
        format!("{} {}", self.units.format(mgdl), self.units.label())
    }
}
