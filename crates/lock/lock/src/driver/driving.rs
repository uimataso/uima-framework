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
    /// Each lock not yet let go, nor lost to another.
    leases: HashMap<Uuid, Leased>,
    /// Whether the driver is stopping, so it keeps no lock.
    stopping: bool,
    pub(super) extending: JoinSet<ExtendReply>,
    pub(super) unlocking: JoinSet<UnlockReply>,
}

/// A lock not yet let go.
#[derive(Debug)]
enum Leased {
    /// Extended until its guard lets it go.
    Kept(Kept),
    /// No longer extended, but let go once its guard is, so another can't take it while its
    /// holder stops; once stopping, only until it may have expired.
    Unkept { expires: Instant },
}

impl<P: LockProvider> Driving<P> {
    pub(super) fn new(provider: Arc<P>) -> Self {
        Self {
            provider,
            leases: HashMap::new(),
            stopping: false,
            extending: JoinSet::new(),
            unlocking: JoinSet::new(),
        }
    }

    /// When something is next due, if anything is.
    pub(super) fn due(&self) -> Option<Instant> {
        self.leases
            .values()
            .filter_map(|leased| match leased {
                Leased::Kept(kept) => Some(kept.due()),
                Leased::Unkept { expires } => self.stopping.then_some(*expires),
            })
            .min()
    }

    pub(super) fn handle(&mut self, message: Message) {
        match message {
            Message::Held(kept) => {
                let token = kept.lease.token;
                let leased = if self.stopping {
                    // Taken as the driver stopped, so lost at once, as `kept` is dropped.
                    Leased::Unkept {
                        expires: kept.expires(),
                    }
                } else {
                    Leased::Kept(kept)
                };
                self.leases.insert(token, leased);
            }
            Message::Release { lease, done } => match self.forget(&lease) {
                Some(wait) => self.unlock(lease, wait, Some(done)),
                // Lost to another, or may have expired, so there is nothing to let go.
                None => {
                    done.send(Ok(())).ok();
                }
            },
            Message::Dropped { lease } => {
                if let Some(wait) = self.forget(&lease) {
                    self.unlock(lease, wait, None);
                }
            }
        }
    }

    /// Gives up on the leases not extended in time, and once stopping, forgets those that may
    /// have expired.
    pub(super) fn give_up_late(&mut self, now: Instant) {
        let stopping = self.stopping;
        self.leases.retain(|_, leased| match leased {
            Leased::Kept(kept) => {
                if kept.deadline() <= now {
                    warn!(key = kept.lease.key, "lost a lock not extended in time");
                    *leased = Leased::Unkept {
                        expires: kept.expires(),
                    };
                }
                true
            }
            Leased::Unkept { expires } => !stopping || *expires > now,
        });
    }

    /// Sends one extend of the leases due and not being extended already.
    pub(super) fn extend_due(&mut self, now: Instant) {
        let mut due: Vec<&mut Kept> = self
            .leases
            .values_mut()
            .filter_map(|leased| match leased {
                Leased::Kept(kept) if !kept.is_extending() && kept.next_extend <= now => Some(kept),
                _ => None,
            })
            .collect();
        // Waited for only so long that the shortest lease can still be retried in time.
        let Some(wait) = due
            .iter()
            .map(|kept| kept.lease.ttl.get() / Kept::WAITS_PER_TTL)
            .min()
        else {
            return;
        };
        let due: Vec<Lease> = due
            .iter_mut()
            .map(|kept| {
                kept.extending(now + wait);
                kept.lease.clone()
            })
            .collect();

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

    /// Updates the leases still kept from what an extend got, forgetting those the store lost.
    pub(super) fn extended(&mut self, reply: ExtendReply) {
        for lease in &reply.leases {
            if let Some(kept) = self.kept(lease) {
                kept.answered();
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
                    if let Some(kept) = self.kept(lease) {
                        kept.retry_soon();
                    }
                }
                return;
            }
        };
        for (lease, extended) in reply.leases.iter().zip(extended) {
            match extended {
                Extended::Held => {
                    if let Some(kept) = self.kept(lease) {
                        kept.extended(reply.sent);
                    }
                }
                Extended::Lost => {
                    if self.kept(lease).is_some() {
                        self.leases.remove(&lease.token);
                        warn!(key = lease.key, "lost a lock taken by another");
                    }
                }
            }
        }
    }

    /// Stops keeping every lock, cancelling their `lost`, and drops the extends still out.
    ///
    /// Each lock is let go once its guard is, until it may have expired.
    pub(super) fn stop(&mut self) {
        self.stopping = true;
        self.extending.abort_all();
        for leased in self.leases.values_mut() {
            if let Leased::Kept(kept) = leased {
                *leased = Leased::Unkept {
                    expires: kept.expires(),
                };
            }
        }
    }

    /// Whether, once stopping, a lock is still waiting for its guard.
    pub(super) fn holding(&self) -> bool {
        !self.leases.is_empty()
    }

    /// `lease`, if it is still kept.
    fn kept(&mut self, lease: &Lease) -> Option<&mut Kept> {
        match self.leases.get_mut(&lease.token) {
            Some(Leased::Kept(kept)) => Some(kept),
            _ => None,
        }
    }

    /// Forgets `lease`, giving how long to wait on letting it go: its ttl, or once stopping,
    /// until it may have expired. None when there is nothing to let go.
    fn forget(&mut self, lease: &Lease) -> Option<Duration> {
        let expires = match self.leases.remove(&lease.token)? {
            Leased::Kept(kept) => kept.expires(),
            Leased::Unkept { expires } => expires,
        };
        if !self.stopping {
            return Some(lease.ttl.get());
        }
        Some(expires.saturating_duration_since(Instant::now()))
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
