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
  readonly property var stats: doc && doc.stats ? doc.stats : null

  function stale() {
    return !doc || (Date.now() - fetchedAt) > cacheMinutes * 60000
  }

  function refresh(force) {
    if (!force && !stale()) return
    if (snapProc.running) return
    snapProc.running = true
  }

  onOpenedChanged: if (opened) refresh(false)

  function apply(out) {
    var parsed
    try {
      parsed = JSON.parse(String(out || ""))
    } catch (e) {
      root.loadError = "could not read snapshot output — update sugarrush"
      return
    }
    if (parsed.schema !== 1) {
      root.loadError = "this sugarrush speaks snapshot schema " + parsed.schema + ", the panel speaks 1"
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

      Column {
        id: column
        width: parent.width
        spacing: Style.space(12)

        PanelHero {
          title: root.reading ? root.reading.value + " " + (root.doc ? root.doc.units : "") : "—"
          meta: root.reading ? root.reading.arrow + "  " + root.reading.direction : ""
          detail: root.reading
            ? "Δ " + root.reading.delta + " · " + root.reading.age_min + "m ago"
            : root.loadError
          foreground: root.reading ? root.reading.color : root.barForeground
        }

        Chart {
          width: parent.width
          height: Style.space(120)
          doc: root.doc
          foreground: root.barForeground
        }

        PanelSectionHeader {
          text: "Time in range"
          foreground: root.barForeground
          visible: root.stats !== null
        }

        TirBar {
          width: parent.width
          height: Style.space(14)
          stats: root.stats
        }

        Text {
          visible: root.stats !== null
          width: parent.width
          color: root.barForeground
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          text: root.stats
            ? root.stats.window_h + "h · mean " + root.stats.mean
              + " · GMI " + root.stats.gmi + "%"
              + " · CV " + (root.stats.cv === undefined ? "—" : root.stats.cv + "%")
            : ""
        }

        PanelSectionHeader {
          text: "Patterns"
          foreground: root.barForeground
          visible: root.insightDays > 0
        }

        Column {
          width: parent.width
          spacing: Style.space(4)
          visible: root.insightDays > 0

          Repeater {
            model: root.doc && root.doc.insights ? root.doc.insights : []

            Text {
              required property var modelData
              width: column.width
              wrapMode: Text.WordWrap
              color: root.barForeground
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              text: modelData.text
            }
          }

          Text {
            visible: !root.doc || !root.doc.insights || root.doc.insights.length === 0
            color: Qt.darker(root.barForeground, 1.4)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            text: "not enough history yet for patterns"
          }
        }

        Row {
          spacing: Style.space(8)

          // Ui.Button, not PanelActionButton: the latter is an icon button
          // (`iconText`) with no text label.
          Button {
            text: "Refresh"
            foreground: root.barForeground
            fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
            fontSize: Style.font.bodySmall
            horizontalPadding: Style.spacing.controlPaddingX
            verticalPadding: Style.spacing.controlPaddingY
            bordered: true
            onClicked: root.refresh(true)
          }

          Button {
            text: "Open dashboard"
            foreground: root.barForeground
            fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
            fontSize: Style.font.bodySmall
            horizontalPadding: Style.spacing.controlPaddingX
            verticalPadding: Style.spacing.controlPaddingY
            bordered: true
            onClicked: {
              root.close()
              if (root.bar) {
                root.bar.run(root.setting("onClick", "omarchy-launch-floating-terminal-with-presentation sugarrush"))
              }
            }
          }
        }
      }
    }
  }
}
