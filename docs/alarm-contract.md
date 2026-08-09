# The alarm contract

sugarrush is not a medical device, but people leave it running overnight and
trust it to wake them. That trust is the whole product, and everything in this
file exists because a change that looks harmless can quietly break it.

This is the contract the alarm machine keeps. If you change any of it, change
this file in the same commit.

## One pass, one decision

`App::react(now_ms) -> Reaction` is the **only** way the alarm machine
advances. It classifies, updates the episode timers, and returns everything
that should be announced:

| Field | Meaning |
|---|---|
| `state` | the alert state after this pass |
| `notification` | a desktop notification is due |
| `predictive` | a "heading low" warning is due |
| `push` | a webhook POST is due, as `(url, message)` |
| `recovered` | an alerting episode ended on this pass |
| `sound` | the audible alarm should be sounding right now |

Front ends **deliver**; they do not decide. `main::deliver` does it for the
dashboard, `watch::react` for the daemon.

Three rules follow from this, and each of them was once broken:

1. **Consumption is unconditional.** A front end that cannot deliver something
   drops it. It must never skip the `take_*` call, because that leaves the
   machine's idea of "already announced" out of step with reality — and the two
   front ends then disagree.
2. **Whatever sounds also announces.** The dashboard's 3-second ticker used to
   classify and sound without consuming a notification or a push, so a sensor
   gap crossing into `Stale` between refreshes beeped immediately while the
   desktop notification and the escalation webhook waited out the refresh
   interval. Both front ends now call `react` on their fast tick, not a
   hand-rolled subset of it.
3. **`update_urgent` runs before the announcements.** `take_push` reads the
   episode timers it maintains, so escalation must be able to fire on the pass
   that earns it.

## States

Five alert states plus `Stale`, in `src/alert.rs`:

```
UrgentLow  ≤ urgent_low
Low        < low
InRange
High       > high
UrgentHigh ≥ urgent_high
Stale      no reading for longer than stale_minutes
```

Two properties that are easy to break:

- **Staleness beats value.** A reading older than `stale_minutes` cannot be
  trusted as the current level, so `Stale` wins regardless of what the number
  says. A stale 5.6 is not "in range"; it is "we don't know".
- **Hysteresis applies on leaving only.** 4 mg/dL, so a reading hovering on a
  threshold doesn't chatter between two states — but entering an alerting state
  is never delayed. The alarm is quick to fire and slow to clear, never the
  other way round.

## Episodes

An *episode* is one continuous run in a single urgent state. It carries:

- `urgent_since` — when it started, which drives escalation
- `episode_kind` — **which** urgent state. A change of variant (urgent low →
  urgent high) is a new episode: it re-arms snooze, escalation and push.
  Treating it as the same episode meant a low that turned into a high stayed
  snoozed.
- `pushed_episode` / `escalated` — fired-once latches
- `snooze_until` — a deliberate silence, honoured across restarts

The daemon persists all of this to `$XDG_STATE_HOME/sugarrush/watch.json` and
restores it on start (`App::restore_episode`), so a restarted service does not
re-announce an ongoing low, restart an escalation timer, or cancel a snooze
someone set on purpose.

## The handover

The dashboard and the daemon must never alarm in chorus, and must never both
stay quiet.

Both write a heartbeat to `$XDG_RUNTIME_DIR/sugarrush/{tui,watch}.alive`; a
heartbeat is live for 30 seconds. While the dashboard's is live, the daemon
still classifies — so episode state stays correct and a later handover is
seamless — but drops its announcements rather than holding them, or they would
all arrive at once the moment the dashboard closed.

One subtlety worth keeping: **the dashboard only claims the alarm when it
covers everything the daemon would.** The TUI alerts on the active site alone,
so with several sites configured it must not silence a watcher handling all of
them. That hole left a caregiver's other sites unalarmed while the dashboard
was open.

## What can silence the alarm

Every one of these is a way for a night to pass without a sound. They are the
list to check when someone reports "it didn't wake me", and the reason the
alarm self-test exists:

1. `alerts.sound = false`
2. Quiet hours, unless `quiet_urgent_low` and the state is an urgent low
3. An active snooze
4. No working audio player (`src/sound.rs` falls back to the terminal bell)
5. The daemon isn't running, or its unit never started
6. The dashboard is open and has claimed the alarm, but is on another site
7. Readings are arriving, so nothing is `Stale`, but they are wrong
8. `escalate_minutes` is set with no `push_url`, so escalation has no channel

## Testing

The reaction sequence is covered in `src/app.rs`:

- `a_gap_announces_on_the_same_pass_that_sounds_it`
- `a_notification_is_consumed_even_when_it_cannot_be_delivered`
- `escalation_fires_on_the_pass_that_earns_it`
- `recovery_is_reported_once_and_only_after_an_alarm`

A change to the alarm path that does not break one of these has probably not
been tested. Add to them.
