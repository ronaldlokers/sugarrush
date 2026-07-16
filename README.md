# sugarrush

A terminal UI for viewing [Nightscout](https://nightscout.github.io/) CGM
(continuous glucose monitor) sensor data. Built with Rust + [Ratatui](https://ratatui.rs/).

> ⚠️ Not a medical device. Do not use `sugarrush` for treatment decisions.
> Always confirm with your meter/pump/official app.

## Features

**v1**
- Current blood glucose with trend arrow and delta
- Recent readings as a live braille graph
- mg/dL ↔ mmol/L toggle
- Auto-refresh

Planned work is tracked in [GitHub issues](https://github.com/ronaldlokers/sugarrush/issues)
(predictions, alerts, IOB/COB, graph scrolling, settings screen, and more).

## Install

Requires a Rust toolchain (managed here via [mise](https://mise.jdx.dev/)):

```bash
mise install       # installs the pinned Rust toolchain
cargo build --release
```

The binary lands at `target/release/sugarrush`.

## Configuration

Copy the example config and fill it in:

```bash
mkdir -p ~/.config/sugarrush
cp config.example.toml ~/.config/sugarrush/config.toml
chmod 600 ~/.config/sugarrush/config.toml
```

### Nightscout token (read-only)

Do **not** use `API_SECRET` — that is admin-level. Instead create a read-only
token in **Nightscout → Admin Tools**:

1. Add a **Subject** (e.g. `sugarrush`).
2. Give it the `readable` role.
3. Copy the generated access token into `config.toml` as `token`.

The token is sent as a `?token=…` query parameter and only grants read access.

## Keybindings

| Key | Action        |
|-----|---------------|
| `q` / `Esc` | Quit    |
| `r` | Refresh now   |
| `u` | Toggle units  |

## License

MIT © Ronald Lokers
