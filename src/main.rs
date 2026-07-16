mod alert;
mod app;
mod config;
mod nightscout;
mod predict;
mod ui;
mod units;
mod view;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::App;
use config::Config;
use nightscout::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load()?;
    let client = Client::new(&cfg)?;
    let alerts = cfg.alerts.resolve(cfg.units);
    let mut app = App::new(cfg.units, alerts);

    install_panic_hook();
    let mut terminal = setup_terminal()?;
    let res = run(&mut terminal, &mut app, &client, cfg.refresh_secs).await;
    restore_terminal(&mut terminal)?;
    res
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    client: &Client,
    refresh_secs: u64,
) -> Result<()> {
    // Input on a blocking thread, forwarded over a channel.
    let (tx, mut rx) = mpsc::unbounded_channel::<KeyEvent>();
    std::thread::spawn(move || loop {
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press && tx.send(k).is_err() {
                    break;
                }
            }
        }
    });

    refresh(app, client).await;
    terminal.draw(|f| ui::draw(f, app))?;

    let mut ticker = tokio::time::interval(Duration::from_secs(refresh_secs.max(5)));
    ticker.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            maybe_key = rx.recv() => {
                match maybe_key {
                    Some(key) => handle_key(app, client, key).await,
                    None => break,
                }
            }
            _ = ticker.tick() => {
                // Only auto-refetch when following the live edge; a fixed
                // history window doesn't change on its own.
                if app.view.is_live() {
                    refresh(app, client).await;
                }
            }
        }

        if app.should_quit {
            break;
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

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('u') => app.toggle_units(),
        KeyCode::Char('r') => refresh(app, client).await,
        KeyCode::Char('h') | KeyCode::Left => {
            app.view.pan_back(now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char('l') | KeyCode::Right => {
            app.view.pan_forward(now_ms());
            refresh(app, client).await;
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.view.zoom_in();
            refresh(app, client).await;
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            app.view.zoom_out();
            refresh(app, client).await;
        }
        KeyCode::Char('f') | KeyCode::Home => {
            app.view.follow();
            refresh(app, client).await;
        }
        KeyCode::Char('g') => app.begin_date_input(),
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
    match client
        .entries_range(start, end, app.view.span.fetch_count())
        .await
    {
        Ok(entries) => {
            app.entries = entries;
            app.last_error = None;
        }
        Err(e) => app.last_error = Some(e.to_string()),
    }

    app.evaluate_alert(now);
    if app.alerts.desktop {
        if let Some(a) = app.take_notification() {
            notify(a, app.latest().map(|e| e.sgv), app.units);
        }
    }

    // Forecasts only make sense at the live edge. Prefer uploader-published
    // predictions; fall back to a local AR2 projection.
    if app.view.is_live() {
        let device = client.predictions().await.ok().flatten();
        app.predictions = device.unwrap_or_else(|| predict::ar2(&app.entries));
    } else {
        app.predictions.clear();
    }
}

/// Fire a best-effort desktop notification via `notify-send`. Silently does
/// nothing if the binary is absent — it's an optional convenience.
fn notify(alert: alert::Alert, sgv: Option<f64>, units: units::Units) {
    let body = match sgv {
        Some(v) => format!("{} · {} {}", alert.label(), units.format(v), units.label()),
        None => alert.label().to_string(),
    };
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "sugarrush", "-u", alert.urgency(), "sugarrush", &body])
        .spawn();
}

/// Current time in epoch milliseconds.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
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
    execute!(io::stdout(), LeaveAlternateScreen).ok();
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
