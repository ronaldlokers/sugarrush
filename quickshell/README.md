# sugarrush on the Quickshell bar

A bar widget for [Quickshell](https://quickshell.org). Unlike the assets in
[`waybar/`](../waybar/), which work on any compositor, this one targets a
specific host: the **Omarchy 4 shell** (`omarchy-shell`), which loads bar
widgets as plugins. Quickshell on its own has no bar to add a widget to — a
shell has to provide one — so a widget is only as portable as its host.

It shows the reading, trend arrow and delta, coloured by alert state, with the
full sugarrush tooltip on hover:

| Interaction | What happens |
|---|---|
| left click | opens the sugarrush TUI in a floating terminal |
| right click | opens the TUI on the settings screen |
| middle click | fetches now, without waiting for the next poll |
| hover | tooltip: reading, trend, age, and the last hour as a sparkline |

## Install

```bash
mkdir -p ~/.config/omarchy/plugins/sugarrush
cp manifest.json BarWidget.qml ~/.config/omarchy/plugins/sugarrush/
omarchy-shell shell rescanPlugins
omarchy plugin enable sugarrush.glucose
```

`omarchy plugin enable` puts it on the bar; move it with
`omarchy bar move sugarrush.glucose --after omarchy.clock`, and remove it again
with `omarchy plugin disable sugarrush.glucose`.

The widget calls `sugarrush status --format json`, so it needs `sugarrush` on
`PATH` and a configured site (`~/.config/sugarrush/config.toml`) — the same
prerequisites as `sugarrush waybar`.

## Options

Set with `omarchy bar set <widget> <key> <value>`:

| Key | Default | What it does |
|---|---|---|
| `interval` | `60` | seconds between fetches |
| `command` | `sugarrush status --format json` | the command to read a reading from |
| `onClick` | `omarchy-launch-floating-terminal-with-presentation sugarrush` | left click |
| `onRightClick` | the same, plus `--screen settings` | right click |

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

The widget paints itself from the `color` field of `sugarrush status --format
json`, which is the state colour from your sugarrush theme — including the
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
