mod agp;
mod alert;
mod app;
mod bigfont;
mod config;
mod demo;
mod export;
mod follow;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
use tokio::time::MissedTickBehavior;

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
    /// Write a systemd user unit pointing at this binary, and explain how to
    /// enable it.
    InstallUnit,
    /// Print usage and exit.
    Help,
    /// Print the version and exit.
    Version,
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
            "--install-unit" => mode = Some(Mode::InstallUnit),
            "help" | "--help" | "-h" => mode = Some(Mode::Help),
            "--version" | "-V" => mode = Some(Mode::Version),
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
            // Silently ignoring an unrecognised argument meant `sugarrush
            // --help` opened the TUI — and on an unconfigured machine, the
            // first-run wizard, which then asked for a Nightscout token.
            other => {
                eprintln!("sugarrush: unknown argument '{other}'");
                eprintln!("Try 'sugarrush --help'.");
                std::process::exit(2);
            }
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
            // Bars read stdout and ignore stderr, so the warning reaches a
            // human running it by hand without corrupting the bar's input.
            warn_about_config(&cfg.alerts.resolve_checked(cfg.units).1);
            println!("{}", waybar::line(&cfg).await);
            Ok(())
        }
        Mode::Status { format } => {
            let cfg = Config::load()?;
            warn_about_config(&cfg.alerts.resolve_checked(cfg.units).1);
            println!("{}", status::status(&cfg).await.render(format));
            Ok(())
        }
        Mode::Export { days, dir } => run_export(days, dir).await,
        Mode::Watch => watch::run().await,
        Mode::InstallUnit => install_unit(),
        Mode::Help => {
            print_help();
            Ok(())
        }
        Mode::Version => {
            println!("sugarrush {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Mode::Tui { screen, demo } => run_tui(screen, demo).await,
    }
}

