//! [`LockClient`] and its [`LockGuard`]s, which take locks and give them up.

use std::future::Future;
use std::sync::Arc;

use tokio::time::Instant;
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortResult};

use crate::driver::{self, Driver, Kept, LockDriver};
use crate::{Lease, LockProvider, TryAcquire, Ttl};

/// The port use cases take locks through.
#[uima_port_macro::has_port]
pub trait Lock: Send + Sync {
    /// Takes the lock on `key` for `ttl` without waiting, kept alive until the guard is dropped
    /// or released.
    ///
    /// A key this instance holds already is busy too.
    ///
    /// Fails when the store does, or answers too slowly to keep the lock, or once the
    /// [`LockDriver`] has stopped or been dropped. One that fails or is cancelled may leave the
    /// key held, with no guard, until `ttl` ends.
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
    let (driver, to_driver) = LockDriver::new(provider.clone());
    let client = LockClient {
        provider,
        driver: to_driver,
    };
    (client, driver)
}

/// Takes locks from a [`LockProvider`].
///
/// Cloning it is cheap, and every clone shares one [`LockDriver`].
pub struct LockClient<P> {
    provider: Arc<P>,
    driver: Driver,
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
        let failed = |kind, reason: &'static str| {
            let failure = Failure::new(kind, anyhow::Error::msg(reason));
            Err(failure.context(format!("failed to lock {key}")).into())
        };
        let stopped = "the lock driver has stopped";
        if self.driver.is_stopped() {
            return failed(FailureKind::Unavailable, stopped);
        }

        let taken_at = Instant::now();
        let lease = match self.provider.try_acquire(key, ttl).await? {
            TryAcquire::Acquired(lease) => lease,
            TryAcquire::Busy => return Ok(TryLock::Busy),
        };

        if Instant::now() >= Kept::deadline_after(taken_at, lease.ttl.get()) {
            // Taken too slowly to keep, as it may be expiring already, so the driver lets it go.
            self.driver.dropped(lease);
            return failed(
                FailureKind::Timeout,
                "the store answered too slowly to keep it",
            );
        }
        let lost = CancelToken::new();
        if !self.driver.held(lease.clone(), lost.clone(), taken_at) {
            // The driver stopped while the lease was being taken, after the check at the start.
            // Nothing would extend the lease or let it go, so it would keep the key from everyone
            // until its ttl ends; let it go here instead.
            driver::let_go(&*self.provider, &lease, lease.ttl.get()).await?;
            return failed(FailureKind::Unavailable, stopped);
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
    driver: Driver,
}

impl LockGuard {
    /// Cancelled once the lock is lost, as the [`LockDriver`] counts it, or the driver ends.
    ///
    /// The work it guards stops early once it is.
    pub fn lost(&self) -> &CancelToken {
        &self.lost
    }

    /// Lets the lock go, waiting until the store has.
    ///
    /// A lock already lost is not an error, nor is one whose driver has ended. A driver stopped by
    /// its `cancel` ends once every lock is let go or may have expired; one dropped leaves them to
    /// expire.
    pub async fn release(mut self) -> PortResult<()> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        self.driver.release(lease).await
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.driver.dropped(lease);
        }
    }
}
