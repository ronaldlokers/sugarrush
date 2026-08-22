# sugarrush on the Quickshell bar

A bar widget for [Quickshell](https://quickshell.org). Unlike the assets in
[`waybar/`](../waybar/), which work on any compositor, this one targets a
specific host: the **Omarchy 4 shell** (`omarchy-shell`), which loads bar
widgets as plugins. Quickshell on its own has no bar to add a widget to — a
shell has to provide one — so a widget is only as portable as its host.

It shows the reading **with its unit**, the trend arrow and the delta, coloured
by alert state, and opens a panel with the rest of the day. The unit is there
because a bare number names nothing — `10.5` could be a load average; nothing
else on a desktop is reported in mmol/L:

| Interaction | What happens |
|---|---|
| left click | opens the panel — chart, time in range, patterns |
| right click | opens the sugarrush TUI in a floating terminal |
| middle click | fetches now, without waiting for the next poll |

On a vertical bar the pill stacks the reading over its trend arrow and drops
both the unit and the delta, which do not fit 28 pixels.

Against a sugarrush too old to send the reading in parts, the pill falls back
to the line that binary prints — no unit, but no breakage either.

## The panel

Clicking the pill opens a panel carrying what the bar line has no room for.
The wordmark leads, with refetch and open-the-dashboard on the right, and the
rest is a stack of cards that each name the window they describe — because
"mean 8.8" under a six-hour chart is a 24-hour figure, and a panel that does
not say so invites the wrong reading:

- **Now** — the reading at display size beside where it lands in half an hour,
  both coloured by the band they fall in, with the trend and the delta. The
  forecast is sugarrush's own AR2 projection; during a sensor gap it is absent
  rather than guessed, and the arrow goes with it;
- **Last N hours** — the chart: your reading over a typical day for the same
  hours, the median dashed and the 25–75% band shaded, each alert threshold
  ruled and labelled on the value axis, the clock along the bottom.
  The line carries the state, changing colour as it crosses a threshold, and
  seeing tonight sit above the band is the point of it. Hovering the chart
  drops a crosshair on the nearest reading and prints its value and time —
  the reading itself, never an interpolation between two of them. Scroll the
  chart to pan back through the history — days of it, at a quarter-hour step
  once you pass the finely-drawn window, from readings the snapshot already
  fetched for the patterns rather than a second request; the strip underneath is that
  whole window with a box showing where you are, and clicking or dragging it
  jumps. Once panned, the card names the hours on screen and returns to live
  when tapped, so a chart showing 3am never claims to be showing now;
- **Last 24 hours** — the five time-in-range bands, with mean, GMI and CV;
- **Patterns · last N days** — the times of day where lows or highs keep
  happening. The card appears only when there is a pattern to name: no
  repeating low or high means no card, rather than an empty one.

Under the cards runs a status strip — `sensor 9d 5h · 19h left` on the left,
`updated 3m ago` on the right. Those two describe the rig rather than the
glucose, and they are the only things in the panel that do not change every
five minutes, so they sit apart from the cards rather than inside one. The
sensor turns amber inside its last day and red once it is past; the countdown
needs `sensor_days` and a site whose uploader logs "Sensor Start" / "Sensor
Change", and without either you get the age alone, or no strip at all.

The stack scrolls if it outgrows the room a popup is allowed, which four cards
can do on a short screen.

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
cp manifest.json *.png *.svg *.qml ~/.config/omarchy/plugins/sugarrush/
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
| `showUnits` | `true` | print the unit after the reading; turn it off on a crowded bar |
| `showMascot` | `false` | put the sugar cube in front of the reading |
| `command` | `sugarrush waybar` | the command the pill reads a reading from |
| `onClick` | `omarchy-launch-floating-terminal-with-presentation sugarrush` | what the panel's "Open dashboard" runs, and the left-click fallback when the panel cannot load |
| `onRightClick` | the same, plus `--screen settings` | right click |
| `panelHours` | `6` | how much of the overview the chart shows at once |
| `overviewHours` | `24` | how much history is drawn at full resolution (6-72) |
| `insightDays` | `14` | also sets how far back the chart scrolls, at a quarter-hour step |
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

## Settings in the panel

The chips under the wordmark swap the panel between **Glucose** and
**Settings** — glucose rather than "now", since that view carries six hours of
chart and fourteen days of patterns as well as the reading, and rather than
"dashboard", which is what this panel's own button opens.
The settings view edits two different things, and says so by grouping them:

- **Alarm thresholds** and **Alarm** write `~/.config/sugarrush/config.toml`
  through `sugarrush config`, the same serializer and atomic write the
  dashboard's settings screen uses. A value the app would have quietly
  repaired — crossed thresholds, mostly — is refused, and the refusal appears
  under the rows rather than in a log.
- **This panel** writes the widget's own options in the bar's config through
  `omarchy bar set`.

Everything else — sites, tokens, quiet hours, themes — stays in the dashboard.
A panel that dismisses when focus moves is the wrong place to type a token.

## Omarchy menu entries

[`omarchy-menu.jsonc`](omarchy-menu.jsonc) has a sugarrush submenu — dashboard,
snooze 15m/1h, the current reading as a notification, export, settings — to
merge into `~/.config/omarchy/extensions/omarchy-menu.jsonc`. The shell watches
that file, so an edit takes effect without a restart, and every entry is gated
on `command -v sugarrush` so the menu stays clean on a machine without it.

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