/// Headless export: fetch the clinical window and write both files, printing
/// the paths. Useful in a cron job or right before an appointment.
async fn run_export(days: u32, dir: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let days = if days == 0 { cfg.agp_days } else { days }.clamp(1, 90);
    let (alerts, warnings) = cfg.alerts.resolve_checked(cfg.units);
    warn_about_config(&warnings);
    let sites = cfg.resolve_sites()?;
    let client = Client::for_site(&sites[0])?;

    let now = now_ms();
    let start = now - days as i64 * 24 * 3_600_000;
    let entries = client
        .entries_range(start, now, days as usize * 24 * 12 + 200)
        .await?;

    let dir = dir.map(std::path::PathBuf::from).unwrap_or_default();
    let dir = if dir.as_os_str().is_empty() {
        std::path::PathBuf::from(".")
    } else {
        dir
    };
    for path in export::write_pair(&dir, &entries, &alerts, cfg.units, days, now)? {
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
    let (alerts, warnings) = cfg.alerts.resolve_checked(cfg.units);
    // Also to stderr: a misconfigured threshold is worth seeing even if the
    // terminal is about to be taken over by the alternate screen.
    warn_about_config(&warnings);
    let mut app = App::new(&cfg, alerts, sites);
    app.config_warnings = warnings;
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

/// Usage. Kept in one place so the README, the man page and this can't drift.
fn print_help() {
    println!(
        "\
sugarrush {version} — your Nightscout CGM data, in the terminal

USAGE:
    sugarrush [--demo] [--screen settings]   the dashboard
    sugarrush watch                          headless alarm watcher (no terminal needed)
    sugarrush export [--days N] [--out DIR]  CSV + a clinical summary
    sugarrush status [--format FORMAT]       one line for a status bar
    sugarrush waybar                         alias for --format waybar
    sugarrush about                          version, repo and the safety note

OPTIONS:
    --demo                 synthetic data, no config and no network
    --screen settings      open straight to the settings screen
    --days N               export window in days (default: the AGP days setting)
    --out DIR              where to write exports (default: the current directory)
    --format FORMAT        {formats}
    --install-unit         write a systemd user unit for `watch` and explain how to enable it
    -h, --help             this
    -V, --version          print the version

Config lives at ~/.config/sugarrush/config.toml; the first run sets it up.
sugarrush is not a medical device — don't use it for treatment decisions.",
        version = env!("CARGO_PKG_VERSION"),
        formats = status::Format::NAMES,
    );
}

/// Write a systemd user unit that points at *this* binary.
///
/// The shipped unit hardcoded `~/.cargo/bin`, which is where only one of the
/// five install methods puts it, and the README told people to copy a file out
/// of a git checkout they never made. Both produced a service that enables
/// cleanly and then fails at start.
fn install_unit() -> Result<()> {
    let exe = std::env::current_exe().context("could not find this binary's path")?;
    let dir = dirs::config_dir()
        .context("could not resolve the user config dir")?
        .join("systemd/user");
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join("sugarrush-watch.service");

    let unit = format!(
        "\
[Unit]
Description=sugarrush CGM alarm watcher
Documentation=https://github.com/ronaldlokers/sugarrush

[Service]
Type=simple
ExecStart={exe} watch
Restart=always
RestartSec=30
KillSignal=SIGTERM
TimeoutStopSec=10
# Hardening: this reads a credential and talks to one host.
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=%S/sugarrush %t/sugarrush
PrivateDevices=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX

[Install]
# default.target, not graphical-session.target: the alarm should survive
# logging out, and on sway/Hyprland/i3 without uwsm that target never
# activates at all — the unit would enable cleanly and never start.
WantedBy=default.target
",
        exe = exe.display()
    );
    std::fs::write(&path, unit).with_context(|| format!("failed to write {}", path.display()))?;

    println!("Wrote {}", path.display());
    println!();
    println!("Enable it with:");
    println!("    systemctl --user daemon-reload");
    println!("    systemctl --user enable --now sugarrush-watch.service");
    println!();
    println!("To keep it running when you're not logged in:");
    println!("    loginctl enable-linger $USER");
    println!();
    println!("Watch what it's doing:");
    println!("    journalctl --user -fu sugarrush-watch");
    Ok(())
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

    // The first fetch is awaited: there is nothing to show until it lands.
    refresh(app, &client).await;
    terminal.draw(|f| ui::draw(f, app))?;

    let (fetch_tx, mut fetch_rx) = mpsc::unbounded_channel::<(Plan, Gathered)>();
    let (err_tx, mut err_rx) = mpsc::unbounded_channel::<String>();
    let mut fetch = Fetcher::new(client.clone(), fetch_tx, err_tx);

    let mut ticker = tokio::time::interval(Duration::from_secs(app.refresh_secs.max(5)));
    // Delay, not the default Burst: after a stall (a suspended laptop, a slow
    // fetch) Burst fires every missed tick back-to-back. On the 3-second alarm
    // ticker that is a machine-gun of alarm sounds the moment the machine wakes.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick
                         // A fast ticker to loop the audible alarm while an urgent state persists.
    let mut alarm_ticker = tokio::time::interval(Duration::from_secs(3));
    alarm_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    alarm_ticker.tick().await;

    loop {
        tokio::select! {
            maybe_input = rx.recv() => {
                match maybe_input {
                    Some(Input::Key(key)) => handle_key(app, &fetch, key),
                    Some(Input::Mouse(m)) => handle_mouse(app, &fetch, m),
                    Some(Input::Resize) => {} // fall through to the redraw below
                    None => break,
                }
            }
            _ = ticker.tick() => {
                // Only auto-refetch when following the live edge; a fixed
                // history window doesn't change on its own (and never while
                // fetching is paused on a bad token / URL).
                if app.should_auto_refresh() {
                    fetch.request(app);
                }
            }
            Some((p, g)) = fetch_rx.recv() => {
                fetch.deliver(app, p, g);
            }
            Some(e) = err_rx.recv() => {
                app.last_error = Some(e);
            }
            _ = alarm_ticker.tick() => {
                let now = now_ms();
                // Tell a running `sugarrush watch` that the dashboard is up, so
                // it stays quiet instead of alarming alongside us.
                // Only claim the alarm when this window actually covers
                // everything the daemon would. The TUI alerts on the active
                // site alone, so with several configured it must not silence a
                // watcher that is handling all of them — that hole meant a
                // caregiver's other sites went unalarmed while the dashboard
                // was open.
                if !app.demo && app.sites.len() == 1 {
                    watch::heartbeat(watch::Role::Tui, now);
                    // Read the other side of the handshake too: until now
                    // nothing ever did, so a dead watcher and a quiet night
                    // looked identical from in here.
                    app.watcher_alive = watch::is_alive(watch::Role::Watch, now);
                    app.watcher_seen |= app.watcher_alive;
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
                    fetch.request(app);
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
                    fetch.client = client.clone();
                    // New credentials/URL — a previous pause no longer applies.
                    app.resume_fetching();
                    fetch.request(app);
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
fn handle_key(app: &mut App, fetch: &Fetcher, key: KeyEvent) {
    // Ctrl+C / Ctrl+D always quit — raw mode delivers these as keys, not signals.
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
    {
        app.should_quit = true;
        return;
    }

    if app.date_input.is_some() {
        handle_date_input(app, fetch, key.code);
        return;
    }

    // The overlay is up: any key dismisses it, on whatever screen it was opened
    // from. This used to sit inside the dashboard branch, so a `?` pressed on
    // the followers screen could only be cleared by leaving that screen.
    if app.show_help {
        app.show_help = false;
        return;
    }

    if app.screen == Screen::Followers {
        match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('m') | KeyCode::Esc => app.toggle_followers(),
            KeyCode::Char('s') => app.toggle_settings(),
            KeyCode::Char('r') => fetch.request(app),
            KeyCode::Char('?') => app.show_help = true,
            _ => {}
        }
        return;
    }
    if app.screen == Screen::Settings {
        handle_settings_key(app, key.code);
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
                fetch.request(app);
            }
        }
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('s') => app.toggle_settings(),
        KeyCode::Char('u') => app.toggle_units(),
        // An explicit refresh is also how the user retries after a bad
        // token/URL paused automatic fetching.
        KeyCode::Char('r') => {
            app.resume_fetching();
            fetch.request(app);
        }
        KeyCode::Tab => {
            app.cycle_graph_view(1);
            fetch.request(app);
        }
        KeyCode::BackTab => {
            app.cycle_graph_view(-1);
            fetch.request(app);
        }
        // Pan / zoom / jump operate on the timeline, not the AGP profile.
        KeyCode::Char('h') | KeyCode::Left if !app.is_agp() => {
            app.view.pan_back(now_ms());
            fetch.request(app);
        }
        KeyCode::Char('l') | KeyCode::Right if !app.is_agp() => {
            app.view.pan_forward(now_ms());
            fetch.request(app);
        }
        // Whole-window paging and a jump to the far edge of the overview: the
        // keyboard equivalents of dragging and clicking the minimap, which was
        // otherwise mouse-only.
        KeyCode::Char('H') | KeyCode::PageUp if !app.is_agp() => {
            app.view.page_back(now_ms());
            fetch.request(app);
        }
        KeyCode::Char('L') | KeyCode::PageDown if !app.is_agp() => {
            app.view.page_forward(now_ms());
            fetch.request(app);
        }
        KeyCode::End if !app.is_agp() => {
            app.view.jump_to_oldest(now_ms(), app.minimap_span_ms);
            fetch.request(app);
        }
        KeyCode::Char('+') | KeyCode::Char('=') if !app.is_agp() => {
            app.view.zoom_in();
            fetch.request(app);
        }
        KeyCode::Char('-') | KeyCode::Char('_') if !app.is_agp() => {
            app.view.zoom_out();
            fetch.request(app);
        }
        KeyCode::Char('f') | KeyCode::Home if !app.is_agp() => {
            app.view.follow();
            fetch.request(app);
        }
        KeyCode::Char('g') if !app.is_agp() => app.begin_date_input(),
        // Walk day by day without typing a date each time — "how was last
        // night?" is the common case, and `g` makes you spell it out.
        KeyCode::Char('[') if !app.is_agp() => {
            app.view.shift_day(-1, now_ms());
            fetch.request(app);
        }
        KeyCode::Char(']') if !app.is_agp() => {
            app.view.shift_day(1, now_ms());
            fetch.request(app);
        }
        KeyCode::Char('n') => app.next_site(),
        KeyCode::Char('m') => {
            app.toggle_followers();
            // Fetch on entry: the list is the whole point of the screen, and
            // waiting out the refresh interval to see it reads as broken.
            if app.screen == Screen::Followers {
                fetch.request(app);
            }
        }
        KeyCode::Char('a') => app.snooze_alarm(now_ms()),
        KeyCode::Char('e') => app.export_window(now_ms()),
        _ => {}
    }
}

/// Handle a mouse event: a press or drag over the minimap seeks the main
/// window to that time.
fn handle_mouse(app: &mut App, fetch: &Fetcher, m: MouseEvent) {
    if !app.minimap_enabled || app.screen != Screen::Dashboard {
        return;
    }
    let seeking = matches!(m.kind, MouseEventKind::Down(_) | MouseEventKind::Drag(_));
    if seeking && app.minimap_seek(m.column, m.row, now_ms()) {
        fetch.request(app);
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
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('w') => app.save_config(),
        _ => {}
    }
}

/// Handle keys while the date-jump prompt is open.
fn handle_date_input(app: &mut App, fetch: &Fetcher, code: KeyCode) {
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
                    fetch.request(app);
                }
                None => app.last_error = Some(format!("invalid date '{buf}', use YYYY-MM-DD")),
            }
        }
        _ => {}
    }
}

