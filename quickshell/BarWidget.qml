// sugarrush bar widget for the Omarchy Quickshell bar.
//
// Deliberately a plain Item using only the properties the bar injects
// (`bar`, `moduleName`, `settings`), with no import of the shell's internal
// modules. That keeps it working both as a plugin (manifest.json, next to
// this file) and as a `type: "qml"` bar module, and across shell versions
// that move their internals around.
//
// It shells out to `sugarrush status --format json`, which prints the Waybar
// object plus a `color` key holding the state colour from the user's own
// sugarrush theme.

import QtQuick
import Quickshell.Io

Item {
  id: root

  // Injected by the bar at load time.
  property var bar
  property string moduleName: "sugarrush.glucose"
  property var settings: ({})

  // Per-widget options, e.g. `omarchy bar set sugarrush.glucose interval 30`.
  //
  // The option is `command`, not `exec`: the bar reads `exec`, `source` and
  // `type` off the layout entry to decide a slot is a built-in command or QML
  // module, and setting any of them would stop this widget from loading at all.
  readonly property int refreshInterval: (settings && settings.interval > 0 ? settings.interval : 60) * 1000
  readonly property string command: settings && settings.command ? settings.command : "sugarrush status --format json"
  readonly property string onClick: settings && settings.onClick !== undefined
    ? settings.onClick
    : "omarchy-launch-floating-terminal-with-presentation sugarrush"
  readonly property string onRightClick: settings && settings.onRightClick !== undefined
    ? settings.onRightClick
    : "omarchy-launch-floating-terminal-with-presentation sugarrush --screen settings"

  // Last reading, as parsed. Starts as a dash so the bar has something to
  // show before the first fetch returns, and keeps the last good reading if a
  // later fetch fails — a stale number is readable, an empty slot is not.
  property string label: "—"
  property string tooltip: "sugarrush: waiting for the first reading"
  property string stateClass: "stale"
  property string stateColor: ""

  // The bar's shared tooltip only shows for a target that says it is hovered,
  // so this is part of the contract, not a convenience.
  readonly property bool tooltipHovered: pointer.containsMouse

  // A vertical bar is 28px wide, so drop the delta and keep value and trend.
  readonly property bool compact: bar ? bar.vertical === true : false
  readonly property string shownText: {
    if (!compact) return label
    var parts = label.split(" ")
    return parts.length > 2 ? parts.slice(0, parts.length - 1).join(" ") : label
  }

  readonly property color foreground: {
    if (stateColor !== "") return stateColor
    // No colour in the payload (an older sugarrush): fall back to the bar's
    // own two colours so an alarm still stands out.
    if (!bar) return "white"
    return (stateClass === "urgent-low" || stateClass === "urgent-high") ? bar.urgent : bar.foreground
  }

  implicitWidth: compact ? (bar ? bar.barSize : 28) : valueText.implicitWidth + 12
  implicitHeight: bar ? bar.barSize : 26

  // Any run still in flight is dropped first: a fetch that outlives its poll
  // interval has nothing to say that the next one won't. The restart is
  // deferred because setting `running` false and true in one pass is not a
  // change at all, and would leave the widget with no fetch running.
  function refresh() {
    statusProc.running = false
    Qt.callLater(function () { statusProc.running = true })
  }

  // The bar injects `settings` after the widget is built, so the first poll
  // runs with the default command. Fetch again as soon as a configured one
  // lands, rather than showing a dash until the next tick.
  onCommandChanged: pollTimer.restart()

  function apply(out) {
    var payload
    try {
      payload = JSON.parse(String(out || ""))
    } catch (e) {
      // Keep the last reading; only the tooltip admits something went wrong.
      root.tooltip = "sugarrush: could not read status output"
      return
    }
    root.label = payload.text || "—"
    root.tooltip = payload.tooltip || ""
    root.stateClass = payload["class"] || "stale"
    root.stateColor = payload.color || ""
  }

  Process {
    id: statusProc
    command: ["bash", "-lc", root.command]
    // The collectors are named, and read through their ids: an unqualified
    // `text` in these handlers resolves to the Text element below instead.
    stdout: StdioCollector {
      id: outCollector
      waitForEnd: true
      onStreamFinished: root.apply(outCollector.text)
    }
    stderr: StdioCollector {
      id: errCollector
      waitForEnd: true
      onStreamFinished: if (errCollector.text.trim() !== "") console.warn("sugarrush", errCollector.text.trim())
    }
  }

  Timer {
    id: pollTimer
    interval: root.refreshInterval
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  Text {
    id: valueText
    anchors.centerIn: parent
    text: root.shownText
    color: root.foreground
    font.family: root.bar ? root.bar.fontFamily : "monospace"
    font.pixelSize: 12
    font.bold: root.stateClass === "urgent-low" || root.stateClass === "urgent-high"
  }

  MouseArea {
    id: pointer
    anchors.fill: parent
    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton

    onEntered: if (root.bar && root.tooltip !== "") root.bar.showTooltip(root, root.tooltip)
    onExited: if (root.bar) root.bar.hideTooltip(root)

    onClicked: function (mouse) {
      if (!root.bar) return
      if (mouse.button === Qt.MiddleButton) {
        root.refresh()
      } else if (mouse.button === Qt.RightButton) {
        if (root.onRightClick !== "") root.bar.run(root.onRightClick)
      } else if (root.onClick !== "") {
        root.bar.run(root.onClick)
      }
    }
  }
}
