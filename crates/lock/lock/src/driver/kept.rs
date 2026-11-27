//! [`Kept`], a lock the driver keeps, and when it is due.

use tokio::time::Instant;
use uima_cancel::CancelToken;

use crate::{Lease, Ttl};

/// A lock the driver keeps.
///
/// Dropping it cancels `lost`, so a lock is lost once the driver stops keeping it, however that
/// happens.
#[derive(Debug)]
pub(crate) struct Kept {
    pub(super) lease: Lease,
    lost: CancelToken,
    pub(super) next_extend: Instant,
    /// When the last request that held it was sent.
    held_at: Instant,
    /// When the store last answered for it, which may have extended it even when it failed.
    answered_at: Instant,
    /// Until when the extend of it out is waited for, if one is.
    extending: Option<Instant>,
}

impl Kept {
    /// How many times an extend can be waited out per `ttl`, so one retried after it still is
    /// before the deadline.
    pub(super) const WAITS_PER_TTL: u32 = 6;
    /// How many times a lease is extended per `ttl`.
    const EXTENDS_PER_TTL: u32 = 3;
    /// How early, per `ttl`, a lease is extended along with another that is due.
    const EARLY_PER_TTL: u32 = 6;
    /// How many times a lease is retried per `ttl` after a failed extend.
    const RETRIES_PER_TTL: u32 = 10;

    /// When a lease held by a request sent at `sent` counts as lost unless extended again, a
    /// little before the store may expire it.
    pub(crate) fn deadline_after(sent: Instant, ttl: Ttl) -> Instant {
        sent + ttl.get() * 4 / 5
    }

    pub(super) fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        let mut kept = Self {
            lease,
            lost,
            next_extend: taken_at,
            held_at: taken_at,
            answered_at: Instant::now(),
            extending: None,
        };
        kept.extended(taken_at);
        kept
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    pub(super) fn extended(&mut self, sent: Instant) {
        self.next_extend = sent + self.lease.ttl.get() / Self::EXTENDS_PER_TTL;
        self.held_at = sent;
    }

    /// When it counts as lost unless extended, a little before the store may expire it.
    pub(super) fn deadline(&self) -> Instant {
        Self::deadline_after(self.held_at, self.lease.ttl)
    }

    /// When the store may expire it, by this instance's clock: a ttl after it last answered for
    /// it, or after the extend out is given up on, as that may extend it too.
    pub(super) fn expires(&self) -> Instant {
        self.extending.unwrap_or(self.answered_at) + self.lease.ttl.get()
    }

    /// Whether it is to be extended at `now`: it is due, or due soon enough to go along with
    /// another, and no extend of it is out.
    pub(super) fn is_due_soon(&self, now: Instant) -> bool {
        let early = self.lease.ttl.get() / Self::EARLY_PER_TTL;
        !self.is_extending() && self.next_extend <= now + early
    }

    /// Whether an extend of it is out.
    pub(super) fn is_extending(&self) -> bool {
        self.extending.is_some()
    }

    /// Notes an extend of it is out, waited for until `until`.
    pub(super) fn extending(&mut self, until: Instant) {
        self.extending = Some(until);
    }

    /// Notes the store answered the extend out, or it was given up on.
    pub(super) fn answered(&mut self) {
        self.extending = None;
        self.answered_at = Instant::now();
    }

    /// Schedules the next extend soon, after one failed.
    pub(super) fn retry_soon(&mut self) {
        self.next_extend = Instant::now() + self.lease.ttl.get() / Self::RETRIES_PER_TTL;
    }

    /// When it is next due, to be extended or given up on. While it is being extended, only
    /// the latter.
    pub(super) fn due(&self) -> Instant {
        if self.is_extending() {
            self.deadline()
        } else {
            self.next_extend.min(self.deadline())
        }
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.lost.cancel();
    }
}
