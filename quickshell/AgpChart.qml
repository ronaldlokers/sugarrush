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
  // Minute-of-day of the selected bucket, or -1. The median is what a clinic
  // reads; the days behind it are what changes a basal rate, and they are
  // only worth the ink when someone asks for a particular hour.
  property int selected: -1
  // `doc.theme`, or null against a sugarrush too old to send one.
  property var themeColors: null
  property color foreground: "white"

  readonly property int gutter: 34
  readonly property int axisHeight: 14

  onAgpChanged: requestPaint()
  onSelectedChanged: requestPaint()
  onRangeChanged: requestPaint()
  onThemeColorsChanged: requestPaint()
  onForegroundChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  // The same ladder `alert.rs` classifies by, in the same colours: low and
  // high are separate roles in the palette, and drawing both amber threw that
  // distinction away along with the colourblind preset.
  function colorFor(v) {
    if (!range) return root.foreground
    if (v <= range.urgent_low || v >= range.urgent_high) return themed("urgent", "#cc241d")
    if (v < range.low) return themed("low", "#d79921")
    if (v > range.high) return themed("high", "#d79921")
    return themed("in_range", "#98971a")
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

    // ---- the days behind the envelope, for one bucket
    //
    // The band says the middle half of your evenings look fine. It cannot say
    // that three of the fourteen went low, and that is the finding worth
    // acting on. Only ever drawn for the bucket someone asked about: all of
    // them at once is the scatter plot the envelope exists to replace.
    if (root.selected >= 0) {
      var values = root.samplesAt(root.selected)
      var sx = x(root.selected)

      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.35)
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(sx, 0)
      ctx.lineTo(sx, plotH)
      ctx.stroke()

      for (var s = 0; s < values.length; s++) {
        var vy = y(values[s])
        if (vy < 0 || vy > plotH) continue
        // Nudged apart horizontally so two days at the same value are two
        // dots rather than one: the count is the point.
        var jitter = (s % 2 === 0 ? -1 : 1) * Math.min(3, 1 + Math.floor(s / 2))
        ctx.beginPath()
        ctx.arc(sx + jitter, vy, 2, 0, Math.PI * 2)
        ctx.fillStyle = root.colorFor(values[s])
        ctx.fill()
      }
    }
  }

  // The values behind one bucket, by its minute-of-day.
  function samplesAt(minute) {
    var list = agp && agp.samples ? agp.samples : []
    for (var i = 0; i < list.length; i++) {
      if (list[i][0] === minute) return list[i][1]
    }
    return []
  }

  // The bucket nearest an x position, or -1 when the chart has no profile.
  function bucketAt(px) {
    var pts = agp && agp.points ? agp.points : []
    if (pts.length === 0) return -1
    var plotW = width - gutter
    var minute = Math.max(0, Math.min(1440, (px - gutter) / Math.max(1, plotW) * 1440))
    var best = pts[0][0]
    var bestDistance = Infinity
    for (var i = 0; i < pts.length; i++) {
      var d = Math.abs(pts[i][0] - minute)
      if (d < bestDistance) { bestDistance = d; best = pts[i][0] }
    }
    return best
  }

  MouseArea {
    anchors.fill: parent
    onClicked: function (mouse) {
      var at = root.bucketAt(mouse.x)
      // Tapping the selected hour again clears it, so the chart goes back to
      // being a profile rather than needing a second control to undo this one.
      root.selected = (at === root.selected) ? -1 : at
    }
  }
}
