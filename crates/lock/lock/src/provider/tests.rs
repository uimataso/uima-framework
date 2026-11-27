use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use tokio::time::Instant;
use uima_core::{Failure, FailureKind};
use uima_testing::check;

use super::*;
use crate::contract;
use crate::testing::MemoryLockProvider;
use Break::*;

/// One way a [`Broken`] store gets locks wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Break {
    WrongTtl,
    NeverBusy,
    OneKey,
    UnlockKeeps,
    NeverExpires,
    ExtendForgets,
    ExtendByMin,
    ExtendRotated,
    UnlockByKey,
    WholeSeconds,
    ExtendRevives,
    RacyAcquire,
    UnlockExpiredFails,
    ExtendForever,
    ExtendAdds,
    BusyExtends,
    WrongKey,
    ExtendSaysLost,
    UnlockLostFails,
    ExtendAtOnceLoses,
    AcquireWholeSeconds,
    ExtendLateSaysLost,
}

/// What became of a lease.
#[derive(PartialEq, Eq)]
enum Fate {
    Held,
    /// Expired, with nobody taking its key.
    Expired,
    Taken,
}

/// A [`MemoryLockProvider`] broken as `breaks` says.
#[derive(Default)]
struct Broken {
    store: MemoryLockProvider,
    breaks: Option<Break>,
    /// Each key's newest lease, for `UnlockByKey` and `BusyExtends`.
    newest: Mutex<HashMap<String, Lease>>,
    /// The leases let go, for `ExtendRevives`.
    unlocked: Mutex<HashSet<Uuid>>,
    /// When each lease was last extended, for `ExtendAtOnceLoses`.
    last_extended: Mutex<HashMap<Uuid, Instant>>,
    /// When each lease expires, for `ExtendAdds` and `ExtendLateSaysLost`.
    expiries: Mutex<HashMap<Uuid, Instant>>,
}

impl Broken {
    fn new(breaks: Break) -> Self {
        Self {
            breaks: Some(breaks),
            ..Self::default()
        }
    }

    /// What became of `lease`, which is extended when still held.
    async fn fate(&self, lease: &Lease) -> PortResult<Fate> {
        if self.store.extend(std::slice::from_ref(lease)).await? == [Extended::Held] {
            return Ok(Fate::Held);
        }
        match self.store.try_acquire(&lease.key, lease.ttl).await? {
            TryAcquire::Acquired(free) => {
                self.store.unlock(&free).await?;
                Ok(Fate::Expired)
            }
            TryAcquire::Busy => Ok(Fate::Taken),
        }
    }
}

