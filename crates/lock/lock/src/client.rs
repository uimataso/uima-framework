//! [`LockClient`] and [`LockDriver`], which take locks and keep them.

use std::collections::HashMap;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, warn};
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortResult, Uuid};

use crate::{Extended, Lease, LockProvider, TryAcquire, Ttl};

#[cfg(all(test, feature = "testing"))]
mod tests;

/// The port use cases take locks through.
#[uima_port_macro::has_port]
pub trait Lock: Send + Sync {
    /// Takes the lock on `key` for `ttl` without waiting, kept alive until the guard is dropped
    /// or released.
    ///
    /// Fails when the store does, or once the [`LockDriver`] has stopped or been dropped. One that
    /// fails or is cancelled may leave the key held, with no guard, until `ttl` ends.
    fn try_lock(&self, key: &str, ttl: Ttl) -> impl Future<Output = PortResult<TryLock>> + Send;
}

/// What [`Lock::try_lock`] got.
#[derive(Debug)]
pub enum TryLock {
    Locked(LockGuard),
    /// Someone else holds it.
    Busy,
}

/// A [`LockClient`] to take locks from `provider` with, and the [`LockDriver`] that keeps them.
pub fn new<P: LockProvider>(provider: P) -> (LockClient<P>, LockDriver<P>) {
    let provider = Arc::new(provider);
    let (sender, messages) = mpsc::unbounded_channel();
    let client = LockClient {
        provider: provider.clone(),
        driver: sender,
    };
    let driver = LockDriver { provider, messages };
    (client, driver)
}

/// Takes locks from a [`LockProvider`].
///
/// Cloning it is cheap, and every clone shares one [`LockDriver`].
pub struct LockClient<P> {
    provider: Arc<P>,
    driver: mpsc::UnboundedSender<Message>,
}

impl<P> Clone for LockClient<P> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            driver: self.driver.clone(),
        }
    }
}

impl<P: LockProvider> Lock for LockClient<P> {
    async fn try_lock(&self, key: &str, ttl: Ttl) -> PortResult<TryLock> {
        if self.driver.is_closed() {
            let stopped = anyhow!("the lock driver has stopped");
            let failure = Failure::new(FailureKind::Unavailable, stopped);
            return Err(failure.context(format!("failed to lock {key}")).into());
        }

        let taken_at = Instant::now();
        let lease = match self.provider.try_acquire(key, ttl).await? {
            TryAcquire::Acquired(lease) => lease,
            TryAcquire::Busy => return Ok(TryLock::Busy),
        };

        let lost = CancelToken::new();
        let held = Message::Held {
            lease: lease.clone(),
            lost: lost.clone(),
            taken_at,
        };
        if Instant::now() >= deadline(taken_at, lease.ttl.get()) {
            // Taken too slowly to keep, so it may be expiring already.
            lost.cancel();
        }
        if self.driver.send(held).is_err() {
            // The driver stopped while the lease was being taken, after the check above. Nothing
            // would extend the lease or let it go, so it would keep the key from everyone until
            // its ttl ends; let it go here instead.
            self.provider.unlock(&lease).await?;
            let stopped = anyhow!("the lock driver has stopped");
            let failure = Failure::new(FailureKind::Unavailable, stopped);
            return Err(failure.context(format!("failed to lock {key}")).into());
        }

        Ok(TryLock::Locked(LockGuard {
            lease: Some(lease),
            lost,
            driver: self.driver.clone(),
        }))
    }
}

/// Keeps the locks its [`LockClient`] took, extending the held ones, letting go of those dropped or
/// released, and cancelling a guard's [`lost`](LockGuard::lost) once its lease is lost.
///
/// A lease counts as lost once it goes unextended for most of its `ttl`, however slow the store
/// is, so its holder stops before the store lets another take it.
#[must_use = "a lock taken without the driver running expires"]
pub struct LockDriver<P> {
    provider: Arc<P>,
    messages: mpsc::UnboundedReceiver<Message>,
}

