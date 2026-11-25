//! Background work in a service, such as a job that runs on a schedule.
//!
//! # Examples
//!
//! ```
//! # async fn example() -> anyhow::Result<()> {
//! use std::time::Duration;
//!
//! use uima_cancel::CancelToken;
//! use uima_runtime::Subsystem as _;
//! use uima_worker::periodic;
//!
//! struct PurgeExpiredSessions;
//!
//! impl periodic::Job for PurgeExpiredSessions {
//!     fn name(&self) -> &str {
//!         "purge-expired-sessions"
//!     }
//!
//!     async fn run_once(&mut self, _cancel: &CancelToken) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//! }
//!
//! let config = periodic::Config {
//!     behavior: periodic::Behavior::AfterStart { every: Duration::from_secs(60) },
//!     first_run: periodic::FirstRun::Immediately,
//! };
//! let purge = periodic::Runner::new(PurgeExpiredSessions, config)?;
//! let cancel = CancelToken::new();
//! # cancel.cancel();
//! purge.run(cancel).await?;
//! # Ok(())
//! # }
//! ```

pub mod periodic;
