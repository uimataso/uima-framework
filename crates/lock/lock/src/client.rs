//! [`LockClient`] and [`LockDriver`], which take locks and keep them.

use std::collections::HashMap;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, warn};
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortError, PortResult, Uuid};

use crate::{Extended, Lease, LockProvider, TryAcquire};

#[cfg(all(test, feature = "testing"))]
mod tests;

/// The port use cases take locks through.
#[uima_port_macro::has_port]
pub trait Lock: Send + Sync {
    /// Takes the lock on `key` for `ttl` without waiting, kept alive until the guard is dropped
    /// or released.
    ///
    /// Fails when the store does, or once the [`LockDriver`] has stopped or been dropped.
    fn try_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = PortResult<TryLock>> + Send;
}

/// What [`Lock::try_lock`] got.
#[derive(Debug)]
pub enum TryLock {
    Locked(LockGuard),
    /// Someone else holds it.
    Busy,
}

/// A [`LockClient`] to take locks from `provider` with, and the [`LockDriver`] that keeps them.
pub fn new<P: LockProvider>(provider: P) -> (LockClient<P>, LockDriver<P>) {
    let provider = Arc::new(provider);
    let (sender, messages) = mpsc::unbounded_channel();
    let client = LockClient {
        provider: provider.clone(),
        driver: sender,
    };
    let driver = LockDriver { provider, messages };
    (client, driver)
}

/// Takes locks from a [`LockProvider`]. Cloning it is cheap, and every clone shares one
/// [`LockDriver`].
pub struct LockClient<P> {
    provider: Arc<P>,
    driver: mpsc::UnboundedSender<Message>,
}

impl<P> Clone for LockClient<P> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            driver: self.driver.clone(),
        }
    }
}

impl<P: LockProvider> Lock for LockClient<P> {
    async fn try_lock(&self, key: &str, ttl: Duration) -> PortResult<TryLock> {
        if ttl.is_zero() {
            return Err(failure(
                FailureKind::Other,
                format!("{key} locked for 0 seconds"),
            ));
        }
        if self.driver.is_closed() {
            return Err(driver_stopped());
        }

        let taken_at = Instant::now();
        let lease = match self.provider.try_acquire(key, ttl).await? {
            TryAcquire::Acquired(lease) => lease,
            TryAcquire::Busy => return Ok(TryLock::Busy),
        };
        let lost = CancelToken::new();
        let held = Message::Held {
            lease: lease.clone(),
            lost: lost.clone(),
            taken_at,
        };
        if self.driver.send(held).is_err() {
            // Stopped since the check, so no one would keep the lease or let it go.
            self.provider.unlock(&lease).await?;
            return Err(driver_stopped());
        }

        Ok(TryLock::Locked(LockGuard {
            lease: Some(lease),
            lost,
            driver: self.driver.clone(),
        }))
    }
}

/// Keeps the locks its [`LockClient`] took, extending the held ones, letting go of those dropped or
/// released, and cancelling a guard's [`lost`](LockGuard::lost) once its lease is lost.
#[must_use = "a lock taken without the driver running expires"]
pub struct LockDriver<P> {
    provider: Arc<P>,
    messages: mpsc::UnboundedReceiver<Message>,
}

impl<P: LockProvider> LockDriver<P> {
    /// Keeps the locks until `cancel`, then lets go of every lock still held, cancelling their
    /// guards' [`lost`](LockGuard::lost), and ends.
    ///
    /// It keeps the locks taken before it runs too, once it does. Dropping it, run or not, leaves
    /// the locks it kept to expire, and [`Lock::try_lock`] then fails.
    pub async fn run(mut self, cancel: CancelToken) {
        let mut kept = HashMap::new();

        loop {
            let due = kept.values().map(Kept::due).min();
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                message = self.messages.recv() => match message {
                    Some(message) => self.handle(message, &mut kept).await,
                    // No client or guard is left to send any.
                    None => return,
                },
                () = sleep_until(due) => self.extend(&mut kept).await,
            }
        }

