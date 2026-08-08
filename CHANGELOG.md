# Changelog

All notable changes to sugarrush are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[CalVer](https://calver.org/) `YYYY.M.N` (the `N` resets each month).

## [Unreleased]

### Added

- **Step through history a day at a time** with `[` and `]`, keeping the same
  time of day — checking "how was last night?" no longer means typing a date or
  panning there a half-window at a time.
- **The AGP now names its patterns.** Reading a recurring overnight low off a
  percentile fan is a skill; the AGP title now states the worst finding
  outright — `⚠ lows 02:00–05:00 (down to 3.1 mmol/L)` — and the exported
  summary lists every one under a **Patterns** section. A "lows" window is a
  time of day where a quarter of readings or more sit below target; "highs" is
  where the typical reading is above it. Runs shorter than 45 minutes aren't
  reported, and a gap in the data never joins two windows into one.
- **Follow more than one person.** With several `[[sites]]` configured, `m`
  opens a follower view listing everyone at once — value, trend, how old the
  reading is, and the alert state — sorted worst first, with a header that names
  whoever needs attention. A site that can't be read ranks with the urgent ones
  rather than showing a blank row that reads like "fine". `sugarrush watch` now
  watches every configured site too, each with its own independent alert
  episode, naming the site in notifications and in the log.
- **Status-bar output for bars other than Waybar.** `sugarrush status` prints
  one line in the syntax your bar speaks — `--format text` (no markup),
  `tmux`, `polybar`, `i3blocks`, or `waybar` — coloured from your configured
  theme, so the colourblind-safe palette carries over. `sugarrush waybar` is
  unchanged and still prints the same JSON.
- **An always-on alarm watcher.** `sugarrush watch` runs the alert pipeline
  headless — no terminal needed — so a nocturnal low still wakes you when the
  dashboard isn't open. It defers to a running dashboard (both write a
  heartbeat, so you never get two alarms for one low) and persists episode
  state, so restarting the service doesn't re-announce an ongoing low, restart
  an escalation timer, or cancel a snooze. Example systemd user unit in
  `packaging/systemd/`.
- **Export what you're looking at.** Press `e` (or run `sugarrush export`) to
  write two files for the clinical window: a CSV of every reading — oldest
  first, in mg/dL *and* your display unit — and a plain-text summary with sensor
  coverage, five-band time in range, time below range, mean, GMI, CV, and an
  hour-by-hour median/spread profile. Meant for sending to a clinician or
  opening in a spreadsheet, instead of screenshotting a terminal.
  `sugarrush export --days 30 --out ~/` for a different window or directory.

### Fixed

- **The declared minimum Rust version was wrong** — `rust-version` said 1.82,
  but the dependency tree hasn't built on anything below **1.89** for a while,
  so `cargo install` could fail with a confusing dependency error instead of a
  clear "your toolchain is too old". CI now builds against the declared MSRV
  every run, so it can't drift again.
- **Half the uploader requests are gone.** Each refresh fetched
  `/devicestatus` twice — once for battery/IOB/COB and once for the forecast.
  It's now one request, which also rules out the two halves coming from
  different records if the uploader posts in between.

## [2026.8.1] - 2026-08-08

This release is about trusting what's on screen. The alarm path got a
correctness pass — the newest reading is now genuinely the newest, alerts no
longer re-fire every time a value hovers on a threshold, a flat glucose no
longer predicts a low, and a bad token says so and stops retrying instead of
looking like a network outage. Saving settings can no longer lose your
Nightscout token, the site URL and token are now editable in the app (masked,
and no longer echoed by the setup wizard), and an unencrypted `http://` site is
called out rather than accepted silently. The stats panel turned clinical —
five-band time-in-range with time-below-range and CV variability, over a fixed
14-day window — and history is now fully navigable from the keyboard.

### Added

- **Keyboard navigation for the overview strip** — `H` / `L` (or `PgUp` /
  `PgDn`) pan a whole window at a time and `End` jumps to the oldest edge of the
  overview, so moving through history no longer needs a mouse.
- **Clinical time-in-range.** The stats panel now splits readings across the
  five consensus bands (very low / low / in range / high / very high) instead of
  three, calls out the **time below range** percentage — the number that changes
  treatment — and reports **CV**, the standard measure of glycaemic variability,
  highlighted when it exceeds the 36% consensus target.
- **Content-free notifications** — a new `Notification detail` setting
  (`notify_content`) keeps desktop notifications free of your reading and alert
  state, for lock screens and shared displays. They still fire, and urgent ones
  are still critical.
