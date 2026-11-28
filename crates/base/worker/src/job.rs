//! A [`Job`], work a service runs over and over, and [`Measured`], which keeps its metrics.

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures_util::FutureExt as _;
use uima_cancel::CancelToken;
use uima_time::{Clock, SystemClock, TimestampExt as _};

/// The gauge of when the last run ended, however it ended.
pub const LAST_ATTEMPT: &str = "job_last_attempt_timestamp_seconds";

/// The gauge of when the last run that succeeded ended. Until one does, there is none.
pub const LAST_SUCCESS: &str = "job_last_success_timestamp_seconds";

/// The counter of runs, by their `outcome` label, one of `succeeded`, `failed` or `panicked`.
pub const RUNS: &str = "job_runs_total";

/// A job that runs over and over, such as a cleanup, through a runner such as
/// [`periodic::Runner`](crate::periodic::Runner).
pub trait Job: Send {
    /// Names the job as `job_name` in logs and metrics. Each job in a service needs its own,
    /// non-empty name, since jobs sharing a name share their metrics.
    fn name(&self) -> &str;

    /// Runs the job once.
    ///
    /// A run that can take long checks `cancel` to stop early. It gives `Ok` once it ends cleanly,
    /// done or stopped where it is safe to, and an error when something went wrong. After a run
    /// panics, the next one runs on the same `self`, as the panic left it.
    fn run_once(&mut self, cancel: &CancelToken)
    -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// A [`Job`] that keeps the metrics of each of its runs, named by [`LAST_ATTEMPT`],
/// [`LAST_SUCCESS`] and [`RUNS`], each with the job's name as its `job_name` label. Times are in
/// unix seconds, read from `C`.
///
/// A run that panics is counted, then goes on panicking, so the runner still sees the panic.
pub struct Measured<J, C = SystemClock> {
    job: J,
    clock: C,
}

impl<J: Job> Measured<J> {
    /// `job`, measured.
    pub(crate) fn new(job: J) -> Self {
        Self::on(job, SystemClock)
    }
}

impl<J: Job, C: Clock> Measured<J, C> {
    /// `job`, measured with the time from `clock`.
    pub(crate) fn on(job: J, clock: C) -> Self {
        Self { job, clock }
    }
}

impl<J: Job, C: Clock> Job for Measured<J, C> {
    fn name(&self) -> &str {
        self.job.name()
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        // Called inside the caught future, so a panic before `run_once` gives its future is caught
        // too.
        let ran = AssertUnwindSafe(async { self.job.run_once(cancel).await })
            .catch_unwind()
            .await;

        let name = self.job.name().to_owned();
        let gauge = |metric: &'static str, at: f64| {
            metrics::gauge!(metric, "job_name" => name.clone()).set(at);
        };

        let ended_at = self.clock.now().unix_seconds();
        let outcome = match ran {
            Ok(Ok(())) => {
                gauge(LAST_SUCCESS, ended_at);
                "succeeded"
            }
            Ok(Err(_)) => "failed",
            Err(_) => "panicked",
        };
        gauge(LAST_ATTEMPT, ended_at);
        metrics::counter!(RUNS, "job_name" => name, "outcome" => outcome).increment(1);

        ran.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }
}
