mod agp;
mod alert;
mod app;
mod bigfont;
mod config;
mod demo;
mod nightscout;
mod predict;
mod sound;
mod stats;
mod theme;
mod ui;
mod units;
mod view;
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
use nightscout::Client;

/// How the binary was invoked.
enum Mode {
    /// Run the interactive TUI, starting on the given screen; `demo` uses
    /// synthetic data with no config/network.
    Tui { screen: Screen, demo: bool },
    /// Print one Waybar JSON line and exit.
    Waybar,
    /// Print version/about info (and a desktop notification) and exit.
    About,
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut screen = Screen::Dashboard;
    let mut demo = false;
    let mut mode: Option<Mode> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "waybar" => mode = Some(Mode::Waybar),
            "about" => mode = Some(Mode::About),
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
    mode.unwrap_or(Mode::Tui { screen, demo })
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
        Mode::Tui { screen, demo } => run_tui(screen, demo).await,
    }
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
    let res = run(&mut terminal, &mut app).await;
    restore_terminal(&mut terminal)?;
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
                // history window doesn't change on its own.
                if app.view.is_live() {
                    refresh(app, &client).await;
                }
            }
            _ = alarm_ticker.tick() => {
                let now = now_ms();
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

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('s') => app.toggle_settings(),
        KeyCode::Char('u') => app.toggle_units(),
        KeyCode::Char('r') => refresh(app, client).await,
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
        KeyCode::Char('n') => app.next_site(),
        KeyCode::Char('a') => app.snooze_alarm(now_ms()),
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
    match code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('s') | KeyCode::Esc => app.toggle_settings(),
        KeyCode::Char('j') | KeyCode::Down => app.settings_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.settings_move(-1),
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Char('-') => app.settings_adjust(-1),
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('=') => {
            app.settings_adjust(1)
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
        if app.is_agp() {
            app.agp_entries = demo::entries(now - app.agp_span_ms(), now);
        }
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
        // Keep the last-known readings on screen; just flag the outage.
        Err(e) => app.mark_offline(now, e.to_string()),
    }

    // Treatment markers for the visible window (best-effort).
    if let Ok(t) = client.treatments(start, end).await {
        app.treatments = t;
    }

    app.evaluate_alert(now);
    if app.alerts.desktop {
        if let Some(a) = app.take_notification() {
            notify(a, app.latest().map(|e| e.sgv), app.units);
        }
    }
    app.update_urgent(now);
    if let Some(msg) = app.take_push(now) {
        if let Some(url) = app.alerts.push_url.clone() {
            push(&url, &msg).await;
        }
    }
    if let Some(msg) = app.take_predictive(now) {
        if app.alerts.desktop {
            notify_text(&msg);
        }
    }

    // Forecasts and device status only make sense at the live edge.
    if app.view.is_live() {
        let device = client.predictions().await.ok().flatten();
        app.predictions = device.unwrap_or_else(|| predict::ar2(&app.entries));
        if let Ok(status) = client.device_status().await {
            app.device = status;
        }
        if let Ok(started) = client.sensor_start().await {
            app.sensor_start_ms = started;
        }
    } else {
        app.predictions.clear();
    }

    // AGP folds many days of history; fetch its own wider window on demand.
    if app.is_agp() {
        if let Ok(entries) = client
            .entries_range(now - app.agp_span_ms(), now, app.agp_fetch_count())
            .await
        {
            app.agp_entries = entries;
        }
    }

    // Refresh the trailing overview only at the live edge; while dragging into
    // history it stays put (it's a now-anchored strip, so refetching on each
    // drag frame would be wasteful).
    if app.minimap_enabled && (app.view.is_live() || app.minimap_entries.is_empty()) {
        if let Ok(entries) = client
            .entries_range(
                now - app.minimap_span_ms,
                now,
                2 * app.minimap_span_ms as usize / 60_000,
            )
            .await
        {
            app.minimap_entries = entries;
        }
    }
}

/// Fire a best-effort desktop notification for an alert.
fn notify(alert: alert::Alert, sgv: Option<f64>, units: units::Units) {
    let body = match sgv {
        Some(v) => format!("{} · {} {}", alert.label(), units.format(v), units.label()),
        None => alert.label().to_string(),
    };
    desktop_notify(&body, alert.urgency() == "critical");
}

/// Fire a plain desktop notification (used for predictive alerts).
fn notify_text(body: &str) {
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

/// POST an alert message to a webhook / ntfy topic. Best-effort, non-blocking.
async fn push(url: &str, message: &str) {
    if let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        let _ = client.post(url).body(message.to_string()).send().await;
    }
}

/// Current time in epoch milliseconds.
fn now_ms() -> i64 {
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
