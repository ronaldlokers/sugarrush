// sugarrush bar widget for the Omarchy Quickshell bar.
//
// Deliberately a plain Item using only the properties the bar injects
// (`bar`, `moduleName`, `settings`), with no import of the shell's internal
// modules. That keeps it working both as a plugin (manifest.json, next to
// this file) and as a `type: "qml"` bar module, and across shell versions
// that move their internals around.
//
// It shells out to `sugarrush waybar`, which prints the Waybar object plus a
// `color` key holding the state colour from the user's own sugarrush theme.
// That alias, rather than `status --format json`, because it is the spelling
// every released sugarrush understands.

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
  readonly property string command: settings && settings.command ? settings.command : "sugarrush waybar"
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
  // Still parsed, and still the one place a fetch failure is described — the
  // panel reads it. The pill no longer pops it up on hover.
  property string tooltip: "sugarrush: waiting for the first reading"
  property string stateClass: "stale"
  property string stateColor: ""

  // A vertical bar is 28px wide — narrower than "6.1 →" renders — so the
  // delta goes and what is left stacks, one line each, the way the clock
  // stacks its hours over its minutes.
  readonly property bool compact: bar ? bar.vertical === true : false
  readonly property string shownText: {
    if (!compact) return label
    var parts = label.split(" ")
    return parts.length > 2 ? parts.slice(0, parts.length - 1).join("\n") : label
  }

  readonly property color foreground: {
    if (stateColor !== "") return stateColor
    // No colour in the payload (an older sugarrush): fall back to the bar's
    // own two colours so an alarm still stands out.
    if (!bar) return "white"
    return (stateClass === "urgent-low" || stateClass === "urgent-high") ? bar.urgent : bar.foreground
  }

  implicitWidth: compact
    ? (bar ? bar.barSize : 28)
    : mascot.width + valueText.implicitWidth + 14
  implicitHeight: compact
    ? Math.max(bar ? bar.barSize : 26, valueText.implicitHeight + 6)
    : (bar ? bar.barSize : 26)

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

  // The panel is loaded by path, and its failure is survivable: every
  // Omarchy-internal import lives in Panel.qml, so a shell that moves them
  // costs the popup while the pill goes on working.
  readonly property bool panelReady: panelLoader.status === Loader.Ready && panelLoader.item !== null
  readonly property bool opened: panelReady ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelReady ? panelLoader.item.popoutSwitchClosing === true : false

  function injectPanel() {
    if (!panelReady) return
    var target = panelLoader.item
    // `bar` is typed QtObject on the panel, and the widget's own `bar` is
    // undefined until the slot injects it — assigning that undefined is an
    // error, not a no-op.
    if ("bar" in target && root.bar) target.bar = root.bar
    if ("settings" in target && root.settings) target.settings = root.settings
    if ("anchorItem" in target) target.anchorItem = root
    if ("hostWidget" in target) target.hostWidget = root
  }

  function open() { if (panelReady) panelLoader.item.open() }
  function close() { if (panelReady) panelLoader.item.close() }
  function closeForPopoutSwitch() { if (panelReady) panelLoader.item.closeForPopoutSwitch() }

  onBarChanged: injectPanel()
  onSettingsChanged: injectPanel()

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      root.injectPanel()
      Qt.callLater(root.injectPanel)
    }
    onStatusChanged: if (status === Loader.Error) {
      console.warn("sugarrush: panel failed to load; the pill still works")
    }
  }

  // The mascot rides in front of the reading — a silhouette, because detail
  // is what a bar-height icon cannot keep. Vector rather than bitmap: it is
  // asked for at whatever height the bar happens to be, and rasterises at
  // exactly that size instead of being resampled from a fixed one.
  Image {
    id: mascot
    source: Qt.resolvedUrl("mascot.svg")
    // Sized off the reading's own type rather than the bar's height: the
    // shell draws its glyph icons at Style.font.icon (14px against 12px
    // text), and a mascot scaled to the slot instead of the type stood a
    // third taller than every icon beside it.
    height: Math.round(valueText.font.pixelSize * 7 / 6)
    // The artwork is wider than it is tall; squaring it would letterbox the
    // shape into a smaller drawing than the space allows.
    width: Math.round(height * 1051 / 908)
    sourceSize.height: height * 2
    fillMode: Image.PreserveAspectFit
    smooth: true
    visible: !root.compact
    anchors.left: parent.left
    anchors.leftMargin: 3
    anchors.verticalCenter: parent.verticalCenter
  }

  Text {
    id: valueText
    anchors.verticalCenter: parent.verticalCenter
    anchors.horizontalCenter: root.compact ? parent.horizontalCenter : undefined
    anchors.left: root.compact ? undefined : mascot.right
    anchors.leftMargin: root.compact ? 0 : 5
    text: root.shownText
    horizontalAlignment: Text.AlignHCenter
    lineHeight: 0.95
    color: root.foreground
    font.family: root.bar ? root.bar.fontFamily : "monospace"
    font.pixelSize: 12
    font.bold: root.stateClass === "urgent-low" || root.stateClass === "urgent-high"
  }

  MouseArea {
    id: pointer
    anchors.fill: parent
    acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton

    onClicked: function (mouse) {
      if (!root.bar) return
      if (mouse.button === Qt.MiddleButton) {
        root.refresh()
      } else if (mouse.button === Qt.RightButton) {
        if (root.onRightClick !== "") root.bar.run(root.onRightClick)
      } else if (root.panelReady) {
        panelLoader.item.toggle()
      } else if (root.onClick !== "") {
        // No panel — an older shell, or one that moved the internals it is
        // built on. Left click falls back to what it did before the panel
        // existed rather than doing nothing.
        root.bar.run(root.onClick)
      }
    }
  }
}
