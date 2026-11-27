//! The cases a [`LockProvider`] passes, run by each adapter.
//!
//! Cases that wait for a lease to expire do so with tokio's clock, so on a store that expires them
//! by its own clock they take a few seconds each.

use std::time::Duration;

use uima_core::Uuid;
use uima_testing::{assert, check, run_case};

use super::*;

/// The `ttl` the cases take locks for.
const TTL: Ttl = Ttl::MIN;

/// Long enough after [`TTL`] that a lease taken for it has expired.
const EXPIRED: Duration = Duration::from_millis(1500);

/// Runs every case, each on its own provider from `isolated`.
pub async fn run_all<P: LockProvider>(isolated: impl AsyncFn() -> P) {
    run_case(&isolated, acquire_takes_a_free_key).await;
    run_case(&isolated, acquire_takes_the_longest_key).await;
    run_case(&isolated, acquire_on_a_held_key_is_busy).await;
    run_case(&isolated, a_busy_acquire_leaves_the_lease_to_expire).await;
    run_case(&isolated, acquire_at_once_takes_a_key_once).await;
    run_case(&isolated, keys_are_locked_apart).await;
    run_case(&isolated, unlock_frees_the_key).await;
    run_case(&isolated, an_expired_lease_can_be_taken).await;
    run_case(&isolated, a_ttl_is_not_rounded_to_whole_seconds).await;
    run_case(&isolated, extend_keeps_a_lease_past_its_ttl).await;
    run_case(&isolated, an_extended_lease_expires_a_ttl_after_the_extend).await;
    run_case(&isolated, extends_at_once_keep_a_lease_for_one_ttl).await;
    run_case(&isolated, extend_keeps_each_lease_for_its_own_ttl).await;
    run_case(&isolated, extend_tells_each_lease_in_order).await;
    run_case(&isolated, extend_after_unlock_is_lost).await;
    run_case(&isolated, extend_of_an_expired_lease_is_lost).await;
    run_case(&isolated, extend_of_a_lost_lease_leaves_the_new_holders_ttl).await;
    run_case(&isolated, unlock_of_a_lost_lease_leaves_the_new_holder).await;
    run_case(&isolated, unlock_of_an_expired_lease_is_not_an_error).await;
}

pub async fn acquire_takes_a_free_key(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;

    check!(lease.key == key);
    check!(lease.ttl == TTL);
    Ok(())
}

pub async fn acquire_takes_the_longest_key(provider: &impl LockProvider) -> anyhow::Result<()> {
    let mut key = key().as_str().to_owned();
    while key.len() < LockKey::MAX_LEN {
        key.push('k');
    }
    let key = LockKey::new(key)?;
    let lease = acquire(provider, &key, TTL).await?;

    check!(lease.key == key);
    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
    Ok(())
}

pub async fn acquire_on_a_held_key_is_busy(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    acquire(provider, &key, TTL).await?;

    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
    Ok(())
}