/// What a refresh should fetch, snapshotted from `App` *before* the fetch
/// starts.
///
/// The run loop used to `await` the whole fetch chain inside its `select!`,
/// which meant a slow Nightscout froze keyboard input, the redraw and — worst
/// of all — the 3-second alarm ticker. Five sequential requests at a 12s
/// timeout is a minute of a dashboard that looks alive and answers nothing.
/// Splitting the work in three (plan / gather / apply) lets the gather half run
/// on its own task while the loop keeps drawing and sounding the alarm.
struct Plan {
    now: i64,
    start: i64,
    end: i64,
    count: usize,
    live: bool,
    demo: bool,
    minimap_span_ms: i64,
    minimap: bool,
    /// `Some((start, end, count))` when the heavy history buffer is due.
    agp: Option<(i64, i64, usize)>,
    followers: Option<(Vec<config::Site>, config::Alerts)>,
}

/// Whatever the network returned. Every field is best-effort except `entries`,
/// whose failure is what marks the app offline.
#[derive(Default)]
struct Gathered {
    entries: Option<Result<Vec<nightscout::Entry>>>,
    treatments: Option<Result<Vec<nightscout::Treatment>>>,
    device: Option<
        Result<(
            nightscout::DeviceStatus,
            Option<Vec<nightscout::Prediction>>,
        )>,
    >,
    sensor_start: Option<Result<Option<i64>>>,
    agp: Option<Result<Vec<nightscout::Entry>>>,
    minimap: Option<Result<Vec<nightscout::Entry>>>,
    live_edge: Option<Vec<nightscout::Entry>>,
    followers: Option<Vec<follow::SiteStatus>>,
}

