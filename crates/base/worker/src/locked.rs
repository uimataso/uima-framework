//! [`Locked`], which runs a job on one instance at a time.

use std::pin::pin;

use anyhow::Context as _;
use tracing::{debug, warn};
use uima_cancel::{CancelExt as _, CancelToken};
use uima_lock::{Lock, LockKey, TryLock, Ttl};

use crate::Job;

#[cfg(test)]
mod tests;

/// The counter of tries to take a job's lock, by their `outcome` label, one of `taken`, `busy`
/// for a lock another instance holds, or `failed`.
pub const LOCK_TRIES: &str = "job_lock_tries_total";

/// The counter of runs that lost the job's lock before they ended.
pub const LOCKS_LOST: &str = "job_locks_lost_total";

/// A [`Job`] that runs only when it takes the lock on its key, so of the instances running it,
/// one runs at a time.
///
/// It keeps the metrics named by [`LOCK_TRIES`] and [`LOCKS_LOST`], each with the job's name as
/// its `job_name` label.
///
/// A run another instance holds the lock for is left to it, without running the job. One whose
/// lock can't be taken fails. The lock is kept for the run by the
/// [`LockDriver`](uima_lock::LockDriver) the service runs beside, and let go after. A run that
/// loses it, while `cancel` hasn't come, has the job cancelled and fails.
pub struct Locked<J, L> {
    job: J,
    lock: L,
    key: LockKey,
    ttl: Ttl,
}

impl<J: Job, L: Lock> Locked<J, L> {
    /// `job`, run when it takes the lock on `key` from `lock` for `ttl`.
    pub(crate) fn new(job: J, lock: L, key: LockKey, ttl: Ttl) -> Self {
        Self {
            job,
            lock,
            key,
            ttl,
        }
    }
}

impl<J: Job, L: Lock> Job for Locked<J, L> {
    fn name(&self) -> &str {
        self.job.name()
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        let job_name = self.job.name().to_owned();
        let key = &self.key;
        let tried = |outcome: &'static str| {
            metrics::counter!(LOCK_TRIES, "job_name" => job_name.clone(), "outcome" => outcome)
                .increment(1);
        };

        let Some(taken) = self
            .lock
            .try_lock(key, self.ttl)
            .until_cancelled(cancel)
            .await
        else {
            return Ok(());
        };
        let guard = match taken {
            Ok(TryLock::Locked(guard)) => {
                tried("taken");
                guard
            }
            Ok(TryLock::Busy) => {
                tried("busy");
                debug!(job_name, %key, "run left to the instance holding the lock");
                return Ok(());
            }
            Err(error) => {
                tried("failed");
                let error = anyhow::Error::from(error);
                return Err(error.context(format!("failed to take the lock {key}")));
            }
        };

        // Its own token, so cancelling it for a lost lock doesn't stop the runner.
        let run_cancel = cancel.child();
        let mut ran = pin!(self.job.run_once(&run_cancel));
        let ran = tokio::select! {
            ran = &mut ran => ran,
            () = guard.lost().cancelled() => {
                run_cancel.cancel();
                ran.await
            }
        };

        // Lost also once the driver stops, as it does with the service, which is no lost lock.
        let lost = guard.lost().is_cancelled() && !cancel.is_cancelled();
        if let Err(error) = guard.release().await {
            let error = anyhow::Error::from(error);
            warn!(job_name, %key, error = &*error, "failed to let the lock go");
        }
        if lost {
            metrics::counter!(LOCKS_LOST, "job_name" => job_name.clone()).increment(1);
            let lost = format!("lost the lock {key} during the run");
            ran.context(lost.clone())?;
            return Err(anyhow::Error::msg(lost));
        }
        ran
    }
}
