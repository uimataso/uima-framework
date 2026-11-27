use uima_testing::assert;

use super::*;

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
async fn a_lock_the_store_says_is_lost_cancels_lost_and_is_released_without_error() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::LosesThem);

    let lost = tokio::time::timeout(TTL.get(), guard.lost().cancelled()).await;

    check!(lost.is_ok(), "before its ttl ends");
    assert!(let Ok(()) = guard.release().await);
}

#[tokio::test(start_paused = true)]
async fn a_lock_that_fails_to_extend_is_lost_before_its_ttl_ends() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let taken = Instant::now();
    let guard = lock(&client, "purge").await;
    store.extend_and(Extending::Fails);

    lost(&guard).await;

    check!(
        Instant::now() <= taken + TTL.get() * 4 / 5,
        "with a fifth of it left, so the holder stops before another can take it"
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
async fn a_lock_taken_slowly_is_extended_in_time() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    store.reply_in(TTL.get() / 2);
    let guard = lock(&client, "purge").await;
    store.reply_in(Duration::ZERO);

    tokio::time::sleep(TTL.get() * 3).await;

    check!(!guard.lost().is_cancelled());
    check!(held(&store, "purge").await);
}

#[tokio::test(start_paused = true)]
async fn an_extend_answered_for_another_number_of_locks_fails_like_one_that_errs() {
    for answers in [Extending::AnswersNone, Extending::AnswersMore] {
        let store = Faulty::default();
        let (client, _cancel, _driving) = start(&store);
        let guard = lock(&client, "purge").await;
        store.extend_and(answers);

        lost(&guard).await;

        let sent = store.extends_at.lock().unwrap().clone();
        check!(sent.len() > 1, "{answers:?}");
        check!(
            sent.iter()
                .zip(sent.iter().skip(1))
                .all(|(sent, next)| *next - *sent == TTL.get() / 10),
            "retried a tenth of a ttl apart: {answers:?}"
        );
    }
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
    lost(&guard).await;

    let extended = store.last_extended.lock().unwrap().unwrap();
    check!(
        Instant::now() <= extended + TTL.get() * 4 / 5,
        "a little before the store lets another take it"
    );
}

#[tokio::test(start_paused = true)]
async fn locks_taken_apart_are_extended_together() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let _first = lock(&client, "purge").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _second = lock(&client, "report").await;

    tokio::time::sleep(TTL.get() * 3).await;

    let extends = store.extends.lock().unwrap().clone();
    check!(!extends.is_empty());
    check!(
        extends.iter().all(|keys| keys.len() == 2),
        "in one call to the store"
    );
}

#[tokio::test(start_paused = true)]
async fn a_lock_of_a_longer_ttl_is_extended_along_with_a_shorter_one() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let short = lock(&client, "purge").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let long = lock_for(&client, "report", Ttl::from_secs(30)).await;

    tokio::time::sleep(Duration::from_secs(60)).await;

    let extends = store.extends.lock().unwrap().clone();
    check!(
        extends.iter().all(|keys| keys.contains(&key("purge"))),
        "so it takes no call of its own"
    );
    let longer = extends.iter().filter(|keys| keys.contains(&key("report")));
    check!(
        longer.count() <= 60 / 5 + 1,
        "nor is extended more than twice as often as it would be on its own"
    );
    check!(!short.lost().is_cancelled());
    check!(!long.lost().is_cancelled());
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
    lost(&first).await;
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
    let longer = lock_for(&client, "report", Ttl::from_secs(6)).await;
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
async fn a_lock_being_extended_isnt_sent_again_with_one_that_comes_due() {
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let _first = lock(&client, "purge").await;
    *store.extends_take.lock().unwrap() = GIVE_UP;
    tokio::time::sleep(Duration::from_millis(900)).await;
    // Due while the first lock's extend, sent at a third of its ttl, is still out.
    let _second = lock_for(&client, "report", Ttl::MIN).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let extends = store.extends.lock().unwrap().clone();
    check!(extends == [[key("purge")], [key("report")]]);
}

/// On the real clock, as it takes holding up the driver's thread, which the paused clock can't.
#[tokio::test]
async fn a_lock_extended_in_time_is_kept_by_a_driver_that_ran_late() {
    let ttl = Ttl::MIN;
    let store = Faulty::default();
    let (client, _cancel, _driving) = start(&store);
    let guard = lock_for(&client, "purge", ttl).await;
    // The extend sent at a third of the ttl holds the lease, and is answered soon after.
    *store.extends_take.lock().unwrap() = ttl.get() / 10;
    tokio::time::sleep(ttl.get() * 2 / 5).await;

    // Past the deadline the lease had before that extend, with its reply unread.
    std::thread::sleep(ttl.get() / 2);
    tokio::time::sleep(ttl.get() / 20).await;

    check!(
        !guard.lost().is_cancelled(),
        "as the store extended it before its deadline"
    );
}
