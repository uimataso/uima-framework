//! Stopping on a shutdown signal.

use tracing::info;
use uima_cancel::{CancelExt as _, CancelToken};

/// Completes on SIGINT (Ctrl-C) or, on unix, SIGTERM, and logs which one arrived.
///
/// # Panics
///
/// If the signal handlers can't be installed.
pub async fn shutdown_signal() {
    let interrupt = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install the SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install the SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => info!("SIGINT received, shutting down"),
        () = terminate => info!("SIGTERM received, shutting down"),
    }
}

/// Cancels `token` on a [`shutdown_signal`]. Returns once the signal arrives or the token is
/// cancelled elsewhere, so joining it doesn't keep the process up.
pub async fn cancel_on_shutdown_signal(token: &CancelToken) {
    if shutdown_signal().until_cancelled(token).await.is_some() {
        token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uima_testing::check;

    use super::*;
    use crate::TimeoutExt as _;

    #[tokio::test]
    async fn shutdown_signal_waits_for_a_signal() {
        check!(
            shutdown_signal()
                .timeout(Duration::from_millis(50))
                .await
                .is_err()
        );
    }

    // Sends SIGTERM to this process, so it relies on nextest running each test in its own
    // process.
    #[cfg(unix)]
    #[tokio::test]
    async fn sigterm_cancels_the_token() {
        let token = CancelToken::new();
        let watcher = tokio::spawn({
            let token = token.clone();
            async move { cancel_on_shutdown_signal(&token).await }
        });
        // Let the watcher install its handler before the signal arrives.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let pid = std::process::id().to_string();
        let sent = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status();
        check!(sent.unwrap().success());

        let finished = watcher.timeout(Duration::from_secs(5)).await;
        check!(finished.is_ok());
        check!(token.is_cancelled());
    }

    #[tokio::test]
    async fn returns_once_the_token_is_cancelled_elsewhere() {
        let token = CancelToken::new();
        token.cancel();
        let returned = cancel_on_shutdown_signal(&token)
            .timeout(Duration::from_secs(1))
            .await;
        check!(returned.is_ok());
        check!(token.is_cancelled());
    }
}
