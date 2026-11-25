//! [`LockDriver`], which keeps the locks a [`LockClient`](crate::LockClient) took.

use std::collections::HashMap;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;
use tracing::{debug, warn};
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortResult, Uuid};

use crate::{Extended, Lease, LockProvider};

/// Keeps the locks its [`LockClient`](crate::LockClient) took, extending the held ones, letting
/// go of those dropped or released, and cancelling a guard's [`lost`](crate::LockGuard::lost)
/// once its lease is lost.
///
/// A lease counts as lost once it goes unextended for most of its `ttl`, however slow the store
/// is, so its holder stops before the store lets another take it.
#[must_use = "a lock taken without the driver running expires"]
pub struct LockDriver<P> {
    provider: Arc<P>,
    messages: mpsc::UnboundedReceiver<Message>,
    /// Kept so `messages` stays open with every client and guard gone, since the driver still
    /// runs until cancelled, as ending early would stop a service that runs it.
    _open: Driver,
}

impl<P: LockProvider> LockDriver<P> {
    /// A driver over `provider`, and the [`Driver`] to tell it what to keep through.
    pub(crate) fn new(provider: Arc<P>) -> (Self, Driver) {
        let (sender, messages) = mpsc::unbounded_channel();
        let driver = Driver(sender);
        let lock_driver = Self {
            provider,
            messages,
            _open: driver.clone(),
        };
        (lock_driver, driver)
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
    /// it is lost; false once the driver has stopped.
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

/// What a running [`LockDriver`] keeps track of.
struct Driving<P> {
    provider: Arc<P>,
    kept: HashMap<Uuid, Kept>,
    extending: JoinSet<ExtendReply>,
    unlocking: JoinSet<UnlockReply>,
}

impl<P: LockProvider> Driving<P> {
    fn new(provider: Arc<P>) -> Self {
        Self {
            provider,
            kept: HashMap::new(),
            extending: JoinSet::new(),
            unlocking: JoinSet::new(),
        }
    }

    /// When something is next due, if anything is.
    fn due(&self) -> Option<Instant> {
        self.kept.values().map(Kept::due).min()
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Held {
                lease,
                lost,
                taken_at,
            } => {
                self.kept
                    .insert(lease.token, Kept::new(lease, lost, taken_at));
            }
            Message::Release { lease, done } => {
                self.kept.remove(&lease.token);
                self.unlock(lease, Some(done));
            }
            Message::Dropped { lease } => {
                self.kept.remove(&lease.token);
                self.unlock(lease, None);
            }
        }
    }

    /// Gives up on the leases not extended in time, letting them go in case the store still
    /// holds them.
    fn give_up_late(&mut self, now: Instant) {
        let late: Vec<Uuid> = self
            .kept
            .values()
            .filter(|kept| kept.deadline <= now)
            .map(|kept| kept.lease.token)
            .collect();
        for token in late {
            if let Some(kept) = self.kept.remove(&token) {
                warn!(key = kept.lease.key, "lost a lock not extended in time");
                self.unlock(kept.lease.clone(), None);
            }
        }
    }

    /// Sends one extend of the leases due and not being extended already.
    fn extend_due(&mut self, now: Instant) {
        let due: Vec<Lease> = self
            .kept
            .values_mut()
            .filter(|kept| !kept.extending && kept.next_extend <= now)
            .map(|kept| {
                kept.extending = true;
                kept.lease.clone()
            })
            .collect();
        // Waited for only so long that the shortest lease can still be retried in time.
        let Some(wait) = due
            .iter()
            .map(|lease| lease.ttl.get() / Kept::WAITS_PER_TTL)
            .min()
        else {
            return;
        };

        let provider = self.provider.clone();
        self.extending.spawn(async move {
            let result = within(wait, provider.extend(&due)).await;
            ExtendReply {
                sent: now,
                leases: due,
                result,
            }
        });
    }

    /// Updates the leases still kept from what an extend got, dropping those the store lost.
    fn extended(&mut self, reply: ExtendReply) {
        for lease in &reply.leases {
            if let Some(kept) = self.kept.get_mut(&lease.token) {
                kept.extending = false;
            }
        }
        let extended = match reply.result {
            Ok(extended) => extended,
            Err(error) => {
                debug!(
                    error = &error as &dyn Error,
                    "failed to extend locks; retrying"
                );
                for lease in &reply.leases {
                    if let Some(kept) = self.kept.get_mut(&lease.token) {
                        kept.retry_soon();
                    }
                }
                return;
            }
        };
        for (lease, extended) in reply.leases.iter().zip(extended) {
            match extended {
                Extended::Held => {
                    if let Some(kept) = self.kept.get_mut(&lease.token) {
                        kept.extended(reply.sent);
                    }
                }
                Extended::Lost => {
                    if self.kept.remove(&lease.token).is_some() {
                        warn!(key = lease.key, "lost a lock taken by another");
                    }
                }
            }
        }
    }

    /// Stops keeping every lock, cancelling their `lost`, and lets them go.
    fn let_all_go(&mut self) {
        for (_, kept) in self.kept.drain() {
            self.unlocking
                .spawn(unlock(self.provider.clone(), kept.lease.clone(), None));
        }
    }

    fn unlock(&mut self, lease: Lease, done: Option<oneshot::Sender<PortResult<()>>>) {
        self.unlocking
            .spawn(unlock(self.provider.clone(), lease, done));
    }
}

/// A lock the driver keeps.
///
/// Dropping it cancels `lost`, so a lock is lost once the driver stops keeping it, however that
/// happens.
pub(crate) struct Kept {
    lease: Lease,
    lost: CancelToken,
    next_extend: Instant,
    /// When it counts as lost unless extended, a little before the store may expire it, from
    /// when the last request that held it was sent.
    deadline: Instant,
    /// Whether an extend of it is out.
    extending: bool,
}

impl Kept {
    /// How many times a lease is extended per `ttl`.
    const EXTENDS_PER_TTL: u32 = 3;
    /// How many times a lease is retried per `ttl` after a failed extend.
    const RETRIES_PER_TTL: u32 = 10;
    /// How many times an extend can be waited out per `ttl`, so one retried after it still is
    /// before the deadline.
    const WAITS_PER_TTL: u32 = 6;

