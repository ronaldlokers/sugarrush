# Changelog

All notable changes to sugarrush are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning is
[CalVer](https://calver.org/) `YYYY.M.N` (the `N` resets each month).

## [Unreleased]

### Fixed

- **Health and delivery joins survive person renames safely.** Concurrent
  follower results and new delivery receipts now carry immutable site IDs;
  legacy name-only receipts remain readable without letting a reused display
  name inherit another person's current delivery status.

### Added

- **Treatment submissions have a privacy-safe receipt view.** `treatments`
  filters the bounded local audit by person and time, emits text/JSON/CSV, and
  exposes stable operation IDs for reconciliation without notes or credentials.

- **Managed watchers now point to usable diagnostics on every platform.**
  macOS and Windows services write to a private user-data log, install/status
  print its path plus the strict health command, and uninstall says that logs
  remain; platform session and independent dead-man limits are documented.

- **Operational health is explicit instead of collapsing unlike guarantees.**
  JSON now separates process, data, configured-channel, suppression, and known
  delivery status; `--strict-delivery` gives external monitors an opt-in
  degraded exit policy without claiming that an accepted alert was received.

- **Multi-person exports can no longer silently choose the first person.** A
  follower must select `--site NAME` or explicitly request `--all`; filenames
  and output identify their subject and the matching cache/timezone are used.

- **Private cache storage is inspectable and selectively erasable.** `cache
  status` reports each person's entry count, date span, and bytes without
  printing readings; confirmed clear commands target one person or everyone.

- **People now have immutable internal identities.** Renaming a display label
  no longer moves alarm episodes, snoozes, cached readings, or treatment audit
  receipts between people; legacy configurations receive a deterministic ID
  and newly added sites receive a generated UUID.

- **Treatment writes are available behind an explicit security boundary.** A
  separate per-site CarePortal token is masked in Settings and capability-
  checked before every confirmed `treatment` command; amounts, timestamps, and
  notes are validated and each accepted/rejected attempt is recorded in an
  owner-only audit without credentials or note text.

- **Offline history is available as an explicit privacy choice.** An opt-in,
  owner-only per-site cache gives instant startup and outage context without
  ever presenting cached data as a live fetch; retention is bounded to 1–90
  days and disabling it deletes the local record.

- **Caregiver review now follows the person's context.** Each site can set an
  IANA timezone used by AGP bucketing, pattern names, CSV offsets, and clinical
  summaries; alert history can be filtered with `--site` or emitted as
  structured JSON/CSV for private audit workflows.

- **The always-on watcher now has native service management on every shipped
  desktop platform.** One command installs, starts, inspects, or removes a
  systemd user service on Linux, a launchd agent on macOS, or a Task Scheduler
  task on Windows; the old `--install-unit` spelling remains an alias.

- **Alarm delivery is now auditable and monitorable.** Alert history records
  privacy-safe channel outcomes using careful accepted/rejected language, and
  `sugarrush health --json` exposes watcher liveness, per-site freshness,
  snoozes, alarm state, and the latest delivery attempt for external monitors.

- **Caregiver actions now target the right person.** The follower list has a
  stable selected row that opens that person's dashboard with Enter and
  snoozes only that person with `a`; the CLI accepts `--site NAME` and requires
  explicit `--all` when several sites are configured.

### Fixed

- **Fast alarm transitions and watcher webhooks now survive restarts.** The
  three-second reaction path persists episode latches immediately, while a
  bounded owner-only webhook outbox retries outside the alarm loop, resolves
  destinations from current config, and cancels obsolete sends on recovery.

- **Private cache opt-out is now durable across running processes.** Cache
  merges and deletion are serialized, disabling writes a persistent boundary
  that stale dashboards cannot cross, and write/deletion failures are surfaced
  instead of silently pretending history was retained or removed.

- **Treatment writes now survive ambiguous failures safely.** An operation is
  durably recorded before sending, transport uncertainty is distinct from
  rejection, explicit retries reuse one UUID, remote acceptance is reported
  even if final auditing fails, future entries are rejected, and interactive
  writes require reviewing the person, amounts, and timestamp.

- Webhook delivery no longer blocks the watcher's three-second alarm loop, so
  a slow destination cannot delay sound or stale-data detection for other
  followed people.

- **Settings now protect changes and credentials as a complete workflow.** Push
  destinations can be replaced without revealing embedded topics or tokens,
  new and edited Nightscout sites must return a fresh reading before saving,
  and leaving with dirty settings requires an explicit Save, Discard, or Cancel
  decision; Discard restores the last loaded or saved configuration.
- **The follower screen now scales past one terminalful of people.** It has
  worst-first scrolling and paging with explicit above/below affordances, a
  screen-specific help overlay, bounded names and concise failure text so long
  input cannot break the safety columns, and a header that keeps the worst
  state visible while the list is scrolled.
- **First-run setup now gets users all the way to a working dashboard.** The
  wizard has an explicit exit hatch and Nightscout token-help link, verifies
  that the site returns a reading from the last hour instead of treating an
  empty response as success, and finishes with the key dashboard controls plus
  the commands for installing and testing the always-on alarm.
- **Each followed site can now have its own alert settings.** A person can
  inherit the global thresholds and channels or use a complete override edited
  from the same settings screen; the dashboard, followers list, status output
  and headless watcher all classify that site with its effective settings.
- **Sites can now be added and removed in the settings screen.** Site names,
  URLs and read-only tokens are editable in-app, additions never copy another
  person's token, and the final site cannot be removed accidentally.
- **The documentation now covers the human and technical on-ramp for
  following.** It explains consent, shared expectations, read-only access and
  the limits of remote monitoring, then points Libre and Dexcom users to the
  maintained Nightscout uploader paths so a new user can get readings into
  Nightscout before configuring sugarrush.
- **`sugarrush about` is now a real diagnostic.** The issue template asks for
  its output, and it printed a version number and the safety note — so every
  bug report arrived without the answers that matter for a CGM alarm. It now
  reports the build and toolchain, terminal and session type, config path and
  validity, site count with hosts (not URLs) and whether a token is set,
  thresholds, which alarm channels are switched on, whether a watcher or
  dashboard is running, any active snooze, and how many alerts were logged this
  week. No secrets: the token is reported as set or not set.
- **`sugarrush --man` writes a man page.** Packagers had nothing to install as
  `sugarrush.1`, so `man sugarrush` said "No manual entry" everywhere. The man
  page, `--help` and the README command table now all render from one table in
  the source, with a test that keeps the README in step.
- **`sugarrush alerts` shows what the alarm has actually done.** Nothing kept a
  record, so "did it go off last night, and for how long?" was unanswerable —
  the systemd journal only exists if you run the daemon that way, is rotated by
  someone else's policy, and says nothing about alarms the dashboard handled.
  Episodes are now logged (owner-only, 90 days) and `sugarrush alerts --days 7`
  prints them with durations. An empty report says so *and* says it might mean
  nothing was running to notice — a quiet week and a dead watcher look
  identical otherwise.
- **The header always says whether your alarm is armed.** Four things could
  silence it with no on-screen evidence — quiet hours, a snooze, a watcher that
  stopped, and an alarm with nothing switched on to announce with — and the app
  never mentioned any of them. One chip now answers it, naming the most
  suppressing condition: `⚑ alarm armed · watcher up`, `☾ quiet until 07:00 ·
  urgent lows only`, `⏸ alarm snoozed · 12m left`, `⚠ watcher stopped`,
  `⚑ alarm off`. Escalation configured with no push channel is called out
  alongside as `⚠ escalation inactive`. It lives in the header, so unlike the
  old snooze chip it survives an error state.
- **`sugarrush watch --test` checks that the alarm can actually reach you.**
  There are eight independent reasons a night can pass without a sound — the
  alarm switched off, quiet hours, a forgotten snooze, no working audio player,
  no watcher running, a dead notification daemon, a broken push URL, or
  escalation configured with no channel to escalate on — and nothing in the app
  could tell you which applied. The self-test walks all of them, plays a real
  sound, sends a real notification and a real webhook, and exits non-zero if
  anything that is switched on doesn't work. `--quiet` checks without making a
  noise. There's a **Test the alarm** row in the settings screen for the
  audible half.
- **`sugarrush snooze` silences the alarm daemon.** Until now the only way to
  stop a 3am alarm from `sugarrush watch` was `systemctl --user stop`, which
  also disarms the *next* one. `sugarrush snooze 15m` (or `2h`, or `off`)
  silences it without stopping it, works whether or not a watcher is currently
  running, and survives a service restart. Pressing `a` in the dashboard now
  does the same, so a snooze isn't lost when you close it.
- **Every time-in-range band now has a number, not just a colour.** The stats
  panel printed only "in range" and "below"; above-range and very-low existed
  solely as segments of the bar — and on the default palette two of those
  segments are the same red. The line now reads e.g. `43% in range · 29% below
  (14% very low) · 29% above`, shedding detail whole as the pane narrows rather
  than clipping a percentage into a different number.
- **The clinical export cites its sources.** Time-in-range and CV goals now
  reference the 2019 international consensus (Battelino et al., Diabetes Care)
  and GMI references Bergenstal et al. 2018, with a note that the consensus
  targets are stated for 70–180 mg/dL while the percentages are computed
  against your configured thresholds — so a clinician can tell whether they're
  comparing like with like.

### Fixed

- **Alarm runtime edge cases now fail visibly without blocking detection.** The
  watcher warns when a configured push destination uses cleartext HTTP,
  implausibly future-dated readings cannot suppress stale-data detection,
  audio-player discovery runs off the async alarm loop, and the documented
  delivery policy explains why failed one-shot notifications are surfaced but
  not blindly replayed after recovery.
- **Local files and Nightscout responses now have explicit safety bounds.**
  Alarm WAVs live in an owner-only runtime directory instead of predictable
  shared-temp paths, alert-log append and compaction are serialized across the
  TUI and watcher, oversized API bodies are rejected at 8 MiB, and unreadable
  or corrupt watcher state is reported instead of silently looking empty.
- **Opening a multi-site dashboard no longer silences every followed person's
  watcher for 30 seconds.** The TUI wrote an alarm-claim heartbeat at startup
  before checking how many sites it covered. Startup and later site changes now
  use the same rule: only a single-site dashboard claims the alarm, while a
  multi-site watcher remains active and its liveness is shown in the header.
- **A slow followed site can no longer stall every alarm.** The headless
  watcher used to await entries and device status for each person in sequence
  inside the same loop that rechecks alarms every three seconds. Polls now run
  concurrently in the background, with at most one per site, so stalled
  Nightscout requests cannot delay local stale detection or another person's
  alarm cadence.
- **Watcher retries and concurrent saves can no longer erase alarm state.** A
  site skipped during retry backoff used to disappear from the persisted
  episode map, and a `sugarrush snooze` command issued while the daemon was
  polling could be overwritten by its older snapshot. Every site now remains
  restart-safe, while serialized updates merge the latest external snooze
  immediately before the watcher saves.
- **A sensor gap now notifies and escalates as fast as it starts beeping.** The
  audible alarm ran on a 3-second tick while notifications and the escalation
  webhook only went out on a refresh, so a gap that crossed into "no recent
  readings" between refreshes sounded immediately and then stayed silent on
  every other channel for up to a full refresh interval. All channels now fire
  on the same pass.
- **Refreshes are faster and ask for less.** The five supplementary reads
  (treatments, device status, sensor age, history, overview) were awaited one
  after another, costing the sum of five round trips to your Nightscout; they
  now go out together and cost the slowest one. The sensor-age lookup — a
  second `/treatments` request every cycle for a number that changes twice a
  month — is now cached for 30 minutes.
- **The AGP fan renders on terminals without 24-bit colour.** It was painted
  as RGB cell backgrounds, which collapse on 16-colour consoles, tmux and SSH
  sessions with no `COLORTERM` — leaving an unlabelled median line where the
  percentile fan should be. Those terminals now get the fan as shaded blocks in
  the theme colour, which also distinguishes the two bands by texture rather
  than colour alone.
- **The AGP legend no longer disappears when there's something to report.** The
  "median + IQR + 5/95" key was replaced by the pattern headline, so the reader
  lost the key to the chart exactly when the chart had a finding. Both are
  shown.
- **`--demo` is no longer silently ignored by the subcommands.** `sugarrush
  watch --demo` looked like a safe way to try the alarm out and instead started
  the daemon against the real site and the real config; `export`, `status` and
  `waybar` ignored the flag the same way. They now say so and exit non-zero.
- **The AGP no longer calls one bad night a pattern.** Insights were guarded by
  how long a run lasted but not by how many days fed it, and with a single
  day's readings the 25th percentile and the median are the same number — so
  one rough night could be named as a recurring overnight low, on screen and in
  the clinician export. A time-of-day pattern now needs readings from at least
  three separate days.
- **Chart time labels no longer collide into a date that never existed.** On a
  narrow terminal the three `MM-DD HH:MM` stamps under the graph overlapped and
  rendered as text like `8-09 01:-09` — a wrong reading of when, on a chart
  people read clinically. Labels now shrink to the clock alone (the pane title
  already carries the dated range) and drop middles before they can overlap;
  the AGP hour labels thin the same way.
- **`?` now works on every screen.** Pressing it in the caregiver view did
  nothing, then the overlay appeared unbidden on the next dashboard render; the
  settings screen — the one with thirty rows of unexplained options — had no
  help key at all. The overlay opens over whatever screen you're on, any key
  closes it, and both footers advertise it. On settings it lists the settings
  keys rather than the graph ones.
- **Two settings rows explain themselves again.** Pressing `←`/`→` on the site
  URL or on push alerts is meant to answer "press enter to edit" and "set
  push_url in config.toml"; both messages were written and then erased before
  anything drew them, so the rows looked like dead keys.
- **Text prompts put the real cursor where you're typing.** The site URL, the
  token and the date-jump prompt drew a fake blinking `_` and never positioned
  the terminal cursor, so screen-reader caret tracking and braille cursor
  routing had nothing to follow — worst on the token field, where the text is
  bullets and the caret is the only cue. The blink is gone too: it could not be
  turned off, which is a problem for anyone with a migraine or vestibular
  trigger.
- **The live dot now reports the connection, not the view.** A green `●` sat
  next to a red authentication error, because the dot belonged to the
  `live`/`history` view mode and knew nothing about the network. The dot is now
  green when the data is current, amber `◌` during a sensor gap, and red `✖`
  when the site is unreachable; `live`/`history` stays as the view label.
- **One failure, one explanation.** A rejected token produced three different
  messages across three panels — "no data in this window…", "no readings in
  this window…" and "loading overview…", the last of which was untrue: nothing
  was loading and nothing would. Empty panels now give the same reason, and a
  paused fetch never claims to be loading.
- **The glucose reading no longer disappears on a short terminal.** Below
  roughly 22 rows the fixed-height panes were crushed while the graph kept its
  full size, so `current` collapsed to a border and the number itself was not
  on screen at all — on a tiling window manager, a phone SSH session, or a
  terminal at 200% zoom. The layout now sheds panes deliberately: stats first,
  then the overview strip, then `current`'s borders in favour of a one-line
  readout with the value, arrow, state and range bar. The reading is the last
  thing to go.
- **A slow or wedged Nightscout no longer freezes the dashboard.** The run
  loop waited for the whole fetch chain — up to five requests at a 12-second
  timeout each — before it would handle a keypress, redraw, or sound the alarm.
  A site that accepted the connection and then went quiet left an app that
  looked alive and answered nothing, with the alarm silent for the duration.
  Fetches now run in the background: keys, the graph and the alarm keep working
  throughout, and the reading updates when the data lands.
- **The alarm no longer fires a burst after a stall.** Missed ticks were
  replayed back-to-back, so waking a suspended laptop set off one alarm sound
  per missed three-second tick instead of one alarm.
- **A warning no longer hides the way to fix it.** Any error or warning took
  over the whole footer, including `? help` and `s settings` — and two of them
  (a readable config file, an unencrypted site) never go away on their own, so
  a user in that state never saw a keybinding hint again. Warnings now share
  the line with `? help`, and are shortened with an ellipsis rather than being
  cut off mid-sentence.
- **The alarm banner is readable on every palette.** It drew black text on the
  alert colour, which measured as low as 2.2:1 — worst of all on the
  colourblind palette, the one chosen for legibility. The text colour is now
  picked from the background it sits on. The banner also honours the theme for
  *every* state; "no data" was hardcoded and ignored your palette entirely.
- **Status-bar output says how bad it is, not just what colour.** Only the
  Waybar format carried the alert state, so on tmux, polybar and i3blocks the
  colour was the entire signal — and `--format text`, the one a shell prompt or
  a screen reader reads, carried no state at all. Non-normal readings are now
  prefixed `!!`, `!` or `?`.
- **The default palette gives "low" and "urgent low" different colours.** Both
  were plain red, so the split the time-in-range bar, the range bar and the
  followers list all draw was invisible without reading the label.

### Fixed

- **No more "heading low" invented across a sensor gap.** The short-term
  forecast assumed its two readings were five minutes apart and never checked,
  so a pair either side of a dropout was extrapolated as if the change had
  happened in five minutes — a flat trend across 40 minutes projected a low and
  fired a prediction. When the spacing isn't right, there is now no forecast
  rather than a fabricated one.
- **The watcher polls once a minute instead of every few seconds.** It inherited
  the dashboard's interval, which is tuned for a responsive screen — about
  17,000 requests a day per site against a self-hosted Nightscout, and a radio
  kept awake for readings that arrive every five minutes. It also now respects
  its own retry backoff instead of hammering a site that is down.
- **Another user on the same machine can't silence your alarm.** Without a
  per-user runtime directory the alarm handshake used a shared path in `/tmp`,
  where anyone could pose as a running dashboard and keep the watcher quiet
  indefinitely. The path is now per-user, and a heartbeat that isn't ours is
  ignored rather than obeyed.
- **The watcher's saved state is written safely.** It was rewritten in place
  every cycle, so an interrupted write left a truncated file that loaded as
  "no state" — cancelling an active snooze and restarting an escalation timer,
  the two things it exists to prevent. It's now written atomically and
  owner-only, since in follower mode it names another person.
- **Duplicate readings no longer flatten the delta.** A site fed by two
  uploaders holds each reading twice, which made the change-since-last-reading
  show 0 during a genuine rise and double-counted those minutes in the stats.
- **Sensor age stops disappearing.** It was read from the newest 50 treatments
  of any kind, which for a pump user covers about three days — while a sensor
  lasts ten to fourteen, so the sensor-change event fell off the end. The
  server is now asked for sensor events specifically.
- **Two sites can't share a name.** Alarm state is tracked per site name, so a
  duplicate meant announcing a low for one person marked it announced for the
  other. Startup now says so instead.

### Fixed

- **Push alerts now say whose reading it is, and respect your privacy
  setting.** The webhook is the only channel that reaches a phone, and with
  several sites configured it sent a bare "URGENT LOW" with no way to tell
  which person it was about. It also always spelled out the glucose value, even
  with `Notification detail` set to *generic* — so a setting people turn on for
  privacy was shipping their reading to a third-party broker anyway.
- **An unencrypted webhook is now flagged**, the way an unencrypted site URL
  already was. A `http://` push URL sends your alerts in clear text; the
  settings row now says so. (The topic path is still never displayed — it's a
  password in all but name.)
- **Exports are written owner-only, and tell you where they went.** Files
  holding two weeks of glucose readings were created world-readable, while the
  config file holding a read-only token was carefully created `0600`. The
  in-app `e` key also reported a bare filename, so it wasn't clear which
  directory your health data had landed in; it now prints the full path.
- **Exported CSVs can't corrupt or execute in a spreadsheet.** Fields are now
  quoted and escaped, and a value that a spreadsheet would run as a formula is
  neutralised — which matters because the trend value comes from the server,
  and in follower mode that's someone else's server.
- **The config-permissions warning now appears in every mode.** It only showed
  in the dashboard, so the person running the headless watcher — the one least
  likely to open the dashboard — never learned their token file had become
  readable by others.

### Added

- **`sugarrush --help` and `--version`.** There were none: `--help` opened the
  dashboard, and on a machine with no config it opened the *setup wizard* and
  started asking for a Nightscout token. Unknown arguments now say so and exit
  instead of being silently ignored.
- **`sugarrush watch --install-unit`** writes a systemd user service pointing at
  wherever your binary actually is, and prints the commands to enable it. The
  old instructions told you to copy a file out of a git checkout — which four of
  the five install methods never produce — and the unit hardcoded a path only
  `cargo install` uses.
- **A troubleshooting section and a command table in the README**, including
  every reason the alarm can be silent, in the order worth checking.

### Fixed

- **The systemd unit now survives logging out.** It was tied to
  `graphical-session.target`, which sway, Hyprland without uwsm, i3 and bare X
  never activate — so `systemctl --user enable --now` appeared to work and then
  never started again after a reboot. It now uses `default.target`, drops
  `PartOf=`, and ships with hardening directives.
- **`config.example.toml` set `refresh_secs` inside `[minimap]`**, where it
  parsed as a minimap key and was silently ignored — so copying the example and
  changing the refresh interval did nothing. A test now parses the shipped
  example and asserts what it actually means.
- **The keybinding overlay is sized from its contents.** It was a fixed 56
  columns, which clipped its longest line and cut off "press any key to close",
  so the overlay never said how to leave it. It also now mentions that
  `watch`, `export` and `status` exist.
- **A broken release can no longer publish to the AUR.** The job ran whenever
  the release workflow wasn't *cancelled* — and a failure isn't a cancellation.

### Fixed

- **Looking at yesterday no longer silences today's alarm.** Panning or jumping
  into history made the app classify the *historical* reading, so an urgent low
  happening right now read as "in range" — and because the dashboard tells the
  watch daemon to stay quiet while it's open, the whole system went silent for
  as long as you were reading history. Only the graph is historical now; the
  alarm always follows the live edge.
- **A site that has never connected now raises the alarm.** A watcher started
  with a wrong token, or against a site that was never reachable, reported
  "in range" indefinitely — indistinguishable from a healthy quiet night. After
  the staleness window with nothing received at all, that's now a sensor gap
  like any other.
- **With several sites configured, the dashboard no longer silences the
  watcher.** The dashboard alerts on the site you're viewing, but it was
  telling `sugarrush watch` — which covers *every* configured site — to stand
  down, so a caregiver's other people went unalarmed whenever the dashboard was
  open. It now only claims the alarm when it genuinely covers everything.
- **An undelivered desktop notification is reported instead of assumed.** If no
  notification daemon is running the D-Bus call fails silently, so "Desktop:
  on" could be a lie — and paired with a failing audio player, that's two dead
  channels both reporting healthy. The footer now says when notifications
  aren't getting through.

### Fixed

- **The Nightscout token can no longer leak into an error message.** Because
  Nightscout takes the token as a URL query parameter, and the HTTP client
  included the full URL in its errors, `sugarrush export` printed the token in
  cleartext whenever a request failed — into cron mail, the journal, terminal
  scrollback, and any bug report someone pasted. Request URLs are now stripped
  from every client error before it can be displayed.
- **A units mismatch in `config.toml` can no longer disable the low alarm.**
  The example config is written in mmol/L, so changing `units` to `mgdl` and
  nothing else left thresholds like `low = 3.9` **mg/dL** in force — and since
  every real reading sits above them, a 40 mg/dL hypo was classified as *urgent
  high*. Thresholds are now checked against the physiological range on load: an
  implausible value falls back to the safe default, crossed thresholds are put
  back in order, and each correction is reported on stderr and in the footer,
  naming the value and pointing at `units`. The settings screen has enforced
  these rules for a while; the config-file path now matches it.
- **Snoozing a sensor gap no longer silences the low that follows it.** An
  alert episode was tracked as "urgent or not" rather than as *which* urgent
  state, so a sensor gap and the urgent low that arrived when the sensor came
  back were treated as one continuous episode: silencing the gap at 03:00 also
  silenced the 40 mg/dL reading two minutes later, swallowed its push, and left
  the escalation timer running from the gap — announcing "STILL URGENT LOW
  after 20 min" for a low that was two minutes old. A change of urgent state is
  now a new emergency: it re-arms the alarm, pushes at its own onset, and
  restarts its own escalation clock. Repeated readings of the *same* urgent
  state still share one episode, so a snooze keeps working.
- **A failed audio player no longer counts as a sounded alarm.** sugarrush
  picked the first player it could *launch* — but launching only proves the
  program exists, not that it reached an audio server. `paplay` is installed
  almost everywhere and exits immediately when the server isn't reachable (a
  service started before the session, an SSH login, a container), with its
  error already discarded — so the alarm was silent and the terminal-bell
  fallback was never reached. sugarrush now checks that the player is still
  playing shortly after launch, moves on to the next one if it isn't, and rings
  the bell when none of them work. A player that fails is remembered, so it's
  tried once rather than every few seconds all night.

