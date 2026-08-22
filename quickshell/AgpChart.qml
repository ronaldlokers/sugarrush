// The ambulatory glucose profile: every day in the window folded onto one
// 24-hour clock.
//
// Not a time series. The x axis is time of day, so a bump at 03:00 means "this
// happens at 3am", which is the question the 6-hour chart next door cannot
// answer. Two envelopes and a median, which is what a clinic reads.

import QtQuick
import qs.Commons

Canvas {
  id: root

  // `{ days, step_min, points: [[minute, p05, p25, p50, p75, p95], ...] }`
  property var agp: null
  property var range: null
  property color foreground: "white"

  readonly property int gutter: 34
  readonly property int axisHeight: 14

  onAgpChanged: requestPaint()
  onRangeChanged: requestPaint()
  onForegroundChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()

  function colorFor(v) {
    if (!range) return root.foreground
    if (v <= range.urgent_low || v >= range.urgent_high) return "#cc241d"
    if (v < range.low || v > range.high) return "#d79921"
    return "#98971a"
  }

  onPaint: {
    var ctx = getContext("2d")
    ctx.reset()
    var pts = agp && agp.points ? agp.points : []
    if (pts.length < 2 || !range) return

    var plotW = width - gutter
    var plotH = height - axisHeight
    if (plotW <= 0 || plotH <= 0) return

    // The vertical scale covers the thresholds as well as the data: a profile
    // that never goes low still has to show where low is, or the envelope
    // floats in a space with no meaning.
    var lo = range.urgent_low, hi = range.urgent_high
    for (var i = 0; i < pts.length; i++) {
      if (pts[i][1] < lo) lo = pts[i][1]
      if (pts[i][5] > hi) hi = pts[i][5]
    }
    var pad = (hi - lo) * 0.08
    lo -= pad; hi += pad

    var x = function (minute) { return gutter + minute / 1440 * plotW }
    var y = function (v) { return plotH - (v - lo) / (hi - lo) * plotH }

    // Envelopes, widest first: 5–95 as the outline of what happens at all,
    // 25–75 as where most days actually sit.
    function envelope(loIdx, hiIdx, alpha) {
      ctx.beginPath()
      for (var i = 0; i < pts.length; i++) {
        var px = x(pts[i][0]), py = y(pts[i][hiIdx])
        if (i === 0) ctx.moveTo(px, py)
        else ctx.lineTo(px, py)
      }
      for (var j = pts.length - 1; j >= 0; j--) ctx.lineTo(x(pts[j][0]), y(pts[j][loIdx]))
      ctx.closePath()
      ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, alpha)
      ctx.fill()
    }

    ctx.save()
    ctx.beginPath()
    ctx.rect(gutter, 0, plotW, plotH)
    ctx.clip()

    envelope(1, 5, 0.10)
    envelope(2, 4, 0.20)

    // Threshold rules, in the panel's foreground rather than in alert colours:
    // they are scale marks, and a red line drawn across every profile says
    // nothing about this one.
    var rules = [
      { v: range.urgent_high, a: 0.28 },
      { v: range.high, a: 0.5 },
      { v: range.low, a: 0.5 },
      { v: range.urgent_low, a: 0.28 }
    ]
    for (var r = 0; r < rules.length; r++) {
      ctx.beginPath()
      ctx.moveTo(gutter, y(rules[r].v))
      ctx.lineTo(width, y(rules[r].v))
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, rules[r].a)
      ctx.lineWidth = 1
      ctx.stroke()
    }

    // The median, segment by segment in the colour of the band it is in — the
    // one line someone actually traces with a finger.
    for (var k = 1; k < pts.length; k++) {
      ctx.beginPath()
      ctx.moveTo(x(pts[k - 1][0]), y(pts[k - 1][3]))
      ctx.lineTo(x(pts[k][0]), y(pts[k][3]))
      ctx.strokeStyle = root.colorFor(pts[k][3])
      ctx.lineWidth = 2
      ctx.lineJoin = "round"
      ctx.lineCap = "round"
      ctx.stroke()
    }
    ctx.restore()

    // Left gutter: the thresholds, since they are the numbers a reader
    // measures the profile against.
    ctx.font = "10px monospace"
    ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.7)
    ctx.textAlign = "right"
    for (var g = 0; g < rules.length; g++) {
      var label = String(rules[g].v)
      ctx.fillText(label, gutter - 5, y(rules[g].v) + 3)
    }

    // Clock along the bottom, every six hours: enough to place a bump in the
    // day without crowding a panel-width chart.
    ctx.textAlign = "center"
    for (var h = 0; h <= 24; h += 6) {
      var hx = x(h * 60)
      ctx.beginPath()
      ctx.moveTo(hx, plotH)
      ctx.lineTo(hx, plotH + 3)
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.3)
      ctx.stroke()
      // The last label would hang off the edge, and 24:00 is 00:00 anyway.
      if (h < 24) {
        ctx.fillText(h === 0 ? "00" : String(h), Math.max(hx, gutter + 6), plotH + 12)
      }
    }
  }
}
