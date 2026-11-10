//! Reconnect/backoff and heartbeat bookkeeping, kept free of any I/O so it
//! can be unit tested.

use std::time::Duration;

/// Exponential backoff with full jitter.
#[derive(Debug)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Backoff {
            base,
            max,
            attempt: 0,
        }
    }

    /// Deterministic delay for the current attempt (before jitter), then
    /// advances the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let exp = self.base.saturating_mul(1u32 << self.attempt.min(16));
        let delay = exp.min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Like [`Self::next_delay`] but with full jitter applied: a uniform
    /// random duration in `[0, delay]`.
    pub fn next_delay_jittered(&mut self) -> Duration {
        let delay = self.next_delay();
        delay.mul_f64(rand::random::<f64>())
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

/// What to do after a gateway connection ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    /// Reconnect and RESUME with the stored session_id/seq.
    Resume,
    /// Reconnect with a fresh IDENTIFY (session is dead).
    Reidentify,
    /// Do not reconnect (e.g. authentication failure).
    Fatal,
}

/// Maps a gateway close code to a reconnect policy, per the Discord docs.
pub fn policy_for_close_code(code: u16) -> ReconnectPolicy {
    match code {
        // Authentication failed, invalid shard/intents, etc.: no retry.
        4004 | 4010 | 4011 | 4012 | 4013 | 4014 => ReconnectPolicy::Fatal,
        // Invalid seq / session timed out: new session required.
        4007 | 4009 => ReconnectPolicy::Reidentify,
        // Everything else (including normal network drops): try to resume.
        _ => ReconnectPolicy::Resume,
    }
}

/// Heartbeat ACK tracking. Discord requires that a client that misses an ACK
/// between two heartbeats treats the connection as zombied and reconnects.
#[derive(Debug)]
pub struct HeartbeatTracker {
    interval: Duration,
    acked: bool,
}

impl HeartbeatTracker {
    pub fn new(interval_ms: u64) -> Self {
        HeartbeatTracker {
            interval: Duration::from_millis(interval_ms),
            acked: true,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Initial delay before the first heartbeat: `interval * jitter` where
    /// jitter is in `[0, 1)`, as required by the gateway docs.
    pub fn first_delay(&self, jitter: f64) -> Duration {
        self.interval.mul_f64(jitter.clamp(0.0, 0.999_999))
    }

    /// Call when sending a heartbeat. Returns `false` if the previous
    /// heartbeat was never ACKed (zombied connection; caller must reconnect).
    pub fn on_send(&mut self) -> bool {
        let ok = self.acked;
        self.acked = false;
        ok
    }

    /// Call when a heartbeat ACK (op 11) arrives.
    pub fn on_ack(&mut self) {
        self.acked = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        for _ in 0..10 {
            b.next_delay();
        }
        assert_eq!(b.next_delay(), Duration::from_secs(60));
    }

    #[test]
    fn backoff_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.attempt(), 0);
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn jittered_delay_is_bounded() {
        let mut b = Backoff::new(Duration::from_secs(4), Duration::from_secs(60));
        for _ in 0..50 {
            b.reset();
            let d = b.next_delay_jittered();
            assert!(d <= Duration::from_secs(4));
        }
    }

    #[test]
    fn close_code_policies() {
        assert_eq!(policy_for_close_code(4004), ReconnectPolicy::Fatal);
        assert_eq!(policy_for_close_code(4013), ReconnectPolicy::Fatal);
        assert_eq!(policy_for_close_code(4007), ReconnectPolicy::Reidentify);
        assert_eq!(policy_for_close_code(4009), ReconnectPolicy::Reidentify);
        assert_eq!(policy_for_close_code(4000), ReconnectPolicy::Resume);
        assert_eq!(policy_for_close_code(1006), ReconnectPolicy::Resume);
    }

    #[test]
    fn heartbeat_ack_tracking() {
        let mut hb = HeartbeatTracker::new(41_250);
        assert_eq!(hb.interval(), Duration::from_millis(41_250));
        // First send is fine (initial state counts as ACKed).
        assert!(hb.on_send());
        // No ACK arrived: next send reports a zombied connection.
        assert!(!hb.on_send());
        hb.on_ack();
        assert!(hb.on_send());
    }

    #[test]
    fn heartbeat_first_delay_in_range() {
        let hb = HeartbeatTracker::new(10_000);
        assert_eq!(hb.first_delay(0.0), Duration::ZERO);
        assert!(hb.first_delay(0.5) == Duration::from_millis(5_000));
        assert!(hb.first_delay(2.0) < Duration::from_millis(10_000));
    }
}