- **Unsaved settings are visible.** Settings apply live but only `w` writes them
  to `config.toml`; the settings header now says `· unsaved changes (w to save)`
  so quitting can't silently discard them.
- **Fix a bad site without leaving the app** — the settings screen gains a
  **Site** section with the Nightscout URL and read-only token. Press `Enter` on
  a row to edit it; the app reconnects immediately, and `w` saves as usual. The
  token is masked while typing and never rendered back (the row reads
  `set · ••••••`).
- **Unencrypted sites are called out.** A plain `http://` site sends your token
  and readings in the clear, so the dashboard footer now says so and the setup
  wizard asks you to confirm before accepting one. Loopback addresses
  (`localhost`, `127.0.0.1`) are exempt — they never leave the machine.
- **Push alerts are now a settings row** (`Alarm → Push alerts`) — toggle the
  urgent-alert webhook on or off in the app without deleting the configured
  `push_url`. The row shows only the host, never the full topic/path, and the
  new `push_enabled` config key persists the choice.

### Fixed

- **`Esc` no longer quits the app.** From the dashboard it returns to the live
  edge (and closes the help overlay or a prompt, as before) — quitting is `q`,
  which is what every other screen already did.
- **The current-value pane keeps its range label when compact.** On a short
  terminal the colour-independent label (`LOW`, `in range`) was pushed onto a
  line that got clipped, leaving colour as the only cue; it now sits on the
  headline next to the reading.
- **The overview strip says why it's empty** — "loading overview…" before the
  first fetch, "no readings in this window" after — instead of drawing a bare
  box.
- **The audible alarm falls back to the terminal bell** when no system audio
  player is available (headless boxes, minimal containers, a bare SSH login),
  instead of failing silently — silence is indistinguishable from "glucose is
  fine".
- **The setup wizard no longer echoes your token** to the terminal — it's masked
  as you type, so it doesn't survive in the scrollback.
- **URLs are normalized instead of failing mysteriously.** A bare
  `mysite.example.com`, a trailing slash, or a pasted
  `…/api/v1/entries.json?count=10` now resolves to the right base URL, in the
  wizard, in the settings editor, and when loading an existing `config.toml`.
- **A partly-failed refresh now says so.** When the readings arrive but a
  companion fetch doesn't, the affected panels (IOB/COB, treatment markers,
  forecast, sensor age, history, overview) were left showing old values with
  the connection still reading as healthy. The footer now names what's stale.
- **The connection recovers while you're browsing history.** Backoff retries
  were skipped unless you were at the live edge, so an outage that started
  while you were panning stayed until you refreshed by hand. The backoff also
  now actually reaches its documented 60s ceiling (it stopped at 40s).
- **No more "heading low in ~0 min" from a stale forecast.** Uploader forecasts
  are timestamped when the pump published them; an old one is entirely in the
  past, and its points were being reported as an imminent crossing. Past points
  are now skipped.
- **A delta too small to show no longer renders as `-0.0`** in the dashboard or
  the Waybar module — the sign follows the value as displayed, so a flat trend
  reads flat.
- **The predict-horizon setting is honest about its reach** — above 30 minutes
  it notes that the local (AR2) forecast only projects that far, so a longer
  horizon needs uploader predictions from Loop or OpenAPS.
- **Saving settings can no longer lose your token.** `config.toml` is now
  written to a temp file (created owner-only, flushed to disk) and renamed into
  place, instead of being truncated and rewritten — so a crash, a full disk, or
  a power cut mid-save leaves the old config intact rather than an empty file
  with the only copy of your Nightscout token gone.
- **The setup wizard no longer builds `config.toml` by string interpolation** —
  values are serialized by the TOML library, so a URL or token containing a
  quote or newline can't corrupt the file or inject extra settings into it. The
  file is also created with `0600` from the start, rather than being chmodded
  after the token was already written.
- **The audible alarm no longer leaks processes.** Each alarm spawns a system
  audio player; those were never reaped, so a long overnight urgent state piled
  up hundreds of zombie processes. Finished players are now cleaned up on every
  play.
- **Alert thresholds can't cross any more** — urgent-low ≤ low ≤ high ≤
  urgent-high is enforced while editing, so you can't silently disable a band
  by dragging one threshold past its neighbour.