### Added

- **The dashboard now tells you whether the alarm watcher is running.** Nothing
  ever read the watcher's heartbeat, so a dead `sugarrush watch` and a quiet
  night looked exactly the same. The header shows `⚑ watcher up` while it's
  alive and `⚠ watcher stopped` if it was running and then wasn't — and stays
  quiet for anyone who doesn't use the daemon, so it's information rather than
  nagging.
- **`sugarrush watch` says it's alive even when nothing happens.** It logged
  only alert transitions, so the morning after a missed alarm an empty journal
  could mean "glucose was flat all night" *or* "the daemon was dead" with no
  way to tell them apart. It now writes a line every 15 minutes —
  `ok · 5.6 mmol/L · in range · 2m ago` — so a quiet journal proves it was
  watching.
- **Step through history a day at a time** with `[` and `]`, keeping the same
  time of day — checking "how was last night?" no longer means typing a date or
  panning there a half-window at a time.
- **The AGP now names its patterns.** Reading a recurring overnight low off a
  percentile fan is a skill; the AGP title now states the worst finding
  outright — `⚠ lows 02:00–05:00 (down to 3.1 mmol/L)` — and the exported
  summary lists every one under a **Patterns** section. A "lows" window is a
  time of day where a quarter of readings or more sit below target; "highs" is
  where the typical reading is above it. Runs shorter than 45 minutes aren't
  reported, and a gap in the data never joins two windows into one.
