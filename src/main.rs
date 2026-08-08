mod agp;
mod alert;
mod app;
mod bigfont;
mod config;
mod demo;
mod export;
mod nightscout;
mod predict;
mod sound;
mod stats;
mod status;
mod theme;
mod ui;
mod units;
mod view;
mod watch;
mod waybar;
mod wizard;

use std::io::{self, IsTerminal, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{App, Screen};
use config::Config;
use nightscout::{Client, FetchError};

/// How the binary was invoked.
enum Mode {
    /// Run the interactive TUI, starting on the given screen; `demo` uses
    /// synthetic data with no config/network.
    Tui { screen: Screen, demo: bool },
    /// Print one Waybar JSON line and exit.
    Waybar,
    /// Print one status-bar line in the given format and exit.
    Status { format: status::Format },
    /// Print version/about info (and a desktop notification) and exit.
    About,
    /// Write a CSV + summary of the last `days` days and exit.
    Export { days: u32, dir: Option<String> },
    /// Run the headless alarm watcher until killed.
    Watch,
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut screen = Screen::Dashboard;
    let mut demo = false;
    let mut mode: Option<Mode> = None;
    let mut export_days: Option<u32> = None;
    let mut status_format: Option<String> = None;
    let mut export_dir: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "waybar" => mode = Some(Mode::Waybar),
            "status" => {
                mode = Some(Mode::Status {
                    format: status::Format::Text,
                })
            }
            "--format" => {
                i += 1;
                status_format = args.get(i).cloned();
            }
            "about" => mode = Some(Mode::About),
            "export" => mode = Some(Mode::Export { days: 0, dir: None }),
            "watch" => mode = Some(Mode::Watch),
            "--days" => {
                i += 1;
                export_days = args.get(i).and_then(|v| v.parse().ok());
            }
            "--out" => {
                i += 1;
                export_dir = args.get(i).cloned();
            }
            "--demo" => demo = true,
            "--screen" => {
                i += 1;
                if args.get(i).map(String::as_str) == Some("settings") {
                    screen = Screen::Settings;
                }
            }
            _ => {}
        }
        i += 1;
    }
    match mode {
        // `--days` / `--out` are only meaningful for export; fill them in here
        // so the flags can appear on either side of the subcommand.
        Some(Mode::Export { .. }) => Mode::Export {
            days: export_days.unwrap_or(0),
            dir: export_dir,
        },
        Some(Mode::Status { .. }) => Mode::Status {
            format: status_format
                .as_deref()
                .map(|name| {
                    status::Format::parse(name).unwrap_or_else(|| {
                        eprintln!(
                            "unknown --format '{name}'. Available: {}",
                            status::Format::NAMES
                        );
                        std::process::exit(2)
                    })
                })
                .unwrap_or(status::Format::Text),
        },
        Some(m) => m,
        None => Mode::Tui { screen, demo },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match parse_args() {
        Mode::About => {
            print_about();
            Ok(())
        }
        Mode::Waybar => {
            let cfg = Config::load()?;
            println!("{}", waybar::line(&cfg).await);
            Ok(())
        }
        Mode::Status { format } => {
            let cfg = Config::load()?;
            println!("{}", status::status(&cfg).await.render(format));
            Ok(())
        }
        Mode::Export { days, dir } => run_export(days, dir).await,
        Mode::Watch => watch::run().await,
        Mode::Tui { screen, demo } => run_tui(screen, demo).await,
    }
}

