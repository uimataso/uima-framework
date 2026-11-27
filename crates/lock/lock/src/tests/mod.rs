use std::error::Error as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortError, PortResult};
use uima_testing::check;

use crate::testing::MemoryLockProvider;
use crate::{
    Extended, Lease, Lock, LockClient, LockGuard, LockProvider, TryAcquire, TryLock, Ttl, new,
};

mod extending;
mod letting_go;
mod stopping;
mod try_lock;

const TTL: Ttl = Ttl::from_secs(3);

/// Longer than any test waits, so one that never sees its event fails instead of hanging.
const GIVE_UP: Duration = Duration::from_secs(3600);

/// How a [`Faulty`] store extends leases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Extending {
    #[default]
    Works,
    Fails,
    LosesThem,
    /// Answers for none of the leases.
    AnswersNone,
    /// Answers for one lease more than it was sent.
    AnswersMore,
    Panics,
}

/// A [`MemoryLockProvider`] that is as slow or failing as set.
#[derive(Clone, Default)]
struct Faulty {
    store: Arc<MemoryLockProvider>,
    extending: Arc<Mutex<Extending>>,
    /// How long a reply to each call takes, after the store has done it.
    replies_in: Arc<Mutex<Duration>>,
    /// How long an acquire takes to reach the store.
    acquires_land_in: Arc<Mutex<Duration>>,
    /// How long an extend takes to reach the store.
    extends_land_in: Arc<Mutex<Duration>>,
    /// How much longer an extend's reply takes.
    extends_take: Arc<Mutex<Duration>>,
    /// How long an unlock takes to reach the store.
    unlocks_land_in: Arc<Mutex<Duration>>,
    /// How much longer an unlock's reply takes.
    unlocks_take: Arc<Mutex<Duration>>,
    /// When the last extend that held its leases was sent.
    last_extended: Arc<Mutex<Option<Instant>>>,
    /// The keys of each extend that reached the store.
    extends: Arc<Mutex<Vec<Vec<String>>>>,
    /// When each of them did.
    extends_at: Arc<Mutex<Vec<Instant>>>,
    acquire_fails: Arc<AtomicBool>,
    unlock_fails: Arc<AtomicBool>,
    /// Cancelled as a lock is taken, to stop the driver then.
    on_acquire: Arc<Mutex<Option<CancelToken>>>,
}

impl Faulty {
    fn extend_and(&self, extending: Extending) {
        *self.extending.lock().unwrap() = extending;
    }

    fn reply_in(&self, after: Duration) {
        *self.replies_in.lock().unwrap() = after;
    }

    async fn reply(&self) {
        let after = *self.replies_in.lock().unwrap();
        tokio::time::sleep(after).await;
    }
}

impl LockProvider for Faulty {
    async fn try_acquire(&self, key: &str, ttl: Ttl) -> PortResult<TryAcquire> {
        let landing = *self.acquires_land_in.lock().unwrap();
        tokio::time::sleep(landing).await;
        let acquired = if self.acquire_fails.load(Ordering::SeqCst) {
            Err(down())
        } else {
            self.store.try_acquire(key, ttl).await
        };
        let stop = self.on_acquire.lock().unwrap().take();
        if let Some(stop) = stop {
            stop.cancel();
        }
        self.reply().await;
        acquired
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        let landing = *self.extends_land_in.lock().unwrap();
        tokio::time::sleep(landing).await;
        let keys = leases.iter().map(|lease| lease.key.clone()).collect();
        self.extends.lock().unwrap().push(keys);
        self.extends_at.lock().unwrap().push(Instant::now());
        let extending = *self.extending.lock().unwrap();
        let extended = match extending {
            Extending::Works => {
                *self.last_extended.lock().unwrap() = Some(Instant::now());
                self.store.extend(leases).await
            }
            Extending::Fails => Err(down()),
            Extending::LosesThem => Ok(vec![Extended::Lost; leases.len()]),
            Extending::AnswersNone => Ok(Vec::new()),
            Extending::AnswersMore => self.store.extend(leases).await.map(|mut extended| {
                extended.push(Extended::Held);
                extended
            }),
            Extending::Panics => panic!("the store panicked"),
        };
        self.reply().await;
        let longer = *self.extends_take.lock().unwrap();
        tokio::time::sleep(longer).await;
        extended
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        let landing = *self.unlocks_land_in.lock().unwrap();
        tokio::time::sleep(landing).await;
        let unlocked = if self.unlock_fails.load(Ordering::SeqCst) {
            Err(down())
        } else {
            self.store.unlock(lease).await
        };
        self.reply().await;
        let longer = *self.unlocks_take.lock().unwrap();
        tokio::time::sleep(longer).await;
        unlocked
    }
}

fn down() -> PortError<!> {
    Failure::new(FailureKind::Unavailable, anyhow!("down")).into()
}

/// A client over `store`, with its driver running until the token is cancelled.
fn start(store: &Faulty) -> (LockClient<Faulty>, CancelToken, JoinHandle<()>) {
    let (client, driver) = new(store.clone());
    let cancel = CancelToken::new();
    let driving = tokio::spawn(driver.run(cancel.clone()));
    (client, cancel, driving)
}

/// The guard `try_lock` gives on `key`, failing when it is busy.
async fn lock(client: &impl Lock, key: &str) -> LockGuard {
    lock_for(client, key, TTL).await
}

/// As [`lock`], for a `ttl` of the test's own.
async fn lock_for(client: &impl Lock, key: &str, ttl: Ttl) -> LockGuard {
    match client.try_lock(key, ttl).await {
        Ok(TryLock::Locked(guard)) => guard,
        other => panic!("{key} not locked: {other:?}"),
    }
}

/// Waits until `guard`'s lock is lost.
async fn lost(guard: &LockGuard) {
    tokio::time::timeout(GIVE_UP, guard.lost().cancelled())
        .await
        .expect("the lock is lost");
}

/// Waits until the driver has ended.
async fn ended(driving: JoinHandle<()>) {
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .expect("the driver ends")
        .unwrap();
}

/// Whether the store holds `key`, asked without the client.
async fn held(store: &Faulty, key: &str) -> bool {
    match store.store.try_acquire(key, TTL).await.unwrap() {
        TryAcquire::Busy => true,
        TryAcquire::Acquired(lease) => {
            store.store.unlock(&lease).await.unwrap();
            false
        }
    }
}

/// Lets the driver handle what it was sent, on the paused clock.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

/// Generic code can use the port, clone the client and run the driver, on any provider.
#[expect(dead_code, reason = "checked by compiling")]
fn generic_callers_compile<P, C, L>(provider: P, context: Arc<C>, lock: Arc<L>)
where
    P: LockProvider,
    C: crate::HasLock + Send + Sync + 'static,
    L: Lock + 'static,
{
    let (client, driver) = new(provider);
    tokio::spawn(driver.run(CancelToken::new()));
    drop(client.clone());
    tokio::spawn(async move {
        let port = context.lock();
        if let Ok(TryLock::Locked(guard)) = port.try_lock("purge", TTL).await {
            tokio::task::yield_now().await;
            drop(guard);
        }
    });
    tokio::spawn(async move { lock.try_lock("purge", TTL).await.is_ok() });
}
