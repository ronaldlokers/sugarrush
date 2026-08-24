// The night just gone, small: shape, target band, and the low points marked.
//
// It answers one question — was the night fine — so it is deliberately not the
// six-hour chart shrunk. No axes, no clock, no panning; the times are in the
// card's title and the numbers are underneath.

import QtQuick

Canvas {
  id: root

  // `[[epoch_ms, value], ...]`, oldest first.
  property var series: []
  property var range: null
  property var themeColors: null
  property color foreground: "white"

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  onSeriesChanged: requestPaint()
  onRangeChanged: requestPaint()
  onThemeColorsChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()

  onPaint: {
    var ctx = getContext("2d")
    ctx.reset()
    var pts = series || []
    if (pts.length < 2 || !range) return

    var lo = range.urgent_low
    var hi = range.urgent_high
    for (var i = 0; i < pts.length; i++) {
      lo = Math.min(lo, pts[i][1])
      hi = Math.max(hi, pts[i][1])
    }
    var pad = (hi - lo) * 0.08
    lo -= pad; hi += pad

    var t0 = pts[0][0]
    var span = Math.max(1, pts[pts.length - 1][0] - t0)
    var x = function (ms) { return (ms - t0) / span * width }
    var y = function (v) { return height - (v - lo) / Math.max(0.0001, hi - lo) * height }

    // The target band, so a dip reads as a dip below something.
    ctx.fillStyle = themed("in_range", "#98971a")
    ctx.globalAlpha = 0.12
    ctx.fillRect(0, y(range.high), width, Math.max(1, y(range.low) - y(range.high)))
    ctx.globalAlpha = 1

    ctx.lineWidth = 1.6
    ctx.lineJoin = "round"
    for (var j = 1; j < pts.length; j++) {
      var v = pts[j][1]
      ctx.strokeStyle = v <= range.urgent_low || v >= range.urgent_high
        ? themed("urgent", "#cc241d")
        : (v < range.low ? themed("low", "#d79921")
           : (v > range.high ? themed("high", "#d79921") : themed("in_range", "#98971a")))
      ctx.beginPath()
      ctx.moveTo(x(pts[j - 1][0]), y(pts[j - 1][1]))
      ctx.lineTo(x(pts[j][0]), y(pts[j][1]))
      ctx.stroke()
    }

    // Every excursion below range marked, not only the lowest: two separate
    // lows at 3am and 5am is a different night from one long one.
    for (var k = 0; k < pts.length; k++) {
      if (pts[k][1] >= range.low) continue
      var below = k === 0 || pts[k - 1][1] >= range.low
      if (!below) continue
      ctx.beginPath()
      ctx.arc(x(pts[k][0]), y(pts[k][1]), 2.5, 0, Math.PI * 2)
      ctx.fillStyle = themed("urgent", "#cc241d")
      ctx.fill()
    }
  }
}
