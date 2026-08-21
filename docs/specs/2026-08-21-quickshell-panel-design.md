# A popup panel for the Quickshell widget

Design, 2026-08-21. Status: approved, not yet implemented.

## What this is

The Quickshell bar widget in [`quickshell/`](../../quickshell/) shows one line:
reading, trend, delta. Everything else about the day — the shape of the last
few hours, how much of it was in range, whether the lows keep happening at the
same time — is only in the TUI.

This adds a popup panel to that widget: click the pill, get a chart, the
time-in-range bands, and the pattern insights, without leaving the bar. It also
adds the one thing that makes such a panel possible — a sugarrush command that
prints a whole snapshot as JSON, rather than a single bar line.

Omarchy calls a widget's popup a *panel*; the underlying Quickshell type is
`PopupWindow`. Both names appear below, and mean the same surface.

## What it is not

- Not a second dashboard. Treatments, followers, settings editing, graph
  navigation, and exports stay in the TUI; the panel's footer opens it.
- Not a site switcher. The panel shows the active site. Multi-site users get
  the followers screen in the TUI, as today.
- Not a replacement for the pill. Everything here is behind a click; the bar
  line keeps working, and keeps working even if the panel fails to load.

## Part 1 — the data feed

### The command

```
sugarrush snapshot [--hours N] [--days N] [--site NAME] [--demo]
```

Defaults: `--hours 6`, `--days 14`. `--days 0` skips the multi-day fetch
entirely, so a caller that only wants a chart pays only for a chart.

It prints one JSON document to stdout and exits 0.

### Why a new subcommand

`status` renders a *line*. Every `Format` it knows — waybar, i3blocks, polybar,
tmux, text — is a string built from one `Status` struct, and the module says so
in its first paragraph. A nested document with a series, five bands and a list
of insights is a different contract, and bending `Format` around it would leave
`status.rs` serving two masters.

So: a new `src/snapshot.rs`, composing what already exists —
`nightscout::Client` for the fetches, `alert::evaluate` for the state,
`stats::{tir, mean_mgdl, gmi, cv_pct}` for the numbers, and
`agp::{profile_in, insights}` for the patterns. No new analysis code, and
therefore no second opinion about what "in range" means.

### The document

```json
{
  "schema": 1,
  "units": "mmol/L",
  "generated_at": 1755797000000,
  "now": {
    "value": "6.4",
    "arrow": "→",
    "direction": "Flat",
    "delta": "+0.1",
    "class": "in-range",
    "color": "#98971a",
    "age_min": 4
  },
  "range": { "urgent_low": 3.1, "low": 3.9, "high": 10.0, "urgent_high": 13.9 },
  "series": [[1755796700000, 6.3], [1755797000000, 6.4]],
  "stats": {
    "window_h": 24,
    "mean": 6.8,
    "gmi": 6.2,
    "cv": 31.0,
    "tir": { "very_low": 0.0, "low": 2.1, "in_range": 78.4, "high": 17.5, "very_high": 2.0 }
  },
  "insights": [
    { "kind": "lows", "window": "02:00–05:00", "extreme": 3.2, "text": "lows cluster overnight" }
  ]
}
```

Three rules the shape follows:

**Everything is in display units.** `series` values, `range` bounds, `mean`,
and `insights[].extreme` are all in whatever `units` says, already converted and
already rounded the way sugarrush rounds. The QML never converts, so it cannot
disagree with the app about what 6.4 means. This keeps the split CLAUDE.md
mandates — mg/dL internally, display units at the edge — with the edge here
being the serializer.

**`schema` is first and is a number.** A newer widget against an older binary
reads a shape it doesn't know, and should say so rather than guess. Bump it only
for a breaking change; adding a field is not one.

**Failure is still JSON.** No site, no network, no readings — the document comes
back as

```json
{ "schema": 1, "generated_at": 1755797000000, "error": "no site configured" }
```

