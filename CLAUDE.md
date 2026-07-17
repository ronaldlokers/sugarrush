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
## Cutting a release

Releases are driven by **cargo-dist**; all publish secrets
(`CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_TOKEN`, `AUR_SSH_KEY`) are already set.
To ship a version:

1. **Update `CHANGELOG.md`** — move items from `## [Unreleased]` into a new
   `## [YYYY.M.N] - <date>` section (Keep a Changelog format). cargo-dist
   extracts this section verbatim for the GitHub Release notes, so keep it
   good.
2. **Bump the version** in `Cargo.toml` to the same CalVer (`cargo build` to
   update `Cargo.lock`), on a branch → PR → merge.
3. **Tag and push**: `git tag vYYYY.M.N && git push origin vYYYY.M.N`.

That one tag fans out automatically:

- **Release** workflow (dist) — builds every target, publishes a public GitHub
  Release with archives, checksums, and the `curl|sh` / PowerShell installers,
  and pushes the Homebrew formula to `ronaldlokers/homebrew-tap`.
- **Publish crate** — `cargo publish` to crates.io (`cargo install` / `cargo
  binstall sugarrush`).
- **Publish to AUR** — runs after the Release completes (keyed off
  `workflow_run`, *not* `on: release`, since `GITHUB_TOKEN`-created releases
  don't fire release events); renders the PKGBUILD and pushes `sugarrush-bin`.

Notes for future release work:

- The repo is **public** — required so release-asset downloads (installers,
  binstall, Homebrew, AUR) work unauthenticated.
- Release-asset downloads in CI must use `gh release download` (the browser
  `releases/download/...` URL can 404 for dist-created releases).
- If a channel's job fails after the release exists, re-run it against the
  same tag — no re-tag needed: `gh run rerun <id> --failed` (Homebrew) or
  `gh workflow run aur.yml -f tag=vYYYY.M.N` (AUR).

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
