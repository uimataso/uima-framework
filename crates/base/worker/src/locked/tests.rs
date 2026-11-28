use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use uima_core::{Failure, FailureKind, PortResult};
use uima_lock::testing::MemoryLockProvider;
use uima_lock::{Extended, Lease, LockClient, LockProvider, TryAcquire};
use uima_testing::{assert, check};

use super::*;
use crate::Measured;

const KEY: LockKey = LockKey::from_static("purge");
const TTL: Ttl = Ttl::MIN;

/// A store in memory that can be rigged to fail the next acquire, or to take the held lease away
/// on the next extend.
#[derive(Default)]
struct Rigged {
    memory: MemoryLockProvider,
    fail_next: AtomicBool,
    steal_next: AtomicBool,
}

impl LockProvider for Rigged {
    async fn try_acquire(&self, key: &LockKey, ttl: Ttl) -> PortResult<TryAcquire> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(Failure::new(FailureKind::Unavailable, anyhow::anyhow!("down")).into());
        }
        self.memory.try_acquire(key, ttl).await
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        if self.steal_next.swap(false, Ordering::SeqCst) {
            for lease in leases {
                self.memory.unlock(lease).await?;
            }
        }
        self.memory.extend(leases).await
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        self.memory.unlock(lease).await
    }
}

/// A job whose runs each take `takes`, unless its `cancel` comes first and it stops `stopping`
/// later, failing when `fails_cancelled`, and that counts its runs and those cancelled.
#[derive(Clone)]
struct Held {
    takes: Duration,
    stopping: Duration,
    fails_cancelled: bool,
    ran: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}

impl Job for Held {
    fn name(&self) -> &str {
        "purge"
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        self.ran.fetch_add(1, Ordering::SeqCst);
        if tokio::time::sleep(self.takes)
            .until_cancelled(cancel)
            .await
            .is_none()
        {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.stopping).await;
            anyhow::ensure!(!self.fails_cancelled, "stopped halfway");
        }
        Ok(())
    }
}

fn held(takes: Duration) -> Held {
    Held {
        takes,
        stopping: Duration::ZERO,
        fails_cancelled: false,
        ran: Arc::default(),
        cancelled: Arc::default(),
    }
}

/// What a recorder saw: each counter by its name and `outcome`, and the names of the gauges.
#[derive(Debug, Default, PartialEq)]
struct Seen {
    counters: BTreeMap<String, u64>,
    gauges: BTreeSet<String>,
}

fn seen_in(snapshotter: &Snapshotter) -> Seen {
    let mut seen = Seen::default();
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        let key = key.key();
        let label = |name| key.labels().find(|label| label.key() == name);
        check!(label("job_name").map(|label| label.value()) == Some("purge"));
        let name = key.name().to_owned();
        match value {
            DebugValue::Counter(count) => {
                let name = match label("outcome") {
                    Some(outcome) => format!("{name}{{{}}}", outcome.value()),
                    None => name,
                };
                seen.counters.insert(name, count);
            }
            DebugValue::Gauge(_) => {
                seen.gauges.insert(name);
            }
            DebugValue::Histogram(_) => panic!("unexpected histogram {name}"),
        }
    }
    seen
}

fn counters(counts: &[(&str, u64)]) -> BTreeMap<String, u64> {
    counts
        .iter()
        .map(|&(name, count)| (name.to_owned(), count))
        .collect()
}

/// What a run of [`run_once`] ended with, whether the key was free as it ended, and the metrics.
struct Ran {
    ended: anyhow::Result<()>,
    let_go: bool,
    seen: Seen,
}

/// Runs `job` once, [`Measured`] inside [`Locked`] on locks from `store`, beside `meanwhile`,
/// which takes locks from the same client and has its own `cancel` and the driver's, cancelled
/// once it ends.
async fn run_once(
    job: Held,
    store: Rigged,
    meanwhile: impl AsyncFnOnce(&LockClient<Rigged>, &CancelToken),
) -> Ran {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let (locks, driver) = uima_lock::new(store);
    let cancel = CancelToken::new();
    let mut one = Locked::new(Measured::new(job), locks.clone(), KEY, TTL);

    let ((ended, let_go), ()) = tokio::join!(
        async {
            // Yields first, so `meanwhile` can take the lock before the run.
            tokio::task::yield_now().await;
            let ended = one.run_once(&cancel).await;
            // Right away, before the driver has had a turn.
            let let_go = matches!(locks.try_lock(&KEY, TTL).await, Ok(TryLock::Locked(_)));
            (ended, let_go)
        },
        async {
            tokio::join!(
                async {
                    meanwhile(&locks, &cancel).await;
                    cancel.cancel();
                },
                driver.run(cancel.clone()),
            );
        },
    );
    Ran {
        ended,
        let_go,
        seen: seen_in(&snapshotter),
    }
}

/// Measured's gauges, for a run that ran the job.
fn ran_gauges() -> BTreeSet<String> {
    [crate::LAST_ATTEMPT, crate::LAST_SUCCESS]
        .map(str::to_owned)
        .into()
}

