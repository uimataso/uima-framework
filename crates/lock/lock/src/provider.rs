//! [`LockProvider`], the store locks are kept in.

use std::future::Future;

use uima_core::{PortResult, Uuid};

use crate::{LockKey, Ttl};

#[cfg(feature = "testing")]
pub mod contract;
#[cfg(all(test, feature = "testing"))]
mod tests;

/// The store locks are kept in, such as a database table.
///
/// Apps take locks through a [`LockClient`](crate::LockClient), which calls it. A lease expires by
/// the store's clock, not the instance's.
pub trait LockProvider: Send + Sync + 'static {
    /// Takes the lock for `ttl` without waiting, if it is free or its last lease has expired.
    fn try_acquire(
        &self,
        key: &LockKey,
        ttl: Ttl,
    ) -> impl Future<Output = PortResult<TryAcquire>> + Send;

    /// Extends each lease by its own `ttl`, from now.
    ///
    /// Gives whether each is still held, in the order of `leases`.
    fn extend(&self, leases: &[Lease]) -> impl Future<Output = PortResult<Vec<Extended>>> + Send;

    /// Lets the lease go.
    ///
    /// One that has expired or been taken is not an error.
    fn unlock(&self, lease: &Lease) -> impl Future<Output = PortResult<()>> + Send;
}

/// What [`LockProvider::try_acquire`] got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryAcquire {
    Acquired(Lease),
    /// Someone else holds it.
    Busy,
}

/// A held lock, as the store knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub key: LockKey,
    /// Made anew at each acquisition, so a holder extends or lets go only its own lease.
    pub token: Uuid,
    pub ttl: Ttl,
}

/// Whether a lease was still held when extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extended {
    Held,
    /// It had expired or been let go, and may since have been taken.
    Lost,
}
