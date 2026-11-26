use std::error::Error as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uima_cancel::CancelToken;
use uima_core::{Failure, FailureKind, PortError, PortResult};
use uima_testing::{assert, check};

use crate::testing::MemoryLockProvider;
use crate::{
    Extended, Lease, Lock, LockClient, LockGuard, LockProvider, TryAcquire, TryLock, Ttl, new,
};

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
}

/// A [`MemoryLockProvider`] that is as slow or failing as set.
#[derive(Clone, Default)]
struct Faulty {
    store: Arc<MemoryLockProvider>,
    extending: Arc<Mutex<Extending>>,
    /// How long a reply to each call takes, after the store has done it.
    replies_in: Arc<Mutex<Duration>>,
    /// How much longer an extend's reply takes.
    extends_take: Arc<Mutex<Duration>>,
    /// How much longer an unlock's reply takes.
    unlocks_take: Arc<Mutex<Duration>>,
    /// When the last extend that held its leases was sent.
    last_extended: Arc<Mutex<Option<Instant>>>,
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
        let extending = *self.extending.lock().unwrap();
        let extended = match extending {
            Extending::Works => {
                *self.last_extended.lock().unwrap() = Some(Instant::now());
                self.store.extend(leases).await
            }
            Extending::Fails => Err(down()),
            Extending::LosesThem => Ok(vec![Extended::Lost; leases.len()]),
        };
        self.reply().await;
        let longer = *self.extends_take.lock().unwrap();
        tokio::time::sleep(longer).await;
        extended
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
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

fn down() -> uima_core::PortError<!> {
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
    match client.try_lock(key, TTL).await {
        Ok(TryLock::Locked(guard)) => guard,
        other => panic!("{key} not locked: {other:?}"),
    }
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

#[tokio::test(start_paused = true)]
async fn try_lock_takes_a_free_key_and_finds_a_held_one_busy() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    let _guard = lock(&client, "purge").await;

    assert!(let Ok(TryLock::Busy) = client.try_lock("purge", TTL).await);
    let _other = lock(&client, "other").await;
}

#[tokio::test(start_paused = true)]
async fn a_held_lock_is_kept_past_its_ttl() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    tokio::time::sleep(TTL.get() * 10).await;

    check!(held(&store, "purge").await);
    check!(!guard.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn release_lets_the_lock_go_before_it_returns() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    assert!(let Ok(()) = guard.release().await);

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_guard_lets_the_lock_go_without_waiting_for_its_ttl() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    let worked: anyhow::Result<()> = async {
        let _guard = lock(&client, "purge").await;
        anyhow::bail!("failed while holding it")
    }
    .await;
    check!(worked.is_err());
    settle().await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_the_store_says_is_lost_cancels_lost() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::LosesThem);

    let lost = tokio::time::timeout(TTL.get(), guard.lost().cancelled()).await;

    check!(lost.is_ok(), "before its ttl ends");
}

#[tokio::test(start_paused = true)]
async fn a_lock_that_fails_to_extend_is_lost_before_its_ttl_ends() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Fails);

    let lost = tokio::time::timeout(TTL.get(), guard.lost().cancelled()).await;

    check!(
        lost.is_ok(),
        "so the holder stops before another can take it"
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_the_driver_cancels_lost_and_lets_each_lock_go_once_its_holder_stops() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let first = lock(&client, "purge").await;
    let second = lock(&client, "report").await;

    cancel.cancel();
    settle().await;
    check!(first.lost().is_cancelled());
    check!(second.lost().is_cancelled());
    check!(
        held(&store, "purge").await,
        "so another can't take it while the holder stops"
    );
    check!(!driving.is_finished());

    assert!(
        let Ok(Ok(())) = tokio::time::timeout(TTL.get() / 2, first.release()).await,
        "once the store has, not once every lock is let go"
    );
    check!(!held(&store, "purge").await, "once released");
    drop(second);
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();
    check!(!held(&store, "report").await, "once dropped");
}

