//! [`LockDriver`], which keeps the locks a [`LockClient`](crate::LockClient) took.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinError;
use tokio::time::Instant;
use uima_cancel::CancelToken;
use uima_core::PortResult;

use crate::{Lease, LockProvider};
use driving::Driving;
pub(crate) use driving::let_go;
pub(crate) use kept::Kept;

mod driving;
mod kept;

/// Keeps the locks its [`LockClient`](crate::LockClient) took, extending the held ones, letting
/// go of those dropped or released, and cancelling a guard's [`lost`](crate::LockGuard::lost)
/// once its lease is lost.
///
/// A lease counts as lost once the store says it was let go or taken, or once it goes unextended
/// for most of its `ttl`, however slow the store is, so its holder stops before the store lets
/// another take it.
#[must_use = "a lock taken without the driver running expires"]
pub struct LockDriver<P> {
    provider: Arc<P>,
    messages: mpsc::UnboundedReceiver<Message>,
}

impl<P: LockProvider> LockDriver<P> {
    /// A driver over `provider`, and the [`Driver`] to tell it what to keep through.
    pub(crate) fn new(provider: Arc<P>) -> (Self, Driver) {
        let (sender, messages) = mpsc::unbounded_channel();
        (Self { provider, messages }, Driver(sender))
    }

    /// Keeps the locks until `cancel`, then cancels every guard's
    /// [`lost`](crate::LockGuard::lost), lets go of every lock still held, and ends.
    ///
    /// Locks taken before it starts are kept too. Once it has started, dropping it cancels the
    /// guards' `lost` as well, but leaves their locks to expire. A driver dropped without
    /// running cancels nothing.
    pub async fn run(mut self, cancel: CancelToken) {
        let mut driving = Driving::new(self.provider);

        loop {
            let due = driving.due();
            tokio::select! {
                // Cancel first, so the driver stops however busy it is.
                biased;
                () = cancel.cancelled() => break,
                // None once every client and guard is gone, which skips it only this time
                // round, so the driver still runs until cancelled.
                Some(message) = self.messages.recv() => driving.handle(message),
                Some(reply) = driving.extending.join_next() => driving.extended(joined(reply)),
                Some(reply) = driving.unlocking.join_next() => joined(reply).answer(),
                () = sleep_until(due) => {
                    let now = Instant::now();
                    driving.give_up_late(now);
                    driving.extend_due(now);
                }
            }
        }

        // Closed first, so a lock taken from now on is let go by its taker. Every lost is
        // cancelled before any unlock is waited on, and the extends still out are dropped.
        self.messages.close();
        driving.extending.abort_all();
        while let Ok(message) = self.messages.try_recv() {
            driving.handle(message);
        }
        driving.let_all_go();
        while let Some(reply) = driving.unlocking.join_next().await {
            joined(reply).answer();
        }
    }
}

/// How a [`LockClient`](crate::LockClient) and its guards tell the [`LockDriver`] what to keep.
#[derive(Clone, Debug)]
pub(crate) struct Driver(mpsc::UnboundedSender<Message>);

impl Driver {
    /// Whether the driver has stopped or been dropped.
    pub(crate) fn is_stopped(&self) -> bool {
        self.0.is_closed()
    }

    /// Has the driver keep `lease`, taken by a request sent at `taken_at`, and cancel `lost` once
    /// it is lost.
    ///
    /// True when the driver will keep it. False when the driver has stopped, so nothing will
    /// extend the lease or let it go, and the caller lets it go itself.
    pub(crate) fn held(&self, lease: Lease, lost: CancelToken, taken_at: Instant) -> bool {
        let held = Message::Held {
            lease,
            lost,
            taken_at,
        };
        self.0.send(held).is_ok()
    }

    /// Has the driver let `lease` go, waiting until the store has.
    ///
    /// A driver that has ended is not an error, having let go of it as it stopped, or left it to
    /// expire.
    pub(crate) async fn release(&self, lease: Lease) -> PortResult<()> {
        let (done, released) = oneshot::channel();
        if self.0.send(Message::Release { lease, done }).is_err() {
            return Ok(());
        }
        match released.await {
            Ok(released) => released,
            // The driver ended before answering.
            Err(_) => Ok(()),
        }
    }

    /// Has the driver let `lease` go soon, without waiting.
    pub(crate) fn dropped(&self, lease: Lease) {
        // A driver that has ended let go of it as it stopped, or left it to expire.
        self.0.send(Message::Dropped { lease }).ok();
    }
}

/// What a [`Driver`] sends.
#[derive(Debug)]
enum Message {
    /// A lock was taken by a request sent at `taken_at`; keep it alive and cancel `lost` once it
    /// is lost.
    Held {
        lease: Lease,
        lost: CancelToken,
        taken_at: Instant,
    },
    /// Let the lock go; answer through `done` once the store has.
    Release {
        lease: Lease,
        done: oneshot::Sender<PortResult<()>>,
    },
    /// A guard was dropped; let the lock go.
    Dropped { lease: Lease },
}

/// What a store call's task gave, passing its panic on.
fn joined<T>(joined: Result<T, JoinError>) -> T {
    joined.unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()))
}

/// Sleeps until `at`, or forever without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
