//! Running a job over and over on a schedule.
//!
//! # Examples
//!
//! ```
//! # async fn example() -> anyhow::Result<()> {
//! use std::time::Duration;
//!
//! use uima_cancel::CancelToken;
//! use uima_runtime::Subsystem as _;
//! use uima_runtime::periodic;
//!
//! struct PurgeExpiredSessions;
//!
//! impl periodic::Job for PurgeExpiredSessions {
//!     fn name(&self) -> &str {
//!         "purge-expired-sessions"
//!     }
//!
//!     async fn run_once(&mut self, _cancel: &CancelToken) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//! }
//!
//! let config = periodic::Config {
//!     behavior: periodic::Behavior::AfterStart { every: Duration::from_secs(60) },
//!     first_run: periodic::FirstRun::Immediately,
//! };
//! let purge = periodic::Runner::new(PurgeExpiredSessions, config)?;
//! let cancel = CancelToken::new();
//! # cancel.cancel();
//! purge.run(cancel).await?;
//! # Ok(())
//! # }
//! ```

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt as _;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uima_cancel::{CancelExt as _, CancelToken};
use uima_time::{Clock, SystemClock, TimestampExt as _};

use crate::Subsystem;

#[cfg(test)]
mod tests;

/// The gauge of when a job's last run succeeded, or it started if none has yet, in unix seconds,
/// with the job's name as its `job_name` label, as in the job's logs.
pub const HEARTBEAT: &str = "job_last_success_timestamp_seconds";

/// A job that runs over and over, such as a cleanup, through [`Runner`].
pub trait Job: Send {
    /// Names the job as `job_name` in logs and in the [`HEARTBEAT`] gauge. Each job in a service
    /// needs its own, non-empty name, since jobs sharing a name share one heartbeat.
    fn name(&self) -> &str;

    /// Runs the job once.
    ///
    /// A run that can take long checks `cancel` to stop early, and gives `Ok` when it does, since
    /// an error from a run that ends after the cancel is what [`Runner`] ends with. After a run
    /// panics, the next one runs on the same `self`, as the panic left it.
    fn run_once(&mut self, cancel: &CancelToken)
    -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// When a job runs.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub behavior: Behavior,
    pub first_run: FirstRun,
}

/// How the next run is scheduled from the last one.
#[derive(Clone, Copy, Debug)]
pub enum Behavior {
    /// `every` after the last run started; a run that takes longer is followed right away.
    AfterStart { every: Duration },
    /// `every` after the last run ended.
    AfterEnd { every: Duration },
    /// On a grid of `every` from when the first run is due, without scheduling drift; a run that
    /// takes longer skips the slots it overran.
    Aligned { every: Duration },
}

/// When the first run happens.
#[derive(Clone, Copy, Debug)]
pub enum FirstRun {
    /// As soon as the runner starts.
    Immediately,
    /// Once [`Behavior`]'s `every` has passed.
    OnSchedule,
}

/// A [`Subsystem`] that runs a [`Job`] on its [`Config`] until cancelled.
///
/// A failed run is logged and the job runs again on schedule; a run that panics is logged and
/// counts as failed. It keeps the [`HEARTBEAT`] gauge, so while the service stays up, an alert on
/// its age finds a job that stopped or keeps failing. A restart sets it to the start again.
///
/// Once cancelled, it ends without waiting for the next run, and a run in progress finishes
/// first, as long as [`Subsystem::run`] is awaited to the end; dropping it drops the run where it
/// is. If that run fails, [`Subsystem::run`] gives its error.
pub struct Runner<J> {
    job: J,
    config: Config,
}

impl<J: Job> Runner<J> {
    /// Fails when `config`'s `every` is zero.
    pub fn new(job: J, config: Config) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !config.behavior.every().is_zero(),
            "{} runs every 0 seconds",
            job.name()
        );

        Ok(Self { job, config })
    }
}

impl<J: Job> Subsystem for Runner<J> {
    fn name(&self) -> &str {
        self.job.name()
    }

    async fn run(self, cancel: CancelToken) -> anyhow::Result<()> {
        self.run_on(cancel, &SystemClock).await
    }
}

