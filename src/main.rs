mod app;
mod config;
mod nightscout;
mod ui;
mod units;

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
                    Some(KeyCode::Char('q')) | Some(KeyCode::Esc) => app.should_quit = true,
                    Some(KeyCode::Char('u')) => app.toggle_units(),
                    Some(KeyCode::Char('r')) => refresh(app, client).await,
                    _ => {}
                }
            }
            _ = ticker.tick() => {
                refresh(app, client).await;
            }
        }

        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, app))?;
    }
    Ok(())
}

async fn refresh(app: &mut App, client: &Client) {
    match client.entries().await {
        Ok(entries) => {
            app.entries = entries;
            app.last_error = None;
        }
        Err(e) => app.last_error = Some(e.to_string()),
    }
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
