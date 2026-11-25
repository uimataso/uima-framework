//! A lock that one instance of a service holds at a time, across instances, such as to run a job
//! on only one of them.
//!
//! Every lock is a lease, which expires after its `ttl` unless extended, so a holder that is stuck
//! or gone loses it. [`new`] gives a [`LockClient`] to take locks with, and a [`LockDriver`] that
//! keeps the held ones alive and lets the dropped ones go, which must run for the client to work.
//!
//! # Examples
//!
//! ```
//! # async fn example(provider: impl uima_lock::LockProvider) -> anyhow::Result<()> {
//! use uima_cancel::CancelToken;
//! use uima_lock::{Lock as _, TryLock, Ttl};
//!
//! let (client, driver) = uima_lock::new(provider);
//! let cancel = CancelToken::new();
//! let driving = tokio::spawn(driver.run(cancel.clone()));
//!
//! if let TryLock::Locked(guard) = client.try_lock("purge", Ttl::from_secs(30)).await? {
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
//! # Features
//!
//! - `testing`: adds `testing::MemoryLockProvider` and the `contract` every provider passes.

mod client;
mod driver;
mod provider;
#[cfg(feature = "testing")]
pub mod testing;
mod ttl;

pub use client::{HasLock, Lock, LockClient, LockGuard, TryLock, new};
pub use driver::LockDriver;
#[cfg(feature = "testing")]
pub use provider::contract;
pub use provider::{Extended, Lease, LockProvider, TryAcquire};
pub use ttl::{Ttl, TtlErrorKind};
