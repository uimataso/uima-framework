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
//! # async fn example() -> anyhow::Result<()> {
//! use std::time::Duration;
//!
//! use uima_cancel::CancelToken;
//! use uima_lock::testing::MemoryLockProvider;
//! use uima_lock::{Lock as _, TryLock};
//!
//! let (client, driver) = uima_lock::new(MemoryLockProvider::default());
//! let cancel = CancelToken::new();
//! let driving = tokio::spawn(driver.run(cancel.clone()));
//!
//! if let TryLock::Locked(guard) = client.try_lock("purge", Duration::from_secs(30)).await? {
//!     // Work, stopping early once `guard.lost()` is cancelled.
//!     guard.release().await?;
//! }
//!
//! cancel.cancel();
//! driving.await?;
//! # Ok(())
//! # }
//! # let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build();
//! # runtime.unwrap().block_on(example()).unwrap();
//! ```
//!
//! # Features
//!
//! - `testing`: [`testing::MemoryLockProvider`] and the [`contract`] every provider passes.

mod client;
mod provider;
#[cfg(feature = "testing")]
pub mod testing;

pub use client::{Lock, LockClient, LockDriver, LockGuard, TryLock, new};
#[cfg(feature = "testing")]
pub use provider::contract;
pub use provider::{Extended, Lease, LockProvider, TryAcquire};
