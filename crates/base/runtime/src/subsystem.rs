//! A long-running part of a service.

use std::future::Future;

use uima_cancel::CancelToken;

/// A part of a service that runs until the service stops, such as its HTTP server or a worker.
pub trait Subsystem: Send {
    /// Names the subsystem, such as in logs.
    fn name(&self) -> &str;

    /// Runs until `cancel` is cancelled or the subsystem stops on its own.
    ///
    /// Fails when the subsystem stopped because of an error.
    fn run(self, cancel: CancelToken) -> impl Future<Output = anyhow::Result<()>> + Send;
}