/// Headless export: fetch the clinical window and write both files, printing
/// the paths. Useful in a cron job or right before an appointment.
async fn run_export(days: u32, dir: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let days = if days == 0 { cfg.agp_days } else { days }.clamp(1, 90);
    let alerts = cfg.alerts.resolve(cfg.units);
    let sites = cfg.resolve_sites()?;
    let client = Client::for_site(&sites[0])?;

    let now = now_ms();
    let start = now - days as i64 * 24 * 3_600_000;
    let entries = client
        .entries_range(start, now, days as usize * 24 * 12 + 200)
        .await?;

    let dir = dir.map(std::path::PathBuf::from).unwrap_or_default();
    for (format, body) in [
        (export::Format::Csv, export::csv(&entries, cfg.units)),
        (
            export::Format::Report,
            export::report(&entries, &alerts, cfg.units, days, now),
        ),
    ] {
        let path = dir.join(export::filename(format, now));
        std::fs::write(&path, body)
            .with_context(|| format!("failed to write {}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}

async fn run_tui(screen: Screen, demo: bool) -> Result<()> {
    let cfg = if demo {
        Config::demo()
    } else {
        // No config yet: guide the user through setup on a terminal, or point
        // them at the file when running non-interactively.
        let path = Config::path()?;
        if !path.exists() {
            if std::io::stdin().is_terminal() {
                wizard::run().await?;
            } else {
                anyhow::bail!(
                    "no config at {}. Copy config.example.toml there (set url + token), \
                     or run sugarrush in a terminal for guided setup.",
                    path.display()
                );
            }
        }
        Config::load()?
    };
    let sites = cfg.resolve_sites()?;
    let alerts = cfg.alerts.resolve(cfg.units);
    let mut app = App::new(&cfg, alerts, sites);
    app.screen = screen;
    app.demo = demo;
    app.perm_warning = !demo && Config::perms_too_open();

    install_panic_hook();
    let mut terminal = setup_terminal(app.minimap_enabled)?;
    if !demo {
        watch::heartbeat(watch::Role::Tui, now_ms());
    }
    let res = run(&mut terminal, &mut app).await;
    restore_terminal(&mut terminal)?;
    // Hand the alarm back to the watcher immediately on exit, rather than
    // leaving it deferring until the heartbeat goes stale.
    if !demo {
        watch::clear_heartbeat(watch::Role::Tui);
    }
    res
}

/// Print name/version/repo and a not-a-medical-device note, and also fire a
/// desktop notification (used by the Waybar About menu).
fn print_about() {
    let version = env!("CARGO_PKG_VERSION");
    let repo = "https://github.com/ronaldlokers/sugarrush";
    let body = format!(
        "Nightscout CGM TUI\n{repo}\nNot a medical device — do not use for treatment decisions."
    );
    println!("sugarrush v{version}\n{body}");
    desktop_notify(&format!("v{version}\n{body}"), false);
}

/// One input event forwarded from the reader thread.
enum Input {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Terminal was resized — triggers a redraw.
    Resize,
}

async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    let mut client = Client::for_site(app.active_site())?;
    // Input on a blocking thread, forwarded over a channel.
    let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
    std::thread::spawn(move || loop {
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            let forwarded = match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => tx.send(Input::Key(k)),
                Ok(Event::Mouse(m)) => tx.send(Input::Mouse(m)),
                Ok(Event::Resize(_, _)) => tx.send(Input::Resize),
                _ => continue,
            };
            if forwarded.is_err() {
                break;
            }
        }
    });

    refresh(app, &client).await;
    terminal.draw(|f| ui::draw(f, app))?;

    let mut ticker = tokio::time::interval(Duration::from_secs(app.refresh_secs.max(5)));
    ticker.tick().await; // consume the immediate first tick
                         // A fast ticker to loop the audible alarm while an urgent state persists.
    let mut alarm_ticker = tokio::time::interval(Duration::from_secs(3));
    alarm_ticker.tick().await;

    loop {
        tokio::select! {
            maybe_input = rx.recv() => {
                match maybe_input {
                    Some(Input::Key(key)) => handle_key(app, &client, key).await,
                    Some(Input::Mouse(m)) => handle_mouse(app, &client, m).await,
                    Some(Input::Resize) => {} // fall through to the redraw below
                    None => break,
                }
            }
            _ = ticker.tick() => {
                // Only auto-refetch when following the live edge; a fixed
                // history window doesn't change on its own (and never while
                // fetching is paused on a bad token / URL).
                if app.should_auto_refresh() {
                    refresh(app, &client).await;
                }
            }
            _ = alarm_ticker.tick() => {
                let now = now_ms();
                // Tell a running `sugarrush watch` that the dashboard is up, so
                // it stays quiet instead of alarming alongside us.
                if !app.demo {
                    watch::heartbeat(watch::Role::Tui, now);
                }
                // Re-classify locally every few seconds (no network) so a sensor
                // gap escalates to a Stale alarm promptly instead of waiting for
                // the next full refresh.
                app.evaluate_alert(now);
                app.update_urgent(now);
                if app.alarm_active(now) {
                    sound::alarm(app.alarm_tone());
                }
                // Retry the connection sooner than the normal interval when down.
                if app.should_retry(now) {
                    refresh(app, &client).await;
                }
            }
        }

        if app.should_quit {
            break;
        }
        // Rebuild the client and reload when the active site changed.
        if app.site_dirty {
            match Client::for_site(app.active_site()) {
                Ok(c) => {
                    client = c;
                    // New credentials/URL — a previous pause no longer applies.
                    app.resume_fetching();
                    refresh(app, &client).await;
                }
                Err(e) => app.last_error = Some(e.to_string()),
            }
            app.site_dirty = false;
        }
        // Rebuild the ticker if the refresh interval was changed in settings.
        if app.refresh_dirty {
            ticker = tokio::time::interval(Duration::from_secs(app.refresh_secs.max(5)));
            ticker.tick().await;
            app.refresh_dirty = false;
        }
        terminal.draw(|f| ui::draw(f, app))?;
    }
    Ok(())
}

/// Dispatch a keypress, either into the date-jump prompt or the dashboard.
async fn handle_key(app: &mut App, client: &Client, key: KeyEvent) {
    // Ctrl+C / Ctrl+D always quit — raw mode delivers these as keys, not signals.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
    {
        app.should_quit = true;
        return;
    }

    if app.date_input.is_some() {
        handle_date_input(app, client, key.code).await;
        return;
    }

    if app.screen == Screen::Settings {
        handle_settings_key(app, key.code);
        return;
    }

    // While the help overlay is up, any key dismisses it.
    if app.show_help {
        app.show_help = false;
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        // Esc backs out, everywhere: it closes the help overlay and cancels a
        // prompt (handled above), and here it returns to the live edge. It used
        // to quit the app outright, which is a lot to trigger by reflex.
        KeyCode::Esc => {
            if app.view.is_live() {
                app.status = Some("press q to quit".to_string());
            } else {
                app.view.follow();
                refresh(app, client).await;
            }
        }
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('s') => app.toggle_settings(),
        KeyCode::Char('u') => app.toggle_units(),
        // An explicit refresh is also how the user retries after a bad
        // token/URL paused automatic fetching.
        KeyCode::Char('r') => {
            app.resume_fetching();
            refresh(app, client).await;
        }
        KeyCode::Tab => {
            app.cycle_graph_view(1);
            refresh(app, client).await;
        }
        KeyCode::BackTab => {
            app.cycle_graph_view(-1);
            refresh(app, client).await;
        }
        // Pan / zoom / jump operate on the timeline, not the AGP profile.
        KeyCode::Char('h') | KeyCode::Left if !app.is_agp() => {
            app.view.pan_back(now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char('l') | KeyCode::Right if !app.is_agp() => {
            app.view.pan_forward(now_ms());
            refresh(app, client).await;
        }
        // Whole-window paging and a jump to the far edge of the overview: the
        // keyboard equivalents of dragging and clicking the minimap, which was
        // otherwise mouse-only.
        KeyCode::Char('H') | KeyCode::PageUp if !app.is_agp() => {
            app.view.page_back(now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char('L') | KeyCode::PageDown if !app.is_agp() => {
            app.view.page_forward(now_ms());
            refresh(app, client).await;
        }
        KeyCode::End if !app.is_agp() => {
            app.view.jump_to_oldest(now_ms(), app.minimap_span_ms);
            refresh(app, client).await;
        }
        KeyCode::Char('+') | KeyCode::Char('=') if !app.is_agp() => {
            app.view.zoom_in();
            refresh(app, client).await;
        }
        KeyCode::Char('-') | KeyCode::Char('_') if !app.is_agp() => {
            app.view.zoom_out();
            refresh(app, client).await;
        }
        KeyCode::Char('f') | KeyCode::Home if !app.is_agp() => {
            app.view.follow();
            refresh(app, client).await;
        }
        KeyCode::Char('g') if !app.is_agp() => app.begin_date_input(),
        // Walk day by day without typing a date each time — "how was last
        // night?" is the common case, and `g` makes you spell it out.
        KeyCode::Char('[') if !app.is_agp() => {
            app.view.shift_day(-1, now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char(']') if !app.is_agp() => {
            app.view.shift_day(1, now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char('n') => app.next_site(),
        KeyCode::Char('a') => app.snooze_alarm(now_ms()),
        KeyCode::Char('e') => app.export_window(now_ms()),
        _ => {}
    }
}

/// Handle a mouse event: a press or drag over the minimap seeks the main
/// window to that time.
async fn handle_mouse(app: &mut App, client: &Client, m: MouseEvent) {
    if !app.minimap_enabled || app.screen != Screen::Dashboard {
        return;
    }
    let seeking = matches!(m.kind, MouseEventKind::Down(_) | MouseEventKind::Drag(_));
    if seeking && app.minimap_seek(m.column, m.row, now_ms()) {
        refresh(app, client).await;
    }
}

/// Handle keys on the settings screen. All edits apply live; `w` persists.
fn handle_settings_key(app: &mut App, code: KeyCode) {
    // A row being edited as text swallows the keys — otherwise typing a URL
    // would trigger the single-letter shortcuts underneath it.
    if app.field_edit.is_some() {
        match code {
            KeyCode::Esc => app.cancel_field_edit(),
            KeyCode::Enter => app.commit_field_edit(),
            KeyCode::Backspace => app.field_edit_backspace(),
            KeyCode::Char(c) => app.field_edit_push(c),
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('s') | KeyCode::Esc => app.toggle_settings(),
        KeyCode::Char('j') | KeyCode::Down => app.settings_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.settings_move(-1),
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Char('-') => app.settings_adjust(-1),
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => {
            app.settings_adjust(1)
        }
        KeyCode::Enter => {
            app.begin_field_edit();
        }
        KeyCode::Char('w') => app.save_config(),
        _ => {}
    }
}

/// Handle keys while the date-jump prompt is open.
async fn handle_date_input(app: &mut App, client: &Client, code: KeyCode) {
    match code {
        KeyCode::Esc => app.cancel_date_input(),
        KeyCode::Backspace => {
            if let Some(buf) = app.date_input.as_mut() {
                buf.pop();
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() || c == '-' => {
            if let Some(buf) = app.date_input.as_mut() {
                buf.push(c);
            }
        }
        KeyCode::Enter => {
            let buf = app.date_input.take().unwrap_or_default();
            match view::parse_date(&buf) {
                Some(date) => {
                    app.view.jump_to(date, now_ms());
                    refresh(app, client).await;
                }
                None => app.last_error = Some(format!("invalid date '{buf}', use YYYY-MM-DD")),
            }
        }
        _ => {}
    }
}

async fn refresh(app: &mut App, client: &Client) {
    let now = now_ms();
    let (start, end) = app.view.bounds(now);
    app.view_start = start;
    app.view_end = end;

    // Demo mode: synthesize everything locally, no network.
    if app.demo {
        app.entries = demo::entries(start, end);
        app.mark_online(now);
        app.evaluate_alert(now);
        if app.view.is_live() {
            app.predictions = predict::ar2(&app.entries);
            app.device = demo::device();
            app.treatments = demo::treatments(now);
        } else {
            app.predictions.clear();
        }
        if app.minimap_enabled {
            app.minimap_entries = demo::entries(now - app.minimap_span_ms, now);
        }
        // Always synthesized: the stats panel reads this clinical-window
        // buffer even outside the AGP view.
        app.agp_entries = demo::entries(now - app.agp_span_ms(), now);
        return;
    }
    match client
        .entries_range(start, end, app.view.span.fetch_count())
        .await
    {
        Ok(entries) => {
            app.entries = entries;
            app.mark_online(now);
        }
        // Keep the last-known readings on screen; just flag the outage. A
        // config-level failure (bad token / URL) is not a transient outage —
        // App pauses retries after a few of them.
        Err(e) => {
            let permanent = e.downcast_ref::<FetchError>().is_some();
            app.mark_offline(now, e.to_string(), permanent);
        }
    }

    // Supplementary fetches only happen when the primary read succeeded — on an
    // outage we skip them and go straight to (local) alert evaluation, so a
    // stalled network can't pile up doomed requests behind it.
    if app.online {
        // Which supplementary reads failed. They're best-effort — the glucose
        // trace is what matters — but silently showing a stale IOB or a missing
        // carb marker as if it were current is its own kind of wrong, so the
        // dashboard says which part is missing.
        let mut missing: Vec<&str> = Vec::new();

        // Treatment markers for the visible window (best-effort).
        match client.treatments(start, end).await {
            Ok(t) => app.treatments = t,
            Err(_) => missing.push("treatments"),
        }

        // Forecasts and device status only make sense at the live edge — and
        // must be refreshed *before* predictive alerts read them below.
        if app.view.is_live() {
            // One devicestatus fetch feeds both the uploader panel and the
            // forecast; falling back to the local AR2 projection when the
            // uploader publishes none (or the fetch failed).
            let published = match client.device_status().await {
                Ok((status, predicted)) => {
                    app.device = status;
                    predicted
                }
                Err(_) => {
                    missing.push("device");
                    None
                }
            };
            app.predictions = published.unwrap_or_else(|| predict::ar2(&app.entries));
            match client.sensor_start().await {
                Ok(started) => app.sensor_start_ms = started,
                Err(_) => missing.push("sensor age"),
            }
        } else {
            app.predictions.clear();
        }

        // The `agp_days` history buffer feeds both the AGP view and the stats
        // panel's clinical TIR/mean/GMI, so it's kept warm in every view — but
        // it's a heavy fetch (up to 90 days of readings), so outside the AGP
        // view it only refreshes when empty or older than 15 minutes.
        let agp_stale = now - app.agp_fetched_ms > 15 * 60 * 1000;
        if app.is_agp() || app.agp_entries.is_empty() || agp_stale {
            match client
                .entries_range(now - app.agp_span_ms(), now, app.agp_fetch_count())
                .await
            {
                Ok(entries) => {
                    app.agp_entries = entries;
                    app.agp_fetched_ms = now;
                }
                Err(_) => missing.push("history"),
            }
        }

        // Refresh the trailing overview only at the live edge; while dragging
        // into history it stays put (it's a now-anchored strip, so refetching
        // on each drag frame would be wasteful).
        if app.minimap_enabled && (app.view.is_live() || app.minimap_entries.is_empty()) {
            match client
                .entries_range(
                    now - app.minimap_span_ms,
                    now,
                    2 * app.minimap_span_ms as usize / 60_000,
                )
                .await
            {
                Ok(entries) => app.minimap_entries = entries,
                Err(_) => missing.push("overview"),
            }
        }
        app.set_partial(&missing);
    } else if !app.view.is_live() {
        app.predictions.clear();
    }

    // Alert evaluation and notifications run every refresh, online or not, so a
    // sensor gap still escalates to a Stale alarm.
    app.evaluate_alert(now);
    if app.alerts.desktop {
        if let Some(a) = app.take_notification() {
            notify(
                a,
                app.latest().map(|e| e.sgv),
                app.units,
                app.alerts.notify_content,
            );
        }
    }
    app.update_urgent(now);
    if let Some(msg) = app.take_push(now) {
        if let Some(url) = app.alerts.push_url.clone() {
            // The push webhook is a safety channel (unacknowledged-urgent
            // escalation); a dead URL must not fail silently.
            if !push(&url, &msg).await {
                app.last_error = Some("push notification failed — check push_url".to_string());
            }
        }
    }
    if let Some(msg) = app.take_predictive(now) {
        if app.alerts.desktop {
            if app.alerts.notify_content {
                notify_text(&msg);
            } else {
                notify_text("alert — open sugarrush");
            }
        }
    }
}

/// Fire a best-effort desktop notification for an alert.
pub(crate) fn notify(alert: alert::Alert, sgv: Option<f64>, units: units::Units, content: bool) {
    // Content-free mode still fires — and still as critical, so it breaks
    // through Do Not Disturb — but says nothing a lock screen shouldn't show.
    if !content {
        desktop_notify("alert — open sugarrush", alert.urgency() == "critical");
        return;
    }
    let body = match sgv {
        Some(v) => format!("{} · {} {}", alert.label(), units.format(v), units.label()),
        None => alert.label().to_string(),
    };
    desktop_notify(&body, alert.urgency() == "critical");
}

/// Fire a plain desktop notification (used for predictive alerts).
pub(crate) fn notify_text(body: &str) {
    desktop_notify(body, false);
}

/// Cross-platform desktop notification (Linux / macOS / Windows) via
/// notify-rust. Best-effort — errors are ignored.
fn desktop_notify(body: &str, critical: bool) {
    let mut n = notify_rust::Notification::new();
    n.summary("sugarrush").body(body).appname("sugarrush");
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        n.urgency(if critical {
            notify_rust::Urgency::Critical
        } else {
            notify_rust::Urgency::Normal
        });
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = critical;
    }
    let _ = n.show();
}

/// POST an alert message to a webhook / ntfy topic. Returns whether the request
/// was accepted (2xx), so the caller can surface a dead push URL.
pub(crate) async fn push(url: &str, message: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return false;
    };
    client
        .post(url)
        .body(message.to_string())
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Current time in epoch milliseconds.
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn setup_terminal(mouse: bool) -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    if mouse {
        execute!(stdout, EnableMouseCapture).context("failed to enable mouse capture")?;
    }
    Terminal::new(CrosstermBackend::new(stdout)).context("failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    restore();
    terminal.show_cursor().ok();
    Ok(())
}

/// Undo the terminal setup. Safe to call from anywhere, including a panic hook,
/// as it operates on `stdout` directly rather than the `Terminal`.
fn restore() {
    disable_raw_mode().ok();
    // DisableMouseCapture is harmless if capture was never enabled.
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen).ok();
}

/// Restore the terminal before the default panic handler prints, so a panic
/// leaves a usable shell and a readable message instead of a garbled screen.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        original(info);
    }));
}