- **Follow more than one person.** With several `[[sites]]` configured, `m`
  opens a follower view listing everyone at once — value, trend, how old the
  reading is, and the alert state — sorted worst first, with a header that names
  whoever needs attention. A site that can't be read ranks with the urgent ones
  rather than showing a blank row that reads like "fine". `sugarrush watch` now
  watches every configured site too, each with its own independent alert
  episode, naming the site in notifications and in the log.
- **Status-bar output for bars other than Waybar.** `sugarrush status` prints
  one line in the syntax your bar speaks — `--format text` (no markup),
  `tmux`, `polybar`, `i3blocks`, or `waybar` — coloured from your configured
  theme, so the colourblind-safe palette carries over. `sugarrush waybar` is
  unchanged and still prints the same JSON.
- **An always-on alarm watcher.** `sugarrush watch` runs the alert pipeline
  headless — no terminal needed — so a nocturnal low still wakes you when the
  dashboard isn't open. It defers to a running dashboard (both write a
  heartbeat, so you never get two alarms for one low) and persists episode
  state, so restarting the service doesn't re-announce an ongoing low, restart
  an escalation timer, or cancel a snooze. Example systemd user unit in
  `packaging/systemd/`.
- **Export what you're looking at.** Press `e` (or run `sugarrush export`) to
  write two files for the clinical window: a CSV of every reading — oldest
  first, in mg/dL *and* your display unit — and a plain-text summary with sensor
  coverage, five-band time in range, time below range, mean, GMI, CV, and an
  hour-by-hour median/spread profile. Meant for sending to a clinician or
  opening in a spreadsheet, instead of screenshotting a terminal.
  `sugarrush export --days 30 --out ~/` for a different window or directory.