#[tokio::test(start_paused = true)]
async fn a_run_takes_the_lock_and_lets_it_go_after() {
    let job = held(Duration::ZERO);
    let Ran {
        ended,
        let_go,
        seen,
    } = run_once(job.clone(), Rigged::default(), async |_, _| {
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await;

    check!(ended.is_ok());
    check!(let_go, "let go by the time the run ends");
    check!(job.ran.load(Ordering::SeqCst) == 1);
    check!(
        seen == Seen {
            counters: counters(&[
                ("job_lock_tries_total{taken}", 1),
                ("job_runs_total{succeeded}", 1),
            ]),
            gauges: ran_gauges(),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_another_instance_holds_the_lock_for_is_left_to_it_and_keeps_no_job_metrics() {
    let job = held(Duration::ZERO);
    let Ran { ended, seen, .. } = run_once(job.clone(), Rigged::default(), async |locks, _| {
        let _held = locks.try_lock(&KEY, TTL).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await;

    check!(ended.is_ok());
    check!(job.ran.load(Ordering::SeqCst) == 0);
    check!(
        seen == Seen {
            counters: counters(&[("job_lock_tries_total{busy}", 1)]),
            gauges: BTreeSet::new(),
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_whose_lock_cant_be_taken_fails_without_running_the_job() {
    let job = held(Duration::ZERO);
    let store = Rigged::default();
    store.fail_next.store(true, Ordering::SeqCst);
    let Ran { ended, seen, .. } = run_once(job.clone(), store, async |_, _| {
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await;

    assert!(let Err(error) = ended);
    check!(format!("{error:#}").starts_with("failed to take the lock purge: "));
    check!(job.ran.load(Ordering::SeqCst) == 0);
    check!(seen.counters == counters(&[("job_lock_tries_total{failed}", 1)]));
    check!(seen.gauges.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_run_that_loses_the_lock_is_cancelled_and_fails() {
    for fails_cancelled in [false, true] {
        let job = Held {
            fails_cancelled,
            ..held(Duration::from_secs(5))
        };
        let store = Rigged::default();
        store.steal_next.store(true, Ordering::SeqCst);
        let Ran { ended, seen, .. } = run_once(job.clone(), store, async |_, _| {
            tokio::time::sleep(Duration::from_secs(6)).await;
        })
        .await;

        assert!(let Err(error) = ended);
        let (message, outcome) = match fails_cancelled {
            false => ("lost the lock purge during the run", "succeeded"),
            true => (
                "lost the lock purge during the run: stopped halfway",
                "failed",
            ),
        };
        check!(format!("{error:#}") == message);
        check!(job.cancelled.load(Ordering::SeqCst) == 1);
        check!(
            seen.counters
                == counters(&[
                    ("job_lock_tries_total{taken}", 1),
                    ("job_locks_lost_total", 1),
                    (&format!("job_runs_total{{{outcome}}}"), 1),
                ]),
            "the job's metrics take what it gave"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn stopping_while_holding_the_lock_is_no_lost_lock() {
    // Stopping slowly, so the driver has stopped and cancelled `lost` by the time it ends.
    let job = Held {
        stopping: Duration::from_secs(1),
        ..held(Duration::from_secs(5))
    };
    let Ran { ended, seen, .. } = run_once(job.clone(), Rigged::default(), async |_, _| {
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await;

    check!(ended.is_ok());
    check!(job.cancelled.load(Ordering::SeqCst) == 1);
    check!(
        seen.counters
            == counters(&[
                ("job_lock_tries_total{taken}", 1),
                ("job_runs_total{succeeded}", 1),
            ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_runner_on_one_instance_counts_only_the_runs_it_ran() {
    use uima_runtime::Subsystem as _;

    use crate::periodic::{Behavior, Config, FirstRun, Runner};

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let (locks, driver) = uima_lock::new(Rigged::default());
    let cancel = CancelToken::new();
    let config = Config {
        behavior: Behavior::AfterStart {
            every: Duration::from_secs(10),
        },
        first_run: FirstRun::Immediately,
    };
    let job = held(Duration::ZERO);
    let runner = Runner::new(job.clone(), config)
        .unwrap()
        .on_one_instance(locks.clone(), KEY, TTL);

    // The holder first, so it takes the lock before the first run.
    let ((), ended, ()) = tokio::join!(
        async {
            let TryLock::Locked(guard) = locks.try_lock(&KEY, TTL).await.unwrap() else {
                panic!("the key is free before the first run");
            };
            // Past the runs due at 0s, 10s and 20s.
            tokio::time::sleep(Duration::from_secs(25)).await;
            guard.release().await.unwrap();
            // Past the run due at 30s.
            tokio::time::sleep(Duration::from_secs(10)).await;
            cancel.cancel();
        },
        runner.run(cancel.clone()),
        driver.run(cancel.clone()),
    );

    check!(ended.is_ok());
    check!(job.ran.load(Ordering::SeqCst) == 1);
    let seen = seen_in(&snapshotter);
    check!(
        seen.counters
            == counters(&[
                ("job_lock_tries_total{busy}", 3),
                ("job_lock_tries_total{taken}", 1),
                ("job_runs_total{succeeded}", 1),
            ])
    );
}
