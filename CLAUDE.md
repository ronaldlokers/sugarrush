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

## Definition of done for a user-visible change

A feature or UX PR isn't done until each of these holds. Skip the ones that
genuinely don't apply — but say which and why in the PR.

1. **Gates green** — `fmt --check`, `clippy -D warnings`, build, and test.
2. **Setting?** — wired into the settings screen *and* `config.example.toml`
   (see [new settings go in the settings menu](#important-new-settings-go-in-the-settings-menu)).
3. **`CHANGELOG.md`** — a user-facing bullet under `## [Unreleased]`.
4. **Demo** — `assets/demo.tape` demonstrates it and `assets/demo.gif` is
   regenerated, if it's visible in `--demo`.
5. **README** — keybindings table and feature list updated (see below).
6. **Visually verified** — the affected view checked against `--demo`.

The sections below are the "how" for the non-obvious items.

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

## IMPORTANT: keep the changelog and demo in sync

Any change that adds or alters a **user-visible feature** must, in the same PR:

1. **Update `CHANGELOG.md`** — add a bullet under `## [Unreleased]` in the
   right Keep-a-Changelog group (`### Added` / `### Changed` / `### Fixed`;
   create the group if missing). cargo-dist turns this into the release notes,
   so write it for a user, not as a commit message.
2. **Keep the demo GIF current** — the `assets/demo.gif` in the README is the
   first thing people see, so it must reflect the current UI. If the feature is
   visible in `--demo`:
   - Extend `assets/demo.tape` so the recording actually *shows* the new
     feature (add the keystrokes + a `Sleep` to let it land).
   - Regenerate the GIF and commit it alongside the tape:

     ```bash
     mise exec -- cargo build --release
     mise x vhs ttyd -- vhs assets/demo.tape   # writes assets/demo.gif
     ```

Internal-only changes (refactors, tests, CI, docs) need neither.

## Keep the README current

The README is the project's public face; it must match the shipped UI. When a
change adds or alters a user-visible feature or a keybinding, update **both**:

- the **Keybindings** table — every key the dashboard and settings screen
  handle, and
- the **What it does** feature list — so the prose reflects what the app can do.

## Verify UI changes visually

`Chart` / `Canvas` / layout bugs don't surface in `cargo test`. Before merging
any change to rendering (`src/ui.rs`) or graph/layout behaviour, look at it on
synthetic data:

```bash
mise exec -- cargo build --release
./target/release/sugarrush --demo
```

For a reviewable artefact, capture the affected view with a one-off vhs tape
(`Screenshot out.png`). "It compiled" is not "it looks right".