impl<P: LockProvider> LockDriver<P> {
    /// Keeps the locks until `cancel`, then cancels every guard's [`lost`](LockGuard::lost), lets
    /// go of every lock still held, and ends.
    ///
    /// Locks taken before it starts are kept too. Once it has started, dropping it cancels the
    /// guards' `lost` as well, but leaves their locks to expire. A driver dropped without
    /// running cancels nothing.
    pub async fn run(mut self, cancel: CancelToken) {
        let mut driving = Driving::new(self.provider);
        // Whether a client or guard is left to send messages.
        let mut open = true;

        loop {
            let due = driving.due();
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                // With none left, it still lets go of the locks dropped last, and runs until
                // cancelled, since ending early would stop a service that runs it.
                message = self.messages.recv(), if open => match message {
                    Some(message) => driving.handle(message),
                    None => open = false,
                },
                reply = poll(driving.extending.as_mut()) => driving.extended(reply),
                Some(reply) = driving.unlocking.next() => reply.answer(),
                () = sleep_until(due) => {
                    let now = Instant::now();
                    driving.give_up_late(now);
                    driving.extend_due(now);
                }
            }
        }

        // Closed first, so a lock taken from now on is let go by its taker. Every lost is
        // cancelled before any unlock is waited on, and an extend still out is dropped.
        self.messages.close();
        driving.extending = None;
        while let Ok(message) = self.messages.try_recv() {
            driving.handle(message);
        }
        driving.let_all_go();
        while let Some(reply) = driving.unlocking.next().await {
            reply.answer();
        }
    }
}

/// A held lock, kept alive by the [`LockDriver`] until it is released or dropped.
///
/// Dropping it lets the lock go soon after, without waiting; [`release`](Self::release) waits.
#[derive(Debug)]
#[must_use = "dropping it lets the lock go"]
pub struct LockGuard {
    /// Taken by `release`, so dropping it after sends nothing.
    lease: Option<Lease>,
    lost: CancelToken,
    driver: mpsc::UnboundedSender<Message>,
}

impl LockGuard {
    /// Cancelled once the lock is lost, as when its lease went unextended for most of its `ttl`,
    /// someone else took it, or the driver ended.
    ///
    /// The work it guards stops early once it is.
    pub fn lost(&self) -> &CancelToken {
        &self.lost
    }

    /// Lets the lock go, waiting until the store has.
    ///
    /// A lock already lost is not an error, nor is one whose driver has ended. A driver stopped by
    /// its `cancel` let go of every lock as it did; one dropped left them to expire.
    pub async fn release(mut self) -> PortResult<()> {
        let Some(lease) = self.lease.take() else {
            return Ok(());
        };
        let (done, released) = oneshot::channel();
        if self.driver.send(Message::Release { lease, done }).is_err() {
            return Ok(());
        }
        released.await.unwrap_or(Ok(()))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            // A stopped driver has let go of it already.
            self.driver.send(Message::Dropped { lease }).ok();
        }
    }
}

/// What a [`LockClient`] and its guards tell the [`LockDriver`].
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
    /// The one extend out, if any.
    extending: Option<Call<ExtendReply>>,
    unlocking: FuturesUnordered<Call<UnlockReply>>,
}

impl<P: LockProvider> Driving<P> {
    fn new(provider: Arc<P>) -> Self {
        Self {
            provider,
            kept: HashMap::new(),
            extending: None,
            unlocking: FuturesUnordered::new(),
        }
    }

    /// When something is next due, if anything is. While an extend is out, only the deadlines
    /// are.
    fn due(&self) -> Option<Instant> {
        self.kept
            .values()
            .map(|kept| {
                if self.extending.is_some() {
                    kept.deadline
                } else {
                    kept.due()
                }
            })
            .min()
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

    /// Sends one extend of the leases due, unless one is out already.
    fn extend_due(&mut self, now: Instant) {
        if self.extending.is_some() {
            return;
        }
        let due: Vec<Lease> = self
            .kept
            .values()
            .filter(|kept| kept.next_extend <= now)
            .map(|kept| kept.lease.clone())
            .collect();
        if due.is_empty() {
            return;
        }

        let provider = self.provider.clone();
        self.extending = Some(Box::pin(async move {
            let result = provider.extend(&due).await;
            ExtendReply {
                sent: now,
                leases: due,
                result,
            }
        }));
    }

    /// Updates the leases still kept from what an extend got, dropping those the store lost.
    fn extended(&mut self, reply: ExtendReply) {
        self.extending = None;
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
                .push(unlock(self.provider.clone(), kept.lease.clone(), None));
        }
    }

