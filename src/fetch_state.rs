//! Connection health, retry backoff, and fetch-error state.

/// Consecutive config-level failures (bad token / URL) before automatic
/// fetching pauses. More than one, so a server that briefly answers 401 during
/// a restart doesn't strand a working setup.
pub(super) const CONFIG_FAIL_LIMIT: u32 = 3;

#[derive(Debug)]
pub(super) struct FetchState {
    online: bool,
    last_ok_ms: Option<i64>,
    fetch_fails: u32,
    config_fails: u32,
    fetch_paused: bool,
    next_retry_at: Option<i64>,
    last_error: Option<String>,
    partial: Option<String>,
}

impl Default for FetchState {
    fn default() -> Self {
        Self {
            online: true,
            last_ok_ms: None,
            fetch_fails: 0,
            config_fails: 0,
            fetch_paused: false,
            next_retry_at: None,
            last_error: None,
            partial: None,
        }
    }
}

impl FetchState {
    pub(super) fn set_partial(&mut self, missing: &[&str]) {
        self.partial = (!missing.is_empty()).then(|| missing.join(", "));
    }

    pub(super) fn mark_online(&mut self, now_ms: i64) {
        self.online = true;
        self.last_ok_ms = Some(now_ms);
        self.fetch_fails = 0;
        self.config_fails = 0;
        self.fetch_paused = false;
        self.next_retry_at = None;
        self.last_error = None;
    }

    /// Record a failure and schedule 5s → 10s → 20s → 40s → 60s backoff.
    pub(super) fn mark_offline(&mut self, now_ms: i64, err: String, permanent: bool) {
        self.online = false;
        self.partial = None;
        self.fetch_fails = self.fetch_fails.saturating_add(1);
        if permanent {
            self.config_fails = self.config_fails.saturating_add(1);
        } else {
            self.config_fails = 0;
        }
        if self.config_fails >= CONFIG_FAIL_LIMIT {
            self.fetch_paused = true;
            self.next_retry_at = None;
            self.last_error = Some(format!("{err} · retries paused, press r to retry"));
            return;
        }
        let secs = (5u64 << (self.fetch_fails.min(5) - 1)).min(60);
        self.next_retry_at = Some(now_ms + secs as i64 * 1000);
        self.last_error = Some(err);
    }

    pub(super) fn resume(&mut self) {
        self.fetch_paused = false;
        self.config_fails = 0;
        self.fetch_fails = 0;
        self.next_retry_at = None;
    }

    pub(super) fn should_retry(&self, now_ms: i64) -> bool {
        !self.online && !self.fetch_paused && self.next_retry_at.is_some_and(|t| now_ms >= t)
    }

    pub(super) fn online(&self) -> bool {
        self.online
    }

    pub(super) fn last_ok_ms(&self) -> Option<i64> {
        self.last_ok_ms
    }

    pub(super) fn fetch_paused(&self) -> bool {
        self.fetch_paused
    }

    pub(super) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(super) fn set_last_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    pub(super) fn partial(&self) -> Option<&str> {
        self.partial.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_reaches_sixty_seconds_and_stays_there() {
        let mut state = FetchState::default();
        for secs in [5, 10, 20, 40, 60, 60] {
            state.mark_offline(1_000, "offline".into(), false);
            assert!(!state.should_retry(1_000 + secs * 1_000 - 1));
            assert!(state.should_retry(1_000 + secs * 1_000));
        }
    }

    #[test]
    fn repeated_config_failures_pause_until_resumed() {
        let mut state = FetchState::default();
        for _ in 0..CONFIG_FAIL_LIMIT {
            state.mark_offline(1_000, "bad token".into(), true);
        }
        assert!(state.fetch_paused());
        assert!(!state.should_retry(i64::MAX));
        state.resume();
        assert!(!state.fetch_paused());
    }

    #[test]
    fn success_clears_failure_state() {
        let mut state = FetchState::default();
        state.mark_offline(1_000, "offline".into(), false);
        state.set_partial(&["device"]);
        state.mark_online(2_000);
        assert!(state.online());
        assert_eq!(state.last_ok_ms(), Some(2_000));
        assert_eq!(state.last_error(), None);
        assert!(!state.should_retry(i64::MAX));
    }
}
