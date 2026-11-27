//! A lock that one instance of a service holds at a time, across instances, such as to run a job
//! on only one of them.
//!
//! # Examples
//!
//! ```
//! # async fn example(provider: impl uima_lock::LockProvider) -> anyhow::Result<()> {
//! use uima_cancel::CancelToken;
//! use uima_lock::{Lock as _, LockKey, TryLock, Ttl};
//!
//! let (client, driver) = uima_lock::new(provider);
//! let cancel = CancelToken::new();
//! let driving = tokio::spawn(driver.run(cancel.clone()));
//!
//! const PURGE: LockKey = LockKey::from_static("purge");
//!
//! if let TryLock::Locked(guard) = client.try_lock(&PURGE, Ttl::from_secs(30)).await? {
//!     // Work, stopping early once `guard.lost()` is cancelled.
//!     guard.release().await?;
//! }
//!
//! cancel.cancel();
//! driving.await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Leases
//!
//! Every lock is a lease, which expires after its `ttl` unless extended, so an instance that hangs
//! or is gone loses its locks. [`new`] gives a [`LockClient`] to take locks with, and a
//! [`LockDriver`] that keeps the held ones alive and lets the dropped ones go, which must run for
//! the client to work. It keeps a lock for as long as its guard is held, also by a task that is
//! stuck.
//!
//! # Writes
//!
//! A lock keeps instances from doing the same work at once; it doesn't guard writes. A holder that
//! takes more than a fifth of its `ttl` to stop once [`lost`](LockGuard::lost) is cancelled may
//! overlap the next one, so a write that must not happen twice is idempotent, or checks in its
//! transaction that the lock is still held.
//!
//! # Features
//!
//! - `testing`: adds `testing::MemoryLockProvider` and the `contract` every provider passes.

mod client;
mod driver;
mod key;
mod provider;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(all(test, feature = "testing"))]
mod tests;
mod ttl;

pub use client::{HasLock, Lock, LockClient, LockGuard, TryLock, new};
pub use driver::LockDriver;
pub use key::{LockKey, LockKeyErrorKind};
#[cfg(feature = "testing")]
pub use provider::contract;
pub use provider::{Extended, Lease, LockProvider, TryAcquire};
pub use ttl::{Ttl, TtlErrorKind};
