mod app;
mod config;
mod nightscout;
mod ui;
mod units;
mod view;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
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
    let mut app = App::new(cfg.units);

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
    let (tx, mut rx) = mpsc::unbounded_channel::<KeyCode>();
    std::thread::spawn(move || loop {
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press && tx.send(k.code).is_err() {
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
                    Some(code) => handle_key(app, client, code).await,
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
async fn handle_key(app: &mut App, client: &Client, code: KeyCode) {
    if app.date_input.is_some() {
        handle_date_input(app, client, code).await;
        return;
    }

    match code {
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
    let (start, end) = app.view.bounds(now_ms());
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
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}
