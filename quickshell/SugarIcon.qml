// The panel's identity mark: a drop over a trace.
//
// Drawn rather than set in a glyph, the way the shell's own panel icons are
// (TailscaleIcon, DropboxIcon): the plugin then depends on no icon font being
// installed, and the mark scales with the hero.

import QtQuick

Item {
  id: root

  property real iconSize: 22
  property color color: "white"

  implicitWidth: iconSize
  implicitHeight: iconSize

  onColorChanged: canvas.requestPaint()
  onIconSizeChanged: canvas.requestPaint()

  Canvas {
    id: canvas
    anchors.fill: parent

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      var s = Math.min(width, height)
      var cx = width / 2

      // The drop: a point at the top opening into a circle at the bottom.
      var r = s * 0.26
      var cy = s * 0.42
      ctx.fillStyle = root.color
      ctx.beginPath()
      ctx.moveTo(cx, s * 0.06)
      ctx.quadraticCurveTo(cx + r * 1.25, cy - r * 0.2, cx + r, cy)
      ctx.arc(cx, cy, r, 0, Math.PI)
      ctx.quadraticCurveTo(cx - r * 1.25, cy - r * 0.2, cx, s * 0.06)
      ctx.closePath()
      ctx.fill()

      // The trace it sits on, so the mark reads as glucose rather than water.
      ctx.strokeStyle = root.color
      ctx.lineWidth = Math.max(1, s * 0.09)
      ctx.lineCap = "round"
      ctx.lineJoin = "round"
      ctx.beginPath()
      ctx.moveTo(s * 0.08, s * 0.88)
      ctx.lineTo(s * 0.32, s * 0.88)
      ctx.lineTo(s * 0.46, s * 0.72)
      ctx.lineTo(s * 0.62, s * 0.94)
      ctx.lineTo(s * 0.76, s * 0.82)
      ctx.lineTo(s * 0.94, s * 0.82)
      ctx.stroke()
    }
  }
}
