//! Wall-clock time for the pieces that stamp their own answers.
//!
//! The Daemon Core never reads a clock: the host reads the time once per
//! [`crate::daemon::Frame`] and the kernel schedules against that. The cache
//! is the one consumer that needs "now" on its own, to decide whether a stored
//! AniList answer is still fresh.

use std::time::{SystemTime, UNIX_EPOCH};

/// A point in time, Unix epoch milliseconds.
pub type Millis = i64;

/// Source of "now". Swapped for a virtual clock in tests.
pub trait Clock {
    fn now_millis(&self) -> Millis;
}

/// Wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> Millis {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as Millis)
            .unwrap_or(0)
    }
}
