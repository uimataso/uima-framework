//! [`LockClient`] and its [`LockGuard`]s, which take locks and give them up.

use std::future::Future;
use std::sync::Arc;

use anyhow::anyhow;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortResult};

use crate::driver::{Kept, LockDriver};
use crate::{Lease, LockProvider, TryAcquire, Ttl};

#[cfg(all(test, feature = "testing"))]
mod tests;

/// The port use cases take locks through.
#[uima_port_macro::has_port]
pub trait Lock: Send + Sync {
    /// Takes the lock on `key` for `ttl` without waiting, kept alive until the guard is dropped
    /// or released.
    ///
    /// Fails when the store does, or once the [`LockDriver`] has stopped or been dropped. One that
    /// fails or is cancelled may leave the key held, with no guard, until `ttl` ends.
    fn try_lock(&self, key: &str, ttl: Ttl) -> impl Future<Output = PortResult<TryLock>> + Send;
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
        driver: sender.clone(),
    };
    let driver = LockDriver::new(provider, messages, sender.clone());
    (client, driver)
}

/// Takes locks from a [`LockProvider`].
///
/// Cloning it is cheap, and every clone shares one [`LockDriver`].
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
    async fn try_lock(&self, key: &str, ttl: Ttl) -> PortResult<TryLock> {
        let stopped = || {
            let stopped = anyhow!("the lock driver has stopped");
            Failure::new(FailureKind::Unavailable, stopped).context(format!("failed to lock {key}"))
        };
        if self.driver.is_closed() {
            return Err(stopped().into());
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
        if Instant::now() >= Kept::deadline_after(taken_at, lease.ttl.get()) {
            // Taken too slowly to keep, so it may be expiring already.
            lost.cancel();
        }
        if self.driver.send(held).is_err() {
            // The driver stopped while the lease was being taken, after the check above. Nothing
            // would extend the lease or let it go, so it would keep the key from everyone until
            // its ttl ends; let it go here instead.
            self.provider.unlock(&lease).await?;
            return Err(stopped().into());
        }

        Ok(TryLock::Locked(LockGuard {
            lease: Some(lease),
            lost,
            driver: self.driver.clone(),
        }))
    }
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
    /// Cancelled once the lock is lost, as when its lease went unextended for most of its `ttl`,
    /// someone else took it, or the driver ended.
    ///
    /// The work it guards stops early once it is.
    pub fn lost(&self) -> &CancelToken {
        &self.lost
    }

    /// Lets the lock go, waiting until the store has.
    ///
    /// A lock already lost is not an error, nor is one whose driver has ended. A driver stopped by
    /// its `cancel` let go of every lock as it did; one dropped left them to expire.
    pub async fn release(mut self) -> PortResult<()> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        let (done, released) = oneshot::channel();
        if self.driver.send(Message::Release { lease, done }).is_err() {
            return Ok(());
        }
        match released.await {
            Ok(released) => released,
            // The driver ended before answering.
            Err(_) => Ok(()),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            // A driver that has ended let go of it as it stopped, or left it to expire.
            self.driver.send(Message::Dropped { lease }).ok();
        }
    }
}

/// What a [`LockClient`] and its guards tell the [`LockDriver`].
#[derive(Debug)]
pub(crate) enum Message {
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
