# CLAUDE.md

Guidance for working in this repo.

## What this is

`sugarrush` is a terminal UI (Rust + [Ratatui](https://ratatui.rs)) for viewing
self-hosted [Nightscout](https://nightscout.github.io/) CGM data: live blood
glucose, trend, history, forecasts, alerts, and stats. Not a medical device.

## Layout

| File | Responsibility |
|------|----------------|
| `src/main.rs` | Entry point, CLI parsing (`waybar`/`about`/`--screen`), run loop, input |
| `src/app.rs` | `App` state + the settings screen (`Field`, edit, persist) |
| `src/config.rs` | `Config` and its parts (`AlertsConfig`, `Site`, `GraphStyle`, `MinimapConfig`) |
| `src/nightscout.rs` | REST client + data models (entries, devicestatus, treatments) |
| `src/ui.rs` | All rendering |
| `src/view.rs` | Graph viewport (pan/zoom/jump) |
| `src/alert.rs` | Alert classification |
| `src/predict.rs` | AR2 forecast |
| `src/stats.rs` | Time-in-range, mean, GMI |
| `src/theme.rs` | Configurable colors |
| `src/units.rs` | mg/dL ↔ mmol/L |
| `src/waybar.rs` | One-shot Waybar JSON output |

## Commands

Rust is pinned via [mise](https://mise.jdx.dev); prefix cargo with `mise exec --`:

```bash
mise exec -- cargo build
mise exec -- cargo test
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo fmt --all
```

CI (`.github/workflows/ci.yml`) gates on **fmt `--check`, `clippy -D warnings`,
build, and test** — all four must pass. Run them locally before pushing.

## Conventions

- **Commits**: conventional-commit style (`feat:`, `fix:`, `docs:`…), lowercase
  imperative. Never commit to `main`; use short `feat/…` / `fix/…` branches and
  open a PR.
- **Units**: alert thresholds are stored in **mg/dL** internally; config values
  are written in the user's display unit and converted on load
  (`AlertsConfig::resolve`). Keep that split.
- **Config persistence**: settings are serialized back to `config.toml` from
  `App::build_config` (`src/app.rs`). Anything user-editable must round-trip.
- **`waybar/` examples stay compositor-generic** — no distro-specific config
  (no Omarchy helpers, no bespoke launchers). It ships to the general public.
- **Versioning (CalVer)**: `YYYY.M.N` — year, month (not zero-padded), and an
  incremental `N` that resets each month (first release of a month is `.1`).
  E.g. `2026.7.1` → `2026.7.2` → (August) `2026.8.1`. Pick the next version
  from today's date; tags are `v`-prefixed (`v2026.7.1`). It's valid SemVer, so
  Cargo/crates.io accept it.
- **Releases** are driven by cargo-dist: a `v`-tag builds all targets and
  publishes a GitHub Release + installers; separate workflows publish to
  crates.io / Homebrew / AUR (gated on their secrets).

## IMPORTANT: new settings go in the settings menu

When you add a new configurable setting, wire it into the in-app **settings
screen** — do not leave it config-file-only. For a setting to be complete:

1. Add the field to the relevant struct in `src/config.rs` (with a serde
   default) and to `App` in `src/app.rs`.
2. Add a `Field` variant in `src/app.rs` and include it in `Field::ALL`.
3. Render its value in `App::field_value`.
4. Make it editable in `App::settings_adjust` (toggle / step / cycle).
5. Persist it in `App::build_config` so `w` writes it back.
6. Document it in `config.example.toml`.

A setting that exists in `config.toml` but not in the settings screen is
considered incomplete.
