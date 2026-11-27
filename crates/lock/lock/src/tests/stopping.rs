use uima_testing::assert;

use super::*;

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
    ended(driving).await;
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
    let _held = lock_for(&client, "audit", longer).await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    settle().await;
    tokio::time::sleep(longer.get() - Duration::from_secs(1)).await;
    assert!(
        let Ok(Ok(())) = tokio::time::timeout(TTL.get() / 2, released.release()).await,
        "at once, as it may have expired"
    );
    drop(dropped);
    ended(driving).await;

    check!(
        Instant::now() <= started + longer.get(),
        "by when the last lock would expire"
    );
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
    ended(driving).await;
}

#[tokio::test(start_paused = true)]
async fn stopping_with_a_slow_store_cancels_every_lost_at_once() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let first = lock(&client, "purge").await;
    let second = lock(&client, "report").await;

    store.reply_in(GIVE_UP);
    // Past the first extend, which hangs.
    tokio::time::sleep(TTL.get() / 3 + Duration::from_millis(1)).await;
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

    cancel.cancel();
    ended(driving).await;

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
    ended(driving).await;

    let extended = store.last_extended.lock().unwrap().unwrap();
    check!(
        Instant::now() <= extended + TTL.get(),
        "as an unlock after then would do nothing"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lock_extended_as_the_driver_stops_is_let_go_once_dropped() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    // The first extend, sent at a third of the ttl, reaches the store late, and its reply
    // hangs.
    *store.extends_land_in.lock().unwrap() = Duration::from_millis(400);
    *store.extends_take.lock().unwrap() = GIVE_UP;
    tokio::time::sleep(TTL.get() / 3 + Duration::from_millis(450)).await;

    cancel.cancel();
    settle().await;
    // Past when it would expire but for that extend, had it reached the store as sent.
    tokio::time::sleep(TTL.get() - Duration::from_millis(250)).await;
    check!(held(&store, "purge").await, "as the extend kept it");
    drop(guard);
    ended(driving).await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_kept_a_while_is_let_go_once_dropped_after_the_driver_stops() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    tokio::time::sleep(TTL.get() + Duration::from_millis(100)).await;

    cancel.cancel();
    settle().await;
    // Past when it would expire but for its extends.
    tokio::time::sleep(TTL.get() / 2).await;
    drop(guard);
    ended(driving).await;

    check!(!held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn stopping_with_a_lock_being_extended_ends_a_sixth_of_its_ttl_later() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let _guard = lock(&client, "purge").await;
    *store.extends_take.lock().unwrap() = GIVE_UP;
    tokio::time::sleep(TTL.get() / 3 + Duration::from_millis(1)).await;

    let cancelled = Instant::now();
    cancel.cancel();
    ended(driving).await;

    check!(Instant::now() <= cancelled + TTL.get() + TTL.get() / 6);
}

#[tokio::test(start_paused = true)]
async fn a_lock_taken_slowly_is_let_go_once_dropped_after_the_driver_stops() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    *store.acquires_land_in.lock().unwrap() = TTL.get() / 3;
    let asked = Instant::now();
    let guard = lock(&client, "purge").await;

    cancel.cancel();
    settle().await;
    tokio::time::sleep(TTL.get() - Duration::from_millis(100)).await;
    check!(
        Instant::now() >= asked + TTL.get(),
        "past when it would expire had the store taken it as asked"
    );
    drop(guard);
    ended(driving).await;

    check!(!held(&store, "purge").await);
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
async fn a_store_call_that_panics_ends_the_driver_and_cancels_lost() {
    let store = Faulty::default();
    let (client, _cancel, driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Panics);

    lost(&guard).await;
    let ended = tokio::time::timeout(GIVE_UP, driving)
        .await
        .expect("the driver ends");

    check!(ended.is_err_and(|error| error.is_panic()), "passing it on");
    assert!(let Err(_) = client.try_lock(&key("report"), TTL).await);
}

#[tokio::test(start_paused = true)]
async fn an_extend_sent_as_the_driver_stops_doesnt_reach_the_store_after() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    let _guard = lock(&client, "purge").await;
    *store.extends_land_in.lock().unwrap() = Duration::from_millis(400);
    // Past the first extend being sent, but not past it reaching the store.
    tokio::time::sleep(TTL.get() / 3 + Duration::from_millis(100)).await;

    cancel.cancel();
    tokio::time::sleep(TTL.get()).await;

    check!(
        store.extends.lock().unwrap().is_empty(),
        "so the lock isn't kept longer for a holder told to stop"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lock_is_let_go_once_dropped_after_a_shorter_one_expired_as_the_driver_stops() {
    let store = Faulty::default();
    let (client, cancel, driving) = start(&store);
    let _shorter = lock(&client, "purge").await;
    let longer = lock_for(&client, "report", Ttl::from_secs(30)).await;

    cancel.cancel();
    tokio::time::sleep(TTL.get() + Duration::from_millis(100)).await;
    check!(!driving.is_finished(), "while the longer one may be held");
    drop(longer);
    ended(driving).await;

    check!(!held(&store, "report").await);
}

#[tokio::test(start_paused = true)]
async fn release_of_a_lock_that_may_have_expired_doesnt_wait_for_another_being_let_go() {
    let store = Faulty::default();
    let (client, cancel, _driving) = start(&store);
    let shorter = lock(&client, "purge").await;
    let longer = lock_for(&client, "report", Ttl::from_secs(30)).await;
    *store.unlocks_take.lock().unwrap() = GIVE_UP;

    cancel.cancel();
    settle().await;
    drop(longer);
    tokio::time::sleep(TTL.get() + Duration::from_millis(100)).await;
    let released = tokio::time::timeout(TTL.get(), shorter.release()).await;

    assert!(let Ok(Ok(())) = released, "at once, as there is nothing to let go");
}
