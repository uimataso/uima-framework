//! Process-level plumbing for a service's `main`, such as running its [`Subsystem`]s until a
//! shutdown signal.
//!
//! # Examples
//!
//! ```no_run
//! # async fn example(
//! #     server: impl uima_runtime::Subsystem,
//! #     purge: uima_runtime::periodic::Runner<impl uima_runtime::periodic::Job>,
//! # ) -> anyhow::Result<()> {
//! use uima_cancel::{CancelExt as _, CancelToken};
//! use uima_runtime::Subsystem as _;
//!
//! // Each part stops the others when it ends, such as when the server fails to bind.
//! let cancel = CancelToken::new();
//! let ((), served, purged) = tokio::join!(
//!     uima_runtime::cancel_on_shutdown_signal(&cancel),
//!     server.run(cancel.clone()).cancel_on_exit(&cancel),
//!     purge.run(cancel.clone()).cancel_on_exit(&cancel),
//! );
//! served?;
//! purged?;
//! # Ok(())
//! # }
//! ```

pub mod periodic;

mod signal;
mod subsystem;
mod timeout;

pub use signal::{cancel_on_shutdown_signal, shutdown_signal};
pub use subsystem::Subsystem;
pub use timeout::TimeoutExt;
