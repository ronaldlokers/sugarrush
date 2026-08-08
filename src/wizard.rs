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
    println!("  ⚠ Not a medical device. Don't use it for treatment decisions —");
    println!("  always confirm with your meter, pump, or official app.\n");
    println!("  No config found. Let's connect to your Nightscout site.");
    println!("  Use a read-only token (Nightscout → Admin Tools → Subject with the");
    println!("  `readable` role). Not your API_SECRET.\n");

    loop {
        let typed = prompt("  Nightscout URL (https://…): ")?;
        // The token is echoed nowhere: it's the one secret in this flow, and a
        // terminal scrollback (or a shoulder) outlives the setup session.
        let token = prompt_secret("  Read-only token: ")?;
        if typed.is_empty() || token.is_empty() {
            println!("  Both the URL and token are required.\n");
            continue;
        }
        let url = match crate::config::normalize_site_url(&typed) {
            Ok(u) => u,
            Err(e) => {
                println!("  {e}\n");
                continue;
            }
        };
        if url != typed {
            println!("  Using {url}");
        }

        let site = Site {
            name: "default".to_string(),
            url: url.clone(),
            token: token.clone(),
        };
        // Plain http sends the token — and the glucose data — in the clear.
        // Refuse by default, but a self-hosted site on a trusted LAN is a real
        // setup, so let the user say so explicitly.
        if site.is_insecure() && !confirm_insecure()? {
            continue;
        }
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

/// Read a line without echoing it, for secrets. Falls back to a normal
/// (echoing) prompt if the terminal won't switch to raw mode.
fn prompt_secret(label: &str) -> Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    print!("{label}");
    io::stdout().flush().ok();
    if enable_raw_mode().is_err() {
        return prompt("");
    }
    let mut buf = String::new();
    let outcome = loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Enter => break Ok(()),
                    KeyCode::Char('c') | KeyCode::Char('d') if ctrl => {
                        break Err(anyhow::anyhow!("setup cancelled"))
                    }
                    KeyCode::Backspace => {
                        if buf.pop().is_some() {
                            // Erase the masking character too.
                            print!("\u{8} \u{8}");
                            io::stdout().flush().ok();
                        }
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        print!("•");
                        io::stdout().flush().ok();
                    }
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(e) => break Err(anyhow::Error::new(e).context("failed to read input")),
        }
    };
    let _ = disable_raw_mode();
    println!();
    outcome?;
    Ok(buf.trim().to_string())
}

/// Confirm an unencrypted site, which sends the token in clear text.
fn confirm_insecure() -> Result<bool> {
    println!("  ⚠ That URL is plain http:// — the token and your glucose data");
    println!("    travel unencrypted, readable by anything on the network path.");
    let ans = prompt("    Type 'insecure' to use it anyway, or press Enter to re-enter: ")?;
    if ans.eq_ignore_ascii_case("insecure") {
        Ok(true)
    } else {
        println!();
        Ok(false)
    }
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

/// Write a minimal config.toml, atomically and owner-only.
fn write_config(path: &Path, url: &str, token: &str, units: &str) -> Result<()> {
    Config::write_atomic(path, &config_body(url, token, units)?)
}

/// Render the starter config. The values are serialized by the `toml` crate
/// rather than interpolated into a string: a URL or token containing a quote,
/// backslash, or newline would otherwise break the file — or inject arbitrary
/// extra keys into it, silently overriding settings from a pasted value.
fn config_body(url: &str, token: &str, units: &str) -> Result<String> {
    let mut table = toml::Table::new();
    table.insert("url".into(), url.into());
    table.insert("token".into(), token.into());
    table.insert("units".into(), units.into());
    table.insert("refresh_secs".into(), 30.into());
    let body = toml::to_string_pretty(&table).context("failed to serialize config")?;
    Ok(format!(
        "# sugarrush config — created by first-run setup\n{body}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_values_cannot_inject_config_keys() {
        // A token containing TOML syntax must survive as *data*, not become
        // extra keys that silently override real settings.
        let token = "abc\"\nrefresh_secs = 1\nurl = \"https://evil.example\"\n#";
        let body = config_body("https://ns.example", token, "mmol").unwrap();
        let cfg: Config = toml::from_str(&body).unwrap();
        assert_eq!(cfg.token.as_deref(), Some(token));
        assert_eq!(cfg.url.as_deref(), Some("https://ns.example"));
        assert_eq!(cfg.refresh_secs, 30);
    }
}
