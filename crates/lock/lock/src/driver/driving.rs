//! [`Driving`], the state of a running [`LockDriver`](super::LockDriver).

use std::collections::HashMap;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::{debug, warn};
use uima_core::{Failure, FailureKind, PortResult, Uuid};

use super::{Kept, Message};
use crate::{Extended, Lease, LockProvider};

/// What a running [`LockDriver`](super::LockDriver) keeps track of.
pub(super) struct Driving<P> {
    provider: Arc<P>,
    kept: HashMap<Uuid, Kept>,
    pub(super) extending: JoinSet<ExtendReply>,
    pub(super) unlocking: JoinSet<UnlockReply>,
}

impl<P: LockProvider> Driving<P> {
    pub(super) fn new(provider: Arc<P>) -> Self {
        Self {
            provider,
            kept: HashMap::new(),
            extending: JoinSet::new(),
            unlocking: JoinSet::new(),
        }
    }

    /// When something is next due, if anything is.
    pub(super) fn due(&self) -> Option<Instant> {
        self.kept.values().map(Kept::due).min()
    }

    pub(super) fn handle(&mut self, message: Message) {
        match message {
            Message::Held(kept) => {
                self.kept.insert(kept.lease.token, kept);
            }
            Message::Release { lease, done } => {
                self.kept.remove(&lease.token);
                let wait = lease.ttl.get();
                self.unlock(lease, wait, Some(done));
            }
            Message::Dropped { lease } => {
                self.kept.remove(&lease.token);
                let wait = lease.ttl.get();
                self.unlock(lease, wait, None);
            }
        }
    }

    /// Gives up on the leases not extended in time, letting them go in case the store still
    /// holds them.
    pub(super) fn give_up_late(&mut self, now: Instant) {
        let late: Vec<Uuid> = self
            .kept
            .values()
            .filter(|kept| kept.deadline() <= now)
            .map(|kept| kept.lease.token)
            .collect();
        for token in late {
            if let Some(kept) = self.kept.remove(&token) {
                warn!(key = kept.lease.key, "lost a lock not extended in time");
                self.unlock(kept.lease.clone(), kept.lease.ttl.get(), None);
            }
        }
    }

    /// Sends one extend of the leases due and not being extended already.
    pub(super) fn extend_due(&mut self, now: Instant) {
        let due: Vec<Lease> = self
            .kept
            .values_mut()
            .filter(|kept| !kept.extending && kept.next_extend <= now)
            .map(|kept| {
                kept.extending = true;
                kept.lease.clone()
            })
            .collect();
        // Waited for only so long that the shortest lease can still be retried in time.
        let Some(wait) = due
            .iter()
            .map(|lease| lease.ttl.get() / Kept::WAITS_PER_TTL)
            .min()
        else {
            return;
        };

        let provider = self.provider.clone();
        self.extending.spawn(async move {
            let result = within(wait, provider.extend(&due)).await;
            ExtendReply {
                sent: now,
                leases: due,
                result,
            }
        });
    }

    /// Updates the leases still kept from what an extend got, dropping those the store lost.
    pub(super) fn extended(&mut self, reply: ExtendReply) {
        for lease in &reply.leases {
            if let Some(kept) = self.kept.get_mut(&lease.token) {
                kept.extending = false;
            }
        }
        let extended = match reply.result {
            Ok(extended) => extended,
            Err(error) => {
                debug!(
                    error = &error as &dyn Error,
                    "failed to extend locks; retrying"
                );
                for lease in &reply.leases {
                    if let Some(kept) = self.kept.get_mut(&lease.token) {
                        kept.retry_soon();
                    }
                }
                return;
            }
        };
        for (lease, extended) in reply.leases.iter().zip(extended) {
            match extended {
                Extended::Held => {
                    if let Some(kept) = self.kept.get_mut(&lease.token) {
                        kept.extended(reply.sent);
                    }
                }
                Extended::Lost => {
                    if self.kept.remove(&lease.token).is_some() {
                        warn!(key = lease.key, "lost a lock taken by another");
                    }
                }
            }
        }
    }

    /// Stops keeping every lock, cancelling their `lost`, and lets them go, giving up on each
    /// once the store may have expired it.
    pub(super) fn let_all_go(&mut self) {
        let now = Instant::now();
        let kept: Vec<Kept> = self.kept.drain().map(|(_, kept)| kept).collect();
        for kept in kept {
            let wait = kept.expires().saturating_duration_since(now);
            self.unlock(kept.lease.clone(), wait, None);
        }
    }

    fn unlock(
        &mut self,
        lease: Lease,
        wait: Duration,
        done: Option<oneshot::Sender<PortResult<()>>>,
    ) {
        let provider = self.provider.clone();
        self.unlocking.spawn(async move {
            let result = let_go(&*provider, &lease, wait).await;
            UnlockReply { result, done }
        });
    }
}

/// What an extend sent at `sent` got for `leases`.
pub(super) struct ExtendReply {
    sent: Instant,
    leases: Vec<Lease>,
    result: PortResult<Vec<Extended>>,
}

/// What an unlock got, for the [`release`](crate::LockGuard::release) waiting on it, if any.
pub(super) struct UnlockReply {
    result: PortResult<()>,
    done: Option<oneshot::Sender<PortResult<()>>>,
}

impl UnlockReply {
    /// Gives the result to the [`release`](crate::LockGuard::release) waiting on it, if any.
    pub(super) fn answer(self) {
        if let Some(done) = self.done {
            done.send(self.result).ok();
        }
    }
}

/// Lets `lease` go, giving up after `wait`, once the store may have expired it anyway.
pub(crate) async fn let_go(
    provider: &impl LockProvider,
    lease: &Lease,
    wait: Duration,
) -> PortResult<()> {
    let result = within(wait, provider.unlock(lease)).await;
    if let Err(error) = &result {
        warn!(
            key = lease.key,
            error = error as &dyn Error,
            "failed to let a lock go"
        );
    }
    result
}

/// What `call` gives, or a timeout failure once `wait` has passed.
async fn within<T>(wait: Duration, call: impl Future<Output = PortResult<T>>) -> PortResult<T> {
    match tokio::time::timeout(wait, call).await {
        Ok(result) => result,
        Err(_) => {
            let late = anyhow!("no answer within {wait:?}");
            Err(Failure::new(FailureKind::Timeout, late).into())
        }
    }
}
