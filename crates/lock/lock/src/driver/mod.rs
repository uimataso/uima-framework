//! [`LockDriver`], which keeps the locks a [`LockClient`](crate::LockClient) took.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
/// for four fifths of its `ttl`, however slow the store is, so a holder that stops within the
/// fifth left stops before the store lets another take it.
#[must_use = "a lock taken without the driver running expires"]
pub struct LockDriver<P> {
    provider: Arc<P>,
    messages: mpsc::UnboundedReceiver<Message>,
    /// Set once it stops, so no lock is taken from then on.
    stopping: Arc<AtomicBool>,
}

impl<P: LockProvider> LockDriver<P> {
    /// A driver over `provider`, and the [`ToDriver`] to tell it what to keep through.
    pub(crate) fn new(provider: Arc<P>) -> (Self, ToDriver) {
        let (sender, messages) = mpsc::unbounded_channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let driver = ToDriver {
            sender,
            stopping: stopping.clone(),
        };
        let lock_driver = Self {
            provider,
            messages,
            stopping,
        };
        (lock_driver, driver)
    }

    /// Keeps the locks until `cancel`, then cancels every guard's
    /// [`lost`](crate::LockGuard::lost), lets go of each lock once its guard is dropped or
    /// released, or not at all once it may have expired, and ends.
    ///
    /// So it ends as soon as the work it guards does, and at the latest once every lock may have
    /// expired: a `ttl` after, or a sixth of it more for one being extended as it stops.
    ///
    /// Locks taken before it starts are kept too. Dropping it, run or not, cancels the guards'
    /// `lost` as well, but leaves their locks to expire.
    pub async fn run(mut self, cancel: CancelToken) {
        let mut driving = Driving::new(self.provider);

        loop {
            let due = driving.due();
            tokio::select! {
                // Cancel first, so the driver stops however busy it is, then what is due, so a
                // lease is given up on in time however many messages and replies come in.
                biased;
                () = cancel.cancelled() => break,
                () = sleep_until(due) => {
                    let now = Instant::now();
                    driving.give_up_late(now);
                    driving.extend_due(now);
                }
                // None once every client and guard is gone, which skips it only this time
                // round, so the driver still runs until cancelled.
                Some(message) = self.messages.recv() => driving.handle(message),
                Some(reply) = driving.extending.join_next() => driving.extended(joined(reply)),
                Some(reply) = driving.unlocking.join_next() => joined(reply).answer(),
            }
        }

        // Stopping first, so a lock taken from now on is let go by its taker. Every lost is
        // cancelled at once, but each lock is let go only once its holder stops, so another
        // can't take it meanwhile.
        self.stopping.store(true, Ordering::SeqCst);
        driving.stop();
        // The locks taken before then, too.
        while let Ok(message) = self.messages.try_recv() {
            driving.handle(message);
        }
        while driving.holding() {
            let due = driving.due();
            tokio::select! {
                biased;
                () = sleep_until(due) => driving.give_up_late(Instant::now()),
                Some(message) = self.messages.recv() => driving.handle(message),
                Some(reply) = driving.unlocking.join_next() => joined(reply).answer(),
            }
        }

        // What was sent as the last lock was let go. A lock among them, taken by a client that
        // checked just before the driver stopped, is lost at once and left to expire.
        self.messages.close();
        while let Ok(message) = self.messages.try_recv() {
            driving.handle(message);
        }
        while let Some(reply) = driving.unlocking.join_next().await {
            joined(reply).answer();
        }
    }
}

/// How a [`LockClient`](crate::LockClient) and its guards tell the [`LockDriver`] what to keep.
#[derive(Clone, Debug)]
pub(crate) struct ToDriver {
    sender: mpsc::UnboundedSender<Message>,
    stopping: Arc<AtomicBool>,
}

impl ToDriver {
    /// Whether the driver is stopping, has stopped, or has been dropped.
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopping.load(Ordering::SeqCst) || self.sender.is_closed()
    }

    /// Has the driver keep `lease`, taken by a request sent at `taken_at`, and cancel `lost` once
    /// it is lost.
    ///
    /// True when the driver will keep it. False when the driver is stopping or has stopped, so
    /// nothing will extend the lease or let it go, and the caller lets it go itself.
    pub(crate) fn keep(&self, lease: Lease, lost: CancelToken, taken_at: Instant) -> bool {
        if self.is_stopped() {
            return false;
        }
        let kept = Kept::new(lease, lost, taken_at);
        self.sender.send(Message::Keep(kept)).is_ok()
    }

    /// Has the driver let `lease` go, waiting until the store has.
    ///
    /// A driver that has ended is not an error, having ended once the lease may have expired, or
    /// left it to expire.
    pub(crate) async fn release(&self, lease: Lease) -> PortResult<()> {
        let (done, released) = oneshot::channel();
        if self.sender.send(Message::Release { lease, done }).is_err() {
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
        // A driver that has ended did so once the lease may have expired, or left it to expire.
        self.sender.send(Message::Dropped { lease }).ok();
    }
}

/// What a [`ToDriver`] sends.
#[derive(Debug)]
enum Message {
    /// A lock was taken; keep it alive. Dropped unread, as with the driver, it cancels `lost`.
    Keep(Kept),
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
