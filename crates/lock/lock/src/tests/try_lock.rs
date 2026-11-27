use uima_testing::assert;

use super::*;

#[tokio::test(start_paused = true)]
async fn try_lock_takes_a_free_key_and_finds_a_held_one_busy() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    let _guard = lock(&client, "purge").await;

    assert!(let Ok(TryLock::Busy) = client.try_lock("purge", TTL).await);
    assert!(let Ok(TryLock::Busy) = client.clone().try_lock("purge", TTL).await, "from a clone");
    let _other = lock(&client, "other").await;
}

#[tokio::test(start_paused = true)]
async fn try_lock_fails_once_the_driver_has_stopped_or_been_dropped() {
    let store = Faulty::default();
    let (client, driver) = new(store.clone());
    drop(driver);
    assert!(let Err(error) = client.try_lock("purge", TTL).await, "dropped");
    assert!(let PortError::Failed(failure) = &error);
    check!(failure.kind() == FailureKind::Unavailable);
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
    assert!(let Ok(TryAcquire::Acquired(_)) = store.store.try_acquire("purge", TTL).await);
    assert!(let Err(_) = client.try_lock("purge", TTL).await, "held by another");
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_as_the_driver_stops_for_another_held_fails_and_is_let_go() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    let _other = lock(&client, "report").await;
    *store.on_acquire.lock().unwrap() = Some(cancel);
    store.reply_in(Duration::from_millis(1));

    assert!(let Err(_) = client.try_lock("purge", TTL).await);

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_as_the_driver_stops_fails_within_its_ttl_however_slow_the_unlock() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    *store.on_acquire.lock().unwrap() = Some(cancel);
    store.reply_in(Duration::from_millis(1));
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    let within = TTL.get() + Duration::from_millis(100);
    let locked = tokio::time::timeout(within, client.try_lock("purge", TTL)).await;

    assert!(let Ok(Err(_)) = locked);
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_too_slowly_to_keep_fails_and_is_let_go() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    store.reply_in(TTL.get() * 9 / 10);
    *store.unlocks_land_in.lock().unwrap() = Duration::from_millis(100);

    assert!(let Err(PortError::Failed(failure)) = client.try_lock("purge", TTL).await);
    check!(failure.kind() == FailureKind::Timeout);
    store.reply_in(Duration::ZERO);
    settle().await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_try_lock_fails_to_let_go_still_gives_why_it_failed() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    store.unlock_fails.store(true, Ordering::SeqCst);

    store.reply_in(TTL.get() * 9 / 10);
    assert!(let Err(error) = client.try_lock("purge", TTL).await, "too slow");
    assert!(let PortError::Failed(failure) = &error);
    check!(failure.kind() == FailureKind::Timeout);
    check!(error.to_string() == "failed to lock purge");

    *store.on_acquire.lock().unwrap() = Some(cancel);
    store.reply_in(Duration::from_millis(1));
    assert!(let Err(error) = client.try_lock("report", TTL).await, "stopped");
    assert!(let PortError::Failed(failure) = &error);
    check!(failure.kind() == FailureKind::Unavailable);
    check!(error.to_string() == "failed to lock report");
}

#[tokio::test(start_paused = true)]
async fn try_lock_gives_the_stores_failure() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);

    store.acquire_fails.store(true, Ordering::SeqCst);

    assert!(let Err(_) = client.try_lock("purge", TTL).await);
}