- **The newest reading is now always the newest reading** — entries are sorted
  by timestamp on arrival instead of trusting the order Nightscout (or a proxy
  or mirror in front of it) happened to return, so the current value, delta,
  staleness check, and forecast can't silently key off an older reading.
- **Alerts no longer flap on a threshold.** A reading hovering on a boundary
  (69 → 71 → 69, well inside CGM noise) used to re-fire the notification, the
  audible alarm, and the push webhook every single time. Clearing an alert now
  requires moving 4 mg/dL past the threshold; raising one is unchanged, so a
  real low still alarms on the first reading that crosses.
- **A flat glucose no longer predicts a low.** The predictive alert and its ETA
  now follow the centre of the forecast rather than the edge of the uncertainty
  cone, which widens with the horizon and so crossed a threshold even when
  glucose was perfectly steady.
- **A bad token or URL now says so and stops retrying** instead of being
  retried forever as if it were a network outage. After three consecutive
  authentication (401/403) or not-found (404) responses, automatic fetching
  pauses with an explanatory message; press `r`, or switch/edit the site, to
  resume.
- **Stats are now clinical, not cosmetic** — time-in-range, mean, and GMI are
  computed over a fixed window of the last N days (the `AGP days` setting,
  default 14 — the clinical standard) instead of whatever slice of the graph
  happened to be on screen, so panning or zooming no longer changes them. The
  stats panel title names the window (e.g. `stats · 14d`).

## [2026.7.3] - 2026-07-18

This release hardens the safety-critical alarm path and reworks the graphs. The
audible alarm can no longer stall silently — the Nightscout client now times
out, a total sensor dropout raises a Stale alarm, sensor-error codes no longer
fire a false urgent-low, and a wrong token says so instead of reading as
"offline". Visually, both the AGP and the short-term forecast now render as
filled percentile/uncertainty bands, chart shading lines up with the axes, the
colourblind palette renders on every terminal (including tmux/SSH), and there's
a `?` keybinding overlay. It also adds a first-run units prompt and an in-app
"not a medical device" reminder.

### Added

- The **AGP view** now renders as a filled percentile fan (a shaded 5–95 and a
  brighter inter-quartile band) with a single bright median line, and its title
  shows the target range — much closer to a clinical AGP than the previous flat
  lines.
- A **keybinding help overlay** — press `?` for a full cheatsheet. The footer
  now falls back to a terse hint set on narrow terminals (it previously clipped
  silently, hiding settings/site/snooze) while always keeping `? help` visible.
- The dashboard footer now shows a **snooze indicator** with a countdown while
  the audible alarm is silenced, so it's clear the alarm is off and for how long.

### Changed

- The **forecast cone** on the timeline is now a filled low–high band (matching
  the AGP fan) with the centre line drawn on top, instead of two dim edge lines,
  and it emanates from the last reading (the AR2 fallback's initial jump no
  longer leaves the fan floating above the current dot).
- **In-app safety note** — the "not a medical device" reminder now appears in
  the running app (a dim note in the header, and up front in the first-run
  wizard), not only in the README and `about`.
- **Alert banner** now follows the theme/colourblind palette (it was hardcoded
  red/yellow), and the selected settings row is highlighted full-width.
- **Dashboard polish** — the range bar now shows all four zones (urgent-low /
  low / in-range / high / urgent-high) and uses integer mg/dL labels; IOB and
  COB stand out from the dimmed device line; and the graph labels its carb and
  bolus markers with a small legend.

### Fixed

- **Chart background tints now line up with the chart.** The in-range band, the
  AGP fan, and the forecast fill are painted onto the terminal buffer, and the
  plot-rect geometry didn't match ratatui's — it was two rows too tall and used
  the wrong left gutter, so the shading spilled past the axes and sat offset from
  the lines/points it shades (the forecast fan looked detached from the current
  reading). The geometry now replicates ratatui's chart layout exactly.
- **Graph theming** — the shaded in-range band is now derived from the in-range
  palette colour (it was hardcoded green, so it broke the colourblind preset),
  and the y-axis shows integer mg/dL values instead of spurious decimals.
- **Setup** — the first-run wizard now asks for the display unit (mmol/L or
  mg/dL) instead of always defaulting to mmol/L, so mg/dL users aren't dropped
  into the wrong unit.
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

[Unreleased]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.3...HEAD
[2026.7.3]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.2...v2026.7.3
[2026.7.2]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.1...v2026.7.2
[2026.7.1]: https://github.com/ronaldlokers/sugarrush/releases/tag/v2026.7.1
