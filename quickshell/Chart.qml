// The last N hours as a line, over a typical day and between the alert
// thresholds. Canvas rather than Shapes: one paint over a few arrays, and new
// data is a single requestPaint().
//
// The axes are drawn rather than labelled with a legend box: the value ticks
// sit at the thresholds themselves, so a coloured line is its own label.
//
// The scale lives in properties rather than inside onPaint, because the hover
// crosshair has to land on the same pixels the line was drawn at — two copies
// of that arithmetic would drift the moment either changed.

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
  readonly property color inRangeColor: "#98971a"

  // The colour a reading is drawn in — the same ladder `alert.rs` classifies
  // by, so a segment of the line means what the pill means.
  function colorFor(value) {
    if (!range) return foreground
    if (value <= range.urgent_low || value >= range.urgent_high) return urgentColor
    if (value < range.low || value > range.high) return warnColor
    return inRangeColor
  }

  // Room for the value ticks on the left and the clock on the bottom.
  readonly property int gutterLeft: 34
  readonly property int gutterBottom: 14

  readonly property real plotX: gutterLeft
  readonly property real plotW: Math.max(1, width - gutterLeft)
  readonly property real plotH: Math.max(1, height - gutterBottom)

  readonly property real firstAt: series.length > 0 ? series[0][0] : 0
  readonly property real lastAt: series.length > 0 ? series[series.length - 1][0] : 1
  readonly property real span: Math.max(1, lastAt - firstAt)

  // The scale always covers the alert bounds, so the thresholds are on screen
  // even on a flat day well inside them, and always covers the band, so a wide
  // typical day is not clipped.
  readonly property var bounds: {
    if (!range || series.length === 0) return { lo: 0, hi: 1 }
    var lo = range.urgent_low
    var hi = range.urgent_high
    for (var i = 0; i < series.length; i++) {
      lo = Math.min(lo, series[i][1])
      hi = Math.max(hi, series[i][1])
    }
    if (band) {
      for (var b = 0; b < band.points.length; b++) {
        lo = Math.min(lo, band.points[b][1])
        hi = Math.max(hi, band.points[b][3])
      }
    }
    var pad = (hi - lo) * 0.08 || 1
    return { lo: lo - pad, hi: hi + pad }
  }

  function xOf(t) { return plotX + (t - firstAt) / span * plotW }
  function yOf(v) { return plotH - (v - bounds.lo) / (bounds.hi - bounds.lo) * plotH }

  // The reading under the pointer, or -1. Snapped to a real reading rather
  // than interpolated: the panel should never show a number the sensor did
  // not report.
  property int hoverIndex: -1
  readonly property var hovered: hoverIndex >= 0 && hoverIndex < series.length
    ? series[hoverIndex]
    : null

  function nearestIndex(px) {
    if (series.length === 0) return -1
    var best = 0
    var bestGap = Number.MAX_VALUE
    for (var i = 0; i < series.length; i++) {
      var gap = Math.abs(xOf(series[i][0]) - px)
      if (gap < bestGap) {
        bestGap = gap
        best = i
      }
    }
    return best
  }

  function clockAt(ms) {
    var when = new Date(ms)
    return ("0" + when.getHours()).slice(-2) + ":" + ("0" + when.getMinutes()).slice(-2)
  }

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

      var plotX = root.plotX
      var plotH = root.plotH
      var dim = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.55)
      ctx.font = "10px " + Style.font.family

      // ---- the typical day, behind everything else
      if (root.band && root.band.points.length > 1) {
        var pts = root.band.points
        ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.13)
        ctx.beginPath()
        ctx.moveTo(root.xOf(pts[0][0]), root.yOf(pts[0][3]))
        for (var u = 1; u < pts.length; u++) ctx.lineTo(root.xOf(pts[u][0]), root.yOf(pts[u][3]))
        for (var d = pts.length - 1; d >= 0; d--) ctx.lineTo(root.xOf(pts[d][0]), root.yOf(pts[d][1]))
        ctx.closePath()
        ctx.fill()

        ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.45)
        ctx.lineWidth = 1
        ctx.setLineDash([3, 3])
        ctx.beginPath()
        ctx.moveTo(root.xOf(pts[0][0]), root.yOf(pts[0][2]))
        for (var m = 1; m < pts.length; m++) ctx.lineTo(root.xOf(pts[m][0]), root.yOf(pts[m][2]))
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
        var ly = root.yOf(levels[l].value)
        if (ly < 0 || ly > plotH) continue
        var lc = Qt.color(levels[l].color)
        ctx.strokeStyle = Qt.rgba(lc.r, lc.g, lc.b, 0.55)
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
      var step = 2 * 3600000
      var firstTick = root.firstAt - (root.firstAt % step) + step
      ctx.fillStyle = dim
      ctx.textAlign = "center"
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15)
      for (var tick = firstTick; tick <= root.lastAt; tick += step) {
        var tx = root.xOf(tick)
        ctx.beginPath()
        ctx.moveTo(tx, 0)
        ctx.lineTo(tx, plotH)
        ctx.stroke()
        ctx.fillText(root.clockAt(tick), tx, height - 2)
      }

      // ---- today, one segment at a time so the line carries its own state
      ctx.lineWidth = 2
      ctx.lineCap = "round"
      for (var j = 1; j < root.series.length; j++) {
        var from = root.series[j - 1]
        var to = root.series[j]
        // The segment takes the colour of where it arrives: a line crossing
        // into the low band should already look low when it gets there.
        ctx.strokeStyle = root.colorFor(to[1])
        ctx.beginPath()
        ctx.moveTo(root.xOf(from[0]), root.yOf(from[1]))
        ctx.lineTo(root.xOf(to[0]), root.yOf(to[1]))
        ctx.stroke()
      }
    }
  }

  // ---- hover: a crosshair on the reading under the pointer
  Rectangle {
    id: crosshair
    visible: root.hovered !== null
    width: 1
    height: root.plotH
    y: 0
    x: root.hovered ? Math.round(root.xOf(root.hovered[0])) : 0
    color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.45)
  }

  Rectangle {
    id: marker
    visible: root.hovered !== null
    width: 7
    height: 7
    radius: width / 2
    x: root.hovered ? Math.round(root.xOf(root.hovered[0])) - width / 2 : 0
    y: root.hovered ? Math.round(root.yOf(root.hovered[1])) - height / 2 : 0
    color: root.hovered ? root.colorFor(root.hovered[1]) : "transparent"
    border.width: 1
    // The panel's own ground, so the dot reads as lifted off the line rather
    // than outlined in a colour the theme never asked for.
    border.color: Color.popups.background
  }

  // Styled from the theme's tooltip tokens — the same ones PanelToolTip uses —
  // rather than a black box of its own: this is a tooltip in everything but
  // its trigger, and it should look like the rest of the shell's.
  // A plain Rectangle rather than the shell's BorderSurface: that lives in
  // qs.Ui, and this file stays on qs.Commons so the chart keeps working if the
  // shell moves its widgets around. The colours are the theme's either way.
  Rectangle {
    id: readout
    visible: root.hovered !== null
    color: Color.tooltip.background
    border.width: Style.normalBorderWidth
    border.color: Color.tooltip.border
    radius: Style.cornerRadius
    implicitWidth: readoutText.implicitWidth + Style.spacing.controlPaddingX * 2
    implicitHeight: readoutText.implicitHeight + Style.spacing.controlPaddingY * 2
    width: implicitWidth
    height: implicitHeight
    // Kept inside the plot, and flipped below the point when the reading sits
    // near the top — a label clipped by the card explains nothing.
    x: root.hovered
      ? Math.max(root.plotX, Math.min(root.width - width, root.xOf(root.hovered[0]) - width / 2))
      : 0
    y: {
      if (!root.hovered) return 0
      var above = root.yOf(root.hovered[1]) - height - 8
      return above < 0 ? root.yOf(root.hovered[1]) + 8 : above
    }

    Text {
      id: readoutText
      anchors.centerIn: parent
      color: Color.tooltip.text
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      text: root.hovered
        ? root.hovered[1].toFixed(1) + "  " + root.clockAt(root.hovered[0])
        : ""
    }
  }

  MouseArea {
    id: probe
    anchors.fill: parent
    anchors.leftMargin: root.gutterLeft
    anchors.bottomMargin: root.gutterBottom
    hoverEnabled: true
    // No buttons: the panel scrolls under this, and a MouseArea that accepted
    // presses would swallow the drag.
    acceptedButtons: Qt.NoButton

    onPositionChanged: function (mouse) {
      root.hoverIndex = root.nearestIndex(mouse.x + root.gutterLeft)
    }
    onExited: root.hoverIndex = -1
  }
}
