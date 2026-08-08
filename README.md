# sugarrush

**Your [Nightscout](https://nightscout.github.io/) CGM data, in the terminal.**
A fast, keyboard-driven TUI for glanceable blood glucose — live value, history,
forecast, alerts, and stats — built with Rust + [Ratatui](https://ratatui.rs/).

[![CI](https://github.com/ronaldlokers/sugarrush/actions/workflows/ci.yml/badge.svg)](https://github.com/ronaldlokers/sugarrush/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/ronaldlokers/sugarrush?sort=semver)](https://github.com/ronaldlokers/sugarrush/releases/latest)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)

![sugarrush running in demo mode](assets/demo.gif)

> ⚠️ **Not a medical device.** Don't use `sugarrush` for treatment decisions —
> always confirm with your meter, pump, or official app.

## Try it in 5 seconds

No Nightscout, no config, no network — just synthetic data:

```bash
sugarrush --demo
```

That's the recording above. When you're ready, point it at your own site
([configure](#configuration)).

## What it does

**At a glance**
- Big, colour-coded current value with trend arrow, delta, and a plain-text
  range label (readable without colour)
- Time-in-range across the five clinical bands (very low → very high) with
  time-below-range called out, mean glucose, GMI (estimated A1c), and CV
  (glycaemic variability) over a fixed clinical window (last 14 days by
  default), plus device status (battery, sensor age, last seen)
- Insulin-on-board / carbs-on-board, with carb & bolus markers on the graph

**History & forecast**
- Switchable **graph views** (`Tab`) — a 3h or 24h timeline, or an **AGP**
  (ambulatory glucose profile) folding days of readings into a percentile band
- Live braille/dot graph you can **pan** (`h`/`l`), **zoom** (`+`/`-`,
  1h–24h), and **jump to a date** (`g`)
- A 24h **minimap** you click or drag to move the window
- Short-term **forecast cone** (uploader predictions or a local AR2 fallback)
  showing the high/low uncertainty band, with a "now" line and a
  *time-to-low/high* ETA

**Alerts & safety**
- A **headless watcher** (`sugarrush watch`) that keeps alarming with no
  terminal open — the 3am case — and stays quiet while the dashboard is up
- In-TUI banner + cross-platform desktop notifications (Linux/macOS/Windows),
  switchable to **content-free** so nothing readable lands on a lock screen
- **Audible alarm** for urgent lows/highs with snooze, per-level tones,
  **quiet hours**, and unacknowledged-alarm **escalation** (incl. phone push)
- Predictive alerts before a threshold is crossed; offline vs. sensor-gap
  distinction so you know *why* data stopped

**Share it**
- **Export** the clinical window (`e`, or `sugarrush export`) as a CSV of every
  reading plus a plain-text summary — time in range across the five bands, mean,
  GMI, CV, and an hour-by-hour profile — to send to a clinician or open in a
  spreadsheet

**Yours to shape**
- In-app **settings screen** (`s`) — edit units, thresholds, alarms, theme,
  and more live, then save back to `config.toml`; the **site URL and token** are
  editable there too, so a bad token is fixed without leaving the app
- Configurable colours (incl. a colorblind-safe preset), graph style, and
  **multiple sites** (`n` to switch)
- **Status-bar output** for Waybar, tmux, polybar, i3blocks, or anything that
  takes plain text (see [Status bars](#status-bars))

## Install

```bash
# Arch (AUR)
yay -S sugarrush-bin

# Homebrew (macOS/Linux)
brew install ronaldlokers/tap/sugarrush

# crates.io (compiles from source)
cargo install sugarrush

# …or a prebuilt binary via cargo-binstall (no compile)
cargo binstall sugarrush

# …or the shell installer (Linux/macOS) — grabs the right prebuilt binary
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ronaldlokers/sugarrush/releases/latest/download/sugarrush-installer.sh | sh
```

Prebuilt archives (Linux gnu/musl, macOS x86_64/arm64, Windows) are attached to
every [release](https://github.com/ronaldlokers/sugarrush/releases). From a
checkout: `cargo build --release` (binary at `target/release/sugarrush`).

## Configuration

First run with no config launches an interactive setup wizard. Prefer to do it
by hand? Copy the example:

```bash
mkdir -p ~/.config/sugarrush
cp config.example.toml ~/.config/sugarrush/config.toml
chmod 600 ~/.config/sugarrush/config.toml
```

### Nightscout token (read-only)

Do **not** use `API_SECRET` (admin-level). Create a read-only token in
**Nightscout → Admin Tools**:

1. Add a **Subject** (e.g. `sugarrush`).
2. Give it the `readable` role.
3. Copy its access token into `config.toml` as `token`.

It's sent as a `?token=…` query parameter and only grants read access — which
is why the site should be **https**. Over plain `http://` the token and your
readings are visible to anything on the network path; sugarrush warns in the
footer (loopback addresses excepted). The URL itself is forgiving: a bare
`mysite.example.com` or a pasted `…/api/v1/entries.json` is normalized to the
base URL.

### Token storage & permissions

The token is stored **in plaintext** in `config.toml`. It's read-only (exposes
your glucose data, not account control), but keep the file private —
`chmod 600`. The setup wizard already does this, and sugarrush warns in the
footer if the file is group/world-readable. No `token_cmd`/env indirection by
design: file-only, documented.

## Keybindings

| Key | Action |
|-----|--------|
| `q` | Quit |
| `?` | Toggle the keybinding help overlay |
| `r` | Refresh now (also resumes fetching after a token/URL error) |
| `u` | Toggle mg/dL ↔ mmol/L |
| `Tab` / `Shift+Tab` | Switch graph view (3h / 24h / AGP) |
| `h` / `←` · `l` / `→` | Pan back / forward in time |
| `H` / `L` · `PgUp` / `PgDn` | Pan a whole window at a time |
| `+` / `-` | Zoom window (1h/3h/6h/12h/24h) |
| `g` | Jump to a date (`YYYY-MM-DD`) |
| `End` | Jump to the start of the overview strip |
| `f` / `Home` / `Esc` | Return to live |
| `e` | Export the clinical window (CSV + summary) |
| `a` | Snooze the audible alarm |
| `n` | Switch site (multi-site) |
| `s` | Open / close settings |

Settings screen: `↑`/`↓` select, `←`/`→` change, `Enter` edit (site URL / token),
`w` save, `s`/`Esc` back.
When the minimap is on, click or drag it to move the window — or use `H`/`L`
and `End` for the same navigation from the keyboard.

## Status bars

`sugarrush status` prints one line and exits — the reading, trend arrow, and
delta, coloured by alert state — in whatever syntax your bar speaks:

```bash
sugarrush status                      # 5.6 → +0.2
sugarrush status --format tmux        # #[fg=#98971a]5.6 → +0.2#[default]
sugarrush status --format polybar     # %{F#98971a}5.6 → +0.2%{F-}
sugarrush status --format i3blocks    # full text / short text / colour
sugarrush status --format waybar      # {"text":…,"tooltip":…,"class":…}
```

Colours follow your configured theme, so the colourblind-safe palette carries
over to the bar. Plain `text` has no markup at all — use it in a shell prompt,
a macOS menu-bar helper, or anything that colours its own output.

Wiring it up:

```bash
# tmux (~/.tmux.conf)
set -g status-right '#(sugarrush status --format tmux) | %H:%M'
set -g status-interval 60

# polybar (config.ini)
[module/glucose]
type = custom/script
exec = sugarrush status --format polybar
interval = 60

# i3blocks (~/.config/i3blocks/config)
[glucose]
command=sugarrush status --format i3blocks
interval=60
```

`sugarrush waybar` still prints the same JSON it always has (it's
`--format waybar` under the hood). Example Waybar assets in
[`waybar/`](waybar/): the custom module, a Graph/Settings/About menu (Waybar
≥ 0.11.0), per-state CSS, and Hyprland float rules.

## Always-on alarm

The dashboard can only alarm while a terminal is open, which is the wrong shape
for the job. `sugarrush watch` runs the same alert pipeline headless — fetch,
classify, notify, sound, escalate, push — and logs each transition to stdout:

```bash
sugarrush watch
```

It's safe to leave running alongside the TUI: both processes write a heartbeat,
and the watcher goes quiet whenever the dashboard is on screen, so you never get
two alarms for one low. It also persists episode state, so restarting the
service doesn't re-announce a low you already saw, reset an escalation timer, or
cancel a snooze.

To run it as a user service (example unit in
[`packaging/systemd/`](packaging/systemd/sugarrush-watch.service)):

```bash
mkdir -p ~/.config/systemd/user
cp packaging/systemd/sugarrush-watch.service ~/.config/systemd/user/
systemctl --user enable --now sugarrush-watch.service
journalctl --user -fu sugarrush-watch    # what it's seeing
```

> It's still not a medical device, and it's still only as reliable as the
> machine it runs on, your network, and your Nightscout site. Treat it as one
> layer, not the only one.

## Export

Press `e` in the app, or run it headless — handy from cron, or the morning of
an appointment:

```bash
sugarrush export                     # the AGP-days window, into the current dir
sugarrush export --days 30 --out ~/  # a month, somewhere else
```

Both write two files with a shared timestamped name: `…​.csv` (every reading,
oldest first, in mg/dL *and* your display unit) and `….txt` (a summary: sensor
coverage, five-band time in range, time below range, mean, GMI, CV, and an
hour-by-hour median/spread profile). The text file is fixed-width on purpose —
it survives email and a printer.

Other subcommands: `sugarrush about` (version + a notification) and
`sugarrush --screen settings` (open straight to settings).

## Roadmap

Planned and in-progress work lives in the
[GitHub issues](https://github.com/ronaldlokers/sugarrush/issues) — see the
[product roadmap](https://github.com/ronaldlokers/sugarrush/issues/51).

## License

MIT © Ronald Lokers
