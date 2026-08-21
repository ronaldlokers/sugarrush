# sugarrush on the Quickshell bar

A bar widget for [Quickshell](https://quickshell.org). Unlike the assets in
[`waybar/`](../waybar/), which work on any compositor, this one targets a
specific host: the **Omarchy 4 shell** (`omarchy-shell`), which loads bar
widgets as plugins. Quickshell on its own has no bar to add a widget to — a
shell has to provide one — so a widget is only as portable as its host.

It shows the reading, trend arrow and delta, coloured by alert state, with the
full sugarrush tooltip on hover, and opens a panel with the rest of the day:

| Interaction | What happens |
|---|---|
| left click | opens the panel — chart, time in range, patterns |
| right click | opens the sugarrush TUI in a floating terminal |
| middle click | fetches now, without waiting for the next poll |
| hover | tooltip: reading, trend, age, and the last hour as a sparkline |

On a vertical bar the pill stacks the reading over its trend arrow and drops
the delta, which does not fit 28 pixels.

## The panel

Clicking the pill opens a panel carrying what the bar line has no room for:

- the last few hours as a chart: your reading over a typical day for the same
  hours — the median dashed, the 25–75% band shaded — with each alert threshold
  drawn in its own colour and labelled on the value axis, and the clock along
  the bottom. The line itself carries the state, changing colour as it crosses
  a threshold. Seeing tonight sit above the band is the point of it;
- the five time-in-range bands, with mean, GMI and CV for the window they
  cover;
- the pattern insights — the times of day where lows or highs keep happening.
  The section appears only when there is a pattern to name: no repeating low or
  high means no section, rather than an empty one;
- the wordmark across the top, then the reading, its trend, and two icon
  buttons on the trailing edge: refetch now, and open the full TUI.

The panel calls `sugarrush snapshot`, which needs a sugarrush new enough to
have that command; the pill does not, and keeps working either way. If the
command is missing or too old the panel says so instead of drawing an empty
frame.

It fetches when opened, not on a timer, and reuses its last document for
`panelCacheMinutes`. So the heavy part — the multi-day history the patterns
need — is paid for only when someone is actually looking.

## Install

```bash
mkdir -p ~/.config/omarchy/plugins/sugarrush
cp manifest.json logo.png *.qml ~/.config/omarchy/plugins/sugarrush/
omarchy-shell shell rescanPlugins
omarchy plugin enable sugarrush.glucose
```

`omarchy plugin enable` puts it on the bar; move it with
`omarchy bar move sugarrush.glucose --after omarchy.clock`, and remove it again
with `omarchy plugin disable sugarrush.glucose`.

The widget calls `sugarrush waybar`, so it needs `sugarrush` on `PATH` and a
configured site (`~/.config/sugarrush/config.toml`).

## Options

Set with `omarchy bar set <widget> <key> <value>`:

| Key | Default | What it does |
|---|---|---|
| `interval` | `60` | seconds between pill fetches |
| `command` | `sugarrush waybar` | the command the pill reads a reading from |
| `onClick` | `omarchy-launch-floating-terminal-with-presentation sugarrush` | what the panel's "Open dashboard" runs, and the left-click fallback when the panel cannot load |
| `onRightClick` | the same, plus `--screen settings` | right click |
| `panelHours` | `6` | the panel chart's window |
| `insightDays` | `14` | history behind the patterns and the chart's typical-day band; `0` hides the patterns and skips the query |
| `panelCacheMinutes` | `5` | how stale the panel's document may be when it opens |
| `snapshotCommand` | `sugarrush snapshot` | the command the panel reads its document from |

```bash
omarchy bar set sugarrush.glucose interval 30
omarchy bar set sugarrush.glucose onClick "foot -a sugarrush-float sugarrush"
```

Two notes on those:

- The option is `command`, not `exec`. The bar treats any widget carrying
  `exec`, `source` or `type` as one of its own built-in command/QML modules and
  never loads the plugin.
- `omarchy plugin disable` drops the widget's options along with its place on
  the bar, so set them again after re-enabling.

## Colours

The widget paints itself from the `color` field of `sugarrush waybar`, which is
the state colour from your sugarrush theme — including the
colourblind palette. Nothing to theme here, and no stylesheet to keep in sync,
unlike the Waybar module's CSS classes.

Against an older sugarrush that doesn't emit `color`, it falls back to the
bar's own foreground, and to the bar's urgent colour for the two urgent states.

## Editing the widget

The shell compiles plugin QML once per process. Editing an installed
`BarWidget.qml` — or `omarchy plugin disable` / `enable` — will not pick up
your changes; restart `omarchy-shell` to load them.

## Without the plugin

If you would rather not install a plugin, the bar's own command module can run
sugarrush directly. It has no popup or tooltip beyond the text, but it needs no
QML. In `~/.config/omarchy/shell.json`:

```json
{
  "bar": {
    "layout": {
      "right": [
        {
          "id": "sugarrush",
          "type": "command",
          "exec": "sugarrush waybar",
          "interval": 60,
          "onClick": "omarchy-launch-floating-terminal-with-presentation sugarrush"
        }
      ]
    }
  }
}
```
