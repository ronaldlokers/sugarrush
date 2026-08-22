// The sugarrush popup panel for the Omarchy bar.
//
// Every shell-internal import in this plugin lives in this file. BarWidget.qml
// reaches it through a Loader by path and checks the Loader's status, so a
// shell release that moves its internals costs the panel and nothing else —
// the pill keeps showing the reading.

import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "sugarrush.glucose"
  ipcTarget: "sugarrush.glucose"

  property var anchorItem: null
  // The bar tracks the widget in its slot, not this nested panel, so the
  // popout coordinator and the open-panel dot have to be handed that widget.
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property int panelHours: setting("panelHours", 6)
  // What gets fetched, and so how far back the chart can be scrolled. The
  // overview strip spans exactly this. Same 6-72h range the dashboard's
  // minimap uses.
  readonly property int overviewHours: Math.max(panelHours, setting("overviewHours", 24))
  // How far the chart scrolls. The patterns keep their own, longer window.
  readonly property int scrollbackHours: Math.max(overviewHours, setting("scrollbackHours", 72))
  readonly property int insightDays: setting("insightDays", 14)
  readonly property int cacheMinutes: setting("panelCacheMinutes", 5)
  readonly property string snapshotCommand: setting("snapshotCommand", "sugarrush snapshot")
  // The same binary, asked a different question. Derived so a widget pointed
  // at a build directory edits that build's config too.
  readonly property string configCommand: snapshotCommand.replace(/snapshot.*$/, "config")
  readonly property string healthCommand: snapshotCommand.replace(/snapshot.*$/, "health --json")

  property var doc: null
  property string loadError: ""

  // "glucose" or "settings". A view, not a second panel: the cards below
  // simply swap, so the header, the strip and the popout identity stay put.
  //
  // Named for the data, not the moment: the view holds six hours of chart and
  // fourteen days of patterns as well as the reading, and it already contains
  // a card called Now. "Dashboard" is taken — that is the TUI, which the
  // button beside these chips opens.
  property string view: "glucose"
  // key -> value, as `sugarrush config` prints it.
  property var config: ({})
  property string configError: ""
  property double fetchedAt: 0

  readonly property var reading: doc && doc.now ? doc.now : null

  // Nightscout's trend names are machine words — "FortyFiveDown" reads as
  // FORTYFIVEDOWN once the hero styles it. The arrow already carries the
  // shape; this gives the same thing in words for anyone reading rather than
  // glancing.
  function trendWords(direction) {
    switch (String(direction)) {
      case "DoubleUp": return "rising fast"
      case "SingleUp": return "rising"
      case "FortyFiveUp": return "drifting up"
      case "Flat": return "steady"
      case "FortyFiveDown": return "drifting down"
      case "SingleDown": return "falling"
      case "DoubleDown": return "falling fast"
      default: return "trend unknown"
    }
  }
  readonly property var stats: doc && doc.stats ? doc.stats : null
  readonly property var sensor: doc && doc.sensor ? doc.sensor : null
  readonly property var forecast: doc && doc.forecast ? doc.forecast : null

  // The reading against its projection, as a ratio of the shell's display
  // type so both follow a theme's font scale. 1.8 lands on 50px at the
  // default scale; the projection is exactly half — a note beside the
  // reading, not a second headline.
  readonly property int readingPx: Math.round(Style.font.displayLarge * 1.8)
  readonly property int forecastPx: Math.round(readingPx / 2)

  // The same ladder the chart and the pill use, so a number means the same
  // thing wherever it appears.
  function classColor(klass) {
    switch (klass) {
      case "urgent-low":
      case "urgent-high": return "#cc241d"
      case "low":
      case "high": return "#d79921"
      case "in-range": return "#98971a"
      default: return barForeground
    }
  }

  // "6d 4h", the way the dashboard writes it.
  function ageWords(hours) {
    var d = Math.floor(hours / 24)
    var h = hours % 24
    return d > 0 ? d + "d " + h + "h" : h + "h"
  }

  // The countdown carries the state, so it is coloured like everything else:
  // amber inside the last day, red once it is over.
  function sensorColor() {
    if (!sensor || sensor.expired === undefined) return Qt.darker(barForeground, 1.35)
    if (sensor.expired) return "#cc241d"
    if (sensor.expires_in_h <= 24) return "#d79921"
    return Qt.darker(barForeground, 1.35)
  }

  function sensorWords() {
    if (!sensor) return ""
    var line = "sensor " + ageWords(sensor.age_h)
    if (sensor.expired === true) return line + " · expired"
    if (sensor.expires_in_h !== undefined) return line + " · " + ageWords(sensor.expires_in_h) + " left"
    return line
  }
  readonly property var agp: doc && doc.agp ? doc.agp : null
  readonly property bool hasInsights: insightDays > 0 && loadError === ""
    && doc && doc.insights && doc.insights.length > 0

  function stale() {
    return !doc || (Date.now() - fetchedAt) > cacheMinutes * 60000
  }

  // What the alarm machinery is doing, from `sugarrush health --json`. The
  // panel can show a perfect graph while nothing is watching overnight, and
  // that is precisely the state someone needs to be told about.
  property var health: null

  function alarmWords() {
    if (!health) return ""
    if (health.watcher_alive !== true) return "not watching"
    if (health.alarm_configured !== true) return "no alarm set"
    var sites = health.sites || []
    for (var i = 0; i < sites.length; i++) {
      var until = sites[i].snoozed_until_ms
      if (until) {
        var left = Math.round((until - Date.now()) / 60000)
        if (left > 0) return "snoozed " + left + "m"
      }
    }
    return "alarm armed"
  }

  readonly property bool alarmArmed: alarmWords() === "alarm armed"

  // Armed is the quiet state, and everything else is worth an eye: nothing
  // watching is the failure the panel exists to make visible, a snooze is
  // deliberate but temporary.
  function alarmColor() {
    var words = alarmWords()
    if (words === "not watching" || words === "no alarm set") return "#cc241d"
    if (words.indexOf("snoozed") === 0) return "#d79921"
    return Qt.darker(barForeground, 1.35)
  }

  function loadHealth() {
    healthProc.running = false
    Qt.callLater(function () { healthProc.running = true })
  }

  readonly property string summaryCommand: snapshotCommand.replace(/snapshot.*$/, "summary")
  // "" until a copy is attempted, then what happened. Cleared on the way out
  // so the panel never opens still claiming a copy from an hour ago.
  property string copyState: ""

  function copySummary() {
    root.copyState = "copying…"
    copyProc.running = false
    Qt.callLater(function () { copyProc.running = true })
  }

  function applyHealth(out) {
    try {
      root.health = JSON.parse(String(out || ""))
    } catch (e) {
      // Say nothing rather than claim a state we could not read: an empty
      // chip is honest, "alarm armed" would not be.
      root.health = null
    }
  }

  function refresh(force) {
    if (!force && !stale()) return
    // Drop whatever is in flight and start again, deferred: setting `running`
    // false and true in one pass is not a change at all, and skipping the
    // fetch whenever the property happens to read true loses refreshes for
    // good. The pill's fetch has the same shape for the same reason.
    snapProc.running = false
    Qt.callLater(function () { snapProc.running = true })
  }

  onOpenedChanged: if (opened) { refresh(false); loadHealth(); copyState = "" }

  // A cached document belongs to the command that produced it. When the
  // command changes the cache is about something else, so drop it rather than
  // showing the old source's numbers for up to panelCacheMinutes.
  onSnapshotCommandChanged: {
    if (!doc && loadError === "") return
    doc = null
    loadError = ""
    fetchedAt = 0
    if (opened) refresh(true)
  }

  function loadConfig() {
    // Drop whatever is in flight and start again, deferred. Guarding on
    // `running` is how the pill and the snapshot fetch each lost a refresh:
    // a first run that never finished left the flag true for good.
    configProc.running = false
    Qt.callLater(function () { configProc.running = true })
  }

  function applyConfig(out) {
    var next = ({})
    var lines = String(out || "").split("\n")
    for (var i = 0; i < lines.length; i++) {
      var at = lines[i].indexOf(" = ")
      if (at > 0) next[lines[i].slice(0, at)] = lines[i].slice(at + 3).trim()
    }
    root.config = next
  }

  // Set one setting, then re-read: the CLI refuses values the app would have
  // repaired, so the file is the only honest source of what actually stuck.
  function setConfig(key, value) {
    root.configError = ""
    setProc.command = ["bash", "-lc",
                       root.configCommand + " " + key + " " + value + " 2>&1"]
    setProc.running = false
    Qt.callLater(function () { setProc.running = true })
  }

  // Widget options live in the bar's own config, not sugarrush's.
  function setWidget(key, value) {
    if (root.bar) root.bar.run("omarchy bar set " + root.moduleName + " " + key + " " + value)
  }

  function num(key, fallback) {
    var raw = parseFloat(root.config[key])
    return isNaN(raw) ? fallback : raw
  }

  onViewChanged: if (view === "settings") loadConfig()
  onConfigCommandChanged: if (view === "settings") loadConfig()

  function apply(out) {
    var parsed
    try {
      parsed = JSON.parse(String(out || ""))
    } catch (e) {
      root.loadError = "could not read snapshot output — update sugarrush"
      return
    }
    if (parsed.schema !== 1) {
      // Kept short: the hero's detail is a single line that clips rather
      // than wraps, and a clipped explanation explains nothing.
      root.loadError = "snapshot schema " + parsed.schema + ", panel needs 1"
      return
    }
    root.doc = parsed
    root.loadError = parsed.error ? String(parsed.error) : ""
    root.fetchedAt = Date.now()
  }

  Process {
    id: configProc
    command: ["bash", "-lc", root.configCommand]
    stdout: StdioCollector {
      id: configOut
      waitForEnd: true
      onStreamFinished: root.applyConfig(configOut.text)
    }
  }

  Process {
    id: setProc
    stdout: StdioCollector {
      id: setOut
      waitForEnd: true
      onStreamFinished: {
        var text = setOut.text.trim()
        // The CLI answers "key = value" on success and an error otherwise.
        root.configError = text.indexOf(" = ") > 0 ? "" : text
        root.loadConfig()
      }
    }
  }

  Process {
    id: copyProc
    // `wl-copy` on Wayland, `xclip` for anyone running this under Xwayland or
    // a stray X session. Whichever answers first wins; if neither is
    // installed the panel says so rather than pretending it copied.
    command: ["bash", "-lc",
              root.summaryCommand + " | { wl-copy 2>/dev/null || xclip -selection clipboard 2>/dev/null; }"]
    onExited: function (code) {
      root.copyState = code === 0 ? "copied to the clipboard" : "could not copy — is wl-clipboard installed?"
      copyClear.restart()
    }
  }

  Timer {
    id: copyClear
    interval: 4000
    onTriggered: root.copyState = ""
  }

  Process {
    id: healthProc
    command: ["bash", "-lc", root.healthCommand]
    stdout: StdioCollector {
      id: healthOut
      waitForEnd: true
      onStreamFinished: root.applyHealth(healthOut.text)
    }
    stderr: StdioCollector {
      id: healthErr
      waitForEnd: true
      // A sugarrush too old for `health --json` says so here; the chip stays
      // empty and the rest of the panel is unaffected.
      onStreamFinished: if (healthErr.text.trim() !== "") console.warn("sugarrush panel health", healthErr.text.trim())
    }
  }

  Process {
    id: snapProc
    command: ["bash", "-lc",
              root.snapshotCommand + " --hours " + root.overviewHours
              + " --days " + root.insightDays]
    stdout: StdioCollector {
      id: snapOut
      waitForEnd: true
      onStreamFinished: root.apply(snapOut.text)
    }
    stderr: StdioCollector {
      id: snapErr
      waitForEnd: true
      onStreamFinished: if (snapErr.text.trim() !== "") console.warn("sugarrush panel", snapErr.text.trim())
    }
  }


  // A card: a labelled box saying which window its contents describe. The
  // panel used to stack a 6-hour chart under 24-hour statistics with nothing
  // saying so; naming each section is the whole point of this layout.
  component Card: Rectangle {
    id: card
    property string label: ""
    default property alias content: cardContent.data

    width: parent ? parent.width : 0
    implicitHeight: cardColumn.implicitHeight + Style.space(18)
    color: "transparent"
    radius: Style.cornerRadius
    border.width: 1
    border.color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.16)

    Column {
      id: cardColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: Style.space(9)
      spacing: Style.space(7)

      PanelSectionHeader {
        text: card.label
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
      }

      Column {
        id: cardContent
        width: parent.width
        spacing: Style.space(6)
      }
    }
  }

  // One editable number. The step is the unit the setting is measured in —
  // 0.1 mmol/L for a threshold, a whole day for a sensor — so holding the
  // button walks it the way the settings screen does.
  component SettingRow: Item {
    id: row
    property string label: ""
    property string suffix: ""
    property real value: 0
    property real step: 1
    property int decimals: 0
    property bool enabled: true
    signal changed(real next)

    width: parent ? parent.width : 0
    implicitHeight: Math.max(rowLabel.implicitHeight, minus.implicitHeight)

    Text {
      id: rowLabel
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
      color: root.barForeground
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: row.label
    }

    Row {
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(6)

      PanelActionButton {
        id: minus
        iconText: "−"
        tooltipText: "Less"
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
        enabled: row.enabled
        onClicked: row.changed(row.value - row.step)
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        width: Style.space(58)
        horizontalAlignment: Text.AlignHCenter
        color: root.barForeground
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.font.caption
        text: row.value.toFixed(row.decimals) + row.suffix
      }

      PanelActionButton {
        iconText: "+"
        tooltipText: "More"
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
        enabled: row.enabled
        onClicked: row.changed(row.value + row.step)
      }
    }
  }

  KeyboardPanel {
    id: surface
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    focusTarget: keys
    contentWidth: surface.fittedContentWidth(Style.space(420))
    contentHeight: surface.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keys
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function (direction) { root.switchPanel(direction) }

      // Content can outgrow the room a popup is allowed — a patterns card on
      // top of the other three does it on a short screen — and a capped
      // KeyboardPanel would simply clip the overflow. Scroll it instead, the
      // way the shell's own panels do.
      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(10)

        // Wordmark left, controls right, on one line.
        Item {
          width: parent.width
          implicitHeight: Math.max(mark.height, controls.height)

          Image {
            id: mark
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: Math.round(parent.width / 2)
            source: Qt.resolvedUrl("logo.png")
            sourceSize.width: parent.width * 1.5
            fillMode: Image.PreserveAspectFit
            smooth: true
          }

          Row {
            id: controls
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(4)

            PanelActionButton {
              iconText: ""
              tooltipText: "Fetch now"
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              onClicked: root.refresh(true)
            }

            PanelActionButton {
              iconText: ""
              tooltipText: "Open the dashboard"
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              onClicked: {
                root.close()
                if (root.bar) {
                  root.bar.run(root.setting("onClick", "omarchy-launch-floating-terminal-with-presentation sugarrush"))
                }
              }
            }
          }
        }

        ButtonGroup {
          options: [{ value: "glucose", label: "Glucose" },
                    { value: "profile", label: "Profile" },
                    { value: "settings", label: "Settings" }]
          value: root.view
          foreground: root.barForeground
          fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
          fontSize: Style.font.caption
          focusable: false
          onChanged: function (next) { root.view = next }
        }

        // Whatever went wrong replaces the cards: there is nothing to put in
        // them, and three empty boxes explain less than one sentence does.
        Text {
          visible: (root.loadError !== "" || !root.reading) && root.view === "glucose"
          width: parent.width
          wrapMode: Text.WordWrap
          color: root.barForeground
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.body
          text: root.loadError !== "" ? root.loadError : "waiting for the first reading"
        }

        // Two numbers at the same size: the reading, and where it lands in
        // half an hour. A 9.6 rising to 10.4 is a different evening from a 9.6
        // settling, and that difference is the one the panel exists to show.
        Card {
          label: "Now"
          visible: root.reading !== null && root.loadError === "" && root.view === "glucose"

          Item {
            width: parent.width
            implicitHeight: nowValue.implicitHeight + nowCaption.implicitHeight

            // Baseline-anchored rather than stacked in a Row: the two numbers
            // are different sizes, and boxes aligned at the top leave their
            // digits sitting at different heights.
            Text {
              id: nowValue
              anchors.left: parent.left
              anchors.top: parent.top
              color: root.reading ? root.classColor(root.reading.class) : root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: root.readingPx
              font.bold: true
              text: root.reading ? root.reading.value : "—"
            }

            Text {
              id: heroArrow
              visible: root.forecast !== null
              anchors.left: nowValue.right
              // Clear the caption as well as the number: "mmol/L · falling" is
              // wider than "9.1", and anchoring to the number alone ran the
              // two captions into each other.
              anchors.leftMargin: Style.space(12)
                + Math.max(0, nowCaption.implicitWidth - nowValue.implicitWidth)
              anchors.baseline: nowValue.baseline
              color: Qt.darker(root.barForeground, 1.6)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.title
              text: "→"
            }

            // Only drawn when there is a forecast: a sensor gap makes the
            // projection a fabrication, and predict refuses it rather than
            // guessing. An arrow to nothing would imply one anyway.
            Text {
              id: nextValue
              visible: root.forecast !== null
              anchors.left: heroArrow.right
              anchors.leftMargin: Style.space(12)
              anchors.baseline: nowValue.baseline
              color: root.forecast ? root.classColor(root.forecast.class) : root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: root.forecastPx
              opacity: 0.62
              text: root.forecast ? root.forecast.value.toFixed(1) : ""
            }

            Text {
              id: nowCaption
              anchors.left: nowValue.left
              anchors.top: nowValue.bottom
              color: Qt.darker(root.barForeground, 1.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.reading
                ? (root.doc ? root.doc.units : "") + " · " + root.reading.arrow + " "
                  + root.trendWords(root.reading.direction)
                : ""
            }

            Text {
              visible: root.forecast !== null
              anchors.left: nextValue.left
              anchors.baseline: nowCaption.baseline
              color: Qt.darker(root.barForeground, 1.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.forecast ? "in " + root.forecast.in_min + " min" : ""
            }

            Rectangle {
              id: nowPill
              anchors.right: parent.right
              anchors.top: parent.top
              implicitWidth: pillText.implicitWidth + Style.space(14)
              implicitHeight: pillText.implicitHeight + Style.space(6)
              radius: height / 2
              color: "transparent"
              border.width: 1
              border.color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.28)

              Text {
                id: pillText
                anchors.centerIn: parent
                color: Qt.darker(root.barForeground, 1.2)
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: root.reading
                  ? "Δ " + root.reading.delta + " · " + root.reading.age_min + "m ago"
                  : ""
              }
            }
          }
        }

        Card {
          label: "Last " + root.panelHours + (root.panelHours === 1 ? " hour" : " hours")
          visible: root.loadError === "" && root.view === "glucose"

          Chart {
            width: parent.width
            height: Style.space(130)
            doc: root.doc
            foreground: root.barForeground
          }
        }

        Card {
          label: root.stats
            ? "Last " + root.stats.window_h + (root.stats.window_h === 1 ? " hour" : " hours")
            : ""
          visible: root.stats !== null && root.loadError === "" && root.view === "glucose"

          TirBar {
            width: parent.width
            height: Style.space(12)
            stats: root.stats
          }

          Text {
            width: parent.width
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.stats
              ? "mean " + root.stats.mean.toFixed(1)
                + " · GMI " + root.stats.gmi.toFixed(1) + "%"
                + " · CV " + (root.stats.cv === undefined ? "—" : root.stats.cv.toFixed(1) + "%")
              : ""
          }
        }

        // ---- the profile view
        Card {
          label: root.agp
            ? "Typical day · " + root.agp.days + " days"
            : "Typical day"
          visible: root.loadError === "" && root.view === "profile"

          AgpChart {
            width: parent.width
            height: Style.space(150)
            visible: root.agp !== null
            agp: root.agp
            range: root.doc && root.doc.range ? root.doc.range : null
            foreground: root.barForeground
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.agp
              ? "median, with the middle half and the outer 5–95% behind it"
              : "Not enough days yet — a profile drawn from one or two of them "
                + "cannot tell a habit from a bad Tuesday."
          }

          // The same text `sugarrush export` writes, on the clipboard: the
          // point of a profile is usually a conversation with a clinician,
          // and that conversation happens somewhere else.
          Row {
            spacing: Style.space(10)

            Button {
              text: "Copy summary"
              bordered: true
              focusable: false
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.copySummary()
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              color: Qt.darker(root.barForeground, 1.2)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.copyState
            }
          }
        }

        Card {
          label: "Patterns · last " + root.insightDays + " days"
          // Not gated on there being any: "nothing stands out" is an answer,
          // and a card that vanishes reads as a panel that failed to load.
          visible: root.insightDays > 0 && root.loadError === "" && root.view === "profile"

          Repeater {
            model: root.doc && root.doc.insights ? root.doc.insights : []

            Text {
              required property var modelData
              width: column.width - Style.space(24)
              wrapMode: Text.WordWrap
              color: root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: modelData.text
            }
          }

          Text {
            visible: !root.doc || !root.doc.insights || root.doc.insights.length === 0
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "Nothing recurring stands out over these days."
          }
        }

        // ---- the settings view
        Card {
          label: "Alarm thresholds · " + (root.config["units"] === "mgdl" ? "mg/dL" : "mmol/L")
          visible: root.view === "settings"

          SettingRow {
            label: "Urgent low"
            value: root.num("alerts.urgent_low", 3.5)
            step: root.config["units"] === "mgdl" ? 1 : 0.1
            decimals: root.config["units"] === "mgdl" ? 0 : 1
            onChanged: function (next) { root.setConfig("alerts.urgent_low", next.toFixed(decimals)) }
          }
          SettingRow {
            label: "Low"
            value: root.num("alerts.low", 3.9)
            step: root.config["units"] === "mgdl" ? 1 : 0.1
            decimals: root.config["units"] === "mgdl" ? 0 : 1
            onChanged: function (next) { root.setConfig("alerts.low", next.toFixed(decimals)) }
          }
          SettingRow {
            label: "High"
            value: root.num("alerts.high", 10.0)
            step: root.config["units"] === "mgdl" ? 1 : 0.1
            decimals: root.config["units"] === "mgdl" ? 0 : 1
            onChanged: function (next) { root.setConfig("alerts.high", next.toFixed(decimals)) }
          }
          SettingRow {
            label: "Urgent high"
            value: root.num("alerts.urgent_high", 13.9)
            step: root.config["units"] === "mgdl" ? 1 : 0.1
            decimals: root.config["units"] === "mgdl" ? 0 : 1
            onChanged: function (next) { root.setConfig("alerts.urgent_high", next.toFixed(decimals)) }
          }

          // The CLI refuses a value the app would have quietly repaired —
          // crossed thresholds, mostly — and that refusal belongs on screen
          // rather than in a log nobody reads.
          Text {
            visible: root.configError !== ""
            width: parent.width
            wrapMode: Text.WordWrap
            color: "#cc241d"
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.configError
          }
        }

        Card {
          label: "Alarm"
          visible: root.view === "settings"

          Item {
            width: parent.width
            implicitHeight: Math.max(soundLabel.implicitHeight, soundToggle.implicitHeight)

            Text {
              id: soundLabel
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
              color: root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: "Audible alarm"
            }

            ToggleSwitch {
              id: soundToggle
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              checked: root.config["alerts.sound"] !== "false"
              foreground: root.barForeground
              onToggled: root.setConfig("alerts.sound", checked ? "off" : "on")
            }
          }

          SettingRow {
            label: "Sensor life"
            suffix: " d"
            value: root.num("sensor_days", 10)
            step: 1
            onChanged: function (next) {
              root.setConfig("sensor_days", Math.max(0, Math.min(30, next)).toFixed(0))
            }
          }
        }

        Card {
          label: "This panel"
          visible: root.view === "settings"

          SettingRow {
            label: "Chart window"
            suffix: " h"
            value: root.panelHours
            step: 1
            onChanged: function (next) {
              root.setWidget("panelHours", Math.max(1, Math.min(72, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Overview span"
            suffix: " h"
            value: root.overviewHours
            step: 6
            onChanged: function (next) {
              root.setWidget("overviewHours", Math.max(6, Math.min(72, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Scroll back"
            suffix: " h"
            value: root.scrollbackHours
            step: 12
            onChanged: function (next) {
              root.setWidget("scrollbackHours", Math.max(6, Math.min(336, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Pattern history"
            suffix: " d"
            value: root.insightDays
            step: 1
            onChanged: function (next) {
              root.setWidget("insightDays", Math.max(0, Math.min(90, next)).toFixed(0))
            }
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.45)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "Everything else — sites, tokens, quiet hours, themes — lives in the dashboard."
          }
        }

        // A status strip, not a card: the sensor and the fetch time describe the
        // rig rather than the glucose, and they are the two things here that
        // do not change every five minutes.
        Item {
          width: parent.width
          implicitHeight: Math.max(sensorText.implicitHeight, fetchedText.implicitHeight,
                                   alarmChip.visible ? alarmChip.height : 0)
            + Style.space(8)
          visible: (root.sensor !== null || root.health !== null)
            && root.loadError === "" && root.view === "glucose"

          Rectangle {
            anchors.top: parent.top
            width: parent.width
            height: 1
            color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.16)
          }

          // The alarm's own state, left of the sensor: both describe the rig,
          // and this is the one that decides whether tonight is covered.
          Rectangle {
            id: alarmChip
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.bottomMargin: -Style.space(2)
            visible: alarmChipText.text !== ""
            width: alarmChipText.implicitWidth + Style.space(14)
            height: alarmChipText.implicitHeight + Style.space(6)
            radius: height / 2
            color: "transparent"
            border.width: 1
            border.color: Qt.rgba(root.alarmColor().r, root.alarmColor().g,
                                  root.alarmColor().b, root.alarmArmed ? 0.4 : 0.9)

            Text {
              id: alarmChipText
              anchors.centerIn: parent
              color: root.alarmColor()
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.alarmWords()
            }
          }

          Text {
            id: sensorText
            anchors.left: alarmChip.visible ? alarmChip.right : parent.left
            anchors.leftMargin: alarmChip.visible ? Style.space(8) : 0
            anchors.bottom: parent.bottom
            color: root.sensorColor()
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.sensorWords()
          }

          Text {
            id: fetchedText
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            color: Qt.darker(root.barForeground, 1.45)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.reading ? "updated " + root.reading.age_min + "m ago" : ""
          }
        }
        }
      }
    }
  }
}
