# sugarrush on the Quickshell bar

A bar widget for [Quickshell](https://quickshell.org). Unlike the assets in
[`waybar/`](../waybar/), which work on any compositor, this one targets a
specific host: the **Omarchy 4 shell** (`omarchy-shell`), which loads bar
widgets as plugins. Quickshell on its own has no bar to add a widget to — a
shell has to provide one — so a widget is only as portable as its host.

It shows the reading **with its unit**, the trend arrow and the delta, and
opens a panel with the rest of the day. It wears the bar's own colour until
the reading leaves your target range — see [Colours](#colours). The unit is there
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
The wordmark leads, with refetch and open-the-dashboard on the right, then
chips switching between three views — **Glucose**, **Profile** and
**Settings**. Each view is a stack of cards that name the window they describe
— because "mean 8.8" under a six-hour chart is a 24-hour figure, and a panel
that does not say so invites the wrong reading.

Glucose:

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
  chart to pan back through `scrollbackHours` of history — at a quarter-hour
  step once past the finely-drawn window, from readings the snapshot already
  fetched for the patterns rather than a second request. The default stops at
  three days because a strip a few hundred pixels wide turns a fortnight into
  noise with a viewport box too small to grab; the strip underneath is that
  whole window with a box showing where you are, and clicking or dragging it
  jumps. Once panned, the card names the hours on screen and returns to live
  when tapped, so a chart showing 3am never claims to be showing now;
- **Last 24 hours** — the five time-in-range bands, with mean, GMI and CV.

Profile — the same window read as habit rather than as history:

- **Typical day · N days** — the ambulatory glucose profile: every day folded
  onto one 24-hour clock, the median line over the middle half and the outer
  5–95%. A bump here means "this happens at 3am", which the six-hour chart
  cannot say. It needs at least three days: with one, the median and the
  quartiles are the same number, and a chart that cannot tell a habit from a
  bad Tuesday should not be drawn. **Copy summary** puts the same clinical
  text `sugarrush export` writes on the clipboard, for the conversation that
  usually happens somewhere else;
- **Patterns · last N days** — the times of day where lows or highs keep
  happening, or a line saying nothing recurring stands out.

Under the cards runs a status strip — the alarm's own state as a chip, then
`sensor 9d 5h · 19h left`, with `updated 3m ago` on the right. The chip reads
`alarm armed`, `snoozed 12m`, or `not watching` in red when no daemon is
running, from `sugarrush health --json`: a panel can draw a perfect graph
while nothing is watching tonight, and that is exactly the state worth being
told about. Those describe the rig rather than the glucose, and they are the only things in the panel that do not change every
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
| `showSparkline` | `true` | draw the last hour as a trace after the reading |
| `showArrow`, `showDelta` | — | not widget options: switch them off in sugarrush's own `[bar]` config (or the settings screen), which every bar follows |
| `command` | `sugarrush waybar` | the command the pill reads a reading from |
| `onClick` | `omarchy-launch-floating-terminal-with-presentation sugarrush` | what the panel's "Open dashboard" runs, and the left-click fallback when the panel cannot load |
| `onRightClick` | the same, plus `--screen settings` | right click |
| `panelHours` | `6` | how much of the overview the chart shows at once |
| `overviewHours` | `24` | how much history is drawn at full resolution (6-72) |
| `scrollbackHours` | `72` | how far the chart pans, and the span of the strip (6-336) |
| `insightDays` | `14` | history behind the patterns and the chart's typical-day band; `0` hides the patterns and skips the query |
| `panelCacheMinutes` | `5` | how stale the panel's document may be when it opens |
| `snapshotCommand` | `sugarrush snapshot` | the command the panel reads its document from |

`showUnits` and `showSparkline` can only take something away. sugarrush's own
`[bar]` config decides what the payload carries in the first place — turn
`units` or `sparkline` off there and the pill drops them whatever these say,
along with every other bar you run.

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

In range, the pill is the bar's own foreground — one more thing on the bar
rather than a green light asking to be looked at. Colour is spent on the case
worth spending it on: **only the reading itself** takes the alert colour, and
only when it is out of range, or when a forecast crossing is coming (the
sparkline's end dot goes hollow for that one, since nothing has happened yet).
The unit, the arrow, the delta and the trace stay in the bar's foreground
throughout. Stale data is carried by the leading `?` alone.

The alert colour is the `color` field of `sugarrush waybar` — the state colour
from your sugarrush theme, including the colourblind palette. Nothing to theme
here, and no stylesheet to keep in sync, unlike the Waybar module's CSS
classes. Against an older sugarrush that doesn't emit `color`, it falls back to
the bar's urgent colour.

The panel follows the same theme, from the `theme` object in
`sugarrush snapshot`: the chart, the profile bands, the time-in-range bar, the
sensor countdown and the alarm chip are all painted from your five configured
colours rather than from a copy of the defaults. Switch on the colourblind
preset and the whole panel switches with it.

## Settings in the panel

The chips under the wordmark swap the panel between **Glucose** and
**Settings** — glucose rather than "now", since that view carries six hours of
chart and fourteen days of patterns as well as the reading, and rather than
"dashboard", which is what this panel's own button opens.
The settings view edits two different things, and says so by grouping them:

- **Alarm thresholds**, **Alarm** and **Status bar** write
  `~/.config/sugarrush/config.toml` through `sugarrush config`, the same
  serializer and atomic write the dashboard's settings screen uses. A value the
  app would have quietly repaired — crossed thresholds, mostly — is refused,
  and the refusal appears under the rows rather than in a log. **Status bar**
  switches the parts of the reading off one at a time; it is sugarrush's own
  `[bar]` config, so it applies to every bar sugarrush feeds, not only this
  pill, which is why it is not under "This panel". The pill refetches as soon
  as the write lands rather than waiting out the poll.
- **This panel** writes the widget's own options in the bar's config through
  `omarchy bar set`.

Everything else — sites, tokens, quiet hours, themes — stays in the dashboard.

## Carbs and insulin

Anything logged on your Nightscout site inside the chart's window is drawn in a
lane along the foot of the chart: a dot for carbs, sized by the amount, and a
triangle for insulin, with the amounts labelled wherever the markers are far
enough apart to read. The card's own title carries the totals — `Last 6 hours ·
45g · 5.7u`.

They are deliberately not drawn at the reading's own height: a marker sitting
on the line would be read as a reading. The two take the graph and forecast
colours from your theme rather than the alert ladder, since neither is a
glucose state, and both are told apart by shape as well as colour so the
colourblind preset still separates them.

Entries with no amount — notes, finger sticks, sensor changes, which Nightscout
keeps in the same collection — are left out. A site that logs nothing simply
has no lane.

## Acting from the panel

Under the reading are the two things worth doing at 3am. **Snooze 15m** and
**1h** run `sugarrush snooze`, and while a snooze is running they collapse into
`Wake now · 12m left`, which runs `sugarrush snooze off`. The countdown comes
from `sugarrush health --json`, so a snooze set from the Omarchy menu, the
dashboard or another machine is reflected here as well. If no watcher is
running the buttons are disabled and the panel says so — silencing an alarm
that is not armed would be a button that lies.

**Log** opens a small form — carbs and insulin, nothing else — and **Review in
terminal** hands those amounts to `sugarrush treatment` in a terminal window.
The panel never writes the record itself: the command prints what it is about
to write and asks for the person's name first, and that confirmation is the
guard on a health record rather than a formality to route around. Cancel, or
closing the panel, clears the form.

Logging needs a **treatment write token** for the site — a separate,
write-capable Nightscout token, set under Site in the dashboard's settings.
Without one the Log button is disabled and the panel says why; `sugarrush
treatment` refuses such a site outright, and a button that leads to that
refusal is worse than one that explains itself.

Carbs and insulin are the only fields because they are the two the chart draws
and the two the command needs. A meal remembered three hours late wants
`--at`, which needs a date picker to offer honestly — that one belongs in the
terminal.
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