### Fixed

- **The declared minimum Rust version was wrong** — `rust-version` said 1.82,
  but the dependency tree hasn't built on anything below **1.89** for a while,
  so `cargo install` could fail with a confusing dependency error instead of a
  clear "your toolchain is too old". CI now builds against the declared MSRV
  every run, so it can't drift again.
- **Half the uploader requests are gone.** Each refresh fetched
  `/devicestatus` twice — once for battery/IOB/COB and once for the forecast.
  It's now one request, which also rules out the two halves coming from
  different records if the uploader posts in between.

## [2026.8.1] - 2026-08-08

This release is about trusting what's on screen. The alarm path got a
correctness pass — the newest reading is now genuinely the newest, alerts no
longer re-fire every time a value hovers on a threshold, a flat glucose no
longer predicts a low, and a bad token says so and stops retrying instead of
looking like a network outage. Saving settings can no longer lose your
Nightscout token, the site URL and token are now editable in the app (masked,
and no longer echoed by the setup wizard), and an unencrypted `http://` site is
called out rather than accepted silently. The stats panel turned clinical —
five-band time-in-range with time-below-range and CV variability, over a fixed
14-day window — and history is now fully navigable from the keyboard.

### Added

- **Keyboard navigation for the overview strip** — `H` / `L` (or `PgUp` /
  `PgDn`) pan a whole window at a time and `End` jumps to the oldest edge of the
  overview, so moving through history no longer needs a mouse.