pub async fn a_busy_acquire_leaves_the_lease_to_expire(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let key = key();
    acquire(provider, &key, TTL).await?;
    tokio::time::sleep(TTL.get() / 2).await;
    provider.try_acquire(&key, TTL).await?;

    // Past the lease's ttl, but not past one from when the key was asked for again.
    tokio::time::sleep(TTL.get() * 7 / 10).await;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn acquire_at_once_takes_a_key_once(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    let acquire = || provider.try_acquire(&key, TTL);
    for state in ["free", "expired"] {
        let got = tokio::join!(acquire(), acquire(), acquire(), acquire());
        let got = [got.0?, got.1?, got.2?, got.3?];

        let acquired = got.iter().filter(|got| *got != &TryAcquire::Busy).count();
        check!(acquired == 1, "{state}");
        tokio::time::sleep(EXPIRED).await;
    }
    Ok(())
}

pub async fn keys_are_locked_apart(provider: &impl LockProvider) -> anyhow::Result<()> {
    acquire(provider, &key(), TTL).await?;

    acquire(provider, &key(), TTL).await?;
    Ok(())
}

pub async fn unlock_frees_the_key(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;
    provider.unlock(&lease).await?;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn an_expired_lease_can_be_taken(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    acquire(provider, &key, TTL).await?;
    tokio::time::sleep(EXPIRED).await;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn a_ttl_is_not_rounded_to_whole_seconds(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let ttl = Ttl::new(Duration::from_millis(2500))?;
    // Past its whole seconds, but not its ttl.
    let past_whole_seconds = Duration::from_millis(2200);
    let key = key();
    let lease = acquire(provider, &key, ttl).await?;
    tokio::time::sleep(past_whole_seconds).await;
    check!(
        provider.extend(std::slice::from_ref(&lease)).await? == [Extended::Held],
        "late in its ttl"
    );

    tokio::time::sleep(past_whole_seconds).await;
    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
    Ok(())
}

pub async fn extend_keeps_a_lease_past_its_ttl(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;
    for _ in 0..3 {
        tokio::time::sleep(TTL.get() / 2).await;
        check!(provider.extend(std::slice::from_ref(&lease)).await? == [Extended::Held]);
    }

    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
    Ok(())
}

pub async fn an_extended_lease_expires_a_ttl_after_the_extend(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;
    tokio::time::sleep(TTL.get() / 2).await;
    provider.extend(std::slice::from_ref(&lease)).await?;

    tokio::time::sleep(EXPIRED).await;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn extends_at_once_keep_a_lease_for_one_ttl(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;
    let extend = || provider.extend(std::slice::from_ref(&lease));
    let extended = tokio::join!(extend(), extend(), extend());
    check!([extended.0?, extended.1?, extended.2?].concat() == [Extended::Held; 3]);

    tokio::time::sleep(EXPIRED).await;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn extend_keeps_each_lease_for_its_own_ttl(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let short = acquire(provider, &key(), Ttl::from_secs(2)).await?;
    let long = acquire(provider, &key(), Ttl::from_secs(4)).await?;
    tokio::time::sleep(EXPIRED).await;
    let extended = provider.extend(&[short.clone(), long.clone()]).await?;
    check!(extended == [Extended::Held, Extended::Held]);
    // Past the short one's ttl from the extend, but not the long one's.
    tokio::time::sleep(Duration::from_millis(2250)).await;

    check!(provider.try_acquire(&long.key, TTL).await? == TryAcquire::Busy);
    check!(let TryAcquire::Acquired(_) = provider.try_acquire(&short.key, TTL).await?);
    Ok(())
}

pub async fn extend_tells_each_lease_in_order(provider: &impl LockProvider) -> anyhow::Result<()> {
    let long = Ttl::from_secs(10);
    let first = acquire(provider, &key(), long).await?;
    let lost = acquire(provider, &key(), TTL).await?;
    let last = acquire(provider, &key(), long).await?;
    tokio::time::sleep(EXPIRED).await;
    acquire(provider, &lost.key, TTL).await?;

    let extended = provider.extend(&[first, lost, last]).await?;
    check!(extended == [Extended::Held, Extended::Lost, Extended::Held]);
    Ok(())
}

pub async fn extend_after_unlock_is_lost(provider: &impl LockProvider) -> anyhow::Result<()> {
    let lease = acquire(provider, &key(), TTL).await?;
    provider.unlock(&lease).await?;

    check!(provider.extend(std::slice::from_ref(&lease)).await? == [Extended::Lost]);
    Ok(())
}

pub async fn extend_of_an_expired_lease_is_lost(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let lease = acquire(provider, &key(), TTL).await?;
    tokio::time::sleep(EXPIRED).await;

    check!(provider.extend(std::slice::from_ref(&lease)).await? == [Extended::Lost]);
    Ok(())
}

pub async fn extend_of_a_lost_lease_leaves_the_new_holders_ttl(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let key = key();
    let lost = acquire(provider, &key, Ttl::from_secs(10)).await?;
    provider.unlock(&lost).await?;
    acquire(provider, &key, TTL).await?;

    check!(provider.extend(std::slice::from_ref(&lost)).await? == [Extended::Lost]);
    tokio::time::sleep(EXPIRED).await;

    acquire(provider, &key, TTL).await?;
    Ok(())
}

pub async fn unlock_of_a_lost_lease_leaves_the_new_holder(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let key = key();
    let old = acquire(provider, &key, TTL).await?;
    tokio::time::sleep(EXPIRED).await;
    acquire(provider, &key, TTL).await?;

    assert!(let Ok(()) = provider.unlock(&old).await);
    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
    Ok(())
}

pub async fn unlock_of_an_expired_lease_is_not_an_error(
    provider: &impl LockProvider,
) -> anyhow::Result<()> {
    let lease = acquire(provider, &key(), TTL).await?;
    tokio::time::sleep(EXPIRED).await;

    assert!(let Ok(()) = provider.unlock(&lease).await);
    Ok(())
}

/// A key no other case uses, so cases can share a store.
fn key() -> LockKey {
    LockKey::new(format!("contract-{}", Uuid::now_v7())).expect("a contract key is valid")
}

/// The lease `try_acquire` takes on `key` for `ttl`, failing when it is busy.
async fn acquire(provider: &impl LockProvider, key: &LockKey, ttl: Ttl) -> anyhow::Result<Lease> {
    match provider.try_acquire(key, ttl).await? {
        TryAcquire::Acquired(lease) => Ok(lease),
        TryAcquire::Busy => anyhow::bail!("{key} is busy"),
    }
}
