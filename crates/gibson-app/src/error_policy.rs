//! Consecutive-error policy shared by the raw-window hosts.
//!
//! When the window a screensaver host renders into disappears out from under
//! it (destroyed by the window manager / screensaver daemon before an event
//! could be queued, or the owner crashed), every frame fails. Hosts must stop
//! themselves instead of spinning at 60 Hz and logging errors indefinitely.
//! This tiny policy counts consecutive failures and reports when the caller
//! should give up; any successful frame resets the count so a transient error
//! (surface busy, driver hiccup) never accumulates across minutes.
//!
//! Only the Linux host instantiates it today; the module stays compiled on
//! every platform so its pure-logic tests keep running (macOS / Windows just
//! do not construct the type).
#![allow(dead_code)]

/// A simple consecutive-failure tripwire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsecutiveErrorPolicy {
    /// Number of consecutive failures that triggers giving up.
    limit: u32,
    /// Consecutive failures seen so far.
    consecutive: u32,
}

impl ConsecutiveErrorPolicy {
    /// `limit` consecutive errors trip the policy (at least 1).
    pub const fn new(limit: u32) -> Self {
        let limit = if limit == 0 { 1 } else { limit };
        Self {
            limit,
            consecutive: 0,
        }
    }

    /// Called after a frame that presented successfully: transient errors must
    /// not accumulate.
    pub fn record_success(&mut self) {
        self.consecutive = 0;
    }

    /// Called after a failed frame. Returns `true` once `limit` consecutive
    /// failures have been seen — the caller should stop.
    pub fn record_error(&mut self) -> bool {
        self.consecutive += 1;
        self.consecutive >= self.limit
    }

    /// Current consecutive-failure count (for logging the trip reason).
    pub fn consecutive(&self) -> u32 {
        self.consecutive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_limit_consecutive_errors() {
        let mut policy = ConsecutiveErrorPolicy::new(3);
        assert!(!policy.record_error());
        assert!(!policy.record_error());
        assert!(policy.record_error(), "third consecutive error trips");
        assert_eq!(policy.consecutive(), 3);
    }

    #[test]
    fn success_resets_the_counter() {
        let mut policy = ConsecutiveErrorPolicy::new(3);
        let _ = policy.record_error();
        let _ = policy.record_error();
        policy.record_success();
        assert!(!policy.record_error());
        assert!(!policy.record_error());
        assert!(policy.record_error(), "count restarted after the success");
        assert_eq!(policy.consecutive(), 3);
    }

    #[test]
    fn transient_errors_never_accumulate_across_successes() {
        let mut policy = ConsecutiveErrorPolicy::new(10);
        for _ in 0..50 {
            let _ = policy.record_error();
            policy.record_success();
        }
        assert_eq!(policy.consecutive(), 0);
        assert!(!policy.record_error());
    }

    #[test]
    fn zero_limit_is_treated_as_one() {
        let mut policy = ConsecutiveErrorPolicy::new(0);
        assert!(policy.record_error());
    }
}