#[tokio::test(start_paused = true)]
async fn a_lock_let_go_once_stopping_gave_up_on_it_holds_up_neither_release_nor_the_driver() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let started = Instant::now();
    let released = lock(&client, "purge").await;
    let dropped = lock(&client, "report").await;
    let longer = Ttl::from_secs(30);
    let TryLock::Locked(_held) = client.try_lock("audit", longer).await.unwrap() else {
        panic!("audit is busy")
    };
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    settle().await;
    tokio::time::sleep(longer.get() - Duration::from_secs(1)).await;
    assert!(
        let Ok(Ok(())) = tokio::time::timeout(TTL.get() / 2, released.release()).await,
        "at once, as it may have expired"
    );
    drop(dropped);
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();

    check!(
        Instant::now() <= started + longer.get(),
        "by when the last lock would expire"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_before_the_driver_runs_is_kept_once_it_does() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    let guard = lock(&client, "purge").await;

    let _driving = tokio::spawn(driver.run(CancelToken::new()));
    tokio::time::sleep(TTL.get() * 10).await;

    check!(held(&store, "purge").await);
    check!(!guard.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn try_lock_fails_once_the_driver_has_stopped_or_been_dropped() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    drop(driver);
    assert!(let Err(error) = client.try_lock("purge", TTL).await, "dropped");
    check!(error.to_string() == "failed to lock purge");
    check!(error.source().unwrap().to_string() == "the lock driver has stopped");
    check!(!held(&store, "purge").await, "and it takes no lock");

    let (client, driver) = new(store.clone());
    let cancel = CancelToken::new();
    cancel.cancel();
    tokio::time::timeout(GIVE_UP, driver.run(cancel))
        .await
        .unwrap();
    assert!(let Err(_) = client.try_lock("purge", TTL).await, "stopped");
    check!(!held(&store, "purge").await);

    let (client, driver) = new(store.clone());
    drop(driver);
    let TryAcquire::Acquired(_) = store.store.try_acquire("purge", TTL).await.unwrap() else {
        panic!("purge is busy");
    };
    assert!(let Err(_) = client.try_lock("purge", TTL).await, "held by another");
}

#[tokio::test(start_paused = true)]
async fn the_driver_runs_until_cancelled_after_every_client_is_dropped() {
    let (client, cancel, driving) = start(&Faulty::default());
    drop(client);

    tokio::time::sleep(GIVE_UP / 2).await;
    check!(
        !driving.is_finished(),
        "so a service running it doesn't stop"
    );

    cancel.cancel();
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_lock_whose_extend_failed_once_is_kept() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    store.extend_and(Extending::Fails);
    tokio::time::sleep(TTL.get() / 2).await;
    store.extend_and(Extending::Works);
    tokio::time::sleep(TTL.get() * 10).await;

    check!(held(&store, "purge").await);
    check!(!guard.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn a_lock_whose_extend_hangs_is_lost_before_its_ttl_ends() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    store.reply_in(GIVE_UP);
    let lost = tokio::time::timeout(TTL.get(), guard.lost().cancelled()).await;

    check!(lost.is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_lock_is_lost_by_its_ttl_from_when_it_was_asked_for() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    store.reply_in(TTL.get() / 2);
    let guard = lock(&client, "purge").await;
    store.reply_in(Duration::ZERO);

    store.extend_and(Extending::Fails);
    let lost = tokio::time::timeout(TTL.get() / 2, guard.lost().cancelled()).await;

    check!(
        lost.is_ok(),
        "since the store's ttl started before its reply"
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_unlock_doesnt_hold_up_extending_the_other_locks() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let dropped = lock(&client, "purge").await;
    let kept = lock(&client, "report").await;

    *store.unlocks_take.lock().unwrap() = GIVE_UP;
    drop(dropped);
    tokio::time::sleep(TTL.get() * 2).await;

    check!(held(&store, "report").await);
    check!(!kept.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn release_gives_the_stores_failure() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    store.unlock_fails.store(true, Ordering::SeqCst);

    assert!(let Err(_) = guard.release().await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_as_the_driver_stops_is_let_go() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    *store.on_acquire.lock().unwrap() = Some(cancel);
    // The reply waits for the driver to stop.
    store.reply_in(Duration::from_millis(1));

    assert!(let Err(_) = client.try_lock("purge", TTL).await);
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_as_the_driver_stops_fails_within_its_ttl_however_slow_the_unlock() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    *store.on_acquire.lock().unwrap() = Some(cancel);
    store.reply_in(Duration::from_millis(1));
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    let locked = tokio::time::timeout(TTL.get() * 2, client.try_lock("purge", TTL)).await;

    assert!(let Ok(Err(_)) = locked);
}

#[tokio::test(start_paused = true)]
async fn stopping_with_a_slow_store_cancels_every_lost_at_once() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let first = lock(&client, "purge").await;
    let second = lock(&client, "report").await;

    store.reply_in(GIVE_UP);
    cancel.cancel();
    settle().await;

    check!(first.lost().is_cancelled());
    check!(second.lost().is_cancelled());
    let ended = tokio::time::timeout(TTL.get() * 2, driving).await;
    check!(
        ended.is_ok(),
        "since an unlock after its lease's ttl would do nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_ends_by_when_the_lock_would_expire_while_its_guard_is_held() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let _guard = lock(&client, "purge").await;
    tokio::time::sleep(TTL.get() / 2).await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();

    let extended = store.last_extended.lock().unwrap().unwrap();
    check!(
        Instant::now() <= extended + TTL.get(),
        "as an unlock after then would do nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_with_a_slow_unlock_ends_by_when_the_lock_would_expire_once_let_go() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    tokio::time::sleep(TTL.get() / 2).await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    settle().await;
    drop(guard);
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();

    let extended = store.last_extended.lock().unwrap().unwrap();
    check!(
        Instant::now() <= extended + TTL.get(),
        "as an unlock after then would do nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn a_driver_ended_early_cancels_lost() {
    let store = Faulty::default();
    let (client, _cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    settle().await;

    driving.abort();
    settle().await;

    check!(
        guard.lost().is_cancelled(),
        "so the holder stops before the lease expires"
    );
    check!(held(&store, "purge").await, "left to expire");
}

#[tokio::test(start_paused = true)]
async fn a_driver_dropped_before_it_got_a_lock_cancels_its_lost() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    let guard = lock(&client, "purge").await;
    drop(driver);
    check!(guard.lost().is_cancelled(), "never run");

    let (client, driver) = new(store.clone());
    let guard = lock(&client, "report").await;
    let driving = tokio::spawn(driver.run(CancelToken::new()));
    driving.abort();
    settle().await;
    check!(guard.lost().is_cancelled(), "aborted before it ran");
}

#[tokio::test(start_paused = true)]
async fn a_lock_extended_slowly_is_lost_by_its_ttl_from_when_the_extend_was_sent() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    *store.extends_take.lock().unwrap() = TTL.get() / 8;
    tokio::time::sleep(TTL.get()).await;
    check!(!guard.lost().is_cancelled(), "kept while extends answer");

    store.extend_and(Extending::Fails);
    tokio::time::timeout(GIVE_UP, guard.lost().cancelled())
        .await
        .unwrap();

    let extended = store.last_extended.lock().unwrap().unwrap();
    check!(
        Instant::now() <= extended + TTL.get() * 4 / 5,
        "a little before the store lets another take it"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lock_given_up_is_let_go_once_its_holder_stops() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    *store.extends_take.lock().unwrap() = GIVE_UP;
    tokio::time::timeout(TTL.get(), guard.lost().cancelled())
        .await
        .unwrap();
    settle().await;
    check!(
        held(&store, "purge").await,
        "so another can't take it while the holder stops"
    );

    drop(guard);
    settle().await;
    check!(!held(&store, "purge").await, "before its ttl ends");
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_too_slowly_to_keep_fails_and_is_let_go() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    store.reply_in(TTL.get() * 9 / 10);

    assert!(let Err(PortError::Failed(failure)) = client.try_lock("purge", TTL).await);
    check!(failure.kind() == FailureKind::Timeout);
    store.reply_in(Duration::ZERO);
    settle().await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_guard_dropped_after_every_client_lets_the_lock_go() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    drop(client);
    drop(guard);
    settle().await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn try_lock_fails_while_the_driver_is_stopping() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    let _guard = lock(&client, "purge").await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    settle().await;

    assert!(let Err(_) = client.try_lock("report", TTL).await);
}

#[tokio::test(start_paused = true)]
async fn try_lock_gives_the_stores_failure() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    store.acquire_fails.store(true, Ordering::SeqCst);

    assert!(let Err(_) = client.try_lock("purge", TTL).await);
}

#[tokio::test(start_paused = true)]
async fn release_of_a_lost_lock_is_not_an_error() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    store.extend_and(Extending::LosesThem);
    guard.lost().cancelled().await;

    assert!(let Ok(()) = guard.release().await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_whose_extend_hung_once_is_kept() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    *store.extends_take.lock().unwrap() = GIVE_UP;
    // Past the first extend, which hangs.
    tokio::time::sleep(TTL.get() / 3 + Duration::from_millis(10)).await;
    *store.extends_take.lock().unwrap() = Duration::ZERO;
    tokio::time::sleep(TTL.get() * 2).await;

    check!(held(&store, "purge").await);
    check!(!guard.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn a_hung_extend_doesnt_hold_up_extending_the_locks_taken_after_it() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let first = lock(&client, "purge").await;

    *store.extends_take.lock().unwrap() = GIVE_UP;
    first.lost().cancelled().await;
    *store.extends_take.lock().unwrap() = Duration::ZERO;
    let later = lock(&client, "report").await;
    tokio::time::sleep(TTL.get() * 2).await;

    check!(held(&store, "report").await);
    check!(!later.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn a_hung_extend_of_locks_of_different_ttls_is_retried_in_time_for_the_shortest() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    *store.extends_take.lock().unwrap() = TTL.get() / 8;
    let longer = match client.try_lock("report", Ttl::from_secs(6)).await {
        Ok(TryLock::Locked(guard)) => guard,
        other => panic!("report not locked: {other:?}"),
    };
    tokio::time::sleep(TTL.get() / 3).await;
    // Due for its first extend with the longer lock's, as one batch.
    let shorter = lock(&client, "purge").await;

    tokio::time::sleep(TTL.get() / 6).await;
    *store.extends_take.lock().unwrap() = GIVE_UP;
    // Past the batch, which hangs, and its wait for the shorter ttl.
    tokio::time::sleep(TTL.get() * 11 / 30).await;
    *store.extends_take.lock().unwrap() = TTL.get() / 8;
    tokio::time::sleep(TTL.get()).await;

    check!(!shorter.lost().is_cancelled());
    check!(!longer.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn release_is_not_an_error_when_the_driver_ends_before_answering() {
    let store = Faulty::default();
    let (client, _cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    let released = tokio::spawn(guard.release());
    settle().await;
    driving.abort();

    assert!(let Ok(Ok(())) = tokio::time::timeout(GIVE_UP, released).await.unwrap());
}

/// Generic code can use the port and run the driver on any provider.
#[expect(dead_code, reason = "checked by compiling")]
fn generic_callers_compile<P: LockProvider, C: crate::HasLock + Send + Sync + 'static>(
    provider: P,
    context: Arc<C>,
) {
    let (_client, driver) = new(provider);
    tokio::spawn(driver.run(CancelToken::new()));
    tokio::spawn(async move {
        let lock = context.lock();
        if let Ok(TryLock::Locked(guard)) = lock.try_lock("purge", TTL).await {
            tokio::task::yield_now().await;
            drop(guard);
        }
    });
}