- **Clinical time-in-range.** The stats panel now splits readings across the
  five consensus bands (very low / low / in range / high / very high) instead of
  three, calls out the **time below range** percentage — the number that changes
  treatment — and reports **CV**, the standard measure of glycaemic variability,
  highlighted when it exceeds the 36% consensus target.
- **Content-free notifications** — a new `Notification detail` setting
  (`notify_content`) keeps desktop notifications free of your reading and alert
  state, for lock screens and shared displays. They still fire, and urgent ones
  are still critical.
- **Unsaved settings are visible.** Settings apply live but only `w` writes them
  to `config.toml`; the settings header now says `· unsaved changes (w to save)`
  so quitting can't silently discard them.
- **Fix a bad site without leaving the app** — the settings screen gains a
  **Site** section with the Nightscout URL and read-only token. Press `Enter` on
  a row to edit it; the app reconnects immediately, and `w` saves as usual. The
  token is masked while typing and never rendered back (the row reads
  `set · ••••••`).
- **Unencrypted sites are called out.** A plain `http://` site sends your token
  and readings in the clear, so the dashboard footer now says so and the setup
  wizard asks you to confirm before accepting one. Loopback addresses
  (`localhost`, `127.0.0.1`) are exempt — they never leave the machine.
- **Push alerts are now a settings row** (`Alarm → Push alerts`) — toggle the
  urgent-alert webhook on or off in the app without deleting the configured
  `push_url`. The row shows only the host, never the full topic/path, and the
  new `push_enabled` config key persists the choice.