with exit code 0. This is the same promise `status` makes and for the same
reason: a panel that can render "sugarrush: no site configured" is more useful
than one that renders a parse failure. `now`, `series`, `stats` and `insights`
are absent, not null, in that case; a consumer checks for `error` first.

Partial data follows the same principle rather than failing whole: no readings
in the last `--hours` gives `"series": []`; too little history for a pattern
gives `"insights": []`; fewer than two readings gives `"stats": null`. Each is a
state the panel draws deliberately.

### Fetch shape and cost

- `now` and `series`: `entries_range(now - hours, now, hours * 12 + 12)`.
- `stats`: the last 24h — a second range fetch, or a slice of the multi-day
  fetch when `--days` covers it. Prefer the slice; one fetch beats two.
- `insights`: `entries_range(now - days, now, days * 288 + 288)`, then
  `profile_in(entries, site_timezone)` and `insights(bands, low, high)`. The
  site's configured IANA timezone is passed through, so patterns describe the
  followed person's night, matching what the TUI and exports do.

At `--days 14` that is roughly four thousand entries in one query. That is why
the panel — not the pill — is the only caller, why it caches (Part 2), and why
`--days 0` exists.

`--demo` fills the document from `demo.rs` instead of the network, which gives
the panel something to render with no site configured, and gives us a way to
screenshot it for the README.

### Rust surface

| File | Change |
|---|---|
| `src/snapshot.rs` | new: `Snapshot` types, `snapshot(cfg, hours, days)`, serialization |
| `src/main.rs` | `mod snapshot;`, new `Mode::Snapshot { hours, days }`, arg parsing, `--help` line, and `Mode::Snapshot` added to the `matches!` that decides which modes may carry `--site` (it is rejected everywhere else) |

Nothing in `status.rs`, `agp.rs`, `stats.rs` or `alert.rs` changes shape;
`snapshot` is a consumer of all four.

## Part 2 — the panel

### Files

```
quickshell/
  manifest.json     unchanged (kinds: ["bar-widget"])
  BarWidget.qml     gains the panel host contract
  Panel.qml         new — the popup
  README.md         gains a panel section
```

### The host contract

The bar identifies a panel by the widget in its slot, not by the panel nested
inside it. So `BarWidget.qml` grows exactly what `omarchy.weather` has:

- a `Loader` for `Panel.qml`, and `injectPanel()` pushing `bar`, `settings`,
  `anchorItem` (the pill) and `hostWidget` (itself) into it;
- forwarded `opened`, `open()`, `close()`, `closeForPopoutSwitch()` and
  `popoutSwitchClosing`.

That is what makes `bar.requestPopout` hand over correctly when another widget's
popup opens, and what puts the open-panel dot under the pill.

Click behaviour changes with the panel: **left click toggles the panel**
(it opened the TUI before), right click keeps opening the TUI, middle click
refetches. The README documents the change and the `onClick` option still
overrides it.

### Structure

`Panel.qml` is a `qs.Ui` `Panel` wrapping a `KeyboardPanel` surface with a
`PanelKeyCatcher` and a `Flickable` column:

| Section | Built from | Shows |
|---|---|---|
| Hero | `PanelHero` | reading, arrow, delta, age |
| Chart | `Canvas` | `series` polyline, shaded target band, urgent hairlines, hour ticks |
| Stats | custom `Row` + `PanelSectionHeader` | five-band stacked TIR bar; mean, GMI, CV |
| Insights | `Column` of text rows | `insights[].text` with its window |
| Footer | `PanelActionButton` | last updated, refresh, "Open dashboard" |

`Canvas` rather than `QtQuick.Shapes`: one paint function over an array, no
extra module, and repainting on new data is a single `requestPaint()`.

Colours come from the document (`now.color`) and from `Color`/`Style` for
chrome, so the panel matches both the sugarrush theme and the bar's.

### Data lifecycle

A `Process` runs
`sugarrush snapshot --hours <panelHours> --days <insightDays>`:

- on open, if the last document is older than `panelCacheMinutes`;
- on the refresh button, always;
- never while closed. The pill keeps its own cheap `sugarrush waybar` poll.

