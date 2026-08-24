// The sugarrush popup panel for the Omarchy bar.
//
// Every shell-internal import in this plugin lives in this file. BarWidget.qml
// reaches it through a Loader by path and checks the Loader's status, so a
// shell release that moves its internals costs the panel and nothing else —
// the pill keeps showing the reading.

import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "sugarrush.glucose"
  ipcTarget: "sugarrush.glucose"

  property var anchorItem: null
  // The bar tracks the widget in its slot, not this nested panel, so the
  // popout coordinator and the open-panel dot have to be handed that widget.
  property var hostWidget: null
  readonly property var barIdentity: hostWidget || root

  readonly property int panelHours: setting("panelHours", 6)
  // What gets fetched, and so how far back the chart can be scrolled. The
  // overview strip spans exactly this. Same 6-72h range the dashboard's
  // minimap uses.
  readonly property int overviewHours: Math.max(panelHours, setting("overviewHours", 24))
  // How far the chart scrolls. The patterns keep their own, longer window.
  readonly property int scrollbackHours: Math.max(overviewHours, setting("scrollbackHours", 72))
  readonly property int insightDays: setting("insightDays", 14)
  readonly property int cacheMinutes: setting("panelCacheMinutes", 5)
  // The pill's own poll interval, reused: an open panel refreshing faster than
  // the pill behind it would make the two disagree in the other direction.
  readonly property int refreshSeconds: Math.max(15, setting("interval", 60))
  readonly property string snapshotCommand: setting("snapshotCommand", "sugarrush snapshot")
  // The same binary, asked a different question. Derived so a widget pointed
  // at a build directory edits that build's config too.
  readonly property string configCommand: snapshotCommand.replace(/snapshot.*$/, "config")
  readonly property string healthCommand: snapshotCommand.replace(/snapshot.*$/, "health --json")

  property var doc: null
  property string loadError: ""
  // Set by `setConfig` for a key the pill renders; cleared when it is redrawn.
  property bool refetchPillAfterSet: false

  // The three views, in the order the switcher shows them, so a digit and an
  // arrow key mean the same thing as a click.
  readonly property var views: ["glucose", "profile", "settings"]

  function stepView(direction) {
    var at = views.indexOf(view)
    if (at < 0) return
    // Clamped rather than wrapped: arrowing off the end of a three-item
    // switcher and landing back at the start reads as a glitch.
    view = views[Math.max(0, Math.min(views.length - 1, at + direction))]
  }

  // Whether the shortcut list is on screen. Only ever opened by pressing `?`,
  // so it costs nothing to anyone who never does.
  property bool showKeys: false

  // "glucose" or "settings". A view, not a second panel: the cards below
  // simply swap, so the header, the strip and the popout identity stay put.
  //
  // Named for the data, not the moment: the view holds six hours of chart and
  // fourteen days of patterns as well as the reading, and it already contains
  // a card called Now. "Dashboard" is taken — that is the TUI, which the
  // button beside these chips opens.
  property string view: "glucose"
  // key -> value, as `sugarrush config` prints it.
  property var config: ({})
  property string configError: ""
  // Until `sugarrush config` answers, the panel knows nothing about what is
  // switched on. Every switch reading "on" in the meantime is the panel
  // asserting a state it has not read — briefly, but wrongly, and the two
  // switches that were actually off proved it.
  readonly property bool configLoaded: config && Object.keys(config).length > 0
  property double fetchedAt: 0
  // Seconds until the next automatic fetch. Driven by the ticker below, and
  // only while the panel is on screen.
  property int nextIn: 0

  readonly property var reading: doc && doc.now ? doc.now : null

  // Nightscout's trend names are machine words — "FortyFiveDown" reads as
  // FORTYFIVEDOWN once the hero styles it. The arrow already carries the
  // shape; this gives the same thing in words for anyone reading rather than
  // glancing.
  function trendWords(direction) {
    switch (String(direction)) {
      case "DoubleUp": return "rising fast"
      case "SingleUp": return "rising"
      case "FortyFiveUp": return "drifting up"
      case "Flat": return "steady"
      case "FortyFiveDown": return "drifting down"
      case "SingleDown": return "falling"
      case "DoubleDown": return "falling fast"
      default: return "trend unknown"
    }
  }
  readonly property var stats: doc && doc.stats ? doc.stats : null

  // How much of a window is actually behind its figures. A CGM produces about
  // twelve readings an hour, so the count and the span together say whether a
  // percentage stands on a full day or on the four hours around a sensor
  // change — which are different claims, not a smaller and a larger one.
  function coverage(block) {
    if (!block || !block.window_h || block.readings === undefined) return 1
    return Math.min(1, block.readings / Math.max(1, block.window_h * 12))
  }

  readonly property bool statsThin: stats !== null && coverage(stats) < 0.7
  // Not `baseline`: `Item` already has one — the anchor line — and it is
  // FINAL, so declaring it stops the whole panel loading. Third time a
  // document field has collided with a Qt property here, after `stale` and
  // `palette`, so the rule is now explicit: names from the document get a
  // suffix that says what they are.
  readonly property var baselineStats: doc && doc.baseline ? doc.baseline : null
  readonly property var dayList: doc && doc.days ? doc.days : []
  readonly property var alertList: doc && doc.alerts ? doc.alerts : []

  // "04:55", or "Sat 21:10" once it is not today — an alarm three days ago
  // needs a day name more than it needs a date.
  function alarmWhen(ms) {
    var at = new Date(ms)
    var today = new Date(doc ? doc.generated_at : Date.now())
    var sameDay = at.toDateString() === today.toDateString()
    var clock = Qt.formatTime(at, "HH:mm")
    return sameDay ? clock : Qt.formatDateTime(at, "ddd") + " " + clock
  }

  function alarmLength(item) {
    if (item.minutes === undefined) return "still going"
    return item.minutes + "m"
  }

  // Nothing yet, and nothing wrong: the first fetch of a cold open.
  readonly property bool loading: doc === null && loadError === ""

  // The average of the days drawn, which is the number someone reads the
  // column heights against.
  readonly property real dayAverage: {
    var list = dayList
    if (list.length === 0) return 0
    var total = 0
    for (var i = 0; i < list.length; i++) total += list[i].tir ? list[i].tir.in_range : 0
    return total / list.length
  }

  readonly property string units: doc && doc.units ? doc.units : ""

  // A threshold as the summary prints it, in the reader's own units.
  function bound(role) {
    var range = doc && doc.range ? doc.range : null
    if (!range || range[role] === undefined) return "—"
    return range[role].toFixed(doc && doc.units === "mg/dL" ? 0 : 1)
  }

  // A band's share of the baseline window, one decimal, as a clinic reads it.
  function tirOf(band) {
    if (!baselineStats || !baselineStats.tir) return "—"
    var share = baselineStats.tir[band]
    return share === undefined ? "—" : share.toFixed(1) + "%"
  }

  // What one hour of the profile actually held: the middle of it, the spread,
  // and — the reason for the whole feature — how many of the days went out of
  // range there, which no percentile band can say.
  function hourWords(minute, values) {
    if (!values || values.length === 0) return "No readings in that hour."
    var range = doc && doc.range ? doc.range : null
    var decimals = doc && doc.units === "mg/dL" ? 0 : 1
    var sorted = values.slice().sort(function (a, b) { return a - b })
    var median = sorted[Math.floor(sorted.length / 2)]
    var out = 0
    if (range) {
      for (var i = 0; i < sorted.length; i++) {
        if (sorted[i] < range.low || sorted[i] > range.high) out++
      }
    }
    var clock = ("0" + Math.floor(minute / 60)).slice(-2) + ":"
      + ("0" + (minute % 60)).slice(-2)
    var line = clock + " · median " + median.toFixed(decimals)
      + " · spread " + sorted[0].toFixed(decimals) + "–"
      + sorted[sorted.length - 1].toFixed(decimals)
    if (range) {
      line += " · " + out + " of " + sorted.length + " days out of range"
    }
    return line
  }

  // "11 Aug", from the document's own `YYYY-MM-DD`.
  function dayLabel(date) {
    if (!date) return ""
    var parts = String(date).split("-")
    if (parts.length !== 3) return date
    var months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    return parseInt(parts[2], 10) + " " + months[parseInt(parts[1], 10) - 1]
  }

  // Whole days reads better than "336 h", and the baseline window is always
  // a whole number of them.
  function daysWords(hours) {
    var days = Math.max(1, Math.round(hours / 24))
    return days + (days === 1 ? " day" : " days")
  }

  // Lower is better for some of these and higher for others, so the direction
  // that counts as an improvement is named per row rather than assumed.
  function betterColor(delta, higherIsBetter) {
    if (Math.abs(delta) < 0.05) return Qt.darker(barForeground, 1.35)
    var good = higherIsBetter ? delta > 0 : delta < 0
    return good ? themed("in_range", "#98971a") : themed("high", "#d79921")
  }

  function signed(value, decimals) {
    var text = Math.abs(value).toFixed(decimals)
    if (Math.abs(value) < (decimals === 0 ? 0.5 : 0.05)) return "="
    return (value > 0 ? "+" : "−") + text
  }

  // What was logged in the window the chart covers, as one line. The chart
  // says when; this says how much, which is the number someone repeats out
  // loud when asked what they had.
  readonly property string treatmentTotals: {
    var list = doc && doc.treatments ? doc.treatments : []
    var carbs = 0
    var insulin = 0
    for (var i = 0; i < list.length; i++) {
      if (list[i].carbs > 0) carbs += list[i].carbs
      if (list[i].insulin > 0) insulin += list[i].insulin
    }
    var parts = []
    if (carbs > 0) parts.push(Math.round(carbs) + "g")
    if (insulin > 0) parts.push(insulin.toFixed(1) + "u")
    return parts.join(" · ")
  }
  readonly property var sensor: doc && doc.sensor ? doc.sensor : null
  readonly property var forecast: doc && doc.forecast ? doc.forecast : null

  // The reading against its projection, as a ratio of the shell's display
  // type so both follow a theme's font scale. 1.8 lands on 50px at the
  // default scale; the projection is exactly half — a note beside the
  // reading, not a second headline.
  readonly property int readingPx: Math.round(Style.font.displayLarge * 1.8)
  readonly property int forecastPx: Math.round(readingPx / 2)

  // The resolved palette the document was built with — the user's own,
  // including the colourblind preset. The literals below are the fallback for
  // a sugarrush too old to send one, and nothing else.
  readonly property var themeColors: doc && doc.theme ? doc.theme : null

  function themed(role, fallback) {
    return themeColors && themeColors[role] ? themeColors[role] : fallback
  }

  // The same ladder the chart and the pill use, so a number means the same
  // thing wherever it appears. Low and high are separate roles: painting both
  // amber lost a distinction the palette has always drawn.
  function classColor(klass) {
    switch (klass) {
      case "urgent-low":
      case "urgent-high": return themed("urgent", "#cc241d")
      case "low": return themed("low", "#d79921")
      case "high": return themed("high", "#d79921")
      case "in-range": return themed("in_range", "#98971a")
      default: return barForeground
    }
  }

  // How old a reading may be before it stops being the current one. The
  // alarm's own threshold, so the panel and the thing making the noise cannot
  // disagree about what "now" means.
  readonly property int staleMinutes: doc && doc.range && doc.range.stale_minutes > 0
    ? doc.range.stale_minutes
    : 15

  readonly property bool readingStale: reading
    ? (reading.class === "stale" || reading.age_min >= staleMinutes)
    : false

  // The clock time the reading was taken. Past a few minutes "26m ago" is
  // arithmetic the reader has to do; "last seen 03:14" is the answer.
  function seenAt() {
    if (!doc || !reading) return ""
    return Qt.formatTime(new Date(doc.generated_at - reading.age_min * 60000), "HH:mm")
  }

  // "6d 4h", the way the dashboard writes it.
  function ageWords(hours) {
    var d = Math.floor(hours / 24)
    var h = hours % 24
    return d > 0 ? d + "d " + h + "h" : h + "h"
  }

  // The countdown carries the state, so it is coloured like everything else:
  // amber inside the last day, red once it is over.
  function sensorColor() {
    if (!sensor || sensor.expired === undefined) return Qt.darker(barForeground, 1.35)
    if (sensor.expired) return themed("urgent", "#cc241d")
    if (sensor.expires_in_h <= 24) return themed("high", "#d79921")
    return Qt.darker(barForeground, 1.35)
  }

  function sensorWords() {
    if (!sensor) return ""
    var line = "sensor " + ageWords(sensor.age_h)
    if (sensor.expired === true) return line + " · expired"
    if (sensor.expires_in_h !== undefined) return line + " · " + ageWords(sensor.expires_in_h) + " left"
    return line
  }
  readonly property var agp: doc && doc.agp ? doc.agp : null
  readonly property bool hasInsights: insightDays > 0 && loadError === ""
    && doc && doc.insights && doc.insights.length > 0

  // Whether the cached *document* is too old to reuse — nothing to do with
  // whether the reading in it is stale, which is `readingStale` below. The two
  // were briefly both called "stale", and the property silently shadowed this
  // function, so every refresh threw and the panel showed no data at all.
  function documentExpired() {
    return !doc || (Date.now() - fetchedAt) > cacheMinutes * 60000
  }

  // What the alarm machinery is doing, from `sugarrush health --json`. The
  // panel can show a perfect graph while nothing is watching overnight, and
  // that is precisely the state someone needs to be told about.
  property var health: null

  function alarmWords() {
    if (!health) return ""
    if (health.watcher_alive !== true) return "not watching"
    if (health.alarm_configured !== true) return "no alarm set"
    var sites = health.sites || []
    for (var i = 0; i < sites.length; i++) {
      var until = sites[i].snoozed_until_ms
      if (until) {
        var left = Math.round((until - Date.now()) / 60000)
        if (left > 0) return "snoozed " + left + "m"
      }
    }
    return "alarm armed"
  }

  readonly property bool alarmArmed: alarmWords() === "alarm armed"

  // Armed is the quiet state, and everything else is worth an eye: nothing
  // watching is the failure the panel exists to make visible, a snooze is
  // deliberate but temporary.
  function alarmColor() {
    var words = alarmWords()
    if (words === "not watching" || words === "no alarm set") return themed("urgent", "#cc241d")
    if (words.indexOf("snoozed") === 0) return themed("high", "#d79921")
    return Qt.darker(barForeground, 1.35)
  }

  function loadHealth() {
    healthProc.running = false
    Qt.callLater(function () { healthProc.running = true })
  }

  readonly property string summaryCommand: snapshotCommand.replace(/snapshot.*$/, "summary")
  readonly property string snoozeCommand: snapshotCommand.replace(/snapshot.*$/, "snooze")
  // The site this document is about. `treatment` refuses to write without one
  // — deliberately, since guessing which person a health record belongs to is
  // not a guess software should make — so anything acting on this reading has
  // to name it back.
  readonly property string siteName: doc && doc.site ? doc.site : ""

  // The log form, collapsed until asked for: most opens are someone checking
  // a number, and a panel that grows a data-entry form for all of them is
  // paying for the exception.
  // Whether this site could accept a treatment at all. `treatment` refuses a
  // site with no write token, so a Log button without this walks someone into
  // a command that turns them away.
  readonly property bool canWrite: doc !== null && doc.can_write === true

  property bool logging: false
  property real logCarbs: 0
  property real logInsulin: 0
  readonly property bool logHasAmount: logCarbs > 0 || logInsulin > 0

  // Single-quoted for the shell, with any quote in the name escaped: a site
  // called "Alex's Libre" is a perfectly ordinary thing to configure.
  function shellQuote(text) {
    return "'" + String(text).replace(/'/g, "'\\''") + "'"
  }

  // The amounts go on the command line; the confirmation does not happen here.
  // `treatment` prints what it is about to write and asks for the person's
  // name before it touches a health record, and that check is the reason the
  // panel hands off to a terminal instead of writing the record itself.
  function reviewTreatment() {
    if (!logHasAmount || siteName === "" || !canWrite) return
    var cmd = setting("onClick", "omarchy-launch-floating-terminal-with-presentation sugarrush")
      + " treatment --site " + shellQuote(siteName)
    if (logCarbs > 0) cmd += " --carbs " + logCarbs.toFixed(0)
    if (logInsulin > 0) cmd += " --insulin " + logInsulin.toFixed(1)
    root.close()
    if (root.bar) root.bar.run(cmd)
    root.logging = false
    root.logCarbs = 0
    root.logInsulin = 0
  }

  // What the last action did, in words. Cleared on the way out, like the
  // clipboard note, so a stale "snoozed" never greets the next open.
  property string actionState: ""

  // Minutes left on a snooze, or 0. Read from the daemon's own health report
  // rather than from a timer started when the button was pressed: a snooze
  // set from the menu, the TUI or another machine counts just as much.
  readonly property int snoozedFor: {
    if (!health) return 0
    var sites = health.sites || []
    var most = 0
    for (var i = 0; i < sites.length; i++) {
      var until = sites[i].snoozed_until_ms
      if (until) most = Math.max(most, Math.round((until - Date.now()) / 60000))
    }
    return Math.max(0, most)
  }

  // Silencing an alarm that is not running is a button that lies. The panel
  // already knows, from the same report the alarm chip is drawn from.
  readonly property bool watching: health !== null && health.watcher_alive === true

  function snooze(spec) {
    root.actionState = spec === "off" ? "waking the alarm…" : "snoozing…"
    snoozeProc.command = ["bash", "-lc", root.snoozeCommand + " " + spec + " 2>&1"]
    snoozeProc.running = false
    Qt.callLater(function () { snoozeProc.running = true })
  }

  // "" until a copy is attempted, then what happened. Cleared on the way out
  // so the panel never opens still claiming a copy from an hour ago.
  property string copyState: ""

  function copySummary() {
    root.copyState = "copying…"
    copyProc.running = false
    Qt.callLater(function () { copyProc.running = true })
  }

  function applyHealth(out) {
    try {
      root.health = JSON.parse(String(out || ""))
    } catch (e) {
      // Say nothing rather than claim a state we could not read: an empty
      // chip is honest, "alarm armed" would not be.
      root.health = null
    }
  }

  function refresh(force) {
    if (!force && !documentExpired()) return
    // Drop whatever is in flight and start again, deferred: setting `running`
    // false and true in one pass is not a change at all, and skipping the
    // fetch whenever the property happens to read true loses refreshes for
    // good. The pill's fetch has the same shape for the same reason.
    snapProc.running = false
    Qt.callLater(function () { snapProc.running = true })
  }

  onOpenedChanged: if (opened) { refresh(false); loadHealth(); nextIn = refreshSeconds
                                     copyState = ""; actionState = ""
                                     logging = false; logCarbs = 0; logInsulin = 0
                                     showKeys = false }

  // A cached document belongs to the command that produced it. When the
  // command changes the cache is about something else, so drop it rather than
  // showing the old source's numbers for up to panelCacheMinutes.
  onSnapshotCommandChanged: {
    if (!doc && loadError === "") return
    doc = null
    loadError = ""
    fetchedAt = 0
    if (opened) refresh(true)
  }

  function loadConfig() {
    // Drop whatever is in flight and start again, deferred. Guarding on
    // `running` is how the pill and the snapshot fetch each lost a refresh:
    // a first run that never finished left the flag true for good.
    configProc.running = false
    Qt.callLater(function () { configProc.running = true })
  }

  function applyConfig(out) {
    var next = ({})
    var lines = String(out || "").split("\n")
    for (var i = 0; i < lines.length; i++) {
      var at = lines[i].indexOf(" = ")
      if (at > 0) next[lines[i].slice(0, at)] = lines[i].slice(at + 3).trim()
    }
    root.config = next
  }

  // Set one setting, then re-read: the CLI refuses values the app would have
  // repaired, so the file is the only honest source of what actually stuck.
  function setConfig(key, value) {
    root.configError = ""
    // A `[bar]` key changes what the pill above is drawing, and the pill only
    // looks once a minute. Refetch it once the write has actually landed —
    // doing it here would race the CLI to the file.
    root.refetchPillAfterSet = key.indexOf("bar.") === 0
    setProc.command = ["bash", "-lc",
                       root.configCommand + " " + key + " " + value + " 2>&1"]
    setProc.running = false
    Qt.callLater(function () { setProc.running = true })
  }

  // Widget options live in the bar's own config, not sugarrush's.
  function setWidget(key, value) {
    if (root.bar) root.bar.run("omarchy bar set " + root.moduleName + " " + key + " " + value)
  }

  function num(key, fallback) {
    var raw = parseFloat(root.config[key])
    return isNaN(raw) ? fallback : raw
  }

  onViewChanged: if (view === "settings") loadConfig()
  onConfigCommandChanged: if (view === "settings") loadConfig()

  function apply(out) {
    var parsed
    try {
      parsed = JSON.parse(String(out || ""))
    } catch (e) {
      root.loadError = "could not read snapshot output — update sugarrush"
      return
    }
    if (parsed.schema !== 1) {
      // Kept short: the hero's detail is a single line that clips rather
      // than wraps, and a clipped explanation explains nothing.
      root.loadError = "snapshot schema " + parsed.schema + ", panel needs 1"
      return
    }
    root.doc = parsed
    root.loadError = parsed.error ? String(parsed.error) : ""
    root.fetchedAt = Date.now()
  }

  Process {
    id: configProc
    command: ["bash", "-lc", root.configCommand]
    stdout: StdioCollector {
      id: configOut
      waitForEnd: true
      onStreamFinished: root.applyConfig(configOut.text)
    }
  }

  Process {
    id: setProc
    stdout: StdioCollector {
      id: setOut
      waitForEnd: true
      onStreamFinished: {
        var text = setOut.text.trim()
        // The CLI answers "key = value" on success and an error otherwise.
        root.configError = text.indexOf(" = ") > 0 ? "" : text
        root.loadConfig()
        if (root.refetchPillAfterSet) {
          root.refetchPillAfterSet = false
          if (root.hostWidget) root.hostWidget.refresh()
        }
      }
    }
  }

  Process {
    id: snoozeProc
    stdout: StdioCollector {
      id: snoozeOut
      waitForEnd: true
      onStreamFinished: {
        // Every line, not the last: `snooze` answers with the confirmation
        // and then, when there is one, the caveat — "no watcher running —
        // this arms the next one" is the half worth reading.
        root.actionState = snoozeOut.text.trim().split("\n")
          .filter(function (line) { return line.trim() !== "" })
          .join(" · ")
        // The daemon writes the state; re-read it rather than assuming the
        // write landed, and let the pill catch up at the same time.
        root.loadHealth()
        if (root.hostWidget) root.hostWidget.refresh()
        actionClear.restart()
      }
    }
  }

  // The document was fetched on open, cached for five minutes, and never
  // refreshed while someone sat there watching it. A low coming up would
  // leave the panel quietly untrue while the pill behind it kept updating —
  // the two disagreeing, with the bigger surface being the wrong one.
  //
  // One tick a second, only while open, so the countdown is honest rather
  // than a spinner that says something is happening without saying when.
  Timer {
    id: liveTick
    interval: 1000
    repeat: true
    running: root.opened
    onTriggered: {
      root.nextIn = root.nextIn - 1
      if (root.nextIn <= 0) {
        root.nextIn = root.refreshSeconds
        root.refresh(true)
        root.loadHealth()
      }
    }
  }

  Timer {
    id: actionClear
    interval: 6000
    onTriggered: root.actionState = ""
  }

  Process {
    id: copyProc
    // `wl-copy` on Wayland, `xclip` for anyone running this under Xwayland or
    // a stray X session. Whichever answers first wins; if neither is
    // installed the panel says so rather than pretending it copied.
    command: ["bash", "-lc",
              root.summaryCommand + " | { wl-copy 2>/dev/null || xclip -selection clipboard 2>/dev/null; }"]
    onExited: function (code) {
      root.copyState = code === 0 ? "copied to the clipboard" : "could not copy — is wl-clipboard installed?"
      copyClear.restart()
    }
  }

  Timer {
    id: copyClear
    interval: 4000
    onTriggered: root.copyState = ""
  }

  Process {
    id: healthProc
    command: ["bash", "-lc", root.healthCommand]
    stdout: StdioCollector {
      id: healthOut
      waitForEnd: true
      onStreamFinished: root.applyHealth(healthOut.text)
    }
    stderr: StdioCollector {
      id: healthErr
      waitForEnd: true
      // A sugarrush too old for `health --json` says so here; the chip stays
      // empty and the rest of the panel is unaffected.
      onStreamFinished: if (healthErr.text.trim() !== "") console.warn("sugarrush panel health", healthErr.text.trim())
    }
  }

  Process {
    id: snapProc
    command: ["bash", "-lc",
              root.snapshotCommand + " --hours " + root.overviewHours
              + " --days " + root.insightDays]
    stdout: StdioCollector {
      id: snapOut
      waitForEnd: true
      onStreamFinished: root.apply(snapOut.text)
    }
    stderr: StdioCollector {
      id: snapErr
      waitForEnd: true
      onStreamFinished: if (snapErr.text.trim() !== "") console.warn("sugarrush panel", snapErr.text.trim())
    }
  }


  // A card: a labelled box saying which window its contents describe. The
  // panel used to stack a 6-hour chart under 24-hour statistics with nothing
  // saying so; naming each section is the whole point of this layout.
  component Card: Rectangle {
    id: card
    property string label: ""
    default property alias content: cardContent.data

    width: parent ? parent.width : 0
    implicitHeight: cardColumn.implicitHeight + Style.space(18)
    color: "transparent"
    radius: Style.cornerRadius
    border.width: 1
    border.color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.16)

    Column {
      id: cardColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: Style.space(9)
      spacing: Style.space(7)

      PanelSectionHeader {
        text: card.label
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
      }

      Column {
        id: cardContent
        width: parent.width
        spacing: Style.space(6)
      }
    }
  }

  // One figure, now against the longer window, with the change named. The
  // colour is the answer to "is this better", which is the only reason to
  // print two numbers instead of one.
  component CompareRow: Item {
    id: compare
    property string label: ""
    property real now: 0
    property real then: 0
    property int decimals: 1
    property string suffix: ""
    property bool higherIsBetter: true
    // Some figures have no better direction: a lower mean bought with lows is
    // not an improvement, and colouring it green would say it was.
    property bool neutral: false
    property color nowColor: root.barForeground

    width: parent ? parent.width : 0
    implicitHeight: Math.max(compareLabel.implicitHeight, compareNow.implicitHeight)

    Text {
      id: compareLabel
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
      color: Qt.darker(root.barForeground, 1.35)
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: compare.label
    }

    Row {
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(6)

      Text {
        id: compareNow
        anchors.verticalCenter: parent.verticalCenter
        color: compare.nowColor
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.font.caption
        text: compare.now.toFixed(compare.decimals) + compare.suffix
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        color: Qt.darker(root.barForeground, 1.7)
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.font.caption
        text: "vs " + compare.then.toFixed(compare.decimals) + compare.suffix
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        color: compare.neutral
          ? Qt.darker(root.barForeground, 1.35)
          : root.betterColor(compare.now - compare.then, compare.higherIsBetter)
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.font.caption
        text: root.signed(compare.now - compare.then, compare.decimals)
      }
    }
  }

  // One line of the summary: what it is on the left, what it reads on the
  // right. Tinted only for the five bands, where the colour is the same one
  // the reading is drawn in everywhere else.
  component SummaryRow: Item {
    id: summaryRow
    property string label: ""
    property string value: ""
    property color tint: root.barForeground

    width: parent ? parent.width : 0
    implicitHeight: Math.max(summaryLabel.implicitHeight, summaryValue.implicitHeight)

    Text {
      id: summaryLabel
      anchors.left: parent.left
      anchors.right: summaryValue.left
      anchors.rightMargin: Style.space(8)
      anchors.verticalCenter: parent.verticalCenter
      elide: Text.ElideRight
      color: Qt.darker(root.barForeground, 1.35)
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: summaryRow.label
    }

    Text {
      id: summaryValue
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      color: summaryRow.tint
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: summaryRow.value
    }
  }

  // A bar standing in for a value that has not arrived. Deliberately quiet
  // and slow: it is saying "not yet", and a busy shimmer on a health panel
  // reads as something happening.
  component Skeleton: Rectangle {
    implicitHeight: Style.space(11)
    radius: 3
    color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.10)

    SequentialAnimation on opacity {
      running: true
      loops: Animation.Infinite
      NumberAnimation { from: 0.55; to: 1.0; duration: 900; easing.type: Easing.InOutQuad }
      NumberAnimation { from: 1.0; to: 0.55; duration: 900; easing.type: Easing.InOutQuad }
    }
  }

  // One on/off setting. The switch reads the config document rather than
  // holding its own state, so a write the CLI refuses leaves it showing what
  // the file actually says.
  component SwitchRow: Item {
    id: switchRow
    property string label: ""
    property string key: ""
    // A key that has never been written reads back blank, and the defaults
    // this panel edits are all "on".
    readonly property bool current: {
      var raw = root.config[switchRow.key]
      return raw === undefined || raw === "" ? true : raw !== "false"
    }

    width: parent ? parent.width : 0
    implicitHeight: Math.max(switchLabel.implicitHeight, switchToggle.implicitHeight)

    Text {
      id: switchLabel
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
      color: root.barForeground
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: switchRow.label
    }

    ToggleSwitch {
      id: switchToggle
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      checked: switchRow.current
      foreground: root.barForeground
      onToggled: root.setConfig(switchRow.key, switchRow.current ? "off" : "on")
    }
  }

  // One editable number. The step is the unit the setting is measured in —
  // 0.1 mmol/L for a threshold, a whole day for a sensor — so holding the
  // button walks it the way the settings screen does.
  component SettingRow: Item {
    id: row
    property string label: ""
    property string suffix: ""
    property real value: 0
    property real step: 1
    property int decimals: 0
    property bool enabled: true
    signal changed(real next)

    width: parent ? parent.width : 0
    implicitHeight: Math.max(rowLabel.implicitHeight, minus.implicitHeight)

    Text {
      id: rowLabel
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
      color: root.barForeground
      font.family: root.bar ? root.bar.fontFamily : Style.font.family
      font.pixelSize: Style.font.caption
      text: row.label
    }

    Row {
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(6)

      PanelActionButton {
        id: minus
        iconText: "−"
        tooltipText: "Less"
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
        enabled: row.enabled
        onClicked: row.changed(row.value - row.step)
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        width: Style.space(58)
        horizontalAlignment: Text.AlignHCenter
        color: root.barForeground
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.font.caption
        text: row.value.toFixed(row.decimals) + row.suffix
      }

      PanelActionButton {
        iconText: "+"
        tooltipText: "More"
        foreground: root.barForeground
        fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
        enabled: row.enabled
        onClicked: row.changed(row.value + row.step)
      }
    }
  }

  KeyboardPanel {
    id: surface
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    focusTarget: keys
    contentWidth: surface.fittedContentWidth(Style.space(420))
    contentHeight: surface.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keys
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function (direction) { root.switchPanel(direction) }

      // This ships to people running Hyprland, where the mouse is a last
      // resort, and every one of the panel's three views and dozen settings
      // rows needed a pointer to reach.
      Keys.onPressed: function (event) {
        switch (event.key) {
        case Qt.Key_1: root.view = root.views[0]; break
        case Qt.Key_2: root.view = root.views[1]; break
        case Qt.Key_3: root.view = root.views[2]; break
        case Qt.Key_Left: root.stepView(-1); break
        case Qt.Key_Right: root.stepView(1); break
        case Qt.Key_R: root.refresh(true); break
        case Qt.Key_D:
          root.close()
          if (root.bar) {
            root.bar.run(root.setting("onClick",
              "omarchy-launch-floating-terminal-with-presentation sugarrush"))
          }
          break
        case Qt.Key_Question: root.showKeys = !root.showKeys; break
        default: return
        }
        event.accepted = true
      }

      // Content can outgrow the room a popup is allowed — a patterns card on
      // top of the other three does it on a short screen — and a capped
      // KeyboardPanel would simply clip the overflow. Scroll it instead, the
      // way the shell's own panels do.
      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(10)

        // Wordmark left, controls right, on one line.
        Item {
          width: parent.width
          implicitHeight: Math.max(mark.height, controls.height)

          Image {
            id: mark
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            // A third, not a half. You know which app this is — you clicked
            // its pill to get here — and the number you came for was below
            // the fold on every view but one.
            width: Math.round(parent.width * 0.32)
            source: Qt.resolvedUrl("logo.png")
            sourceSize.width: parent.width * 1.5
            fillMode: Image.PreserveAspectFit
            smooth: true
          }

          // The reading, in the one place that survives scrolling and view
          // switches. On Settings it is the only reminder that the numbers
          // being edited are about something happening right now.
          Row {
            anchors.left: mark.right
            anchors.leftMargin: Style.space(10)
            anchors.right: controls.left
            anchors.rightMargin: Style.space(8)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(5)
            visible: root.reading !== null

            Text {
              anchors.verticalCenter: parent.verticalCenter
              color: !root.reading
                ? root.barForeground
                : (root.readingStale
                   ? Qt.darker(root.barForeground, 1.5)
                   : root.classColor(root.reading.class))
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.body
              font.bold: true
              text: root.reading ? root.reading.value : ""
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              color: Qt.darker(root.barForeground, 1.5)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.reading ? root.reading.arrow : ""
            }
          }

          Row {
            id: controls
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(4)

            // What a spinner should have said all along: not that something
            // is happening, but when.
            Text {
              anchors.verticalCenter: parent.verticalCenter
              rightPadding: Style.space(4)
              color: Qt.darker(root.barForeground, 1.8)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              visible: root.nextIn > 0 && root.loadError === ""
              text: root.nextIn + "s"
            }

            PanelActionButton {
              iconText: ""
              tooltipText: "Fetch now"
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              onClicked: { root.nextIn = root.refreshSeconds; root.refresh(true) }
            }

            PanelActionButton {
              iconText: ""
              tooltipText: "Open the dashboard"
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              onClicked: {
                root.close()
                if (root.bar) {
                  root.bar.run(root.setting("onClick", "omarchy-launch-floating-terminal-with-presentation sugarrush"))
                }
              }
            }
          }
        }

        ButtonGroup {
          options: [{ value: "glucose", label: "Glucose" },
                    { value: "profile", label: "Profile" },
                    { value: "settings", label: "Settings" }]
          value: root.view
          foreground: root.barForeground
          fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
          fontSize: Style.font.caption
          focusable: false
          onChanged: function (next) { root.view = next }
        }

        // Whatever went wrong replaces the cards: there is nothing to put in
        // them, and three empty boxes explain less than one sentence does.
        // A failure replaces the cards, because there is nothing to put in
        // them and three empty boxes explain less than one sentence does.
        Text {
          visible: root.loadError !== "" && root.view === "glucose"
          width: parent.width
          wrapMode: Text.WordWrap
          color: root.barForeground
          font.family: root.bar ? root.bar.fontFamily : Style.font.family
          font.pixelSize: Style.font.body
          text: root.loadError
        }

        // Waiting is not a failure, and it used to look like one: the whole
        // panel collapsed to a single line of text for as long as Nightscout
        // took. Draw the cards that are about to be filled instead — it tells
        // the eye where to look when the numbers land, and the panel keeps a
        // shape of its own rather than being assembled on arrival.
        Card {
          label: "Now"
          visible: root.loading && root.view === "glucose"
          Skeleton { width: Math.round(parent.width * 0.45); height: Style.space(30) }
          Skeleton { width: Math.round(parent.width * 0.3) }
        }

        Card {
          label: "Last " + root.panelHours + (root.panelHours === 1 ? " hour" : " hours")
          visible: root.loading && root.view === "glucose"
          Skeleton { width: parent.width; height: Style.space(64) }
        }

        Card {
          label: "Last 24 hours"
          visible: root.loading && root.view === "glucose"
          Skeleton { width: parent.width; height: Style.space(12) }
          Skeleton { width: Math.round(parent.width * 0.6) }
        }

        // Two numbers at the same size: the reading, and where it lands in
        // half an hour. A 9.6 rising to 10.4 is a different evening from a 9.6
        // settling, and that difference is the one the panel exists to show.
        Card {
          label: "Now"
          visible: root.reading !== null && root.loadError === "" && root.view === "glucose"

          Item {
            width: parent.width
            implicitHeight: nowValue.implicitHeight + nowCaption.implicitHeight

            // Baseline-anchored rather than stacked in a Row: the two numbers
            // are different sizes, and boxes aligned at the top leave their
            // digits sitting at different heights.
            // The widest reading these units can produce, measured but never
            // drawn. Without it the projection beside the number moved every
            // time a digit was gained or lost — motion on a panel you glance
            // at means "something changed", and a digit count is not that.
            Text {
              id: valueSizer
              visible: false
              font: nowValue.font
              text: root.doc && root.doc.units === "mg/dL" ? "888" : "88.8"
            }

            Text {
              id: nowValue
              anchors.left: parent.left
              anchors.top: parent.top
              width: Math.max(implicitWidth, valueSizer.implicitWidth)
              // Greyed out once it is too old to be the current reading: at
              // 3am the number is not wrong, it is absent, and drawing it in
              // its alert colour presents absence as a value.
              color: !root.reading
                ? root.barForeground
                : (root.readingStale
                   ? Qt.darker(root.barForeground, 1.5)
                   : root.classColor(root.reading.class))
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: root.readingPx
              font.bold: true
              text: root.reading ? root.reading.value : "—"
            }

            Text {
              id: heroArrow
              visible: root.forecast !== null
              anchors.left: nowValue.right
              // Clear the caption as well as the number: "mmol/L · falling" is
              // wider than "9.1", and anchoring to the number alone ran the
              // two captions into each other.
              anchors.leftMargin: Style.space(12)
                + Math.max(0, nowCaption.implicitWidth - nowValue.width)
              anchors.baseline: nowValue.baseline
              color: Qt.darker(root.barForeground, 1.6)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.title
              text: "→"
            }

            // Only drawn when there is a forecast: a sensor gap makes the
            // projection a fabrication, and predict refuses it rather than
            // guessing. An arrow to nothing would imply one anyway.
            Text {
              id: nextValue
              visible: root.forecast !== null
              anchors.left: heroArrow.right
              anchors.leftMargin: Style.space(12)
              anchors.baseline: nowValue.baseline
              color: root.forecast ? root.classColor(root.forecast.class) : root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: root.forecastPx
              opacity: 0.62
              text: root.forecast ? root.forecast.value.toFixed(1) : ""
            }

            Text {
              id: nowCaption
              anchors.left: nowValue.left
              anchors.top: nowValue.bottom
              color: Qt.darker(root.barForeground, 1.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: {
                if (!root.reading) return ""
                var units = root.doc ? root.doc.units : ""
                if (root.readingStale) return units + " · last seen " + root.seenAt()
                return units + " · " + root.reading.arrow + " "
                  + root.trendWords(root.reading.direction)
              }
            }

            Text {
              visible: root.forecast !== null
              anchors.left: nextValue.left
              anchors.baseline: nowCaption.baseline
              color: Qt.darker(root.barForeground, 1.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.forecast ? "in " + root.forecast.in_min + " min" : ""
            }

            Rectangle {
              id: nowPill
              anchors.right: parent.right
              anchors.top: parent.top
              implicitWidth: pillText.implicitWidth + Style.space(14)
              implicitHeight: pillText.implicitHeight + Style.space(6)
              radius: height / 2
              color: "transparent"
              border.width: 1
              // A stale reading is the most misleading thing this panel can
              // show, and it used to be the least visible: the age sat in the
              // same grey pill at 3 minutes and at 40.
              border.color: root.readingStale
                ? root.themed("urgent", "#cc241d")
                : Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.28)

              Text {
                id: pillText
                anchors.centerIn: parent
                color: root.readingStale
                  ? root.themed("urgent", "#cc241d")
                  : Qt.darker(root.barForeground, 1.2)
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: {
                  if (!root.reading) return ""
                  if (root.readingStale) return "no reading for " + root.reading.age_min + " min"
                  return "Δ " + root.reading.delta + " · " + root.reading.age_min + "m ago"
                }
              }
            }
          }

          // The reason someone opens this panel at 3am is to silence the thing
          // that woke them. Both commands already ship; neither was reachable
          // from the panel the alarm made them open.
          Row {
            spacing: Style.space(8)

            Button {
              visible: root.snoozedFor <= 0
              text: "Snooze 15m"
              bordered: true
              focusable: false
              enabled: root.watching
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.snooze("15m")
            }

            Button {
              visible: root.snoozedFor <= 0
              text: "1h"
              bordered: true
              focusable: false
              enabled: root.watching
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.snooze("1h")
            }

            // While one is running, the pair becomes what you would actually
            // want next: how long is left, and a way out of it.
            Button {
              visible: root.snoozedFor > 0
              text: "Wake now · " + root.snoozedFor + "m left"
              bordered: true
              focusable: false
              foreground: root.alarmColor()
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.snooze("off")
            }

            Button {
              text: root.logging ? "Cancel" : "Log"
              bordered: true
              focusable: false
              // Gone, not greyed. Logging needs a treatment write token, and
              // most people have not set one up — a permanently dead button
              // is a standing invitation to poke at something that will never
              // work. The README says how to turn it on; the panel of someone
              // who has not is simply a panel without it. Also gone against a
              // document that cannot name its site, which is what an older
              // sugarrush sends.
              visible: root.siteName !== "" && root.canWrite
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: {
                root.logging = !root.logging
                if (!root.logging) { root.logCarbs = 0; root.logInsulin = 0 }
              }
            }
          }

          // The form itself. Carbs and insulin only: they are what the chart
          // draws and what the command needs, and the time is now — logging a
          // meal three hours late is a job for the terminal, which takes an
          // `--at` the panel would need a date picker to offer.
          SettingRow {
            visible: root.logging
            label: "Carbs"
            suffix: " g"
            value: root.logCarbs
            step: 5
            onChanged: function (next) { root.logCarbs = Math.max(0, Math.min(300, next)) }
          }

          SettingRow {
            visible: root.logging
            label: "Insulin"
            suffix: " U"
            value: root.logInsulin
            step: 0.5
            decimals: 1
            onChanged: function (next) { root.logInsulin = Math.max(0, Math.min(50, next)) }
          }

          Row {
            visible: root.logging
            spacing: Style.space(10)

            Button {
              text: "Review in terminal"
              bordered: true
              focusable: false
              // Zero of both would be a write with nothing in it.
              enabled: root.logHasAmount
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.reviewTreatment()
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              width: Math.max(0, parent.parent.width - Style.space(150))
              wrapMode: Text.WordWrap
              color: Qt.darker(root.barForeground, 1.35)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: "sugarrush confirms before writing"
            }
          }

          Text {
            visible: text !== ""
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            // The daemon's own answer when there is one; otherwise the reason
            // the snooze buttons are dead, which is worth more than a greyed
            // button with no explanation.
            text: {
              if (root.actionState !== "") return root.actionState
              if (root.health !== null && !root.watching)
                return "Nothing is watching, so there is nothing to snooze."
              return ""
            }
          }
        }

        Card {
          label: "Last " + root.panelHours + (root.panelHours === 1 ? " hour" : " hours")
            + (root.treatmentTotals !== "" ? " · " + root.treatmentTotals : "")
          visible: root.loadError === "" && root.view === "glucose"

          Chart {
            width: parent.width
            height: Style.space(130)
            doc: root.doc
            foreground: root.barForeground
          }
        }

        Card {
          label: root.stats
            ? "Last " + root.stats.window_h + (root.stats.window_h === 1 ? " hour" : " hours")
              + (root.statsThin ? " · partial" : "")
            : ""
          visible: root.stats !== null && root.loadError === "" && root.view === "glucose"

          TirBar {
            width: parent.width
            height: Style.space(12)
            stats: root.stats
            themeColors: root.themeColors
          }

          // Without the comparison there is nothing to read these against;
          // with it they are the same three numbers doing more work.
          Text {
            visible: root.baselineStats === null
            width: parent.width
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.stats
              ? "mean " + root.stats.mean.toFixed(1)
                + " · GMI " + root.stats.gmi.toFixed(1) + "%"
                + " · CV " + (root.stats.cv === undefined ? "—" : root.stats.cv.toFixed(1) + "%")
              : ""
          }

          // A gap in the readings is not time spent in range, and the bar
          // above cannot tell the difference on its own.
          Text {
            visible: root.statsThin
            width: parent.width
            wrapMode: Text.WordWrap
            color: root.themed("high", "#d79921")
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.stats
              ? "Only " + Math.round(root.coverage(root.stats) * 100) + "% of these hours "
                + "have readings — the rest is a gap, not time in range."
              : ""
          }

          Text {
            visible: root.baselineStats !== null
            width: parent.width
            color: Qt.darker(root.barForeground, 1.7)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.baselineStats ? "against your last " + root.daysWords(root.baselineStats.window_h) : ""
          }

          CompareRow {
            visible: root.baselineStats !== null
            label: "in range"
            now: root.stats ? root.stats.tir.in_range : 0
            then: root.baselineStats ? root.baselineStats.tir.in_range : 0
            suffix: "%"
            higherIsBetter: true
            nowColor: root.themed("in_range", "#98971a")
          }

          CompareRow {
            visible: root.baselineStats !== null
            label: "below range"
            now: root.stats ? root.stats.tir.very_low + root.stats.tir.low : 0
            then: root.baselineStats ? root.baselineStats.tir.very_low + root.baselineStats.tir.low : 0
            suffix: "%"
            // The one figure a clinic reads first, and the only one where
            // less is unambiguously better.
            higherIsBetter: false
            nowColor: root.themed("low", "#d79921")
          }

          CompareRow {
            visible: root.baselineStats !== null
            label: "mean"
            now: root.stats ? root.stats.mean : 0
            then: root.baselineStats ? root.baselineStats.mean : 0
            // Neither direction is better on its own — a lower mean bought
            // with lows is worse — so the change is stated, not judged.
            neutral: true
            nowColor: Qt.darker(root.barForeground, 1.1)
          }

          CompareRow {
            visible: root.baselineStats !== null && root.stats !== null
              && root.stats.cv !== undefined && root.baselineStats.cv !== undefined
            label: "CV"
            now: root.stats && root.stats.cv !== undefined ? root.stats.cv : 0
            then: root.baselineStats && root.baselineStats.cv !== undefined ? root.baselineStats.cv : 0
            suffix: "%"
            higherIsBetter: false
            nowColor: Qt.darker(root.barForeground, 1.1)
          }
        }

        // ---- the profile view
        Card {
          label: root.agp
            ? "Typical day · " + root.agp.days + " days"
            : "Typical day"
          visible: root.loadError === "" && root.view === "profile"

          AgpChart {
            id: agpChart
            width: parent.width
            height: Style.space(150)
            visible: root.agp !== null
            agp: root.agp
            range: root.doc && root.doc.range ? root.doc.range : null
            themeColors: root.themeColors
            foreground: root.barForeground
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: {
              if (!root.agp) {
                return "Not enough days yet — a profile drawn from one or two of them "
                  + "cannot tell a habit from a bad Tuesday."
              }
              if (agpChart.selected < 0) {
                return "median, with the middle half and the outer 5–95% behind it"
                  + " · tap an hour for the days behind it"
              }
              return root.hourWords(agpChart.selected, agpChart.samplesAt(agpChart.selected))
            }
          }

          // The same text `sugarrush export` writes, on the clipboard: the
          // point of a profile is usually a conversation with a clinician,
          // and that conversation happens somewhere else.
          Row {
            spacing: Style.space(10)

            Button {
              text: "Copy summary"
              bordered: true
              focusable: false
              foreground: root.barForeground
              fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
              fontSize: Style.font.caption
              onClicked: root.copySummary()
            }

            Text {
              anchors.verticalCenter: parent.verticalCenter
              color: Qt.darker(root.barForeground, 1.2)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.copyState
            }
          }
        }

        // Beside the typical day rather than in it: the profile says what a day
        // looks like, and this says whether the days are getting better.
        Card {
          label: "Time in range · " + root.dayList.length
            + (root.dayList.length === 1 ? " day" : " days")
          visible: root.dayList.length > 1 && root.loadError === "" && root.view === "profile"

          DayStrip {
            width: parent.width
            height: Style.space(46)
            days: root.dayList
            themeColors: root.themeColors
            foreground: root.barForeground
          }

          Item {
            width: parent.width
            implicitHeight: firstDay.implicitHeight

            Text {
              id: firstDay
              anchors.left: parent.left
              color: Qt.darker(root.barForeground, 1.7)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.dayList.length > 0 ? root.dayLabel(root.dayList[0].date) : ""
            }

            Text {
              anchors.right: parent.right
              color: Qt.darker(root.barForeground, 1.7)
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: "today"
            }
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "average " + root.dayAverage.toFixed(0) + "% in range"
              + " · faded days have too few readings to trust"
          }
        }

        // The text `sugarrush export` writes, on screen before it goes on the
        // clipboard. You should not be sending a clinician a block of text you
        // have never read, and the five bands with their boundaries spelled
        // out is the form they read it in.
        Card {
          label: root.baselineStats
            ? "Clinical summary · " + root.daysWords(root.baselineStats.window_h)
            : "Clinical summary"
          visible: root.baselineStats !== null && root.loadError === "" && root.view === "profile"

          SummaryRow {
            label: "mean glucose"
            value: root.baselineStats ? root.baselineStats.mean.toFixed(1) + " " + root.units : "—"
          }
          SummaryRow {
            label: "GMI"
            value: root.baselineStats ? root.baselineStats.gmi.toFixed(1) + "%" : "—"
          }
          SummaryRow {
            label: "CV"
            value: root.baselineStats && root.baselineStats.cv !== undefined
              ? root.baselineStats.cv.toFixed(1) + "%" : "—"
          }

          SummaryRow {
            label: "very low · below " + root.bound("urgent_low")
            value: root.tirOf("very_low")
            tint: root.themed("urgent", "#cc241d")
          }
          SummaryRow {
            label: "low · " + root.bound("urgent_low") + "–" + root.bound("low")
            value: root.tirOf("low")
            tint: root.themed("low", "#d79921")
          }
          SummaryRow {
            label: "in range · " + root.bound("low") + "–" + root.bound("high")
            value: root.tirOf("in_range")
            tint: root.themed("in_range", "#98971a")
          }
          SummaryRow {
            label: "high · " + root.bound("high") + "–" + root.bound("urgent_high")
            value: root.tirOf("high")
            tint: root.themed("high", "#d79921")
          }
          SummaryRow {
            label: "very high · above " + root.bound("urgent_high")
            value: root.tirOf("very_high")
            tint: root.themed("urgent", "#cc241d")
          }
        }

        // The alarm's own record, which `alertlog.rs` has been keeping since it
        // was written and nothing has ever displayed.
        Card {
          label: "Alarms · " + (root.baselineStats
            ? root.daysWords(root.baselineStats.window_h) : "recent")
          visible: root.loadError === "" && root.view === "profile"

          Repeater {
            model: root.alertList.slice(0, 8)

            Item {
              required property var modelData
              width: column.width - Style.space(24)
              implicitHeight: alarmState.implicitHeight

              Text {
                id: alarmState
                anchors.left: parent.left
                color: root.classColor(
                  parent.modelData.state.indexOf("URGENT") === 0
                    ? (parent.modelData.state.indexOf("LOW") > 0 ? "urgent-low" : "urgent-high")
                    : (parent.modelData.state.indexOf("LOW") >= 0 ? "low" : "high"))
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: parent.modelData.state.toLowerCase()
                  + (parent.modelData.value !== undefined
                     ? " " + parent.modelData.value.toFixed(1) : "")
              }

              Text {
                anchors.right: parent.right
                color: Qt.darker(root.barForeground, 1.35)
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: root.alarmWhen(parent.modelData.at_ms)
                  + " · " + root.alarmLength(parent.modelData)
              }
            }
          }

          // The line that matters most in the whole log: an alarm was raised
          // and never arrived.
          Repeater {
            model: root.alertList.filter(function (a) { return a.failed && a.failed.length > 0 })
                                 .slice(0, 3)

            Text {
              required property var modelData
              width: column.width - Style.space(24)
              wrapMode: Text.WordWrap
              color: root.themed("urgent", "#cc241d")
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.alarmWhen(modelData.at_ms) + " · "
                + modelData.failed.join(" and ") + " never arrived"
            }
          }

          Text {
            visible: root.alertList.length === 0
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            // An empty log is a good fortnight, and should read as one rather
            // than as a card that failed to load.
            text: "No alarms in this window."
          }
        }

        Card {
          label: "Patterns · last " + root.insightDays + " days"
          // Not gated on there being any: "nothing stands out" is an answer,
          // and a card that vanishes reads as a panel that failed to load.
          visible: root.insightDays > 0 && root.loadError === "" && root.view === "profile"

          Repeater {
            model: root.doc && root.doc.insights ? root.doc.insights : []

            Text {
              required property var modelData
              width: column.width - Style.space(24)
              wrapMode: Text.WordWrap
              color: root.barForeground
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: modelData.text
            }
          }

          Text {
            visible: !root.doc || !root.doc.insights || root.doc.insights.length === 0
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.2)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "Nothing recurring stands out over these days."
          }
        }

        // ---- the settings view
        Card {
          label: "Alarm thresholds · " + (root.config["units"] === "mgdl" ? "mg/dL" : "mmol/L")
          visible: root.view === "settings"
          enabled: root.configLoaded
          opacity: root.configLoaded ? 1 : 0.45

          // The band, and only the band. Four numbered stepper rows showed
          // four settings; this shows the one thing they actually are — a
          // target band between two guard rails. A handle lands on a tenth of
          // a mmol/L in about two pixels, so precision did not go with them.
          ThresholdBand {
            width: parent.width
            urgentLow: root.num("alerts.urgent_low", 3.5)
            low: root.num("alerts.low", 3.9)
            high: root.num("alerts.high", 10.0)
            urgentHigh: root.num("alerts.urgent_high", 13.9)
            units: root.config["units"] === "mgdl" ? "mgdl" : "mmol"
            themeColors: root.themeColors
            foreground: root.barForeground
            onChanged: function (role, value) {
              root.setConfig(role, value.toFixed(root.config["units"] === "mgdl" ? 0 : 1))
            }
          }

          // The CLI refuses a value the app would have quietly repaired, and
          // that refusal belongs on screen rather than in a log nobody reads.
          // Crossed thresholds used to be the common case; the handles make
          // those unreachable, so what is left here is the physiological
          // bounds and whatever a future key rejects.
          Text {
            visible: root.configError !== ""
            width: parent.width
            wrapMode: Text.WordWrap
            color: root.themed("urgent", "#cc241d")
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.configError
          }
        }

        Card {
          label: "Alarm"
          visible: root.view === "settings"
          enabled: root.configLoaded
          opacity: root.configLoaded ? 1 : 0.45

          SwitchRow {
            label: "Audible alarm"
            key: "alerts.sound"
          }

          SettingRow {
            label: "Sensor life"
            suffix: " d"
            value: root.num("sensor_days", 10)
            step: 1
            onChanged: function (next) {
              root.setConfig("sensor_days", Math.max(0, Math.min(30, next)).toFixed(0))
            }
          }
        }

        // sugarrush's own `[bar]` config, not this widget's options: what it
        // says goes for every bar the reading reaches, which is why it is
        // here and not under "This panel".
        Card {
          label: "Status bar"
          visible: root.view === "settings"
          enabled: root.configLoaded
          opacity: root.configLoaded ? 1 : 0.45

          SwitchRow {
            label: "Trend arrow"
            key: "bar.arrow"
          }
          SwitchRow {
            label: "Delta"
            key: "bar.delta"
          }
          SwitchRow {
            label: "Unit"
            key: "bar.units"
          }
          SwitchRow {
            label: "Sparkline"
            key: "bar.sparkline"
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.45)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "The reading itself always shows. These apply to every bar sugarrush feeds, not only this one."
          }
        }

        Card {
          label: "This panel"
          visible: root.view === "settings"

          SettingRow {
            label: "Chart window"
            suffix: " h"
            value: root.panelHours
            step: 1
            onChanged: function (next) {
              root.setWidget("panelHours", Math.max(1, Math.min(72, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Overview span"
            suffix: " h"
            value: root.overviewHours
            step: 6
            onChanged: function (next) {
              root.setWidget("overviewHours", Math.max(6, Math.min(72, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Scroll back"
            suffix: " h"
            value: root.scrollbackHours
            step: 12
            onChanged: function (next) {
              root.setWidget("scrollbackHours", Math.max(6, Math.min(336, next)).toFixed(0))
            }
          }

          SettingRow {
            label: "Pattern history"
            suffix: " d"
            value: root.insightDays
            step: 1
            onChanged: function (next) {
              root.setWidget("insightDays", Math.max(0, Math.min(90, next)).toFixed(0))
            }
          }

          Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Qt.darker(root.barForeground, 1.45)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: "Everything else — sites, tokens, quiet hours, themes — lives in the dashboard."
          }
        }

        // Only when asked for. A shortcut list nobody opened is a panel
        // explaining itself to someone who did not ask a question.
        Card {
          label: "Keys"
          visible: root.showKeys

          Repeater {
            model: [
              { key: "1 · 2 · 3", what: "jump to a view" },
              { key: "← →", what: "step between views" },
              { key: "r", what: "fetch now" },
              { key: "d", what: "open the dashboard" },
              { key: "Esc", what: "close" },
              { key: "?", what: "hide this" }
            ]

            Item {
              required property var modelData
              width: column.width - Style.space(24)
              implicitHeight: keyName.implicitHeight

              Text {
                id: keyName
                anchors.left: parent.left
                color: Qt.darker(root.barForeground, 1.35)
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: parent.modelData.key
              }

              Text {
                anchors.right: parent.right
                color: root.barForeground
                font.family: root.bar ? root.bar.fontFamily : Style.font.family
                font.pixelSize: Style.font.caption
                text: parent.modelData.what
              }
            }
          }
        }

        // A status strip, not a card: the sensor and the fetch time describe the
        // rig rather than the glucose, and they are the two things here that
        // do not change every five minutes.
        Item {
          width: parent.width
          implicitHeight: Math.max(sensorText.implicitHeight, fetchedText.implicitHeight,
                                   alarmChip.visible ? alarmChip.height : 0)
            + Style.space(8)
          visible: (root.sensor !== null || root.health !== null)
            && root.loadError === "" && root.view === "glucose"

          Rectangle {
            anchors.top: parent.top
            width: parent.width
            height: 1
            color: Qt.rgba(root.barForeground.r, root.barForeground.g, root.barForeground.b, 0.16)
          }

          // The alarm's own state, left of the sensor: both describe the rig,
          // and this is the one that decides whether tonight is covered.
          Rectangle {
            id: alarmChip
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.bottomMargin: -Style.space(2)
            visible: alarmChipText.text !== ""
            width: alarmChipText.implicitWidth + Style.space(14)
            height: alarmChipText.implicitHeight + Style.space(6)
            radius: height / 2
            color: "transparent"
            border.width: 1
            border.color: Qt.rgba(root.alarmColor().r, root.alarmColor().g,
                                  root.alarmColor().b, root.alarmArmed ? 0.4 : 0.9)

            Text {
              id: alarmChipText
              anchors.centerIn: parent
              color: root.alarmColor()
              font.family: root.bar ? root.bar.fontFamily : Style.font.family
              font.pixelSize: Style.font.caption
              text: root.alarmWords()
            }
          }

          Text {
            id: sensorText
            anchors.left: alarmChip.visible ? alarmChip.right : parent.left
            anchors.leftMargin: alarmChip.visible ? Style.space(8) : 0
            anchors.bottom: parent.bottom
            color: root.sensorColor()
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.sensorWords()
          }

          Text {
            id: fetchedText
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            color: Qt.darker(root.barForeground, 1.45)
            font.family: root.bar ? root.bar.fontFamily : Style.font.family
            font.pixelSize: Style.font.caption
            text: root.reading ? "updated " + root.reading.age_min + "m ago" : ""
          }
        }
        }
      }
    }
  }
}
