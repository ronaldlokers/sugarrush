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
  readonly property int insightDays: setting("insightDays", 14)
  readonly property int cacheMinutes: setting("panelCacheMinutes", 5)
  readonly property string snapshotCommand: setting("snapshotCommand", "sugarrush snapshot")

  property var doc: null
  property string loadError: ""
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
  readonly property bool hasInsights: insightDays > 0 && loadError === ""
    && doc && doc.insights && doc.insights.length > 0

  function stale() {
    return !doc || (Date.now() - fetchedAt) > cacheMinutes * 60000
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

  onOpenedChanged: if (opened) refresh(false)

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
    id: snapProc
    command: ["bash", "-lc", root.snapshotCommand + " --hours " + root.panelHours + " --days " + root.insightDays]
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

        // Whatever went wrong replaces the cards: there is nothing to put in
        // them, and three empty boxes explain less than one sentence does.
        Text {
          visible: root.loadError !== "" || !root.reading
          width: parent.width
          wrapMode: Text.WordWrap
          color: root.barForeground
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.body
          text: root.loadError !== "" ? root.loadError : "waiting for the first reading"
        }

        Card {
          label: "Now"
          visible: root.reading !== null && root.loadError === ""

          Item {
            width: parent.width
            implicitHeight: Math.max(nowRead.implicitHeight, nowPill.implicitHeight)

            Column {
              id: nowRead
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(1)

              Text {
                color: root.barForeground
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.title
                text: root.reading
                  ? root.reading.value + " " + (root.doc ? root.doc.units : "")
                  : ""
              }

              Text {
                color: Qt.darker(root.barForeground, 1.35)
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: root.reading
                  ? root.reading.arrow + "  " + root.trendWords(root.reading.direction)
                  : ""
              }
            }

            Text {
              anchors.right: nowPill.left
              anchors.rightMargin: Style.space(10)
              anchors.verticalCenter: parent.verticalCenter
              visible: root.sensor !== null
              color: root.sensorColor()
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.sensorWords()
            }

            Rectangle {
              id: nowPill
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
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
          visible: root.loadError === ""

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
          visible: root.stats !== null && root.loadError === ""

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

        Card {
          label: "Patterns · last " + root.insightDays + " days"
          visible: root.hasInsights

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
          }
        }
      }
    }
  }
}