    fn unlock(&mut self, lease: Lease, done: Option<oneshot::Sender<PortResult<()>>>) {
        self.unlocking
            .push(unlock(self.provider.clone(), lease, done));
    }
}

/// A lock the driver keeps.
///
/// Dropping it cancels `lost`, so a lock is lost once the driver stops keeping it, however that
/// happens.
struct Kept {
    lease: Lease,
    lost: CancelToken,
    next_extend: Instant,
    /// When it counts as lost unless extended, a little before the store may expire it, from
    /// when the last request that held it was sent.
    deadline: Instant,
}

impl Kept {
    /// How many times a lease is extended per `ttl`.
    const EXTENDS_PER_TTL: u32 = 3;
    /// How many times a lease is retried per `ttl` after a failed extend.
    const RETRIES_PER_TTL: u32 = 10;

    fn new(lease: Lease, lost: CancelToken, taken_at: Instant) -> Self {
        let mut kept = Self {
            lease,
            lost,
            next_extend: taken_at,
            deadline: taken_at,
        };
        kept.extended(taken_at);
        kept
    }

    /// Schedules the next extend after one sent at `sent` held the lease.
    fn extended(&mut self, sent: Instant) {
        let ttl = self.lease.ttl.get();
        self.next_extend = sent + ttl / Self::EXTENDS_PER_TTL;
        self.deadline = deadline(sent, ttl);
    }

    /// Schedules the next extend soon, after one failed.
    fn retry_soon(&mut self) {
        self.next_extend = Instant::now() + self.lease.ttl.get() / Self::RETRIES_PER_TTL;
    }

    fn due(&self) -> Instant {
        self.next_extend.min(self.deadline)
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.lost.cancel();
    }
}

/// A store call the driver has sent, which it polls alongside its other work.
type Call<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What an extend sent at `sent` got for `leases`.
struct ExtendReply {
    sent: Instant,
    leases: Vec<Lease>,
    result: PortResult<Vec<Extended>>,
}

/// What an unlock got, for the [`LockGuard::release`] waiting on it, if any.
struct UnlockReply {
    result: PortResult<()>,
    done: Option<oneshot::Sender<PortResult<()>>>,
}

impl UnlockReply {
    /// Gives the result to the [`LockGuard::release`] waiting on it, if any.
    fn answer(self) {
        if let Some(done) = self.done {
            done.send(self.result).ok();
        }
    }
}

/// Lets `lease` go, giving up after its ttl, when it has expired anyway.
fn unlock<P: LockProvider>(
    provider: Arc<P>,
    lease: Lease,
    done: Option<oneshot::Sender<PortResult<()>>>,
) -> Call<UnlockReply> {
    Box::pin(async move {
        let result = match tokio::time::timeout(lease.ttl.get(), provider.unlock(&lease)).await {
            Ok(result) => result,
            Err(_) => {
                let late = anyhow!("no answer within the lease's ttl");
                Err(Failure::new(FailureKind::Timeout, late).into())
            }
        };
        if let Err(error) = &result {
            warn!(
                key = lease.key,
                error = error as &dyn Error,
                "failed to let a lock go"
            );
        }
        UnlockReply { result, done }
    })
}

/// Waits for `call`, or forever without one.
async fn poll<T>(call: Option<&mut Call<T>>) -> T {
    match call {
        Some(call) => call.await,
        None => std::future::pending().await,
    }
}

/// Sleeps until `at`, or forever without one.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// When a lease extended by a request sent at `sent` counts as lost unless extended again, a
/// little before the store may expire it.
fn deadline(sent: Instant, ttl: Duration) -> Instant {
    sent + ttl * 4 / 5
}