### Fixed

- **`Esc` no longer quits the app.** From the dashboard it returns to the live
  edge (and closes the help overlay or a prompt, as before) — quitting is `q`,
  which is what every other screen already did.
- **The current-value pane keeps its range label when compact.** On a short
  terminal the colour-independent label (`LOW`, `in range`) was pushed onto a
  line that got clipped, leaving colour as the only cue; it now sits on the
  headline next to the reading.
- **The overview strip says why it's empty** — "loading overview…" before the
  first fetch, "no readings in this window" after — instead of drawing a bare
  box.
- **The audible alarm falls back to the terminal bell** when no system audio
  player is available (headless boxes, minimal containers, a bare SSH login),
  instead of failing silently — silence is indistinguishable from "glucose is
  fine".
- **The setup wizard no longer echoes your token** to the terminal — it's masked
  as you type, so it doesn't survive in the scrollback.
- **URLs are normalized instead of failing mysteriously.** A bare
  `mysite.example.com`, a trailing slash, or a pasted
  `…/api/v1/entries.json?count=10` now resolves to the right base URL, in the
  wizard, in the settings editor, and when loading an existing `config.toml`.
- **A partly-failed refresh now says so.** When the readings arrive but a
  companion fetch doesn't, the affected panels (IOB/COB, treatment markers,
  forecast, sensor age, history, overview) were left showing old values with
  the connection still reading as healthy. The footer now names what's stale.
- **The connection recovers while you're browsing history.** Backoff retries
  were skipped unless you were at the live edge, so an outage that started
  while you were panning stayed until you refreshed by hand. The backoff also
  now actually reaches its documented 60s ceiling (it stopped at 40s).