fn plan(app: &mut App, now: i64) -> Plan {
    let (start, end) = app.view.bounds(now);
    app.view_start = start;
    app.view_end = end;
    // The history buffer is heavy (up to 90 days), so outside the AGP view it
    // only refreshes when empty or older than 15 minutes.
    let agp_stale = now - app.agp_fetched_ms > 15 * 60 * 1000;
    let agp = (app.is_agp() || app.agp_entries.is_empty() || agp_stale)
        .then(|| (now - app.agp_span_ms(), now, app.agp_fetch_count()));
    Plan {
        now,
        start,
        end,
        count: app.view.span.fetch_count(),
        live: app.view.is_live(),
        demo: app.demo,
        minimap_span_ms: app.minimap_span_ms,
        minimap: app.minimap_enabled && (app.view.is_live() || app.minimap_entries.is_empty()),
        agp,
        followers: (app.sites.len() > 1 && app.screen == Screen::Followers)
            .then(|| (app.sites.clone(), app.alerts.clone())),
    }
}

/// The network half. Touches no `App`, so it can run on its own task.
async fn gather(client: &Client, p: &Plan) -> Gathered {
    let mut g = Gathered::default();
    if p.demo {
        return g;
    }
    let entries = client.entries_range(p.start, p.end, p.count).await;
    let online = entries.is_ok();
    g.entries = Some(entries);

    // Supplementary reads only happen when the primary one succeeded — on an
    // outage we skip them, so a stalled network can't pile up doomed requests.
    if online {
        g.treatments = Some(client.treatments(p.start, p.end).await);
        if p.live {
            g.device = Some(client.device_status().await);
            g.sensor_start = Some(client.sensor_start().await);
        }
        if let Some((s, e, n)) = p.agp {
            g.agp = Some(client.entries_range(s, e, n).await);
        }
        if p.minimap {
            g.minimap = Some(
                client
                    .entries_range(
                        p.now - p.minimap_span_ms,
                        p.now,
                        2 * p.minimap_span_ms as usize / 60_000,
                    )
                    .await,
            );
        }
        // While browsing history the primary fetch is a historical window, so
        // the live edge has to be fetched separately — the alarm must not go
        // quiet just because someone is looking at last night.
        if !p.live {
            g.live_edge = client
                .entries_range(p.now - 3_600_000, p.now, 24)
                .await
                .ok();
        }
    }
    if let Some((sites, alerts)) = &p.followers {
        g.followers = Some(follow::poll(sites, alerts, p.now).await);
    }
    g
}

