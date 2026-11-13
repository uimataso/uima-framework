//! Stopping on a shutdown signal.

use tracing::info;
use uima_cancel::{CancelExt as _, CancelToken};

/// Completes on SIGINT (Ctrl-C) or, on unix, SIGTERM, which is what `docker stop` sends, and
/// logs which one arrived.
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

/// Cancels `token` on a [`shutdown_signal`]. Returns once either happens, so it can be joined with
/// the rest of the work without keeping the process up after that work has stopped the token
/// itself.
pub async fn cancel_on_shutdown_signal(token: &CancelToken) {
    if shutdown_signal().until_cancelled(token).await.is_some() {
        token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[tokio::test]
    async fn returns_once_the_token_is_cancelled_elsewhere() {
        let token = CancelToken::new();
        token.cancel();
        cancel_on_shutdown_signal(&token).await;
        check!(token.is_cancelled());
    }
}
