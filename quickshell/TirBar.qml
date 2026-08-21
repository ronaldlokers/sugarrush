// Five bands, widths proportional to their share of the window. Colours are
// the conventional CGM ones rather than the theme's, because the bands mean
// the same thing on every theme.

import QtQuick

Item {
  id: root

  property var stats: null

  readonly property var bands: stats && stats.tir
    ? [
      { share: stats.tir.very_low, color: "#cc241d" },
      { share: stats.tir.low, color: "#d79921" },
      { share: stats.tir.in_range, color: "#98971a" },
      { share: stats.tir.high, color: "#d79921" },
      { share: stats.tir.very_high, color: "#cc241d" }
    ]
    : []

  visible: bands.length > 0

  Row {
    anchors.fill: parent
    spacing: 0

    Repeater {
      model: root.bands

      Rectangle {
        required property var modelData
        width: root.width * Math.max(0, modelData.share) / 100
        height: root.height
        color: modelData.color
      }
    }
  }
}