impl<J: Job> Runner<J> {
    /// [`Subsystem::run`], reading the time for the heartbeat from `clock`.
    async fn run_on(mut self, cancel: CancelToken, clock: &impl Clock) -> anyhow::Result<()> {
        // Owned, since each run borrows the job mutably.
        let name = self.job.name().to_owned();
        let job_name = name.as_str();

        // Set after every run, and looked up each time, so a recorder installed after the start
        // gets the last success from the next run on, even while runs fail.
        let mut last_success = clock.now().unix_seconds();

        let beat = |at: f64| metrics::gauge!(HEARTBEAT, "job_name" => name.clone()).set(at);
        beat(last_success);

        info!(job_name, "started");

        let behavior = self.config.behavior;

        let launched = Instant::now();
        let mut next = match self.config.first_run {
            FirstRun::Immediately => launched,
            FirstRun::OnSchedule => add_capped(launched, behavior.every()),
        };

        // Checked again after the wait, which may see its time come and the cancel at once.
        while tokio::time::sleep_until(next)
            .until_cancelled(&cancel)
            .await
            .is_some_and(|()| !cancel.is_cancelled())
        {
            let scheduled = next;
            let started = Instant::now();

            // Called inside the caught future, so a panic before `run_once` gives its future is
            // caught too.
            let ran = AssertUnwindSafe(async { self.job.run_once(&cancel).await })
                .catch_unwind()
                .await;

            let finished = Instant::now();
            next = behavior.next_run(Ran {
                scheduled,
                started,
                finished,
            });
            let took = finished - started;

            if let Ok(Ok(())) = ran {
                last_success = clock.now().unix_seconds();
            }

            beat(last_success);

            let stopping = cancel.is_cancelled();
            match (ran, stopping) {
                (Ok(Ok(())), _) => debug!(job_name, ?took, "run succeeded"),
                (Ok(Err(error)), true) => {
                    return Err(error.context(format!("{job_name} failed while stopping")));
                }
                (Ok(Err(error)), false) => {
                    warn!(job_name, ?took, error = &*error, "run failed");
                }
                (Err(panic), true) => {
                    return Err(
                        panic_error(panic).context(format!("{job_name} panicked while stopping"))
                    );
                }
                (Err(panic), false) => {
                    let error = panic_error(panic);
                    error!(job_name, ?took, error = &*error, "run panicked");
                }
            }
        }

        info!(job_name, "stopped");
        Ok(())
    }
}

impl Behavior {
    fn every(self) -> Duration {
        match self {
            Self::AfterStart { every } | Self::AfterEnd { every } | Self::Aligned { every } => {
                every
            }
        }
    }

    /// When the run after `ran` is due.
    fn next_run(self, ran: Ran) -> Instant {
        let Ran {
            scheduled,
            started,
            finished,
        } = ran;
        match self {
            Self::AfterStart { every } => add_capped(started, every),
            Self::AfterEnd { every } => add_capped(finished, every),
            Self::Aligned { every } => {
                // The first slot at or after `finished`, but always a later one than `scheduled`.
                let overran = finished.saturating_duration_since(scheduled);
                let slots = overran.as_nanos().div_ceil(every.as_nanos()).max(1);
                // At most `overran + every`, so it fits.
                let ahead = Duration::from_nanos_u128(every.as_nanos() * slots);
                add_capped(scheduled, ahead)
            }
        }
    }
}

/// `at` plus `duration`, capped so a huge `duration` can't overflow.
fn add_capped(at: Instant, duration: Duration) -> Instant {
    // Any cap far beyond a process's life works; 30 years is an arbitrary pick.
    const FAR: Duration = Duration::from_secs(30 * 365 * 24 * 60 * 60);
    at + duration.min(FAR)
}

/// When a run was scheduled for, and when it started and finished.
#[derive(Clone, Copy)]
struct Ran {
    scheduled: Instant,
    started: Instant,
    finished: Instant,
}

/// A panic as an error, with the message it was raised with when it has one.
fn panic_error(panic: Box<dyn Any + Send>) -> anyhow::Error {
    let message = panic
        .downcast::<String>()
        .map(|message| *message)
        .or_else(|panic| {
            panic
                .downcast::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|_| "no message".to_owned());
    anyhow::anyhow!(message)
}
