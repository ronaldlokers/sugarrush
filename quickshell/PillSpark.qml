// The bar pill's own sparkline, at the size the pill draws it.
//
// Its only job is to answer "what will the switch above me do", so it is the
// same shape as the real one — an hour of readings, time-spaced, with the
// newest end marked — and nothing more.

import QtQuick

Canvas {
  id: root

  // `[[epoch_ms, value], ...]`, oldest first.
  property var series: []
  property color foreground: "white"

  opacity: 0.72

  onSeriesChanged: requestPaint()
  onForegroundChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()

  onPaint: {
    var ctx = getContext("2d")
    ctx.reset()
    var pts = series || []
    if (pts.length < 2) return

    // The last hour only, like the pill: the panel's document carries more.
    var cutoff = pts[pts.length - 1][0] - 3600000
    var recent = pts.filter(function (p) { return p[0] >= cutoff })
    if (recent.length < 2) recent = pts

    var lo = recent[0][1], hi = recent[0][1]
    for (var i = 1; i < recent.length; i++) {
      lo = Math.min(lo, recent[i][1])
      hi = Math.max(hi, recent[i][1])
    }
    // A flat hour is a real answer, and dividing by its zero range is not.
    var span = hi - lo
    if (span < 0.0001) { lo -= 0.5; hi += 0.5; span = hi - lo }

    var t0 = recent[0][0]
    var tspan = Math.max(1, recent[recent.length - 1][0] - t0)
    var pad = 1.5
    var h = height - pad * 2

    ctx.beginPath()
    for (var j = 0; j < recent.length; j++) {
      // Spaced by time rather than by index, so a gap in the readings shows
      // as a long straight run instead of being closed up.
      var x = (recent[j][0] - t0) / tspan * width
      var y = pad + (1 - (recent[j][1] - lo) / span) * h
      if (j === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y)
    }
    ctx.strokeStyle = root.foreground
    ctx.lineWidth = 1.4
    ctx.lineJoin = "round"
    ctx.lineCap = "round"
    ctx.stroke()

    var lastY = pad + (1 - (recent[recent.length - 1][1] - lo) / span) * h
    ctx.beginPath()
    ctx.arc(width - 1.8, lastY, 1.8, 0, Math.PI * 2)
    ctx.fillStyle = root.foreground
    ctx.fill()
  }
}