    /// When a lease held by a request sent at `sent` counts as lost unless extended again, a
    /// little before the store may expire it.
    pub(crate) fn deadline_after(sent: Instant, ttl: Duration) -> Instant {
        sent + ttl * 4 / 5
    }

    fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        let mut kept = Self {
            lease,
            lost,
            next_extend: taken_at,
            deadline: taken_at,
            extending: false,
        };
        kept.extended(taken_at);
        kept
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    fn extended(&mut self, sent: Instant) {
        let ttl = self.lease.ttl.get();
        self.next_extend = sent + ttl / Self::EXTENDS_PER_TTL;
        self.deadline = Self::deadline_after(sent, ttl);
    }

    /// Schedules the next extend soon, after one failed.
    fn retry_soon(&mut self) {
        self.next_extend = Instant::now() + self.lease.ttl.get() / Self::RETRIES_PER_TTL;
    }

    /// When it is next due: to be extended, or given up on. While it is being extended, only
    /// the latter.
    fn due(&self) -> Instant {
        if self.extending {
            self.deadline
        } else {
            self.next_extend.min(self.deadline)
        }
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.lost.cancel();
    }
}

/// What an extend sent at `sent` got for `leases`.
struct ExtendReply {
    sent: Instant,
    leases: Vec<Lease>,
    result: PortResult<Vec<Extended>>,
}

/// What an unlock got, for the [`release`](crate::LockGuard::release) waiting on it, if any.
struct UnlockReply {
    result: PortResult<()>,
    done: Option<oneshot::Sender<PortResult<()>>>,
}

impl UnlockReply {
    /// Gives the result to the [`release`](crate::LockGuard::release) waiting on it, if any.
    fn answer(self) {
        if let Some(done) = self.done {
            done.send(self.result).ok();
        }
    }
}

/// Lets `lease` go, giving up after its ttl, when it has expired anyway.
async fn unlock<P: LockProvider>(
    provider: Arc<P>,
    lease: Lease,
    done: Option<oneshot::Sender<PortResult<()>>>,
) -> UnlockReply {
    let result = within(lease.ttl.get(), provider.unlock(&lease)).await;
    if let Err(error) = &result {
        warn!(
            key = lease.key,
            error = error as &dyn Error,
            "failed to let a lock go"
        );
    }
    UnlockReply { result, done }
}

/// What a store call's task gave, passing its panic on.
fn joined<T>(joined: Result<T, JoinError>) -> T {
    joined.unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()))
}

/// What `call` gives, or a timeout failure once `wait` has passed.
async fn within<T>(wait: Duration, call: impl Future<Output = PortResult<T>>) -> PortResult<T> {
    match tokio::time::timeout(wait, call).await {
        Ok(result) => result,
        Err(_) => {
            let late = anyhow!("no answer within {wait:?}");
            Err(Failure::new(FailureKind::Timeout, late).into())
        }
    }
}

/// Sleeps until `at`, or forever without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
