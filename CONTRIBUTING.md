# Contributing

Thanks for looking. sugarrush is a small, opinionated app — bug reports and
focused PRs are very welcome; large redesigns are best raised as an issue
first so neither of us wastes an afternoon.

## Getting set up

Rust is pinned with [mise](https://mise.jdx.dev), so prefix cargo with
`mise exec --` (or run `mise install` once and use cargo directly):

```bash
mise exec -- cargo build
mise exec -- cargo test
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo fmt --all
```

You don't need a Nightscout site to work on the UI:

```bash
mise exec -- cargo run -- --demo
```

## What CI checks

All of these must pass — run them locally first:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- build and test on Linux, macOS, and Windows
- `cargo check` on the MSRV in `Cargo.toml`
- `scripts/check-process.sh` — every settings `Field` is on the settings
  screen, every persisted key is in `config.example.toml`
- a `CHANGELOG.md` entry when `src/` changes (bypass with `[skip-changelog]`
  in the PR title for refactors and other non-user-visible work)
- a regenerated `assets/demo.gif` when the rendering changes (bypass with
  `[skip-demo]` for non-visual edits)

## Conventions

- **Commits**: conventional-commit style, lowercase imperative — `fix: …`,
  `feat: …`, `docs: …`. Branch off `main` as `fix/…` or `feat/…` and open a PR;
  don't push to `main`.
- **Units**: alert thresholds are stored in **mg/dL** internally and written to
  config in the user's display unit (`AlertsConfig::resolve` converts). Keep
  that split.
- **New settings go in the settings menu.** A setting that exists only in
  `config.toml` is incomplete — see the checklist in `CLAUDE.md`. The process
  check enforces the parts it can.
- **UI changes get looked at.** `Chart`/`Canvas`/layout bugs don't show up in
  `cargo test`; check your change against `--demo` at more than one terminal
  size before opening the PR.

## Safety-critical code

The alarm path — fetching readings, classifying them, deciding whether to
alarm — is the reason this app exists. Changes there want a test that would
have caught the bug, and a note in the PR about what happens when the input is
missing, stale, or out of order. "It compiled" is not enough for code someone
relies on to wake them up.

sugarrush is not a medical device, and nothing here should encourage anyone to
treat it as one — but it should be trustworthy at 3am.

## Reporting bugs

Use the issue templates. For anything security-related, see
[SECURITY.md](SECURITY.md) instead.
