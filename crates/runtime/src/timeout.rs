//! Putting a time limit on a future.

use std::future::Future;
use std::time::Duration;

/// Puts a time limit on a future.
pub trait TimeoutExt: Future + Sized {
    /// Runs the future for at most `duration`, giving `Ok` with its output if it finishes in time,
    /// otherwise `Err` after dropping the future. Same as [`tokio::time::timeout`].
    fn timeout(self, duration: Duration) -> tokio::time::Timeout<Self> {
        tokio::time::timeout(duration, self)
    }
}

impl<F: Future> TimeoutExt for F {}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn gives_the_output_when_in_time() {
        let output = async { 1 }.timeout(Duration::from_secs(1)).await;
        check!(output.ok() == Some(1));
    }

    #[tokio::test(start_paused = true)]
    async fn gives_an_error_when_too_slow() {
        let slow = tokio::time::sleep(Duration::from_secs(2));
        check!(slow.timeout(Duration::from_secs(1)).await.is_err());
    }
}
