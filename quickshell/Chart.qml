// The last N hours as a line over a shaded target band. Canvas rather than
// Shapes: one paint over an array, and new data is a single requestPaint().

import QtQuick
import qs.Commons

Item {
  id: root

  property var doc: null
  property color foreground: Color.foreground

  readonly property var series: doc && doc.series ? doc.series : []
  readonly property var range: doc && doc.range ? doc.range : null

  onDocChanged: canvas.requestPaint()

  Text {
    anchors.centerIn: parent
    visible: root.series.length === 0
    color: Qt.darker(root.foreground, 1.4)
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    text: "no readings in this window"
  }

  Canvas {
    id: canvas
    anchors.fill: parent
    visible: root.series.length > 0

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      if (root.series.length === 0 || !root.range) return

      // The scale covers the alert bounds as well as the readings, so the
      // target band is always visible even on a flat day well inside it.
      var lo = root.range.urgent_low
      var hi = root.range.urgent_high
      for (var i = 0; i < root.series.length; i++) {
        lo = Math.min(lo, root.series[i][1])
        hi = Math.max(hi, root.series[i][1])
      }
      var pad = (hi - lo) * 0.1 || 1
      lo -= pad
      hi += pad

      var t0 = root.series[0][0]
      var t1 = root.series[root.series.length - 1][0]
      var span = Math.max(1, t1 - t0)
      function x(t) { return (t - t0) / span * width }
      function y(v) { return height - (v - lo) / (hi - lo) * height }

      // Target band first, so the line draws over it.
      ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.08)
      ctx.fillRect(0, y(root.range.high), width, y(root.range.low) - y(root.range.high))

      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.2)
      ctx.lineWidth = 1
      var bounds = [root.range.urgent_low, root.range.urgent_high]
      for (var b = 0; b < bounds.length; b++) {
        ctx.beginPath()
        ctx.moveTo(0, y(bounds[b]))
        ctx.lineTo(width, y(bounds[b]))
        ctx.stroke()
      }

      ctx.strokeStyle = root.doc && root.doc.now ? root.doc.now.color : root.foreground
      ctx.lineWidth = 2
      ctx.beginPath()
      ctx.moveTo(x(root.series[0][0]), y(root.series[0][1]))
      for (var j = 1; j < root.series.length; j++) {
        ctx.lineTo(x(root.series[j][0]), y(root.series[j][1]))
      }
      ctx.stroke()
    }
  }
}