        // Closed, so a lock taken from now on is let go by its taker; those already sent are
        // handled, then every lock still kept is let go.
        self.messages.close();
        while let Some(message) = self.messages.recv().await {
            self.handle(message, &mut kept).await;
        }
        for (_, kept) in kept {
            kept.lost.cancel();
            self.unlock(&kept.lease).await.ok();
        }
    }

    async fn handle(&self, message: Message, kept: &mut HashMap<Uuid, Kept>) {
        match message {
            Message::Held {
                lease,
                lost,
                taken_at,
            } => {
                let held = Kept::new(lease, lost, taken_at);
                kept.insert(held.lease.token, held);
            }
            Message::Release { lease, done } => {
                kept.remove(&lease.token);
                done.send(self.unlock(&lease).await).ok();
            }
            Message::Dropped { lease } => {
                kept.remove(&lease.token);
                self.unlock(&lease).await.ok();
            }
        }
    }

    /// Extends the leases due, after giving up on those not extended in time.
    async fn extend(&self, kept: &mut HashMap<Uuid, Kept>) {
        let now = Instant::now();
        kept.retain(|_, held| {
            let in_time = held.deadline > now;
            if !in_time {
                warn!(key = held.lease.key, "lost a lock not extended in time");
                held.lost.cancel();
            }
            in_time
        });

        let due: Vec<Lease> = kept
            .values()
            .filter(|held| held.next_extend <= now)
            .map(|held| held.lease.clone())
            .collect();
        if due.is_empty() {
            return;
        }

        match self.provider.extend(&due).await {
            Ok(extended) => {
                for (lease, extended) in due.iter().zip(extended) {
                    match extended {
                        Extended::Held => {
                            if let Some(held) = kept.get_mut(&lease.token) {
                                held.extended(now);
                            }
                        }
                        Extended::Lost => {
                            warn!(key = lease.key, "lost a lock taken by another");
                            if let Some(held) = kept.remove(&lease.token) {
                                held.lost.cancel();
                            }
                        }
                    }
                }
            }
            Err(error) => {
                debug!(
                    error = &error as &dyn Error,
                    "failed to extend locks; retrying"
                );
                for lease in &due {
                    if let Some(held) = kept.get_mut(&lease.token) {
                        held.next_extend = now + lease.ttl / RETRIES_PER_TTL;
                    }
                }
            }
        }
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        let unlocked = self.provider.unlock(lease).await;
        if let Err(error) = &unlocked {
            warn!(
                key = lease.key,
                error = error as &dyn Error,
                "failed to let a lock go"
            );
        }
        unlocked
    }
}

/// How often a lease is extended, and retried after a failed extend, per `ttl`.
const EXTENDS_PER_TTL: u32 = 3;
const RETRIES_PER_TTL: u32 = 10;

/// A lock the driver keeps.
struct Kept {
    lease: Lease,
    lost: CancelToken,
    next_extend: Instant,
    /// When it counts as lost unless extended, a little before the store may expire it, from
    /// when the last request that held it was sent.
    deadline: Instant,
}

impl Kept {
    fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        let mut kept = Self {
            lease,
            lost,
            next_extend: taken_at,
            deadline: taken_at,
        };
        kept.extended(taken_at);
        kept
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    fn extended(&mut self, sent: Instant) {
        let ttl = self.lease.ttl;
        self.next_extend = sent + ttl / EXTENDS_PER_TTL;
        self.deadline = sent + ttl * 4 / 5;
    }

    fn due(&self) -> Instant {
        self.next_extend.min(self.deadline)
    }
}

/// Sleeps until `at`, or forever without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn failure(kind: FailureKind, message: String) -> PortError<!> {
    PortError::Failed(Failure::new(kind, anyhow::anyhow!(message)))
}

fn driver_stopped() -> PortError<!> {
    failure(
        FailureKind::Unavailable,
        "the lock driver isn't running".to_owned(),
    )
}

/// A held lock, kept alive by the [`LockDriver`] until it is released or dropped.
///
/// Dropping it lets the lock go soon after, without waiting; [`release`](Self::release) waits.
#[derive(Debug)]
#[must_use = "dropping it lets the lock go"]
pub struct LockGuard {
    /// Taken by `release`, so dropping it after sends nothing.
    lease: Option<Lease>,
    lost: CancelToken,
    driver: mpsc::UnboundedSender<Message>,
}

impl LockGuard {
    /// Cancelled once the lock is lost, as when its lease expired before it was extended, someone
    /// else took it, or the driver stopped.
    ///
    /// The work it guards stops early once it is.
    pub fn lost(&self) -> &CancelToken {
        &self.lost
    }

    /// Lets the lock go, waiting until the store has. A lock already lost is not an error, nor is
    /// one whose driver has stopped, which let go of every lock as it did.
    pub async fn release(mut self) -> PortResult<()> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        let (done, released) = oneshot::channel();
        if self.driver.send(Message::Release { lease, done }).is_err() {
            return Ok(());
        }
        released.await.unwrap_or(Ok(()))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            // A stopped driver has let go of it already.
            self.driver.send(Message::Dropped { lease }).ok();
        }
    }
}

/// What a [`LockClient`] and its guards tell the [`LockDriver`].
#[derive(Debug)]
enum Message {
    /// A lock was taken by a request sent at `taken_at`; keep it alive and cancel `lost` once it
    /// is lost.
    Held {
        lease: Lease,
        lost: CancelToken,
        taken_at: Instant,
    },
    /// Let the lock go; answer through `done` once the store has.
    Release {
        lease: Lease,
        done: oneshot::Sender<PortResult<()>>,
    },
    /// A guard was dropped; let the lock go.
    Dropped { lease: Lease },
}
