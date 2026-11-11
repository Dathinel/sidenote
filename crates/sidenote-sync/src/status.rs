use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    LocalStore,
    CorruptDocument,
    StoreFull,
    DocumentTooLarge,
    DocumentQuota,
    LockPoisoned,
    WorkerUnavailable,
    WorkerTimedOut,
    WorkerCrashed,
    PeerBadData,
    Protocol,
    Transport,
    Membership,
    ConcurrentEdit,
    Incompatible,
}

#[derive(Debug)]
pub struct Failure(pub Reason);
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sync failure: {:?}", self.0)
    }
}
impl std::error::Error for Failure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Offline,
    Connecting,
    Synced,
    Failed(Reason),
}

#[derive(Clone, Debug)]
pub struct PeerStatus {
    pub device: [u8; 32],
    pub sending_bad_data: bool,
    pub reason: Option<Reason>,
    pub last_synced: Option<SystemTime>,
}

/// Full jitter in [0, ceiling], with reset only after a healthy minute.
pub struct Backoff {
    ceiling: Duration,
}
impl Default for Backoff {
    fn default() -> Self {
        Self {
            ceiling: Duration::from_millis(100),
        }
    }
}
impl Backoff {
    #[cfg(test)]
    pub(crate) fn ceiling(&self) -> Duration {
        self.ceiling
    }
    pub fn next_delay(&mut self, healthy: Duration) -> Duration {
        use rand::Rng;
        if healthy >= Duration::from_secs(60) {
            self.ceiling = Duration::from_millis(100);
        }
        let ms = rand::thread_rng().gen_range(0..=self.ceiling.as_millis() as u64);
        self.ceiling = (self.ceiling * 2).min(Duration::from_secs(300));
        Duration::from_millis(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_sessions_do_not_reset_backoff_and_full_jitter_is_capped() {
        let mut b = Backoff::default();
        for _ in 0..30 {
            assert!(b.next_delay(Duration::from_secs(59)) <= Duration::from_secs(300));
        }
        assert_eq!(b.ceiling, Duration::from_secs(300));
        assert!(b.next_delay(Duration::from_secs(60)) <= Duration::from_millis(100));
        assert_eq!(b.ceiling, Duration::from_millis(200));
    }
}
