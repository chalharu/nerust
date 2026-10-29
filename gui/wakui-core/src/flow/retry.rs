//! Clock-injected bounded recovery, with one notification per outage.
//!
//! Toolkit-free render backoff shared by every native backend: failures
//! back off exponentially (100ms doubling, capped at 2s) and rearm on the
//! first success. Time is injected (`Instant` in, no clock read) so the
//! policy stays deterministic under test.

use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct RenderRetry {
    failures: u32,
    deadline: Option<Instant>,
}

impl RenderRetry {
    pub fn ready(&self, now: Instant) -> bool {
        self.deadline.is_none_or(|deadline| now >= deadline)
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// True only for the first failure since the last successful frame.
    pub fn failed(&mut self, now: Instant) -> bool {
        let notify = self.failures == 0;
        self.failures = self.failures.saturating_add(1);
        let millis = (100u64 << (self.failures - 1).min(5)).min(2000);
        self.deadline = Some(now + Duration::from_millis(millis));
        notify
    }

    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            true
        } else {
            false
        }
    }

    pub fn succeeded(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_are_bounded_and_rearm_after_recovery() {
        let mut retry = RenderRetry::default();
        let mut now = Instant::now();
        for index in 0..20 {
            assert_eq!(retry.failed(now), index == 0);
            let deadline = retry.deadline().unwrap();
            assert!(!retry.ready(now));
            assert!(!retry.take_due(now));
            assert!(deadline.duration_since(now) <= Duration::from_secs(2));
            now = deadline;
            assert!(retry.ready(now));
            assert!(retry.take_due(now));
            assert!(!retry.take_due(now));
        }
        retry.succeeded();
        assert!(retry.ready(now));
        assert!(retry.deadline().is_none());
        assert!(retry.failed(now));
    }
}
