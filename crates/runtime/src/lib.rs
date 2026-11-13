//! Process-level plumbing for a service's `main`: stopping on a shutdown signal, and time limits.
//!
//! ```no_run
//! # async fn serve(_: impl std::future::Future<Output = ()>) {}
//! # async fn example() {
//! use uima_cancel::CancelToken;
//!
//! let cancel = CancelToken::new();
//! tokio::join!(
//!     uima_runtime::cancel_on_shutdown_signal(&cancel),
//!     serve(cancel.cancelled()),
//! );
//! # }
//! ```

mod signal;
mod timeout;

pub use signal::{cancel_on_shutdown_signal, shutdown_signal};
pub use timeout::TimeoutExt;
