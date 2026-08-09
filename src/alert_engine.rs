//! Alert episode state and transition bookkeeping.
//!
//! Classification still belongs to `App`, because it reads the current live
//! entry and forecast. This engine owns the state that spans passes: episode
//! identity, notification/push debouncing, escalation timing, and snoozing.

use crate::alert::Alert;

#[derive(Debug, Default)]
pub(super) struct AlertEngine {
    last_reported: Option<Alert>,
    last_notified: Option<Alert>,
    snooze_until: Option<i64>,
    urgent_since: Option<i64>,
    episode_kind: Option<Alert>,
    pushed_episode: bool,
    escalated: bool,
    predicted_notified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PushDue {
    Onset,
    Escalation,
}

impl AlertEngine {
    /// An episode belongs to one urgent state, not to "urgent" in general.
    /// A different state is a different emergency and re-arms every channel.
    pub(super) fn transition_to(&mut self, state: Alert) {
        let kind = state.is_urgent().then_some(state);
        if kind != self.episode_kind {
            self.snooze_until = None;
            self.urgent_since = None;
            self.pushed_episode = false;
            self.escalated = false;
            self.episode_kind = kind;
        }
    }

    pub(super) fn update_urgent(&mut self, state: Alert, now_ms: i64) {
        if state.is_urgent() {
            if self.urgent_since.is_none() {
                self.urgent_since = Some(now_ms);
                self.pushed_episode = false;
                self.escalated = false;
            }
        } else {
            self.urgent_since = None;
        }
    }

    pub(super) fn take_notification(&mut self, state: Alert) -> Option<Alert> {
        if self.last_notified == Some(state) {
            return None;
        }
        self.last_notified = Some(state);
        state.is_alerting().then_some(state)
    }

    pub(super) fn take_push(
        &mut self,
        state: Alert,
        now_ms: i64,
        escalate_minutes: i64,
    ) -> Option<PushDue> {
        if !state.is_urgent() {
            return None;
        }
        if !self.pushed_episode {
            self.pushed_episode = true;
            return Some(PushDue::Onset);
        }
        if escalate_minutes > 0 && !self.escalated {
            if let Some(since) = self.urgent_since {
                if now_ms - since >= escalate_minutes * 60_000 {
                    self.escalated = true;
                    return Some(PushDue::Escalation);
                }
            }
        }
        None
    }

    pub(super) fn record_state(&mut self, state: Alert) -> bool {
        let recovered = self.last_reported.is_some_and(Alert::is_alerting) && !state.is_alerting();
        self.last_reported = Some(state);
        recovered
    }

    pub(super) fn predictive_was_notified(&self) -> bool {
        self.predicted_notified
    }

    pub(super) fn set_predictive_notified(&mut self, notified: bool) {
        self.predicted_notified = notified;
    }

    pub(super) fn snooze(&mut self, until: i64) {
        self.snooze_until = Some(until);
    }

    pub(super) fn snooze_until(&self) -> Option<i64> {
        self.snooze_until
    }

    pub(super) fn last_notified(&self) -> Option<Alert> {
        self.last_notified
    }

    pub(super) fn urgent_since(&self) -> Option<i64> {
        self.urgent_since
    }

    pub(super) fn episode_kind(&self) -> Option<Alert> {
        self.episode_kind
    }

    pub(super) fn pushed_episode(&self) -> bool {
        self.pushed_episode
    }

    pub(super) fn escalated(&self) -> bool {
        self.escalated
    }

    pub(super) fn set_snooze(&mut self, until: Option<i64>) {
        self.snooze_until = until;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn restore(
        &mut self,
        last_notified: Option<Alert>,
        urgent_since: Option<i64>,
        pushed_episode: bool,
        escalated: bool,
        snooze_until: Option<i64>,
        episode_kind: Option<Alert>,
    ) {
        // What we last announced is also the last state reported. Keeping both
        // aligned means a restart can still report the eventual recovery
        // without re-announcing the ongoing episode first.
        self.last_notified = last_notified;
        self.last_reported = last_notified;
        self.urgent_since = urgent_since;
        self.pushed_episode = pushed_episode;
        self.escalated = escalated;
        self.snooze_until = snooze_until;
        self.episode_kind = episode_kind;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_urgent_variant_rearms_the_episode() {
        let mut engine = AlertEngine::default();
        engine.transition_to(Alert::Stale);
        engine.update_urgent(Alert::Stale, 1_000);
        engine.snooze(10_000);
        assert_eq!(
            engine.take_push(Alert::Stale, 1_000, 10),
            Some(PushDue::Onset)
        );

        engine.transition_to(Alert::UrgentLow);
        assert_eq!(engine.episode_kind(), Some(Alert::UrgentLow));
        assert_eq!(engine.snooze_until(), None);
        assert_eq!(engine.urgent_since(), None);
        assert!(!engine.pushed_episode());
        assert!(!engine.escalated());
    }

    #[test]
    fn notification_and_recovery_are_consumed_once() {
        let mut engine = AlertEngine::default();
        assert_eq!(engine.take_notification(Alert::Low), Some(Alert::Low));
        assert_eq!(engine.take_notification(Alert::Low), None);
        assert!(!engine.record_state(Alert::Low));
        assert!(engine.record_state(Alert::InRange));
        assert!(!engine.record_state(Alert::InRange));
    }

    #[test]
    fn escalation_fires_once_when_the_episode_earns_it() {
        let mut engine = AlertEngine::default();
        engine.transition_to(Alert::UrgentHigh);
        engine.update_urgent(Alert::UrgentHigh, 1_000);
        assert_eq!(
            engine.take_push(Alert::UrgentHigh, 1_000, 10),
            Some(PushDue::Onset)
        );
        assert_eq!(engine.take_push(Alert::UrgentHigh, 600_999, 10), None);
        assert_eq!(
            engine.take_push(Alert::UrgentHigh, 601_000, 10),
            Some(PushDue::Escalation)
        );
        assert_eq!(engine.take_push(Alert::UrgentHigh, 1_000_000, 10), None);
    }
}