impl LockProvider for Broken {
    async fn try_acquire(&self, key: &str, ttl: Ttl) -> PortResult<TryAcquire> {
        let (store_key, store_ttl) = match self.breaks {
            Some(OneKey) => ("one", ttl),
            Some(NeverExpires) => (key, Ttl::MAX),
            Some(AcquireWholeSeconds) => (key, whole_seconds(ttl)),
            _ => (key, ttl),
        };
        if self.breaks == Some(RacyAcquire) {
            // Checks the key is free, then takes it later, so another can take it in between.
            let TryAcquire::Acquired(seen) = self.store.try_acquire(key, ttl).await? else {
                return Ok(TryAcquire::Busy);
            };
            self.store.unlock(&seen).await?;
            tokio::task::yield_now().await;
            let token = match self.store.try_acquire(key, ttl).await? {
                TryAcquire::Acquired(lease) => lease.token,
                TryAcquire::Busy => Uuid::now_v7(),
            };
            return Ok(TryAcquire::Acquired(Lease {
                key: key.to_owned(),
                token,
                ttl,
            }));
        }
        let got = self.store.try_acquire(store_key, store_ttl).await?;
        let mut lease = match (got, self.breaks) {
            (TryAcquire::Acquired(lease), _) => lease,
            (TryAcquire::Busy, Some(NeverBusy)) => Lease {
                key: key.to_owned(),
                token: Uuid::now_v7(),
                ttl,
            },
            (TryAcquire::Busy, Some(BusyExtends)) => {
                // Keeps the holder's lease for a ttl more each time another asks.
                let held = self.newest.lock().unwrap().get(key).cloned();
                self.store.extend(held.as_slice()).await?;
                return Ok(TryAcquire::Busy);
            }
            (TryAcquire::Busy, _) => return Ok(TryAcquire::Busy),
        };
        lease.key = if self.breaks == Some(WrongKey) {
            format!("locks:{key}")
        } else {
            key.to_owned()
        };
        lease.ttl = if self.breaks == Some(WrongTtl) {
            Ttl::MAX
        } else {
            ttl
        };
        self.newest
            .lock()
            .unwrap()
            .insert(key.to_owned(), lease.clone());
        self.expiries
            .lock()
            .unwrap()
            .insert(lease.token, Instant::now() + ttl.get());
        Ok(TryAcquire::Acquired(lease))
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        let leases: Vec<Lease> = leases.iter().map(stored).collect();
        let leases = leases.as_slice();
        // Extends each lease for another ttl than its own.
        let extend_for = async |ttl: fn(Ttl) -> Ttl| {
            let leases: Vec<Lease> = leases
                .iter()
                .map(|lease| Lease {
                    ttl: ttl(lease.ttl),
                    ..lease.clone()
                })
                .collect();
            self.store.extend(&leases).await
        };
        match self.breaks {
            Some(ExtendForgets) => Ok(vec![Extended::Held; leases.len()]),
            Some(ExtendForever) => extend_for(|_| Ttl::MAX).await,
            Some(ExtendByMin) => extend_for(|_| Ttl::MIN).await,
            Some(WholeSeconds) => extend_for(whole_seconds).await,
            Some(ExtendAdds) => {
                // Adds each lease's ttl to when it expired, not to now.
                let now = Instant::now();
                let added: Vec<Lease> = leases
                    .iter()
                    .map(|lease| {
                        let mut expiries = self.expiries.lock().unwrap();
                        let expires = expiries.entry(lease.token).or_insert(now);
                        *expires += lease.ttl.get();
                        Lease {
                            ttl: Ttl::new(expires.saturating_duration_since(now))
                                .unwrap_or(lease.ttl),
                            ..lease.clone()
                        }
                    })
                    .collect();
                self.store.extend(&added).await
            }
            Some(ExtendRevives) => {
                // Extends a lease nobody let go or took, expired or not.
                let mut extended = self.store.extend(leases).await?;
                for (lease, extended) in leases.iter().zip(&mut extended) {
                    let unlocked = self.unlocked.lock().unwrap().contains(&lease.token);
                    if *extended == Extended::Lost
                        && !unlocked
                        && let TryAcquire::Acquired(_) =
                            self.store.try_acquire(&lease.key, lease.ttl).await?
                    {
                        *extended = Extended::Held;
                    }
                }
                Ok(extended)
            }
            Some(ExtendAtOnceLoses) => {
                // Extends a lease being extended by another call too, but says it is lost.
                let now = Instant::now();
                let mut extended = self.store.extend(leases).await?;
                for (lease, extended) in leases.iter().zip(&mut extended) {
                    let last = self.last_extended.lock().unwrap().insert(lease.token, now);
                    if last == Some(now) {
                        *extended = Extended::Lost;
                    }
                }
                Ok(extended)
            }
            Some(ExtendLateSaysLost) => {
                // Extends a lease with under three tenths of its ttl left, but says it is lost.
                let now = Instant::now();
                let mut extended = self.store.extend(leases).await?;
                for (lease, extended) in leases.iter().zip(&mut extended) {
                    let expires = now + lease.ttl.get();
                    let expired = self.expiries.lock().unwrap().insert(lease.token, expires);
                    let late = expired.is_some_and(|expired| {
                        expired.saturating_duration_since(now) < lease.ttl.get() * 3 / 10
                    });
                    if late {
                        *extended = Extended::Lost;
                    }
                }
                Ok(extended)
            }
            Some(ExtendSaysLost) => {
                let extended = self.store.extend(leases).await?;
                Ok(vec![Extended::Lost; extended.len()])
            }
            Some(ExtendRotated) => {
                let mut extended = self.store.extend(leases).await?;
                extended.rotate_left(1);
                Ok(extended)
            }
            _ => self.store.extend(leases).await,
        }
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        let lease = &stored(lease);
        self.unlocked.lock().unwrap().insert(lease.token);
        match self.breaks {
            Some(UnlockKeeps) => Ok(()),
            Some(UnlockExpiredFails) => {
                if self.fate(lease).await? == Fate::Expired {
                    return Err(Failure::new(FailureKind::Other, anyhow!("expired")).into());
                }
                self.store.unlock(lease).await
            }
            Some(UnlockLostFails) => {
                if self.fate(lease).await? == Fate::Taken {
                    return Err(Failure::new(FailureKind::Other, anyhow!("lost")).into());
                }
                self.store.unlock(lease).await
            }
            Some(UnlockByKey) => {
                let newest = self.newest.lock().unwrap().get(&lease.key).cloned();
                self.store
                    .unlock(&newest.unwrap_or_else(|| lease.clone()))
                    .await
            }
            _ => self.store.unlock(lease).await,
        }
    }
}

