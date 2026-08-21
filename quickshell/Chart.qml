// The last N hours as a line, over a typical day and between the alert
// thresholds. Canvas rather than Shapes: one paint over a few arrays, and new
// data is a single requestPaint().
//
// The axes are drawn rather than labelled with a legend box: the value ticks
// sit at the thresholds themselves, so a coloured line is its own label.

import QtQuick
import qs.Commons

Item {
  id: root

  property var doc: null
  property color foreground: Color.foreground

  readonly property var series: doc && doc.series ? doc.series : []
  readonly property var range: doc && doc.range ? doc.range : null
  readonly property var band: doc && doc.band ? doc.band : null

  // Conventional CGM colours, matching the time-in-range bar: the bands mean
  // the same thing whatever the theme is.
  readonly property color urgentColor: "#cc241d"
  readonly property color warnColor: "#d79921"

  // Room for the value ticks on the left and the clock on the bottom.
  readonly property int gutterLeft: 34
  readonly property int gutterBottom: 14

  onDocChanged: canvas.requestPaint()
  onWidthChanged: canvas.requestPaint()
  onHeightChanged: canvas.requestPaint()

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

      var plotX = root.gutterLeft
      var plotW = Math.max(1, width - root.gutterLeft)
      var plotH = Math.max(1, height - root.gutterBottom)

      // The scale always covers the alert bounds, so the thresholds are on
      // screen even on a flat day well inside them, and always covers the
      // band, so a wide typical day is not clipped.
      var lo = root.range.urgent_low
      var hi = root.range.urgent_high
      for (var i = 0; i < root.series.length; i++) {
        lo = Math.min(lo, root.series[i][1])
        hi = Math.max(hi, root.series[i][1])
      }
      if (root.band) {
        for (var b = 0; b < root.band.points.length; b++) {
          lo = Math.min(lo, root.band.points[b][1])
          hi = Math.max(hi, root.band.points[b][3])
        }
      }
      var pad = (hi - lo) * 0.08 || 1
      lo -= pad
      hi += pad

      var t0 = root.series[0][0]
      var t1 = root.series[root.series.length - 1][0]
      var span = Math.max(1, t1 - t0)
      function x(t) { return plotX + (t - t0) / span * plotW }
      function y(v) { return plotH - (v - lo) / (hi - lo) * plotH }

      var dim = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.55)
      ctx.font = "10px " + Style.font.family

      // ---- the typical day, behind everything else
      if (root.band && root.band.points.length > 1) {
        var pts = root.band.points
        ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.13)
        ctx.beginPath()
        ctx.moveTo(x(pts[0][0]), y(pts[0][3]))
        for (var u = 1; u < pts.length; u++) ctx.lineTo(x(pts[u][0]), y(pts[u][3]))
        for (var d = pts.length - 1; d >= 0; d--) ctx.lineTo(x(pts[d][0]), y(pts[d][1]))
        ctx.closePath()
        ctx.fill()

        ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.45)
        ctx.lineWidth = 1
        ctx.setLineDash([3, 3])
        ctx.beginPath()
        ctx.moveTo(x(pts[0][0]), y(pts[0][2]))
        for (var m = 1; m < pts.length; m++) ctx.lineTo(x(pts[m][0]), y(pts[m][2]))
        ctx.stroke()
        ctx.setLineDash([])
      }

      // ---- thresholds, each labelled by its own value on the axis
      var levels = [
        { value: root.range.urgent_high, color: root.urgentColor },
        { value: root.range.high, color: root.warnColor },
        { value: root.range.low, color: root.warnColor },
        { value: root.range.urgent_low, color: root.urgentColor }
      ]
      ctx.lineWidth = 1
      for (var l = 0; l < levels.length; l++) {
        var ly = y(levels[l].value)
        if (ly < 0 || ly > plotH) continue
        ctx.strokeStyle = Qt.rgba(
          Qt.color(levels[l].color).r,
          Qt.color(levels[l].color).g,
          Qt.color(levels[l].color).b,
          0.55)
        ctx.beginPath()
        ctx.moveTo(plotX, ly)
        ctx.lineTo(width, ly)
        ctx.stroke()

        ctx.fillStyle = levels[l].color
        ctx.textAlign = "right"
        // One decimal on every tick: "10" beside "13.9" and "4.8" reads as a
        // coarser measurement than its neighbours rather than the same scale.
        ctx.fillText(levels[l].value.toFixed(1), plotX - 5, Math.min(plotH - 1, ly + 3))
      }

      // ---- the clock, every two hours on the hour
      var step = 2 * 3_600_000
      var firstTick = t0 - (t0 % step) + step
      ctx.fillStyle = dim
      ctx.textAlign = "center"
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15)
      for (var tick = firstTick; tick <= t1; tick += step) {
        var tx = x(tick)
        ctx.beginPath()
        ctx.moveTo(tx, 0)
        ctx.lineTo(tx, plotH)
        ctx.stroke()
        var when = new Date(tick)
        var label = ("0" + when.getHours()).slice(-2) + ":" + ("0" + when.getMinutes()).slice(-2)
        ctx.fillText(label, tx, height - 2)
      }

      // ---- today
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
