//! Monotonic retry policy. Clock and jitter inputs are supplied by the caller.

use std::time::{Duration, Instant};

pub(super) const EXCHANGE_INTERVAL: Duration = Duration::from_secs(15);
pub(super) const INCOMPLETE_INTERVAL: Duration = Duration::from_secs(2);
pub(super) const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

/// Retry state is independent of network tasks, vaults and user-facing status.
pub(super) struct Schedule {
    deadline: Instant,
    failures: u32,
}

impl Schedule {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            deadline: now,
            failures: 0,
        }
    }

    pub(super) fn is_due(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    pub(super) fn retry_now(&mut self, now: Instant) {
        self.deadline = now;
    }

    pub(super) fn defer(&mut self, now: Instant, delay: Duration) {
        self.deadline = now + delay;
    }

    pub(super) fn succeeded(&mut self, now: Instant) {
        self.failures = 0;
        self.defer(now, EXCHANGE_INTERVAL);
    }

    pub(super) fn failed(&mut self, now: Instant, jitter: u8) {
        self.failures = self.failures.saturating_add(1);
        let delay = (2u64.saturating_pow(self.failures.min(8)) + u64::from(jitter % 5)).min(300);
        self.defer(now, Duration::from_secs(delay));
    }
}

#[cfg(test)]
mod tests {
    use crate::sync::scheduler::schedule::*;

    #[test]
    fn failures_back_off_and_success_resets_the_next_retry() {
        let now = Instant::now();
        let mut schedule = Schedule::new(now);
        assert!(schedule.is_due(now));

        for expected in [2, 4, 8, 16, 32, 64, 128, 256, 256] {
            schedule.failed(now, 0);
            assert!(!schedule.is_due(now + Duration::from_secs(expected - 1)));
            assert!(schedule.is_due(now + Duration::from_secs(expected)));
        }
        schedule.succeeded(now);
        assert!(!schedule.is_due(now + EXCHANGE_INTERVAL - Duration::from_millis(1)));
        assert!(schedule.is_due(now + EXCHANGE_INTERVAL));
        schedule.failed(now, 4);
        assert!(schedule.is_due(now + Duration::from_secs(6)));
        assert!(!schedule.is_due(now + Duration::from_secs(5)));
        schedule.retry_now(now);
        assert!(schedule.is_due(now));
    }
}
