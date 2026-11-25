use std::sync::Mutex;

use tokio::task::JoinHandle;
use uima_testing::{assert, check};

use super::*;
use crate::testing::MemoryLockProvider;
use crate::{Extended, TryAcquire};

const TTL: Duration = Duration::from_secs(3);

/// Longer than any test waits, so one that never sees its event fails instead of hanging.
const GIVE_UP: Duration = Duration::from_secs(3600);

/// How a [`Faulty`] store extends leases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Extending {
    Works,
    Fails,
    LosesThem,
}

/// A [`MemoryLockProvider`] whose extends go as `extending` says.
#[derive(Clone, Default)]
struct Faulty {
    store: Arc<MemoryLockProvider>,
    extending: Arc<Mutex<Option<Extending>>>,
}

impl Faulty {
    fn extend_and(&self, extending: Extending) {
        *self.extending.lock().unwrap() = Some(extending);
    }
}

impl LockProvider for Faulty {
    async fn try_acquire(&self, key: &str, ttl: Duration) -> PortResult<TryAcquire> {
        self.store.try_acquire(key, ttl).await
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        let extending = self.extending.lock().unwrap().unwrap_or(Extending::Works);
        match extending {
            Extending::Works => self.store.extend(leases).await,
            Extending::Fails => Err(uima_core::PortError::Failed(uima_core::Failure::new(
                uima_core::FailureKind::Unavailable,
                anyhow::anyhow!("down"),
            ))),
            Extending::LosesThem => Ok(vec![Extended::Lost; leases.len()]),
        }
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        self.store.unlock(lease).await
    }
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
async fn try_lock_rejects_a_zero_ttl() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    assert!(let Err(_) = client.try_lock("purge", Duration::ZERO).await);
    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_held_lock_is_kept_past_its_ttl() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;

    tokio::time::sleep(TTL * 10).await;

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

    let lost = tokio::time::timeout(TTL, guard.lost().cancelled()).await;

    check!(lost.is_ok(), "before its ttl ends");
}

#[tokio::test(start_paused = true)]
async fn a_lock_that_fails_to_extend_is_lost_before_its_ttl_ends() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Fails);

    let lost = tokio::time::timeout(TTL, guard.lost().cancelled()).await;

    check!(
        lost.is_ok(),
        "so the holder stops before another can take it"
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_the_driver_lets_every_lock_go_and_cancels_lost() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let first = lock(&client, "purge").await;
    let second = lock(&client, "report").await;

    cancel.cancel();
    tokio::time::timeout(GIVE_UP, driving)
        .await
        .unwrap()
        .unwrap();

    check!(first.lost().is_cancelled());
    check!(second.lost().is_cancelled());
    check!(!held(&store, "purge").await);
    check!(!held(&store, "report").await);
    assert!(let Ok(()) = first.release().await, "it is let go already");
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_before_the_driver_runs_is_kept_once_it_does() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    let guard = lock(&client, "purge").await;

    let _driving = tokio::spawn(driver.run(CancelToken::new()));
    tokio::time::sleep(TTL * 10).await;

    check!(held(&store, "purge").await);
    check!(!guard.lost().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn try_lock_fails_once_the_driver_has_stopped_or_been_dropped() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    drop(driver);
    assert!(let Err(_) = client.try_lock("purge", TTL).await, "dropped");
    check!(!held(&store, "purge").await, "and it takes no lock");

    let (client, driver) = new(store.clone());
    let cancel = CancelToken::new();
    cancel.cancel();
    tokio::time::timeout(GIVE_UP, driver.run(cancel))
        .await
        .unwrap();
    assert!(let Err(_) = client.try_lock("purge", TTL).await, "stopped");
    check!(!held(&store, "purge").await);
}