- **No more "heading low in ~0 min" from a stale forecast.** Uploader forecasts
  are timestamped when the pump published them; an old one is entirely in the
  past, and its points were being reported as an imminent crossing. Past points
  are now skipped.
- **A delta too small to show no longer renders as `-0.0`** in the dashboard or
  the Waybar module — the sign follows the value as displayed, so a flat trend
  reads flat.
- **The predict-horizon setting is honest about its reach** — above 30 minutes
  it notes that the local (AR2) forecast only projects that far, so a longer
  horizon needs uploader predictions from Loop or OpenAPS.
- **Saving settings can no longer lose your token.** `config.toml` is now
  written to a temp file (created owner-only, flushed to disk) and renamed into
  place, instead of being truncated and rewritten — so a crash, a full disk, or
  a power cut mid-save leaves the old config intact rather than an empty file
  with the only copy of your Nightscout token gone.
- **The setup wizard no longer builds `config.toml` by string interpolation** —
  values are serialized by the TOML library, so a URL or token containing a
  quote or newline can't corrupt the file or inject extra settings into it. The
  file is also created with `0600` from the start, rather than being chmodded
  after the token was already written.
- **The audible alarm no longer leaks processes.** Each alarm spawns a system
  audio player; those were never reaped, so a long overnight urgent state piled
  up hundreds of zombie processes. Finished players are now cleaned up on every
  play.
- **Alert thresholds can't cross any more** — urgent-low ≤ low ≤ high ≤
  urgent-high is enforced while editing, so you can't silently disable a band
  by dragging one threshold past its neighbour.
- **The newest reading is now always the newest reading** — entries are sorted
  by timestamp on arrival instead of trusting the order Nightscout (or a proxy
  or mirror in front of it) happened to return, so the current value, delta,
  staleness check, and forecast can't silently key off an older reading.
- **Alerts no longer flap on a threshold.** A reading hovering on a boundary
  (69 → 71 → 69, well inside CGM noise) used to re-fire the notification, the
  audible alarm, and the push webhook every single time. Clearing an alert now
  requires moving 4 mg/dL past the threshold; raising one is unchanged, so a
  real low still alarms on the first reading that crosses.
- **A flat glucose no longer predicts a low.** The predictive alert and its ETA
  now follow the centre of the forecast rather than the edge of the uncertainty
  cone, which widens with the horizon and so crossed a threshold even when
  glucose was perfectly steady.
- **A bad token or URL now says so and stops retrying** instead of being
  retried forever as if it were a network outage. After three consecutive
  authentication (401/403) or not-found (404) responses, automatic fetching
  pauses with an explanatory message; press `r`, or switch/edit the site, to
  resume.
- **Stats are now clinical, not cosmetic** — time-in-range, mean, and GMI are
  computed over a fixed window of the last N days (the `AGP days` setting,
  default 14 — the clinical standard) instead of whatever slice of the graph
  happened to be on screen, so panning or zooming no longer changes them. The
  stats panel title names the window (e.g. `stats · 14d`).

## [2026.7.3] - 2026-07-18

This release hardens the safety-critical alarm path and reworks the graphs. The
audible alarm can no longer stall silently — the Nightscout client now times
out, a total sensor dropout raises a Stale alarm, sensor-error codes no longer
fire a false urgent-low, and a wrong token says so instead of reading as
"offline". Visually, both the AGP and the short-term forecast now render as
filled percentile/uncertainty bands, chart shading lines up with the axes, the
colourblind palette renders on every terminal (including tmux/SSH), and there's
a `?` keybinding overlay. It also adds a first-run units prompt and an in-app
"not a medical device" reminder.

### Added

- The **AGP view** now renders as a filled percentile fan (a shaded 5–95 and a
  brighter inter-quartile band) with a single bright median line, and its title
  shows the target range — much closer to a clinical AGP than the previous flat
  lines.
- A **keybinding help overlay** — press `?` for a full cheatsheet. The footer
  now falls back to a terse hint set on narrow terminals (it previously clipped
  silently, hiding settings/site/snooze) while always keeping `? help` visible.
- The dashboard footer now shows a **snooze indicator** with a countdown while
  the audible alarm is silenced, so it's clear the alarm is off and for how long.

### Changed

- **Settings now explain themselves.** On normal-width terminals the field list
  and a selected-field detail pane use a 55/45 split, while explicit “more”
  markers show when rows continue above or below the viewport; narrow terminals
  retain the focused single-pane list.
- **The AGP now labels what its reference lines mean.** The target range is
  shaded behind the percentile fan, and the low/high rails carry their names
  and values directly on the plot; the persistent legend and bold median remain
  visible above both.
- **The followers screen is now a real at-a-glance table.** It labels the
  display unit and columns, gives every person a coloured severity rail, and
  shows a one-hour sparkline alongside value, delta, state and reading age.

- The **forecast cone** on the timeline is now a filled low–high band (matching
  the AGP fan) with the centre line drawn on top, instead of two dim edge lines,
  and it emanates from the last reading (the AR2 fallback's initial jump no
  longer leaves the fan floating above the current dot).
- **In-app safety note** — the "not a medical device" reminder now appears in
  the running app (a dim note in the header, and up front in the first-run
  wizard), not only in the README and `about`.
- **Alert banner** now follows the theme/colourblind palette (it was hardcoded
  red/yellow), and the selected settings row is highlighted full-width.
- **Dashboard polish** — the range bar now shows all four zones (urgent-low /
  low / in-range / high / urgent-high) and uses integer mg/dL labels; IOB and
  COB stand out from the dimmed device line; and the graph labels its carb and
  bolus markers with a small legend.

### Fixed

