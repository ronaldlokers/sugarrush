# Changelog

All notable changes to sugarrush are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[CalVer](https://calver.org/) `YYYY.M.N` (the `N` resets each month).

## [Unreleased]

## [2026.7.1] - 2026-07-17

First public release. A fast, keyboard-driven terminal UI for viewing
self-hosted [Nightscout](https://nightscout.github.io/) CGM data.

### Added

- **Dashboard** — big current value with trend arrow, delta, and a colour +
  text range label; stats panel with time-in-range, mean glucose + GMI,
  insulin-/carbs-on-board, and device status (battery, sensor age, last seen).
- **History & forecast** — braille/dot graph you can pan (`h`/`l`), zoom
  (`+`/`-`, 1h–24h), and jump to a date (`g`); a 24h minimap you click or drag;
  a short-term forecast overlay (uploader predictions or a local AR2 fallback)
  with a "now" line, a time-to-low/high ETA, and predictive alerts; carb and
  bolus markers on the graph.
- **Alerts & safety** — in-TUI banner plus cross-platform desktop notifications
  (Linux/macOS/Windows); an audible alarm for urgent lows/highs with snooze,
  per-level tones, quiet hours, and unacknowledged-alarm escalation (optional
  phone push); clear offline vs. sensor-gap states with backoff retry.
- **Configuration** — an in-app settings screen (`s`) grouped into sections,
  editing units, refresh, thresholds, alarms, and theme live and saving back to
  `config.toml`; configurable colours with a colourblind-safe preset; multiple
  Nightscout sites (`n` to switch); a first-run setup wizard.
- **Elsewhere** — a Waybar module (`sugarrush waybar`) with a sparkline tooltip
  and click-through; `sugarrush --demo` to try the app on synthetic data with
  no config or network.
- **Distribution** — published to crates.io, the AUR (`sugarrush-bin`), and a
  Homebrew tap; prebuilt binaries + shell/PowerShell installers via cargo-dist.

[Unreleased]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.1...HEAD
[2026.7.1]: https://github.com/ronaldlokers/sugarrush/releases/tag/v2026.7.1
