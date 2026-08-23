// Five bands, widths proportional to their share of the window.
//
// Coloured from the document's palette, which is the user's own: the bands do
// mean the same thing on every theme, but "the same thing" is exactly what a
// theme names. Hard-coding them left anyone on the colourblind preset reading
// a red/green bar. It also flattened low and urgent-low into one red, which
// the palette has always kept apart.

import QtQuick

Item {
  id: root

  property var stats: null
  // `doc.theme`, or null against a sugarrush too old to send one.
  property var themeColors: null

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  readonly property var bands: stats && stats.tir
    ? [
      { share: stats.tir.very_low, color: themed("urgent", "#cc241d") },
      { share: stats.tir.low, color: themed("low", "#d79921") },
      { share: stats.tir.in_range, color: themed("in_range", "#98971a") },
      { share: stats.tir.high, color: themed("high", "#d79921") },
      { share: stats.tir.very_high, color: themed("urgent", "#cc241d") }
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
