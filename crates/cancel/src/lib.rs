//! A token for stopping work cooperatively: whoever holds a clone can cancel it, and every task
//! watching it stops at its next check.
//!
//! ```
//! # async fn serve() {}
//! # async fn run_workers(_: uima_cancel::CancelToken) {}
//! # async fn example() {
//! use uima_cancel::{CancelExt as _, CancelToken};
//!
//! let cancel = CancelToken::new();
//! // If the server stops, for any reason, stop the workers too.
//! let server = serve().cancel_on_finish(&cancel);
//! tokio::join!(server, run_workers(cancel.clone()));
//! # }
//! ```

use std::future::Future;

/// A cancellation token; clones share the same state.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(tokio_util::sync::CancellationToken);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that is cancelled when this one is, but can also be cancelled on its own without
    /// touching this one.
    pub fn child(&self) -> Self {
        Self(self.0.child_token())
    }

    /// Cancels this token and its children. Cancelling again does nothing.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    /// Completes once the token is cancelled. The future is `'static`, so it can be handed on, e.g.
    /// to axum's `with_graceful_shutdown`.
    pub fn cancelled(&self) -> impl Future<Output = ()> + Send + 'static {
        self.0.clone().cancelled_owned()
    }

    /// A guard that cancels the token when dropped, unless [`DropGuard::disarm`]ed first.
    pub fn drop_guard(self) -> DropGuard {
        DropGuard(self.0.drop_guard())
    }
}

/// Ties a future to a [`CancelToken`].
pub trait CancelExt: Future + Sized {
    /// Runs the future until it completes, giving `Some` of its output, or until `token` is
    /// cancelled, giving `None` and dropping the future.
    fn until_cancelled(
        self,
        token: &CancelToken,
    ) -> impl Future<Output = Option<Self::Output>> + use<Self> {
        let token = token.clone();
        async move { token.0.run_until_cancelled(self).await }
    }

    /// Runs the future and cancels `token` once it completes, or once it is dropped unfinished,
    /// so the rest of the work stops with it.
    fn cancel_on_finish(
        self,
        token: &CancelToken,
    ) -> impl Future<Output = Self::Output> + use<Self> {
        let guard = token.clone().drop_guard();
        async move {
            let _guard = guard;
            self.await
        }
    }
}

impl<F: Future> CancelExt for F {}

/// Cancels its token when dropped; see [`CancelToken::drop_guard`].
#[derive(Debug)]
#[must_use = "the token is cancelled as soon as the guard is dropped"]
pub struct DropGuard(tokio_util::sync::DropGuard);

impl DropGuard {
    /// Gives the token back without cancelling it.
    pub fn disarm(self) -> CancelToken {
        CancelToken(self.0.disarm())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uima_testing::check;

    use super::*;

    #[test]
    fn clones_share_the_cancellation() {
        let token = CancelToken::new();
        let clone = token.clone();
        clone.cancel();
        check!(token.is_cancelled());
    }

    #[test]
    fn a_child_follows_its_parent_but_not_the_other_way() {
        let parent = CancelToken::new();
        let child = parent.child();
        child.cancel();
        check!(!parent.is_cancelled());

        let other = parent.child();
        parent.cancel();
        check!(other.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_completes_after_cancel() {
        let token = CancelToken::new();
        let cancelled = token.cancelled();
        token.cancel();
        let completed = tokio::time::timeout(Duration::from_secs(1), cancelled).await;
        check!(completed.is_ok());
    }

    #[tokio::test]
    async fn until_cancelled_gives_the_output_or_none() {
        let token = CancelToken::new();
        check!(async { 1 }.until_cancelled(&token).await == Some(1));

        token.cancel();
        let cut = std::future::pending::<()>().until_cancelled(&token);
        let output = tokio::time::timeout(Duration::from_secs(1), cut).await;
        check!(output == Ok(None));
    }

    #[tokio::test]
    async fn cancel_on_finish_cancels_once_the_future_completes() {
        let token = CancelToken::new();
        check!(async { 1 }.cancel_on_finish(&token).await == 1);
        check!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_on_finish_cancels_when_the_future_is_dropped_unfinished() {
        let token = CancelToken::new();
        let unfinished = std::future::pending::<()>().cancel_on_finish(&token);
        let timed_out = tokio::time::timeout(Duration::from_millis(1), unfinished).await;
        check!(timed_out.is_err());
        check!(token.is_cancelled());
    }

    #[tokio::test]
    async fn the_wrapped_futures_stay_send_and_static() {
        let token = CancelToken::new();
        let finished = tokio::spawn(async { 1 }.cancel_on_finish(&token));
        check!(finished.await.unwrap() == 1);
        let output = tokio::spawn(async { 2 }.until_cancelled(&CancelToken::new()));
        check!(output.await.unwrap() == Some(2));
    }

    #[test]
    fn a_drop_guard_cancels_unless_disarmed() {
        let token = CancelToken::new();
        drop(token.clone().drop_guard());
        check!(token.is_cancelled());

        let token = CancelToken::new();
        let back = token.clone().drop_guard().disarm();
        check!(!token.is_cancelled());
        check!(!back.is_cancelled());
    }
}
