//! The cases a [`LockProvider`] passes, run by each adapter.
//!
//! Cases that wait for a lease to expire do so with tokio's clock, so on a store that expires them
//! by its own clock they take a little more than [`TTL`] each.

use std::time::Duration;

use uima_core::Uuid;
use uima_testing::{assert, check, run_case};

use super::*;

/// The `ttl` the cases take locks for.
pub const TTL: Ttl = Ttl::MIN;

/// Long enough after [`TTL`] that a lease taken for it has expired.
const EXPIRED: Duration = Duration::from_millis(1500);

/// Runs every case, each on its own provider from `isolated`.
pub async fn run_all<P: LockProvider>(isolated: impl AsyncFn() -> P) {
    run_case(&isolated, acquire_takes_a_free_key).await;
    run_case(&isolated, acquire_on_a_held_key_is_busy).await;
    run_case(&isolated, keys_are_locked_apart).await;
    run_case(&isolated, unlock_frees_the_key).await;
    run_case(&isolated, an_expired_lease_can_be_taken).await;
    run_case(&isolated, extend_keeps_a_lease_past_its_ttl).await;
    run_case(&isolated, extend_tells_each_lease_in_order).await;
    run_case(&isolated, extend_after_unlock_is_lost).await;
    run_case(&isolated, unlock_of_a_lost_lease_leaves_the_new_holder).await;
}

pub async fn acquire_takes_a_free_key(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    let lease = acquire(provider, &key, TTL).await?;

    check!(lease.key == key);
    check!(lease.ttl == TTL);
    Ok(())
}

pub async fn acquire_on_a_held_key_is_busy(provider: &impl LockProvider) -> anyhow::Result<()> {
    let key = key();
    acquire(provider, &key, TTL).await?;

    check!(provider.try_acquire(&key, TTL).await? == TryAcquire::Busy);
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
    let first = acquire(provider, &key, TTL).await?;
    tokio::time::sleep(EXPIRED).await;

    let second = acquire(provider, &key, TTL).await?;
    check!(second.token != first.token);
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

/// A key no other case uses, so cases can share a store.
fn key() -> String {
    format!("contract-{}", Uuid::now_v7())
}

/// The lease `try_acquire` takes on `key` for `ttl`, failing when it is busy.
async fn acquire(provider: &impl LockProvider, key: &str, ttl: Ttl) -> anyhow::Result<Lease> {
    match provider.try_acquire(key, ttl).await? {
        TryAcquire::Acquired(lease) => Ok(lease),
        TryAcquire::Busy => anyhow::bail!("{key} is busy"),
    }
}
