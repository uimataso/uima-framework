//! Process-level plumbing for a service's `main`, such as stopping on a shutdown signal.
//!
//! # Examples
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
