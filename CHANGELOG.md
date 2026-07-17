# Changelog

All notable changes to sugarrush are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[CalVer](https://calver.org/) `YYYY.M.N` (the `N` resets each month).

## [Unreleased]

### Added

- The dashboard footer now shows a **snooze indicator** with a countdown while
  the audible alarm is silenced, so it's clear the alarm is off and for how long.

### Fixed

- **Accessibility** — the colourblind-safe palette now uses named ANSI colours
  instead of truecolor hex, so it renders correctly on 16/256-colour terminals,
  tmux, and SSH sessions that lack truecolor (where it previously collapsed
  silently). The current reading is also exposed as plain text alongside the
  big-number glyphs, so screen readers, tmux copy, and braille displays can read
  it.
- **Clearer connection errors** — a wrong or non-readable token now reports
  "authentication failed — check your read-only token (not API_SECRET)" instead
  of a generic "offline", both at runtime (in the header) and during first-run
  setup; unreachable hosts and HTTP errors are also distinguished.
- **Alarm responsiveness** — a sensor gap now escalates to a Stale alarm within
  seconds (re-checked on the alarm tick) instead of waiting for the next full
  refresh, and a failed escalation push (dead `push_url`) is surfaced instead of
  swallowed silently.
- **Alarm reliability** — the audible alarm could stop working silently in
  several cases, now fixed: the Nightscout client had no request timeout (a
  stalled connection froze input and the alarm), a total sensor dropout read as
  "in range" instead of a sensor gap, Nightscout sensor-error codes (0–12) were
  read as a real reading and could fire a false urgent-low, and predictive
  alerts evaluated the previous refresh's forecast. Failed data fetches no
  longer pile up doomed follow-up requests.

## [2026.7.2] - 2026-07-17

This release is a dashboard glow-up. The graph now colour-codes readings by zone
with a shaded in-range band and dashed threshold rails, adds a zoned range bar
under the current value, and gains a switchable **AGP** (ambulatory glucose
profile) view alongside the 3h/24h timelines. The stats panel picks up a
time-in-range bar and a mean sparkline, and short-term forecasts now render as
an **uncertainty cone** — a high/low band — instead of a single line.

### Added

- **Graph view tabs** (`Tab` / `Shift+Tab`) — switch the graph pane between a
  3h or 24h timeline and an **AGP** (ambulatory glucose profile) that folds the
  last N days of readings onto a 24h clock as a percentile band (median +
  25/75 + 5/95). The number of days is configurable in settings (`AGP days`).
- **Dashboard graph glow-up** — readings are colour-coded by zone
  (low/in-range/high) with dashed reference rails at the low/high thresholds,
  the in-range region is shaded as a band behind the trace, and a zoned range
  bar under the big current value shows where it sits between the thresholds.
- **Stats upgrade** — time-in-range is drawn as a stacked zone bar, and the
  mean gets an inline sparkline of recent readings.

### Changed

- **Forecast is now an uncertainty cone** — predictions render as a widening
  high/low band (the plausible range) instead of a single line; the
  time-to-low/high ETA warns on the worst plausible path.

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

[Unreleased]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.2...HEAD
[2026.7.2]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.1...v2026.7.2
[2026.7.1]: https://github.com/ronaldlokers/sugarrush/releases/tag/v2026.7.1