/// `lease` as the store knows it, under the key asked for and not the one `WrongKey` gave.
fn stored(lease: &Lease) -> Lease {
    let key = lease.key.strip_prefix("locks:").unwrap_or(&lease.key);
    Lease {
        key: key.to_owned(),
        ..lease.clone()
    }
}

/// `ttl` cut to whole seconds.
fn whole_seconds(ttl: Ttl) -> Ttl {
    Ttl::new(Duration::from_secs(ttl.get().as_secs())).unwrap()
}

/// Whether `case` fails on `store`.
async fn fails<F>(store: Broken, case: impl FnOnce(Broken) -> F) -> bool
where
    F: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(case(store))
        .await
        .map_or(true, |result| result.is_err())
}

/// The tests of the contract's cases, from the breaks each case is there to catch.
macro_rules! cases {
    ($($($breaks:ident),+ => $case:ident,)*) => {
        const CASES: usize = [$(stringify!($case)),*].len();

        #[tokio::test(start_paused = true)]
        async fn the_contract_fails_each_broken_store() {
            for breaks in [$($($breaks),+),*] {
                let ran = tokio::spawn(contract::run_all(async move || Broken::new(breaks))).await;

                check!(ran.is_err_and(|error| error.is_panic()), "{breaks:?}");
            }
        }

        #[tokio::test(start_paused = true)]
        async fn each_case_fails_the_store_breaking_it() {
            $(
                let case = |provider| async move { contract::$case(&provider).await };
                for breaks in [$($breaks),+] {
                    check!(
                        fails(Broken::new(breaks), case).await,
                        "{} on {breaks:?}",
                        stringify!($case)
                    );
                }
                check!(!fails(Broken::default(), case).await, "{} unbroken", stringify!($case));
            )*
        }
    };
}

cases! {
    WrongTtl, WrongKey => acquire_takes_a_free_key,
    NeverBusy => acquire_on_a_held_key_is_busy,
    BusyExtends => a_busy_acquire_leaves_the_lease_to_expire,
    RacyAcquire => acquire_at_once_takes_a_key_once,
    OneKey => keys_are_locked_apart,
    UnlockKeeps => unlock_frees_the_key,
    NeverExpires => an_expired_lease_can_be_taken,
    AcquireWholeSeconds, WholeSeconds, ExtendLateSaysLost => a_ttl_is_not_rounded_to_whole_seconds,
    ExtendForgets, ExtendSaysLost => extend_keeps_a_lease_past_its_ttl,
    ExtendForever => an_extended_lease_expires_a_ttl_after_the_extend,
    ExtendAdds, ExtendAtOnceLoses => extends_at_once_keep_a_lease_for_one_ttl,
    ExtendByMin => extend_keeps_each_lease_for_its_own_ttl,
    ExtendRotated => extend_tells_each_lease_in_order,
    ExtendForgets => extend_after_unlock_is_lost,
    ExtendRevives => extend_of_an_expired_lease_is_lost,
    UnlockByKey, UnlockLostFails => unlock_of_a_lost_lease_leaves_the_new_holder,
    UnlockExpiredFails => unlock_of_an_expired_lease_is_not_an_error,
}

#[tokio::test(start_paused = true)]
async fn the_contract_runs_every_case_and_passes_a_store_not_broken() {
    let stores = AtomicUsize::new(0);

    contract::run_all(async || {
        stores.fetch_add(1, Ordering::SeqCst);
        Broken::default()
    })
    .await;

    check!(stores.into_inner() == CASES, "one store made for each case");
}
