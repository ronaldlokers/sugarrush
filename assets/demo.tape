# vhs tape for the README demo. Regenerate with:
#   mise x vhs ttyd -- vhs assets/demo.tape
# Records `sugarrush --demo` (synthetic data — no real health data).

Output assets/demo.gif

Set Shell bash
Set FontFamily "CaskaydiaMono Nerd Font"
Set FontSize 15
Set Width 1200
Set Height 720
Set Padding 20
Set Framerate 24

Hide
Type "./target/release/sugarrush --demo"
Enter
Sleep 2.5s
Show

Sleep 2s
# Pan back through history
Type "h"
Sleep 1.2s
Type "h"
Sleep 1.2s
# Zoom the window out
Type "-"
Sleep 1.3s
# Back to the live edge
Type "f"
Sleep 1.5s
# Cycle the graph tabs: 3h -> 24h -> AGP (multi-day percentile profile)
Tab
Sleep 1.5s
Tab
Sleep 2.2s
# Back to the 3h timeline
Tab
Sleep 1.3s
# Toggle units mmol/L <-> mg/dL
Type "u"
Sleep 1.6s
Type "u"
Sleep 1.2s
# Open settings; the Site section is at the top — show the URL editor, then
# cancel out of it
Type "s"
Sleep 1.5s
Enter
Sleep 1.4s
Escape
Sleep 0.8s
# Move down and change a value
Down
Down
Down
Down
Sleep 0.8s
Right
Sleep 1s
Down
Sleep 0.6s
Right
Sleep 1.1s
# `?` works here too, and lists the settings keys rather than the graph ones
Type "?"
Sleep 2.2s
Escape
Sleep 0.8s
# Close settings, quit
Type "s"
Sleep 1.5s
Type "q"
Sleep 0.6s
