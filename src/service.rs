//! Install, inspect, and remove the always-on watcher using the native user
//! service manager on each supported platform.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Install,
    Status,
    Uninstall,
}

pub fn run(action: Action) -> Result<()> {
    let exe = std::env::current_exe().context("could not find this binary's path")?;
    platform(action, &exe)
}

#[cfg(target_os = "linux")]
fn platform(action: Action, exe: &Path) -> Result<()> {
    let dir = dirs::config_dir()
        .context("could not resolve the user config dir")?
        .join("systemd/user");
    let path = dir.join("sugarrush-watch.service");
    match action {
        Action::Install => {
            std::fs::create_dir_all(&dir)?;
            crate::config::Config::write_atomic(&path, &systemd_unit(exe))?;
            command("systemctl", &["--user", "daemon-reload"])?;
            command(
                "systemctl",
                &["--user", "enable", "--now", "sugarrush-watch.service"],
            )?;
            println!("watcher installed and started: {}", path.display());
        }
        Action::Status => {
            command(
                "systemctl",
                &["--user", "status", "sugarrush-watch.service", "--no-pager"],
            )?;
        }
        Action::Uninstall => {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "--now", "sugarrush-watch.service"])
                .status();
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            command("systemctl", &["--user", "daemon-reload"])?;
            println!("watcher service removed");
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform(action: Action, exe: &Path) -> Result<()> {
    let dir = dirs::home_dir()
        .context("could not resolve the home directory")?
        .join("Library/LaunchAgents");
    let path = dir.join("com.sugarrush.watch.plist");
    let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?;
    let domain = format!("gui/{}", uid.trim());
    let target = format!("{domain}/com.sugarrush.watch");
    match action {
        Action::Install => {
            std::fs::create_dir_all(&dir)?;
            crate::config::Config::write_atomic(&path, &launchd_plist(exe))?;
            let _ = Command::new("launchctl")
                .args(["bootout", &target])
                .status();
            command(
                "launchctl",
                &["bootstrap", &domain, path.to_string_lossy().as_ref()],
            )?;
            command("launchctl", &["kickstart", "-k", &target])?;
            println!("watcher installed and started: {}", path.display());
        }
        Action::Status => command("launchctl", &["print", &target])?,
        Action::Uninstall => {
            let _ = Command::new("launchctl")
                .args(["bootout", &target])
                .status();
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            println!("watcher service removed");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn platform(action: Action, exe: &Path) -> Result<()> {
    let task = "sugarrush-watch";
    match action {
        Action::Install => {
            let command_line = windows_task_command(exe);
            command(
                "schtasks.exe",
                &[
                    "/Create",
                    "/F",
                    "/SC",
                    "ONLOGON",
                    "/RL",
                    "LIMITED",
                    "/TN",
                    task,
                    "/TR",
                    &command_line,
                ],
            )?;
            command("schtasks.exe", &["/Run", "/TN", task])?;
            println!("watcher task installed and started");
        }
        Action::Status => command(
            "schtasks.exe",
            &["/Query", "/TN", task, "/V", "/FO", "LIST"],
        )?,
        Action::Uninstall => {
            command("schtasks.exe", &["/Delete", "/F", "/TN", task])?;
            println!("watcher task removed");
        }
    }
    Ok(())
}

fn command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    anyhow::ensure!(status.success(), "{program} exited with {status}");
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn systemd_unit(exe: &Path) -> String {
    let escaped = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "[Unit]\nDescription=sugarrush CGM alarm watcher\nDocumentation=https://github.com/ronaldlokers/sugarrush\n\n[Service]\nType=simple\nExecStart=\"{escaped}\" watch\nRestart=always\nRestartSec=30\nNoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=read-only\nReadWritePaths=%S/sugarrush %t/sugarrush\nPrivateDevices=yes\nRestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n\n[Install]\nWantedBy=default.target\n"
    )
}

#[cfg(any(target_os = "macos", test))]
fn launchd_plist(exe: &Path) -> String {
    let escaped = exe
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.sugarrush.watch</string>\n<key>ProgramArguments</key><array><string>{escaped}</string><string>watch</string></array>\n<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n<key>ProcessType</key><string>Interactive</string>\n</dict></plist>\n"
    )
}

#[cfg(any(windows, test))]
fn windows_task_command(exe: &Path) -> String {
    let escaped = exe.to_string_lossy().replace('\'', "''");
    format!(
        "powershell.exe -NoProfile -WindowStyle Hidden -Command \"& {{ while ($true) {{ & '{escaped}' watch; Start-Sleep -Seconds 30 }} }}\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_definitions_use_the_actual_binary_and_restart() {
        let exe = Path::new("/opt/sugarrush bin/sugarrush");
        let unit = systemd_unit(exe);
        assert!(unit.contains("ExecStart=\"/opt/sugarrush bin/sugarrush\" watch"));
        assert!(unit.contains("Restart=always"));
        let plist = launchd_plist(exe);
        assert!(plist.contains("<string>/opt/sugarrush bin/sugarrush</string>"));
        assert!(plist.contains("<key>KeepAlive</key><true/>"));
        let task = windows_task_command(exe);
        assert!(task.contains("while ($true)"));
        assert!(task.contains("Start-Sleep -Seconds 30"));
    }
}
