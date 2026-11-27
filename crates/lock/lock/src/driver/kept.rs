//! [`Kept`], a lock the driver keeps, and when it is due.

use tokio::time::Instant;
use uima_cancel::CancelToken;

use crate::{Lease, Ttl};

/// A lock the driver keeps.
///
/// Its times, as parts of its `ttl` after the last request that held it was sent:
///
/// - at a third, it is extended, along with any other lease then due within a sixth of its own;
/// - that extend is waited for a sixth at most, so it is answered or given up on by half;
/// - one that failed is retried a tenth after, and so is given up on by about 0.77;
/// - at four fifths, the deadline, it counts as lost, leaving its holder a fifth to stop before
///   the store may expire it.
///
/// Dropping it cancels `lost`, so a lock is lost once the driver stops keeping it, however that
/// happens.
#[derive(Debug)]
pub(crate) struct Kept {
    pub(super) lease: Lease,
    lost: CancelToken,
    pub(super) next_extend: Instant,
    /// When it counts as lost unless extended again.
    deadline: Instant,
    /// When the store may expire it, by this instance's clock.
    expires: Instant,
    /// Whether an extend of it is out.
    extending: bool,
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
    /// How many fifths of its `ttl` a lease counts as held after the request that held it.
    const DEADLINE_FIFTHS: u32 = 4;

    /// When a lease held by a request sent at `sent` counts as lost unless extended again, a
    /// little before the store may expire it.
    pub(crate) fn deadline_after(sent: Instant, ttl: Ttl) -> Instant {
        sent + ttl.get() * Self::DEADLINE_FIFTHS / 5
    }

    /// A lock taken by a request sent at `taken_at`, which the store has just answered.
    pub(super) fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        Self {
            next_extend: taken_at + lease.ttl.get() / Self::EXTENDS_PER_TTL,
            deadline: Self::deadline_after(taken_at, lease.ttl),
            expires: Instant::now() + lease.ttl.get(),
            extending: false,
            lease,
            lost,
        }
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    pub(super) fn extended(&mut self, sent: Instant) {
        self.next_extend = sent + self.lease.ttl.get() / Self::EXTENDS_PER_TTL;
        self.deadline = Self::deadline_after(sent, self.lease.ttl);
    }

    /// When it counts as lost unless extended, a little before the store may expire it.
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// When the store may expire it, by this instance's clock: a ttl after it last answered for
    /// it, or after the extend out is given up on, as that may extend it too.
    pub(super) fn expires(&self) -> Instant {
        self.expires
    }

    /// Whether it is to be extended at `now`: it is due, or due soon enough to go along with
    /// another, and no extend of it is out.
    pub(super) fn is_due_soon(&self, now: Instant) -> bool {
        let early = self.lease.ttl.get() / Self::EARLY_PER_TTL;
        !self.is_extending() && self.next_extend <= now + early
    }

    /// Whether an extend of it is out.
    pub(super) fn is_extending(&self) -> bool {
        self.extending
    }

    /// Notes an extend of it is out, waited for until `until`, so the store may keep it until a
    /// ttl after.
    pub(super) fn extending(&mut self, until: Instant) {
        self.extending = true;
        self.expires = until + self.lease.ttl.get();
    }

    /// Notes the store answered the extend out, or it was given up on, and may keep the lock
    /// until a ttl after.
    pub(super) fn answered(&mut self) {
        self.extending = false;
        self.expires = Instant::now() + self.lease.ttl.get();
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
