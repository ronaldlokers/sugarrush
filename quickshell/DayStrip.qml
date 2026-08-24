// Time in range, one stacked bar per local day, oldest on the left.
//
// The profile next door folds every day onto one clock and answers "what do my
// nights look like". This answers the other question people actually track:
// whether it is getting better. Neither can be read off the other.
//
// Canvas rather than a Repeater of Rectangles: a fortnight is 14 bars of up to
// five bands each, and 70 items with their own bindings is a lot of scene
// graph for something that changes once a day.

import QtQuick

Canvas {
  id: root

  // `[{date, readings, tir: {very_low, low, in_range, high, very_high}}]`,
  // oldest first.
  property var days: []
  property var themeColors: null
  property color foreground: "white"

  // Below this a day is drawn faded: a bar computed from a handful of readings
  // is a rumour, and drawing it like a full day would let one thin morning
  // read as the best day of the fortnight.
  readonly property int thinReadings: 60

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  onDaysChanged: requestPaint()
  onThemeColorsChanged: requestPaint()
  onForegroundChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()

  onPaint: {
    var ctx = getContext("2d")
    ctx.reset()
    var list = days || []
    if (list.length === 0) return

    var gap = list.length > 20 ? 1 : 2
    var slot = width / list.length
    var barW = Math.max(1, slot - gap)

    // Same order as the time-in-range bar above, so a column here and the bar
    // there are read the same way round.
    var bands = [
      { key: "very_low", color: themed("urgent", "#cc241d") },
      { key: "low", color: themed("low", "#d79921") },
      { key: "in_range", color: themed("in_range", "#98971a") },
      { key: "high", color: themed("high", "#d79921") },
      { key: "very_high", color: themed("urgent", "#cc241d") }
    ]

    for (var d = 0; d < list.length; d++) {
      var day = list[d]
      var tir = day.tir
      if (!tir) continue
      var x = d * slot
      var thin = day.readings < root.thinReadings
      var y = 0

      for (var b = 0; b < bands.length; b++) {
        var share = tir[bands[b].key] || 0
        if (share <= 0) continue
        var h = height * share / 100
        ctx.globalAlpha = thin ? 0.3 : 1
        ctx.fillStyle = bands[b].color
        ctx.fillRect(x, y, barW, h)
        y += h
      }

      // A day with nothing in it still gets a place, so the fortnight does not
      // silently close up around a sensor that was off.
      if (y === 0) {
        ctx.globalAlpha = 0.18
        ctx.fillStyle = root.foreground
        ctx.fillRect(x, 0, barW, height)
      }
    }
    ctx.globalAlpha = 1
  }
}
