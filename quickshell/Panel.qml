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
        spacing: Style.space(10)

        // The wordmark leads, and everything else sits under it. The cube in
        // the lockup is the mark, so the hero below carries no icon of its
        // own — one character per panel is plenty.
        Image {
          width: Math.round(parent.width / 2)
          source: Qt.resolvedUrl("logo.png")
          sourceSize.width: parent.width * 1.5
          fillMode: Image.PreserveAspectFit
          smooth: true
        }

        // The reading, its trend, and the panel's controls — the shape
        // tailscale and dropbox give their headers.
        Item {
          id: header
          width: parent.width
          implicitHeight: hero.implicitHeight

          PanelHero {
            id: hero
            width: parent.width
            title: root.reading ? root.reading.value + " " + (root.doc ? root.doc.units : "") : "—"
            meta: root.reading
              ? root.reading.arrow + "  " + root.trendWords(root.reading.direction)
              : root.loadError
            detail: root.reading
              ? "Δ " + root.reading.delta + " · " + root.reading.age_min + "m ago"
              : ""
            // The bar's own foreground, not the alert colour: the chart
            // carries the state in the line itself, and a hero that changed
            // colour under the reading made the number harder to read than
            // the colour was worth.
            foreground: root.barForeground
            fontFamily: root.bar ? root.bar.fontFamily : Style.font.family

            trailingControl: Component {
              Row {
                spacing: Style.space(4)

                PanelActionButton {
                  iconText: "\uf021"
                  tooltipText: "Fetch now"
                  foreground: hero.foreground
                  fontFamily: hero.fontFamily
                  onClicked: root.refresh(true)
                }

                PanelActionButton {
                  iconText: "\uf120"
                  tooltipText: "Open the dashboard"
                  foreground: hero.foreground
                  fontFamily: hero.fontFamily
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

        PanelSeparator {
          foreground: root.barForeground
        }

        Chart {
          width: parent.width
          height: Style.space(140)
          doc: root.doc
          foreground: root.barForeground
          // On an error the hero already says what went wrong; an empty plot
          // saying "no readings" underneath it just repeats the bad news in
          // less useful words.
          visible: root.loadError === ""
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
          // One decimal, explicitly: JSON's 7.0 arrives in JS as 7, and a GMI
          // printed as "7%" next to a mean printed as "8.7" reads like two
          // different precisions of measurement.
          text: root.stats
            ? root.stats.window_h + "h · mean " + root.stats.mean.toFixed(1)
              + " · GMI " + root.stats.gmi.toFixed(1) + "%"
              + " · CV " + (root.stats.cv === undefined ? "—" : root.stats.cv.toFixed(1) + "%")
            : ""
        }

        PanelSectionHeader {
          text: "Patterns"
          foreground: root.barForeground
          // The section exists only when it has something to say. An empty
          // one asked the reader to interpret a blank, and its old caption —
          // "not enough history yet" — was wrong whenever the history was
          // there and simply held no repeating low or high.
          visible: root.hasInsights
        }

        Column {
          width: parent.width
          spacing: Style.space(4)
          visible: root.hasInsights

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
        }
      }
    }
  }
}
