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

  // How much of the fetched window the chart shows at once. The rest is
  // reachable by panning, and visible in the minimap underneath.
  property int viewHours: 6
  // Left edge of the viewport. -1 means "follow the newest reading", which is
  // where it returns whenever panning reaches the right-hand end.
  property real viewStartMs: -1
  readonly property bool live: viewStartMs < 0

  readonly property real viewSpanMs: Math.min(viewHours * 3600000, dataSpanMs)
  readonly property real seriesFirstMs: series.length > 0 ? series[0][0] : 0
  // Panning reaches back as far as the coarse history, not just the window
  // drawn at full resolution.
  readonly property real dataFirstMs: overview.length > 0
    ? Math.min(overview[0][0], seriesFirstMs)
    : seriesFirstMs
  readonly property real dataLastMs: series.length > 0 ? series[series.length - 1][0] : 1
  readonly property real dataSpanMs: Math.max(1, dataLastMs - dataFirstMs)

  function clampStart(ms) {
    return Math.max(dataFirstMs, Math.min(ms, dataLastMs - viewSpanMs))
  }

  // Panning to the right-hand end goes back to following: a chart parked one
  // reading behind live, with no way to tell, is worse than one that scrolls.
  function panBy(deltaMs) {
    var from = live ? dataLastMs - viewSpanMs : viewStartMs
    var next = clampStart(from + deltaMs)
    viewStartMs = (next >= dataLastMs - viewSpanMs - 1000) ? -1 : next
  }

  function follow() { viewStartMs = -1 }

  // One notch moves a twelfth of the window — half an hour on a six-hour
  // chart. A quarter of the window per notch flew past whatever you were
  // trying to look at. Fractional deltas come from touchpads, and scale the
  // same way rather than being rounded up to a whole notch.
  function panByWheel(angleDeltaY) {
    var notches = angleDeltaY / 120
    panBy(-notches * viewSpanMs / 12)
  }

  // What the chart is actually showing, which is not always what was fetched.
  readonly property real windowStart: live ? dataLastMs - viewSpanMs : viewStartMs
  readonly property real windowEnd: windowStart + viewSpanMs

  readonly property var series: doc && doc.series ? doc.series : []
  readonly property var range: doc && doc.range ? doc.range : null
  readonly property var band: doc && doc.band ? doc.band : null
  // How far back panning may reach. The document usually carries more — the
  // patterns want a fortnight — but a strip a few hundred pixels wide turns
  // that into noise with a viewport box too small to grab.
  property int scrollbackHours: 72

  readonly property var allOverview: doc && doc.overview ? doc.overview.points : []

  // The history the band came from, one point per quarter hour, trimmed to
  // the scrollback. It is what makes panning past the fetched window possible
  // without a second request: those readings were fetched anyway.
  readonly property var overview: {
    if (allOverview.length === 0) return []
    var cutoff = allOverview[allOverview.length - 1][0] - scrollbackHours * 3600000
    if (allOverview[0][0] >= cutoff) return allOverview
    var out = []
    for (var i = 0; i < allOverview.length; i++) {
      if (allOverview[i][0] >= cutoff) out.push(allOverview[i])
    }
    return out
  }

  // What the chart draws: the fine series while the viewport is inside it,
  // the coarse history once panned past its start. Fine detail where it can
  // be seen, reach where it cannot.
  readonly property var plotted: (overview.length > 1 && windowStart < seriesFirstMs)
    ? overview
    : series

  // The document's palette — the user's own, including the colourblind
  // preset, which a hard-coded copy of these three silently ignored. The
  // fallbacks are only for a sugarrush too old to send one.
  readonly property var palette: doc && doc.theme ? doc.theme : null
  function themed(role, fallback) {
    return palette && palette[role] ? palette[role] : fallback
  }
  readonly property color urgentColor: themed("urgent", "#cc241d")
  readonly property color lowColor: themed("low", "#d79921")
  readonly property color highColor: themed("high", "#d79921")
  readonly property color inRangeColor: themed("in_range", "#98971a")

  // The colour a reading is drawn in — the same ladder `alert.rs` classifies
  // by, so a segment of the line means what the pill means.
  function colorFor(value) {
    if (!range) return foreground
    if (value <= range.urgent_low || value >= range.urgent_high) return urgentColor
    if (value < range.low) return lowColor
    if (value > range.high) return highColor
    return inRangeColor
  }

  // Room for the value ticks on the left and the clock on the bottom.
  readonly property int gutterLeft: 34
  readonly property int gutterBottom: 14
  // The overview strip, and the gap between it and the clock labels.
  readonly property int minimapHeight: 26
  readonly property int minimapGap: 6

  readonly property real plotX: gutterLeft
  readonly property real plotW: Math.max(1, width - gutterLeft)
  readonly property real plotH: Math.max(1, height - gutterBottom - minimapHeight - minimapGap)

  readonly property real firstAt: windowStart
  readonly property real lastAt: windowEnd
  readonly property real span: Math.max(1, lastAt - firstAt)

  // The scale always covers the alert bounds, so the thresholds are on screen
  // even on a flat day well inside them, and always covers the band, so a wide
  // typical day is not clipped.
  readonly property var bounds: {
    if (!range || series.length === 0) return { lo: 0, hi: 1 }
    var lo = range.urgent_low
    var hi = range.urgent_high
    for (var i = 0; i < plotted.length; i++) {
      if (plotted[i][0] < windowStart || plotted[i][0] > windowEnd) continue
      lo = Math.min(lo, plotted[i][1])
      hi = Math.max(hi, plotted[i][1])
    }
    if (band) {
      for (var b = 0; b < band.points.length; b++) {
        if (band.points[b][0] < windowStart || band.points[b][0] > windowEnd) continue
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

  // "22:00 – 04:00", for a card label that has to say where the chart is.
  readonly property string windowLabel: clockAt(windowStart) + " – " + clockAt(windowEnd)

  function clockAt(ms) {
    var when = new Date(ms)
    return ("0" + when.getHours()).slice(-2) + ":" + ("0" + when.getMinutes()).slice(-2)
  }

  // Both canvases, every time. The overview used to repaint only by luck of
  // timing, and a pan repainted nothing at all: these handlers called a
  // repaint() that was never defined, so every scroll threw a ReferenceError
  // and the chart only caught up when the panel was rebuilt.
  function repaint() {
    canvas.requestPaint()
    minimap.requestPaint()
  }

  onDocChanged: repaint()
  onViewStartMsChanged: repaint()
  onViewHoursChanged: repaint()
  onScrollbackHoursChanged: {
    // A shorter scrollback can leave the viewport outside the data.
    if (!live) viewStartMs = clampStart(viewStartMs)
    repaint()
  }
  onWidthChanged: repaint()
  onHeightChanged: repaint()

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

      // Everything that follows the data is clipped to the plot: the series
      // holds readings either side of the viewport now, and their segments
      // ran left across the value labels.
      ctx.save()
      ctx.beginPath()
      ctx.rect(plotX, 0, width - plotX, plotH)
      ctx.clip()

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

      ctx.restore()

      // ---- thresholds: scale marks, in the foreground colour
      //
      // Not coloured by severity. The line already turns amber or red where it
      // crosses one, and a red rule drawn across a chart the reading never
      // reached read as an alarm rather than as the axis it is. The urgent
      // bounds are drawn fainter than low/high so the target band still reads
      // as the one that matters.
      var levels = [
        { value: root.range.urgent_high, alpha: 0.3 },
        { value: root.range.high, alpha: 0.55 },
        { value: root.range.low, alpha: 0.55 },
        { value: root.range.urgent_low, alpha: 0.3 }
      ]
      ctx.lineWidth = 1
      for (var l = 0; l < levels.length; l++) {
        var ly = root.yOf(levels[l].value)
        if (ly < 0 || ly > plotH) continue
        ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b,
                                  levels[l].alpha)
        ctx.beginPath()
        ctx.moveTo(plotX, ly)
        ctx.lineTo(width, ly)
        ctx.stroke()

        ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b,
                                Math.min(1, levels[l].alpha + 0.25))
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
        // Under the plot, not at the bottom of the canvas: the canvas now
        // also covers the overview strip, and the clock was landing on it.
        ctx.fillText(root.clockAt(tick), tx, plotH + 11)
      }

      // ---- today, one segment at a time so the line carries its own state
      ctx.save()
      ctx.beginPath()
      ctx.rect(plotX, 0, width - plotX, plotH)
      ctx.clip()
      ctx.lineWidth = 2
      ctx.lineCap = "round"
      var line = root.plotted
      for (var j = 1; j < line.length; j++) {
        var from = line[j - 1]
        var to = line[j]
        // The segment takes the colour of where it arrives: a line crossing
        // into the low band should already look low when it gets there.
        ctx.strokeStyle = root.colorFor(to[1])
        ctx.beginPath()
        ctx.moveTo(root.xOf(from[0]), root.yOf(from[1]))
        ctx.lineTo(root.xOf(to[0]), root.yOf(to[1]))
        ctx.stroke()
      }
      ctx.restore()
    }
  }

  // ---- the overview: everything fetched, drawn small, with a box showing
  // which slice the chart above is displaying. Clicking or dragging it moves
  // the viewport, which is the only way to reach a reading that has scrolled
  // off without keyboard focus.
  Canvas {
    id: minimap
    x: root.gutterLeft
    width: Math.max(1, root.width - root.gutterLeft)
    height: root.minimapHeight
    y: root.plotH + root.gutterBottom + root.minimapGap
    visible: points.length > 1 && root.dataSpanMs > root.viewSpanMs + 60000

    function xOfAll(t) {
      return (t - root.dataFirstMs) / root.dataSpanMs * width
    }

    // The coarse history when there is one: the strip is a map of everywhere
    // the chart can go, not only of the part drawn at full resolution.
    readonly property var points: root.overview.length > 1 ? root.overview : root.series

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      if (points.length < 2) return

      var pts = points
      var lo = root.range ? root.range.urgent_low : 0
      var hi = root.range ? root.range.urgent_high : 1
      for (var i = 0; i < pts.length; i++) {
        lo = Math.min(lo, pts[i][1])
        hi = Math.max(hi, pts[i][1])
      }
      var yOfAll = function (v) { return height - (v - lo) / Math.max(0.1, hi - lo) * height }

      // Decimated: at a couple of hundred pixels wide, every fifth reading
      // draws the same shape for a fifth of the work.
      var stride = Math.max(1, Math.floor(pts.length / width * 2))
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.5)
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(xOfAll(pts[0][0]), yOfAll(pts[0][1]))
      for (var j = stride; j < pts.length; j += stride) {
        ctx.lineTo(xOfAll(pts[j][0]), yOfAll(pts[j][1]))
      }
      ctx.stroke()

      var vx = xOfAll(root.windowStart)
      var vw = Math.max(6, xOfAll(root.windowEnd) - vx)
      ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.14)
      ctx.fillRect(vx, 0, vw, height)
      ctx.strokeStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.5)
      ctx.strokeRect(vx + 0.5, 0.5, vw - 1, height - 1)
    }

    MouseArea {
      anchors.fill: parent
      onPressed: function (mouse) { root.jumpToX(mouse.x, parent.width) }
      onPositionChanged: function (mouse) {
        if (pressed) root.jumpToX(mouse.x, parent.width)
      }
      onWheel: function (wheel) {
        root.panByWheel(wheel.angleDelta.y)
        wheel.accepted = true
      }
    }
  }

  // Centre the viewport on the point of the overview that was clicked.
  function jumpToX(px, w) {
    var at = dataFirstMs + (px / Math.max(1, w)) * dataSpanMs
    var next = clampStart(at - viewSpanMs / 2)
    viewStartMs = (next >= dataLastMs - viewSpanMs - 1000) ? -1 : next
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

    // Wheel is handled here, and accepted, because the panel scrolls: the
    // Flickable around these cards is interactive whenever the content
    // overflows, and it takes the wheel before a child WheelHandler ever sees
    // it. Accepting stops the card stack scrolling under the pointer.
    //
    // A quarter of the window per notch: enough to move, small enough to land
    // where you meant to.
    onWheel: function (wheel) {
      root.panByWheel(wheel.angleDelta.y)
      wheel.accepted = true
    }
  }
}
