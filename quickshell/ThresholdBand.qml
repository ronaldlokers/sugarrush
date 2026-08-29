// The four alert thresholds as one band with four handles.
//
// They are not four settings. They are one band with two guard rails, and the
// relationship between them is the thing being edited — which four numbered
// stepper rows hide completely. It is also why crossing them is possible
// enough that `set_key` has to refuse it; here the handles simply cannot pass
// each other, so the illegal state is unreachable rather than rejected.
//
// The steppers stay underneath. Dragging is for seeing the shape; 0.1 mmol/L
// on a 380px panel is what a stepper is for.

import QtQuick

Item {
  id: root

  property real urgentLow: 3.5
  property real low: 3.9
  property real high: 10.0
  property real urgentHigh: 13.9
  // "mgdl" or anything else, which means mmol/L.
  property string units: "mmol"
  property var themeColors: null
  property color foreground: "white"

  // Emitted on release, not on every pixel: each one is a config write.
  //
  // `previous` is what the threshold was when the drag began, so a consumer
  // can offer to put it back without having to remember the old value itself.
  signal changed(string role, real value, real previous)

  readonly property bool mgdl: units === "mgdl"
  readonly property real scaleMin: mgdl ? 40 : 2
  readonly property real scaleMax: mgdl ? 400 : 22
  readonly property real step: mgdl ? 1 : 0.1
  // Handles cannot close up entirely: two thresholds a tenth apart is not a
  // configuration anyone means, and it makes the band unreadable.
  readonly property real minGap: mgdl ? 5 : 0.3

  readonly property int handleWidth: 3
  // How close a press has to land to pick a handle up. Wider than the handle
  // it grabs, because a 3px target is not a target.
  readonly property int grabRadius: 12
  readonly property int bandTop: 18
  readonly property int bandHeight: 16

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  function xOf(value) {
    var t = (value - scaleMin) / Math.max(0.0001, scaleMax - scaleMin)
    return Math.max(0, Math.min(1, t)) * width
  }

  function valueOf(x) {
    var t = Math.max(0, Math.min(1, x / Math.max(1, width)))
    var raw = scaleMin + t * (scaleMax - scaleMin)
    return Math.round(raw / step) * step
  }

  // Which handle a press grabs, and the bounds it may move between. Order is
  // fixed: urgent low ≤ low ≤ high ≤ urgent high.
  function handles() {
    return [
      { role: "alerts.urgent_low", value: urgentLow, lower: scaleMin, upper: low - minGap },
      { role: "alerts.low", value: low, lower: urgentLow + minGap, upper: high - minGap },
      { role: "alerts.high", value: high, lower: low + minGap, upper: urgentHigh - minGap },
      { role: "alerts.urgent_high", value: urgentHigh, lower: high + minGap, upper: scaleMax }
    ]
  }

  property int dragging: -1

  implicitHeight: bandTop + bandHeight + 16

  onUrgentLowChanged: band.requestPaint()
  onLowChanged: band.requestPaint()
  onHighChanged: band.requestPaint()
  onUrgentHighChanged: band.requestPaint()
  onThemeColorsChanged: band.requestPaint()
  onWidthChanged: band.requestPaint()

  Canvas {
    id: band
    anchors.fill: parent

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      var y = root.bandTop
      var h = root.bandHeight

      // Five zones, in the colours the reading itself is drawn in, so the
      // band is a legend for the rest of the panel as much as a control.
      var zones = [
        { from: root.scaleMin, to: root.urgentLow, color: root.themed("urgent", "#cc241d"), alpha: 0.75 },
        { from: root.urgentLow, to: root.low, color: root.themed("low", "#d79921"), alpha: 0.7 },
        { from: root.low, to: root.high, color: root.themed("in_range", "#98971a"), alpha: 0.85 },
        { from: root.high, to: root.urgentHigh, color: root.themed("high", "#d79921"), alpha: 0.7 },
        { from: root.urgentHigh, to: root.scaleMax, color: root.themed("urgent", "#cc241d"), alpha: 0.75 }
      ]
      for (var z = 0; z < zones.length; z++) {
        var x0 = root.xOf(zones[z].from)
        var x1 = root.xOf(zones[z].to)
        if (x1 <= x0) continue
        ctx.globalAlpha = zones[z].alpha
        ctx.fillStyle = zones[z].color
        ctx.fillRect(x0, y, x1 - x0, h)
      }
      ctx.globalAlpha = 1

      // Handles, and the value each one is currently at.
      ctx.font = "9px monospace"
      ctx.textAlign = "center"
      var list = root.handles()
      for (var i = 0; i < list.length; i++) {
        var hx = root.xOf(list[i].value)
        ctx.fillStyle = root.foreground
        ctx.fillRect(hx - root.handleWidth / 2, y - 4, root.handleWidth, h + 8)
        ctx.fillStyle = Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b,
                                root.dragging === i ? 1 : 0.72)
        ctx.fillText(list[i].value.toFixed(root.mgdl ? 0 : 1),
                     Math.max(12, Math.min(root.width - 12, hx)), y - 8)
      }
    }
  }

  MouseArea {
    anchors.fill: parent
    // Fine control belongs to the steppers below; this is for the shape.
    preventStealing: true

    // The handle under the press, or -1. Bounded on purpose: `nearest` used
    // to pick the closest of four across the whole width, so a press anywhere
    // on the band grabbed a threshold, moved it to the press point and wrote
    // it on release. One stray click could set an alarm bound, silently, with
    // no undo. These four numbers decide whether a sound happens at 3am.
    function grabbed(x) {
      var list = root.handles()
      var best = -1
      var bestDistance = root.grabRadius
      for (var i = 0; i < list.length; i++) {
        var d = Math.abs(root.xOf(list[i].value) - x)
        if (d <= bestDistance) { bestDistance = d; best = i }
      }
      return best
    }

    function move(x) {
      var list = root.handles()
      var at = root.dragging
      if (at < 0) return
      var next = Math.max(list[at].lower, Math.min(list[at].upper, root.valueOf(x)))
      switch (at) {
      case 0: root.urgentLow = next; break
      case 1: root.low = next; break
      case 2: root.high = next; break
      case 3: root.urgentHigh = next; break
      }
    }

    property real startedAt: 0

    onPressed: function (mouse) {
      root.dragging = grabbed(mouse.x)
      if (root.dragging >= 0) startedAt = root.handles()[root.dragging].value
      // No jump to the press point. Grabbing a handle and moving it are now
      // separate things: the press picks it up where it already is, and only
      // dragging moves it.
      band.requestPaint()
    }

    onPositionChanged: function (mouse) {
      if (root.dragging < 0) return
      move(mouse.x)
      band.requestPaint()
    }

    onReleased: {
      if (root.dragging < 0) return
      var list = root.handles()
      // One write, on release. Writing per pixel would be a config file
      // rewritten a hundred times to move one threshold.
      // Nothing moved: a press that picked a handle up and put it down in the
      // same place is not an edit, and should not be written or undoable.
      if (list[root.dragging].value !== startedAt) {
        root.changed(list[root.dragging].role, list[root.dragging].value, startedAt)
      }
      root.dragging = -1
      band.requestPaint()
    }
  }
}
