use uima_testing::assert;

use super::*;

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

    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Fails);
    lost(&guard).await;
    store.unlock_fails.store(true, Ordering::SeqCst);
    assert!(let Err(_) = guard.release().await, "given up on, as the store may still hold it");
}

#[tokio::test(start_paused = true)]
async fn release_fails_once_the_store_answers_too_slowly() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    let asked = Instant::now();
    let released = tokio::time::timeout(GIVE_UP, guard.release()).await;

    assert!(let Ok(Err(PortError::Failed(failure))) = released);
    check!(failure.kind() == FailureKind::Timeout);
    check!(
        Instant::now() <= asked + TTL.get(),
        "by when the lock may have expired"
    );
}

#[tokio::test(start_paused = true)]
async fn releasing_a_lock_given_up_on_once_it_may_have_expired_does_nothing() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Fails);
    lost(&guard).await;
    tokio::time::sleep(TTL.get() * 2).await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    let released = tokio::time::timeout(TTL.get() / 2, guard.release()).await;

    assert!(let Ok(Ok(())) = released, "at once, as there is nothing to let go");
}

#[tokio::test(start_paused = true)]
async fn release_after_the_driver_ended_is_not_an_error() {
    let store = Faulty::default();
    let (client, _cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    driving.abort();
    settle().await;

    assert!(let Ok(()) = guard.release().await);
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

    *store.unlocks_land_in.lock().unwrap() = Duration::from_millis(100);
    drop(guard);
    tokio::time::sleep(Duration::from_millis(200)).await;
    check!(!held(&store, "purge").await, "before its ttl ends");
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
