//! Test helpers for code that takes locks.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;
use uima_core::{PortResult, Uuid};

use crate::{Extended, Lease, LockProvider, TryAcquire};

/// A [`LockProvider`] in memory, whose leases expire by tokio's clock, so a test can pause it.
#[derive(Debug, Default)]
pub struct MemoryLockProvider {
    /// Each held key's token and when its lease expires.
    leases: Mutex<HashMap<String, (Uuid, Instant)>>,
}

impl LockProvider for MemoryLockProvider {
    async fn try_acquire(&self, key: &str, ttl: Duration) -> PortResult<TryAcquire> {
        let now = Instant::now();
        let mut leases = self.leases.lock().unwrap();
        if leases.get(key).is_some_and(|&(_, expires)| expires > now) {
            return Ok(TryAcquire::Busy);
        }
        let token = Uuid::now_v7();
        leases.insert(key.to_owned(), (token, now + ttl));
        Ok(TryAcquire::Acquired(Lease {
            key: key.to_owned(),
            token,
            ttl,
        }))
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        let now = Instant::now();
        let mut held = self.leases.lock().unwrap();
        let extended = leases
            .iter()
            .map(|lease| match held.get_mut(&lease.key) {
                Some((token, expires)) if *token == lease.token && *expires > now => {
                    *expires = now + lease.ttl;
                    Extended::Held
                }
                _ => Extended::Lost,
            })
            .collect();
        Ok(extended)
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        let mut leases = self.leases.lock().unwrap();
        if leases
            .get(&lease.key)
            .is_some_and(|&(token, _)| token == lease.token)
        {
            leases.remove(&lease.key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    #[tokio::test(start_paused = true)]
    async fn it_passes_the_contract() {
        contract::run_all(async || MemoryLockProvider::default()).await;
    }

    /// One way a [`Broken`] store gets locks wrong.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Break {
        WrongTtl,
        NeverBusy,
        OneKey,
        UnlockKeeps,
        NeverExpires,
        ExtendForgets,
        ExtendRotated,
        UnlockByKey,
    }

    /// A [`MemoryLockProvider`] broken as `breaks` says.
    #[derive(Default)]
    struct Broken {
        store: MemoryLockProvider,
        breaks: Option<Break>,
        /// Each key's newest lease, for `UnlockByKey`.
        newest: Mutex<HashMap<String, Lease>>,
    }

    impl LockProvider for Broken {
        async fn try_acquire(&self, key: &str, ttl: Duration) -> PortResult<TryAcquire> {
            let (store_key, store_ttl) = match self.breaks {
                Some(Break::OneKey) => ("one", ttl),
                Some(Break::NeverExpires) => (key, ttl * 1000),
                _ => (key, ttl),
            };
            let got = self.store.try_acquire(store_key, store_ttl).await?;
            let mut lease = match (got, self.breaks) {
                (TryAcquire::Acquired(lease), _) => lease,
                (TryAcquire::Busy, Some(Break::NeverBusy)) => Lease {
                    key: key.to_owned(),
                    token: Uuid::now_v7(),
                    ttl,
                },
                (TryAcquire::Busy, _) => return Ok(TryAcquire::Busy),
            };
            lease.key = key.to_owned();
            lease.ttl = if self.breaks == Some(Break::WrongTtl) {
                ttl * 2
            } else {
                ttl
            };
            self.newest
                .lock()
                .unwrap()
                .insert(key.to_owned(), lease.clone());
            Ok(TryAcquire::Acquired(lease))
        }

        async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
            match self.breaks {
                Some(Break::ExtendForgets) => Ok(vec![Extended::Held; leases.len()]),
                Some(Break::ExtendRotated) => {
                    let mut extended = self.store.extend(leases).await?;
                    extended.rotate_left(1);
                    Ok(extended)
                }
                _ => self.store.extend(leases).await,
            }
        }

        async fn unlock(&self, lease: &Lease) -> PortResult<()> {
            match self.breaks {
                Some(Break::UnlockKeeps) => Ok(()),
                Some(Break::UnlockByKey) => {
                    let newest = self.newest.lock().unwrap().get(&lease.key).cloned();
                    self.store
                        .unlock(&newest.unwrap_or_else(|| lease.clone()))
                        .await
                }
                _ => self.store.unlock(lease).await,
            }
        }
    }

    /// Whether `case` fails on a store broken as `breaks` says.
    async fn fails<F>(breaks: Break, case: impl FnOnce(Broken) -> F) -> bool
    where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let broken = Broken {
            breaks: Some(breaks),
            ..Broken::default()
        };
        tokio::spawn(case(broken))
            .await
            .map_or(true, |result| result.is_err())
    }

    #[tokio::test(start_paused = true)]
    async fn a_broken_store_fails_the_contract() {
        let ran = tokio::spawn(contract::run_all(async || Broken {
            breaks: Some(Break::UnlockKeeps),
            ..Broken::default()
        }))
        .await;

        uima_testing::check!(ran.is_err_and(|error| error.is_panic()));
    }

    #[tokio::test(start_paused = true)]
    async fn each_case_fails_the_store_breaking_it() {
        use Break::*;
        use uima_testing::check;

        check!(
            fails(WrongTtl, |p| async move {
                contract::acquire_takes_a_free_key(&p).await
            })
            .await
        );
        check!(
            fails(NeverBusy, |p| async move {
                contract::acquire_on_a_held_key_is_busy(&p).await
            })
            .await
        );
        check!(
            fails(OneKey, |p| async move {
                contract::keys_are_locked_apart(&p).await
            })
            .await
        );
        check!(
            fails(UnlockKeeps, |p| async move {
                contract::unlock_frees_the_key(&p).await
            })
            .await
        );
        check!(
            fails(NeverExpires, |p| async move {
                contract::an_expired_lease_can_be_taken(&p).await
            })
            .await
        );
        check!(
            fails(ExtendForgets, |p| async move {
                contract::extend_keeps_a_lease_past_its_ttl(&p).await
            })
            .await
        );
        check!(
            fails(ExtendRotated, |p| async move {
                contract::extend_tells_each_lease_in_order(&p).await
            })
            .await
        );
        check!(
            fails(ExtendForgets, |p| async move {
                contract::extend_after_unlock_is_lost(&p).await
            })
            .await
        );
        check!(
            fails(UnlockByKey, |p| async move {
                contract::unlock_of_a_lost_lease_leaves_the_new_holder(&p).await
            })
            .await
        );
    }
}