The panel renders the previous document while a refetch is in flight, so
reopening never flashes empty.

### Options

Added to the existing per-widget options, set with `omarchy bar set`:

| Key | Default | Meaning |
|---|---|---|
| `panelHours` | `6` | chart window |
| `insightDays` | `14` | history for patterns; `0` disables the section |
| `panelCacheMinutes` | `5` | how stale a document may be when the panel opens |
| `snapshotCommand` | `sugarrush snapshot` | the command, for non-`PATH` installs |

### States to draw

| State | Panel shows |
|---|---|
| document with data | the full panel |
| `"error"` present | hero replaced by the message; footer still opens the TUI |
| `series: []` | chart area says "no readings in the last N hours" |
| `stats: null` | stats row hidden, not zeroed |
| `insights: []` | "not enough history yet for patterns" |
| command missing / exits non-zero | "needs sugarrush with `snapshot` — update to ≥ VERSION" |
| panel fails to load at all | pill unaffected; click falls back to opening the TUI |

The last row is the point of Part 3.

## Part 3 — where the coupling sits

Every `qs.Ui` and `qs.Commons` import lives in `Panel.qml`. `BarWidget.qml`
stays a plain `Item` that reaches the panel through a `Loader` by path, and
checks `Loader.status` before using it.

So a shell release that moves its internals breaks the panel and nothing else:
the pill still shows the reading, and its click falls back to opening the TUI.
That is the fallback the "native with fallback" option would have bought, minus
a second panel to keep in sync.

## Testing

**Rust — test first, and prove each test fails against the code it covers.**

- the document's shape for a normal fetch, including `schema` and `units`;
- display-unit conversion for `series`, `range`, `mean` and `extreme`, in both
  mmol/L and mg/dL;
- the error document: `error` present, `now`/`series` absent, exit 0;
- empty history: `series: []`, `stats: null`, `insights: []`, no panic;
- insight mapping: an `agp::Insight` becomes the right `kind`, `window` and
  `text`;
- `--days 0` performs no multi-day fetch (assert on the fake client's calls).

A compile error is not proof. Each test must be seen failing on an assertion
against code that runs.

**QML — live, because it cannot be harness-tested.** `qs.Ui` resolves only
inside the shell, so the standalone `qs` harness that verified the pill cannot
load the panel. Verification is on Omarchy, captured as screenshots:

- panel open with real data — chart, bands, insights;
- each empty/error state from the table above, forced with `--demo` and with a
  deliberately broken `snapshotCommand`;
- popout handoff: open the panel, click another bar widget's popup, confirm the
  panel closes and the dot moves;
- a vertical bar (`omarchy bar position left`), then restored;
- the panel's QML edited and the shell restarted — a reminder that plugin QML is
  compiled once per process.

## Documentation

- `quickshell/README.md`: panel section, the new options, the click change.
- root `README.md`: the feature list and the status-bar section mention the
  panel; `snapshot` joins the command table.
- `CHANGELOG.md`: one `### Added` bullet for the panel, one for `snapshot`.
- `waybar/` is untouched.

No demo-gif work: none of this appears in `--demo`'s TUI. No settings-screen
work: the options live in the bar's config, not `config.toml`, so
`scripts/check-process.sh` has nothing to check here.

## Risks

- **`--days 14` is a heavy query.** Mitigated by caching, by the panel being the
  only caller, and by `insightDays: 0`. If it proves slow on real sites, the
  fallback is to read the history cache when it is enabled — deliberately not in
  this design, because the cache is opt-in and the panel must work without it.
- **The panel couples to Omarchy internals.** Accepted deliberately for the
  native look; contained to one file, with the pill unaffected by its failure.
- **`snapshot` is a new public CLI surface.** Once shipped it is a promise.
  `schema` is how it changes later without breaking a widget in the wild.

## Open question

`VERSION` in the "needs sugarrush with `snapshot`" message is decided at
implementation time — it is whichever release first carries the command.
