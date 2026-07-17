//! First-run interactive setup.
//!
//! When there's no config yet and we're on a terminal, walk the user through
//! entering a Nightscout URL + read-only token, live-test the connection, and
//! write a `config.toml`. Plain line-based stdio — runs before the TUI starts.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::{Config, Site};
use crate::nightscout::Client;

/// Run the wizard, writing `config.toml` on success.
pub async fn run() -> Result<()> {
    let path = Config::path()?;
    println!("\n  sugarrush — first-run setup");
    println!("  No config found. Let's connect to your Nightscout site.");
    println!("  Use a read-only token (Nightscout → Admin Tools → Subject with the");
    println!("  `readable` role). Not your API_SECRET.\n");

    loop {
        let url = prompt("  Nightscout URL (https://…): ")?;
        let token = prompt("  Read-only token: ")?;
        if url.is_empty() || token.is_empty() {
            println!("  Both the URL and token are required.\n");
            continue;
        }

        let site = Site {
            name: "default".to_string(),
            url: url.clone(),
            token: token.clone(),
        };
        print!("  Testing connection… ");
        io::stdout().flush().ok();
        match test(&site).await {
            Ok(()) => {
                println!("ok");
                let units = prompt_units()?;
                write_config(&path, &url, &token, units)?;
                println!("\n  Saved to {}. Launching…\n", path.display());
                return Ok(());
            }
            Err(e) => {
                println!("failed");
                println!("  {e}");
                println!("  Check the URL and token and try again (Ctrl+C to quit).\n");
            }
        }
    }
}

/// Read a trimmed line; error on EOF (Ctrl+D) so the caller can exit cleanly.
fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().ok();
    let mut line = String::new();
    let n = io::stdin()
        .read_line(&mut line)
        .context("failed to read input")?;
    if n == 0 {
        bail!("setup cancelled");
    }
    Ok(line.trim().to_string())
}

/// Ask for the display unit; defaults to mmol/L. Toggleable later with `u`.
fn prompt_units() -> Result<&'static str> {
    let ans = prompt("  Units — [1] mmol/L   [2] mg/dL   (default 1): ")?;
    let a = ans.to_lowercase();
    Ok(if a == "2" || a == "mgdl" || a == "mg/dl" {
        "mgdl"
    } else {
        "mmol"
    })
}

/// Verify the site by fetching one recent entry.
async fn test(site: &Site) -> Result<()> {
    let client = Client::for_site(site)?;
    let now = chrono::Utc::now().timestamp_millis();
    client.entries_range(now - 3_600_000, now, 1).await?;
    Ok(())
}

/// Write a minimal config.toml with restrictive permissions.
fn write_config(path: &Path, url: &str, token: &str, units: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let body = format!(
        "# sugarrush config — created by first-run setup\n\
         url = \"{url}\"\n\
         token = \"{token}\"\n\
         units = \"{units}\"\n\
         refresh_secs = 30\n",
    );
    std::fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    set_owner_only(path);
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}