/// The mutation half: fold a finished `Gathered` back into `App`, then run the
/// alert machine. Synchronous, so the run loop can never block in here.
///
/// Returns the push-webhook call to make, if any — the one remaining piece of
/// network in the tail, handed back so the caller can spawn it rather than
/// await it.
#[must_use]
fn apply(app: &mut App, p: &Plan, g: Gathered) -> Option<(String, String)> {
    let now = p.now;

    // Demo mode: synthesize everything locally, no network.
    if p.demo {
        app.entries = demo::entries(p.start, p.end);
        app.mark_online(now);
        app.evaluate_alert(now);
        if p.live {
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
        return None;
    }

    match g.entries {
        Some(Ok(entries)) => {
            if p.live {
                app.live_edge = entries.first().cloned();
            }
            app.entries = entries;
            app.mark_online(now);
        }
        // Keep the last-known readings on screen; just flag the outage. A
        // config-level failure (bad token / URL) is not a transient outage —
        // App pauses retries after a few of them.
        Some(Err(e)) => {
            let permanent = e.downcast_ref::<FetchError>().is_some();
            app.mark_offline(now, e.to_string(), permanent);
        }
        None => {}
    }

    if app.online {
        // Which supplementary reads failed. They're best-effort — the glucose
        // trace is what matters — but silently showing a stale IOB or a missing
        // carb marker as if it were current is its own kind of wrong, so the
        // dashboard says which part is missing.
        let mut missing: Vec<&str> = Vec::new();
        match g.treatments {
            Some(Ok(t)) => app.treatments = t,
            Some(Err(_)) => missing.push("treatments"),
            None => {}
        }
        if p.live {
            // One devicestatus fetch feeds both the uploader panel and the
            // forecast; falling back to the local AR2 projection when the
            // uploader publishes none (or the fetch failed).
            let published = match g.device {
                Some(Ok((status, predicted))) => {
                    app.device = status;
                    predicted
                }
                _ => {
                    missing.push("device");
                    None
                }
            };
            app.predictions = published.unwrap_or_else(|| predict::ar2(&app.entries));
            match g.sensor_start {
                Some(Ok(started)) => app.sensor_start_ms = started,
                _ => missing.push("sensor age"),
            }
        } else {
            app.predictions.clear();
        }
        match g.agp {
            Some(Ok(entries)) => {
                app.agp_entries = entries;
                app.agp_fetched_ms = now;
            }
            Some(Err(_)) => missing.push("history"),
            None => {}
        }
        match g.minimap {
            Some(Ok(entries)) => app.minimap_entries = entries,
            Some(Err(_)) => missing.push("overview"),
            None => {}
        }
        if let Some(live) = g.live_edge {
            app.live_edge = live.first().cloned();
        }
        app.set_partial(&missing);
    } else if !p.live {
        app.predictions.clear();
    }

    if let Some(f) = g.followers {
        app.followers = f;
    }

    // Alert evaluation and notifications run every refresh, online or not, so a
    // sensor gap still escalates to a Stale alarm.
    app.evaluate_alert(now);
    if app.alerts.desktop {
        if let Some(a) = app.take_notification() {
            // A discarded result meant "Desktop: on" could be a lie: with no
            // notification daemon running the D-Bus call fails and nothing is
            // shown, which — paired with a failing audio player — is two dead
            // channels reported as healthy.
            app.notify_failed = !notify(
                a,
                app.live_latest().map(|e| e.sgv),
                app.units,
                app.alerts.notify_content,
            );
        }
    }
    app.update_urgent(now);
    if let Some(msg) = app.take_predictive(now) {
        if app.alerts.desktop {
            if app.alerts.notify_content {
                let _ = notify_text(&msg);
            } else {
                let _ = notify_text("alert — open sugarrush");
            }
        }
    }
    app.take_push(now)
        .zip(app.alerts.push_url.clone())
        .map(|(msg, url)| (url, msg))
}

/// Hands refreshes to background tasks so the run loop never awaits the
/// network.
///
/// Two properties matter for safety. It never blocks: `request` returns
/// immediately, so keys, the redraw and the 3-second alarm ticker keep running
/// through a Nightscout that has stopped answering. And it never queues: while
/// a fetch is out, further requests set `pending` instead of spawning, so
/// holding down `h` to pan can't stack a task per keypress behind a 12-second
/// timeout.
#[derive(Clone)]
struct Fetcher {
    client: Client,
    tx: mpsc::UnboundedSender<(Plan, Gathered)>,
    /// Background failures that need to reach `app.last_error` — the push
    /// webhook is fired off-loop, and a dead URL must not fail silently.
    errors: mpsc::UnboundedSender<String>,
    in_flight: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
}

impl Fetcher {
    fn new(
        client: Client,
        tx: mpsc::UnboundedSender<(Plan, Gathered)>,
        errors: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            client,
            tx,
            errors,
            in_flight: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Snapshot what to fetch and start it. Returns at once.
    fn request(&self, app: &mut App) {
        // Planning is synchronous and updates the visible window, so the
        // redraw that follows this keypress is already correct — only the data
        // arrives late.
        let p = plan(app, now_ms());
        if self.in_flight.swap(true, Ordering::SeqCst) {
            self.pending.store(true, Ordering::SeqCst);
            return;
        }
        let (client, tx, in_flight) =
            (self.client.clone(), self.tx.clone(), self.in_flight.clone());
        tokio::spawn(async move {
            let g = gather(&client, &p).await;
            in_flight.store(false, Ordering::SeqCst);
            let _ = tx.send((p, g));
        });
    }

    /// Fold a finished fetch into `App`, and start the next one if the view
    /// moved while this was in flight.
    fn deliver(&self, app: &mut App, p: Plan, g: Gathered) {
        if let Some((url, msg)) = apply(app, &p, g) {
            let errors = self.errors.clone();
            tokio::spawn(async move {
                if !push(&url, &msg).await {
                    let _ = errors.send("push notification failed — check push_url".to_string());
                }
            });
        }
        if self.pending.swap(false, Ordering::SeqCst) {
            self.request(app);
        }
    }
}

/// Fetch and apply in one go. Used on the paths that genuinely have nothing
/// else to do while waiting — startup, a site switch, an explicit `r`.
async fn refresh(app: &mut App, client: &Client) {
    let p = plan(app, now_ms());
    let g = gather(client, &p).await;
    if let Some((url, msg)) = apply(app, &p, g) {
        // The push webhook is a safety channel (unacknowledged-urgent
        // escalation); a dead URL must not fail silently.
        if !push(&url, &msg).await {
            app.last_error = Some("push notification failed — check push_url".to_string());
        }
    }
}

/// Fire a best-effort desktop notification for an alert.
pub(crate) fn notify(
    alert: alert::Alert,
    sgv: Option<f64>,
    units: units::Units,
    content: bool,
) -> bool {
    // Content-free mode still fires — and still as critical, so it breaks
    // through Do Not Disturb — but says nothing a lock screen shouldn't show.
    if !content {
        return desktop_notify("alert — open sugarrush", alert.urgency() == "critical");
    }
    let body = match sgv {
        Some(v) => format!("{} · {} {}", alert.label(), units.format(v), units.label()),
        None => alert.label().to_string(),
    };
    desktop_notify(&body, alert.urgency() == "critical")
}

/// Fire a plain desktop notification (used for predictive alerts).
pub(crate) fn notify_text(body: &str) -> bool {
    desktop_notify(body, false)
}

/// Cross-platform desktop notification (Linux / macOS / Windows) via
/// notify-rust. Best-effort — errors are ignored.
fn desktop_notify(body: &str, critical: bool) -> bool {
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
    n.show().is_ok()
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
/// Report coerced config values on stderr. Silence here would just be a
/// quieter version of the bug: the user believes the thresholds they wrote are
/// the thresholds in force.
pub(crate) fn warn_about_config(warnings: &[String]) {
    for w in warnings {
        eprintln!("sugarrush: config: {w}");
    }
    // The person running the daemon is the one least likely to open the TUI,
    // and so the one who never learned their token file went group-readable.
    if Config::perms_too_open() {
        eprintln!(
            "sugarrush: config.toml is readable by others — run: chmod 600 ~/.config/sugarrush/config.toml"
        );
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let cfg = Config::demo();
        let alerts = cfg.alerts.resolve(cfg.units);
        let sites = cfg.resolve_sites().unwrap();
        let mut a = App::new(&cfg, alerts, sites);
        a.demo = false;
        a
    }

    fn fetcher(site: &config::Site) -> (Fetcher, mpsc::UnboundedReceiver<(Plan, Gathered)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (etx, _) = mpsc::unbounded_channel();
        (Fetcher::new(Client::for_site(site).unwrap(), tx, etx), rx)
    }

    /// The run loop used to `await` the whole fetch chain inside its `select!`,
    /// so a Nightscout that accepted the connection and then went quiet froze
    /// keyboard input, the redraw and the 3-second alarm ticker for as long as
    /// the request timeouts took — up to a minute across five sequential reads.
    ///
    /// `request` must return immediately against exactly that server.
    #[tokio::test]
    async fn a_stalled_site_does_not_block_the_caller() {
        let site = nightscout::fake::serve_stalled().await;
        let (fetch, _rx) = fetcher(&site);
        let mut app = app();

        let t = std::time::Instant::now();
        fetch.request(&mut app);
        let elapsed = t.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "request() blocked for {elapsed:?} on a stalled site — the run loop \
             would have been frozen for that long, alarm included"
        );
        assert!(
            fetch.in_flight.load(Ordering::SeqCst),
            "the fetch should be out on its own task"
        );
    }

    /// Holding `h` to pan fires a refresh per keypress. Each one used to be
    /// awaited in turn; spawning them instead would just move the pile-up onto
    /// the task queue, with a 12-second timeout apiece. At most one is in
    /// flight, and the rest collapse into a single follow-up.
    #[tokio::test]
    async fn requests_made_while_a_fetch_is_out_collapse_into_one() {
        let site = nightscout::fake::serve_stalled().await;
        let (fetch, mut rx) = fetcher(&site);
        let mut app = app();

        for _ in 0..10 {
            fetch.request(&mut app);
        }

        assert!(fetch.in_flight.load(Ordering::SeqCst));
        assert!(
            fetch.pending.load(Ordering::SeqCst),
            "the later requests should be remembered, not dropped"
        );
        assert!(
            rx.try_recv().is_err(),
            "nothing can have been delivered — the site never answered"
        );
    }

    /// Planning is synchronous and moves the visible window, so the redraw that
    /// follows a keypress is correct even though the data arrives later.
    #[tokio::test]
    async fn planning_moves_the_window_before_the_fetch_returns() {
        let site = nightscout::fake::serve_stalled().await;
        let (fetch, _rx) = fetcher(&site);
        let mut app = app();
        app.view_start = 0;
        app.view_end = 0;

        fetch.request(&mut app);

        assert!(
            app.view_end > app.view_start,
            "the window should already be set when request() returns"
        );
    }
}
