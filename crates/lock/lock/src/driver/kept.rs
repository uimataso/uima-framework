//! [`Kept`], a lock the driver keeps, and when it is due.

use std::time::Duration;

use tokio::time::Instant;
use uima_cancel::CancelToken;

use crate::Lease;

/// A lock the driver keeps.
///
/// Dropping it cancels `lost`, so a lock is lost once the driver stops keeping it, however that
/// happens.
pub(crate) struct Kept {
    pub(super) lease: Lease,
    lost: CancelToken,
    pub(super) next_extend: Instant,
    /// When it counts as lost unless extended, a little before the store may expire it, from
    /// when the last request that held it was sent.
    pub(super) deadline: Instant,
    /// Whether an extend of it is out.
    pub(super) extending: bool,
}

impl Kept {
    /// How many times a lease is extended per `ttl`.
    const EXTENDS_PER_TTL: u32 = 3;
    /// How many times a lease is retried per `ttl` after a failed extend.
    const RETRIES_PER_TTL: u32 = 10;
    /// How many times an extend can be waited out per `ttl`, so one retried after it still is
    /// before the deadline.
    pub(super) const WAITS_PER_TTL: u32 = 6;

    /// When a lease held by a request sent at `sent` counts as lost unless extended again, a
    /// little before the store may expire it.
    pub(crate) fn deadline_after(sent: Instant, ttl: Duration) -> Instant {
        sent + ttl * 4 / 5
    }

    pub(super) fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        let mut kept = Self {
            lease,
            lost,
            next_extend: taken_at,
            deadline: taken_at,
            extending: false,
        };
        kept.extended(taken_at);
        kept
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    pub(super) fn extended(&mut self, sent: Instant) {
        let ttl = self.lease.ttl.get();
        self.next_extend = sent + ttl / Self::EXTENDS_PER_TTL;
        self.deadline = Self::deadline_after(sent, ttl);
    }

    /// Schedules the next extend soon, after one failed.
    pub(super) fn retry_soon(&mut self) {
        self.next_extend = Instant::now() + self.lease.ttl.get() / Self::RETRIES_PER_TTL;
    }

    /// When it is next due: to be extended, or given up on. While it is being extended, only
    /// the latter.
    pub(super) fn due(&self) -> Instant {
        if self.extending {
            self.deadline
        } else {
            self.next_extend.min(self.deadline)
        }
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.lost.cancel();
    }
}
