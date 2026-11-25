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
}