- **Chart background tints now line up with the chart.** The in-range band, the
  AGP fan, and the forecast fill are painted onto the terminal buffer, and the
  plot-rect geometry didn't match ratatui's — it was two rows too tall and used
  the wrong left gutter, so the shading spilled past the axes and sat offset from
  the lines/points it shades (the forecast fan looked detached from the current
  reading). The geometry now replicates ratatui's chart layout exactly.
- **Graph theming** — the shaded in-range band is now derived from the in-range
  palette colour (it was hardcoded green, so it broke the colourblind preset),
  and the y-axis shows integer mg/dL values instead of spurious decimals.
- **Setup** — the first-run wizard now asks for the display unit (mmol/L or
  mg/dL) instead of always defaulting to mmol/L, so mg/dL users aren't dropped
  into the wrong unit.
- **Accessibility** — the colourblind-safe palette now uses named ANSI colours
  instead of truecolor hex, so it renders correctly on 16/256-colour terminals,
  tmux, and SSH sessions that lack truecolor (where it previously collapsed
  silently). The current reading is also exposed as plain text alongside the
  big-number glyphs, so screen readers, tmux copy, and braille displays can read
  it.
- **Clearer connection errors** — a wrong or non-readable token now reports
  "authentication failed — check your read-only token (not API_SECRET)" instead
  of a generic "offline", both at runtime (in the header) and during first-run
  setup; unreachable hosts and HTTP errors are also distinguished.
- **Alarm responsiveness** — a sensor gap now escalates to a Stale alarm within
  seconds (re-checked on the alarm tick) instead of waiting for the next full
  refresh, and a failed escalation push (dead `push_url`) is surfaced instead of
  swallowed silently.
- **Alarm reliability** — the audible alarm could stop working silently in
  several cases, now fixed: the Nightscout client had no request timeout (a
  stalled connection froze input and the alarm), a total sensor dropout read as
  "in range" instead of a sensor gap, Nightscout sensor-error codes (0–12) were
  read as a real reading and could fire a false urgent-low, and predictive
  alerts evaluated the previous refresh's forecast. Failed data fetches no
  longer pile up doomed follow-up requests.

## [2026.7.2] - 2026-07-17

This release is a dashboard glow-up. The graph now colour-codes readings by zone
with a shaded in-range band and dashed threshold rails, adds a zoned range bar
under the current value, and gains a switchable **AGP** (ambulatory glucose
profile) view alongside the 3h/24h timelines. The stats panel picks up a
time-in-range bar and a mean sparkline, and short-term forecasts now render as
an **uncertainty cone** — a high/low band — instead of a single line.

### Added

- **Graph view tabs** (`Tab` / `Shift+Tab`) — switch the graph pane between a
  3h or 24h timeline and an **AGP** (ambulatory glucose profile) that folds the
  last N days of readings onto a 24h clock as a percentile band (median +
  25/75 + 5/95). The number of days is configurable in settings (`AGP days`).
- **Dashboard graph glow-up** — readings are colour-coded by zone
  (low/in-range/high) with dashed reference rails at the low/high thresholds,
  the in-range region is shaded as a band behind the trace, and a zoned range
  bar under the big current value shows where it sits between the thresholds.
- **Stats upgrade** — time-in-range is drawn as a stacked zone bar, and the
  mean gets an inline sparkline of recent readings.

### Changed

- **Forecast is now an uncertainty cone** — predictions render as a widening
  high/low band (the plausible range) instead of a single line; the
  time-to-low/high ETA warns on the worst plausible path.

## [2026.7.1] - 2026-07-17

First public release. A fast, keyboard-driven terminal UI for viewing
self-hosted [Nightscout](https://nightscout.github.io/) CGM data.

### Added

- **Dashboard** — big current value with trend arrow, delta, and a colour +
  text range label; stats panel with time-in-range, mean glucose + GMI,
  insulin-/carbs-on-board, and device status (battery, sensor age, last seen).
- **History & forecast** — braille/dot graph you can pan (`h`/`l`), zoom
  (`+`/`-`, 1h–24h), and jump to a date (`g`); a 24h minimap you click or drag;
  a short-term forecast overlay (uploader predictions or a local AR2 fallback)
  with a "now" line, a time-to-low/high ETA, and predictive alerts; carb and
  bolus markers on the graph.
- **Alerts & safety** — in-TUI banner plus cross-platform desktop notifications
  (Linux/macOS/Windows); an audible alarm for urgent lows/highs with snooze,
  per-level tones, quiet hours, and unacknowledged-alarm escalation (optional
  phone push); clear offline vs. sensor-gap states with backoff retry.
- **Configuration** — an in-app settings screen (`s`) grouped into sections,
  editing units, refresh, thresholds, alarms, and theme live and saving back to
  `config.toml`; configurable colours with a colourblind-safe preset; multiple
  Nightscout sites (`n` to switch); a first-run setup wizard.
- **Elsewhere** — a Waybar module (`sugarrush waybar`) with a sparkline tooltip
  and click-through; `sugarrush --demo` to try the app on synthetic data with
  no config or network.
- **Distribution** — published to crates.io, the AUR (`sugarrush-bin`), and a
  Homebrew tap; prebuilt binaries + shell/PowerShell installers via cargo-dist.

[Unreleased]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.3...HEAD
[2026.7.3]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.2...v2026.7.3
[2026.7.2]: https://github.com/ronaldlokers/sugarrush/compare/v2026.7.1...v2026.7.2
[2026.7.1]: https://github.com/ronaldlokers/sugarrush/releases/tag/v2026.7.1
