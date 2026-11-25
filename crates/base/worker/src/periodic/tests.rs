use std::collections::BTreeMap;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tracing::Level;
use uima_log_tracing_subscriber::testing::LogCapture;
use uima_testing::{assert, check};
use uima_time::testing::SimulatedClock;

use super::*;
use uima_runtime::TimeoutExt as _;

const EVERY: Duration = Duration::from_secs(10);

/// Longer than any test's runner takes, so one that keeps waiting fails instead of hanging. One
/// that never waits still hangs, since the paused clock only moves while the runtime is idle.
const GIVE_UP: Duration = Duration::from_secs(3600);

/// Spawns `subsystem`, which compiles only while any subsystem can be spawned.
fn spawn<S: Subsystem + 'static>(
    subsystem: S,
    cancel: CancelToken,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async move { subsystem.run(cancel).await })
}

/// What one run of a [`Script`] does after taking its time.
#[derive(Clone, Copy, Debug)]
enum Outcome {
    Succeed,
    Fail,
    Panic,
}

/// A job that plays a script of runs, each taking its time and ending in its outcome, records when
/// each started, and cancels its token when the last run starts.
struct Script {
    runs: Vec<(Duration, Outcome)>,
    done: usize,
    launched: Instant,
    started: Arc<Mutex<Vec<Duration>>>,
}

impl Job for Script {
    fn name(&self) -> &str {
        "script"
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        self.started
            .lock()
            .unwrap()
            .push(Instant::now() - self.launched);
        let (takes, outcome) = self.runs[self.done];
        self.done += 1;
        if self.done == self.runs.len() {
            cancel.cancel();
        }
        tokio::time::sleep(takes).await;
        match outcome {
            Outcome::Succeed => Ok(()),
            Outcome::Fail => Err(anyhow::anyhow!("down").context("purge")),
            Outcome::Panic => panic!("broken"),
        }
    }
}

/// A [`Script`] of `runs`, each taking its seconds, and when each starts.
fn script(runs: &[(u64, Outcome)]) -> (Script, Arc<Mutex<Vec<Duration>>>) {
    let started = Arc::new(Mutex::new(Vec::new()));
    let job = Script {
        runs: runs
            .iter()
            .map(|&(secs, outcome)| (Duration::from_secs(secs), outcome))
            .collect(),
        done: 0,
        launched: Instant::now(),
        started: started.clone(),
    };
    (job, started)
}

/// `started` in whole seconds.
fn secs(started: &Mutex<Vec<Duration>>) -> Vec<u64> {
    started
        .lock()
        .unwrap()
        .iter()
        .map(Duration::as_secs)
        .collect()
}

/// When each run of `runs` started, in seconds since the job did, run with `behavior` and
/// `first_run`, and what the job ended with.
async fn play(
    behavior: Behavior,
    first_run: FirstRun,
    runs: &[(u64, Outcome)],
) -> (Vec<u64>, anyhow::Result<()>) {
    let (job, started) = script(runs);
    let config = Config {
        behavior,
        first_run,
    };
    let ended = Runner::new(job, config)
        .unwrap()
        .run(CancelToken::new())
        .timeout(GIVE_UP)
        .await
        .expect("it ends once its last run does");
    (secs(&started), ended)
}

/// Runs that succeed and take `secs` each.
fn taking(secs: &[u64]) -> Vec<(u64, Outcome)> {
    secs.iter().map(|&secs| (secs, Outcome::Succeed)).collect()
}

#[tokio::test(start_paused = true)]
async fn each_behavior_schedules_the_next_run_from_its_own_point() {
    let after_start = Behavior::AfterStart { every: EVERY };
    let (started, _) = play(after_start, FirstRun::Immediately, &taking(&[0, 15, 0, 0])).await;
    check!(
        started == [0, 10, 25, 35],
        "a long run is followed right away"
    );

    let after_end = Behavior::AfterEnd { every: EVERY };
    let (started, _) = play(after_end, FirstRun::Immediately, &taking(&[5, 15, 0])).await;
    check!(started == [0, 15, 40]);

    let aligned = Behavior::Aligned { every: EVERY };
    let (started, _) = play(aligned, FirstRun::Immediately, &taking(&[0, 15, 3, 0])).await;
    check!(
        started == [0, 10, 30, 40],
        "a long run skips the slot it overran"
    );
    let (started, _) = play(aligned, FirstRun::Immediately, &taking(&[10, 0])).await;
    check!(
        started == [0, 10],
        "a run ending on a slot is followed in it"
    );
}

#[tokio::test(start_paused = true)]
async fn aligned_keeps_its_grid_when_a_run_wakes_late() {
    let (job, started) = script(&taking(&[0, 0, 0]));
    let config = Config {
        behavior: Behavior::Aligned { every: EVERY },
        first_run: FirstRun::OnSchedule,
    };
    let mut run = pin!(Runner::new(job, config).unwrap().run(CancelToken::new()));
    check!(run.as_mut().now_or_never().is_none());

    tokio::time::advance(Duration::from_secs(13)).await;
    let ended = run.timeout(GIVE_UP).await;

    check!(matches!(ended, Ok(Ok(()))));
    check!(secs(&started) == [13, 20, 30]);

    let (job, started) = script(&taking(&[8, 0]));
    let mut run = pin!(Runner::new(job, config).unwrap().run(CancelToken::new()));
    check!(run.as_mut().now_or_never().is_none());

    tokio::time::advance(Duration::from_secs(13)).await;
    let ended = run.timeout(GIVE_UP).await;

    check!(matches!(ended, Ok(Ok(()))));
    check!(
        secs(&started) == [13, 30],
        "a late run that overruns skips the slot it overran"
    );
}

#[tokio::test(start_paused = true)]
async fn a_period_of_a_year_waits_a_year() {
    const YEAR: u64 = 365 * 24 * 60 * 60;
    let every = Duration::from_secs(YEAR);
    for behavior in [
        Behavior::AfterStart { every },
        Behavior::AfterEnd { every },
        Behavior::Aligned { every },
    ] {
        let (job, started) = script(&taking(&[0, 0]));
        let config = Config {
            behavior,
            first_run: FirstRun::Immediately,
        };
        let ended = Runner::new(job, config)
            .unwrap()
            .run(CancelToken::new())
            .timeout(every * 2)
            .await;

        check!(matches!(ended, Ok(Ok(()))), "{behavior:?}");
        check!(secs(&started) == [0, YEAR], "{behavior:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_first_run_waits_a_period_when_on_schedule() {
    for behavior in [
        Behavior::AfterStart { every: EVERY },
        Behavior::AfterEnd { every: EVERY },
        Behavior::Aligned { every: EVERY },
    ] {
        let (started, _) = play(behavior, FirstRun::OnSchedule, &taking(&[0, 0])).await;
        check!(started == [10, 20], "{behavior:?}");
    }
}

#[test]
fn new_rejects_a_zero_period() {
    for behavior in [
        Behavior::AfterStart {
            every: Duration::ZERO,
        },
        Behavior::AfterEnd {
            every: Duration::ZERO,
        },
        Behavior::Aligned {
            every: Duration::ZERO,
        },
    ] {
        let config = Config {
            behavior,
            first_run: FirstRun::Immediately,
        };
        let (job, _) = script(&[]);
        assert!(let Err(error) = Runner::new(job, config), "{behavior:?}");
        check!(error.to_string() == "script runs every 0 seconds");
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_or_panicking_run_is_followed_on_schedule() {
    let runs = [
        (0, Outcome::Fail),
        (0, Outcome::Panic),
        (0, Outcome::Succeed),
    ];
    let behavior = Behavior::AfterStart { every: EVERY };
    let logs = LogCapture::start();
    let (started, ended) = play(behavior, FirstRun::Immediately, &runs).await;

    check!(started == [0, 10, 20]);
    check!(ended.is_ok());
    let events = logs.events();
    let logged = |level, message, error| {
        events.iter().any(|event| {
            event.level == level
                && event.message == message
                && event
                    .fields
                    .get("error")
                    .is_some_and(|logged| logged == error)
        })
    };
    check!(
        logged(Level::WARN, "run failed", "purge: down"),
        "the error with its sources: {events:?}"
    );
    check!(logged(Level::ERROR, "run panicked", "broken"), "{events:?}");
    check!(
        events.iter().all(|event| event
            .fields
            .get("job_name")
            .is_some_and(|name| name == "script")),
        "{events:?}"
    );

    let runs = Arc::new(AtomicUsize::new(0));
    let config = Config {
        behavior,
        first_run: FirstRun::Immediately,
    };
    let eager = Runner::new(Eager(runs.clone()), config).unwrap();
    let ended = spawn(eager, CancelToken::new()).timeout(GIVE_UP).await;
    check!(matches!(ended, Ok(Ok(Ok(())))), "{ended:?}");
    check!(
        runs.load(Ordering::SeqCst) == 2,
        "a panic before the run's future is caught too"
    );
}

/// A job whose first run panics before it gives its future, and whose second cancels its token.
struct Eager(Arc<AtomicUsize>);

impl Job for Eager {
    fn name(&self) -> &str {
        "eager"
    }

    fn run_once(
        &mut self,
        cancel: &CancelToken,
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("broken");
        }
        cancel.cancel();
        async { Ok(()) }
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_failing_or_panicking_while_stopping_is_the_error_it_ends_with() {
    let behavior = Behavior::AfterStart { every: EVERY };

    let logs = LogCapture::start();

    let (_, ended) = play(behavior, FirstRun::Immediately, &[(0, Outcome::Fail)]).await;
    assert!(let Err(error) = ended);
    check!(format!("{error:#}") == "script failed while stopping: purge: down");

    let (_, ended) = play(behavior, FirstRun::Immediately, &[(0, Outcome::Panic)]).await;
    assert!(let Err(error) = ended);
    check!(format!("{error:#}") == "script panicked while stopping: broken");
    check!(error.root_cause().to_string() == "broken");

    let logged = logs.count_messages(Level::WARN, "") + logs.count_messages(Level::ERROR, "");
    check!(
        logged == 0,
        "the error it ends with is left to the caller to log"
    );

    let launched = Instant::now();
    let (started, ended) = play(behavior, FirstRun::Immediately, &taking(&[5])).await;
    check!(started == [0]);
    check!(
        Instant::now() - launched == Duration::from_secs(5),
        "a run in progress finishes"
    );
    check!(ended.is_ok());
}

#[test]
fn a_panic_is_an_error_with_its_message_whether_fixed_or_formatted() {
    let number = 7;
    let panics = [
        (std::panic::catch_unwind(|| panic!("broken")), "broken"),
        (
            std::panic::catch_unwind(|| panic!("broken {number}")),
            "broken 7",
        ),
        (
            std::panic::catch_unwind(|| std::panic::panic_any(number)),
            "no message",
        ),
    ];
    for (panic, message) in panics {
        assert!(let Err(panic) = panic);
        check!(panic_error(panic).to_string() == message);
    }
}

/// A job whose runs do nothing.
struct Idle;

impl Job for Idle {
    fn name(&self) -> &str {
        "idle"
    }

    async fn run_once(&mut self, _cancel: &CancelToken) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_ends_it_while_it_waits_for_the_next_run() {
    let config = Config {
        behavior: Behavior::AfterStart {
            every: Duration::from_secs(3600),
        },
        first_run: FirstRun::Immediately,
    };
    let idle = Runner::new(Idle, config).unwrap();
    check!(idle.name() == "idle");
    let cancel = CancelToken::new();
    let launched = Instant::now();

    let task = spawn(idle, cancel.clone());
    tokio::time::sleep(Duration::from_secs(5)).await;
    cancel.cancel();
    let ended = task.timeout(Duration::from_secs(60)).await;

    assert!(let Ok(Ok(ended)) = ended, "it ends once cancelled");
    check!(ended.is_ok());
    check!(Instant::now() - launched == Duration::from_secs(5));
}

/// A job that counts its runs, and fails a run that starts already cancelled, as a job checking
/// `cancel` does.
struct Stopper(Arc<AtomicUsize>);

impl Job for Stopper {
    fn name(&self) -> &str {
        "stopper"
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(!cancel.is_cancelled(), "stopped early");
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_by_the_time_a_run_is_due_skips_it() {
    let config = Config {
        behavior: Behavior::AfterStart { every: EVERY },
        first_run: FirstRun::OnSchedule,
    };

    let runs = Arc::new(AtomicUsize::new(0));
    let cancel = CancelToken::new();
    let task = spawn(
        Runner::new(Stopper(runs.clone()), config).unwrap(),
        cancel.clone(),
    );
    tokio::time::sleep(EVERY).await;
    cancel.cancel();
    let ended = task.timeout(GIVE_UP).await;
    check!(matches!(ended, Ok(Ok(Ok(())))), "{ended:?}");
    check!(runs.load(Ordering::SeqCst) == 0, "cancelled as it was due");

    let runs = Arc::new(AtomicUsize::new(0));
    let cancel = CancelToken::new();
    let stopper = Runner::new(Stopper(runs.clone()), config).unwrap();
    let mut run = pin!(stopper.run(cancel.clone()));
    check!(run.as_mut().now_or_never().is_none());
    tokio::time::advance(Duration::from_secs(9)).await;
    cancel.cancel();
    tokio::time::advance(Duration::from_secs(2)).await;
    let ended = run.timeout(GIVE_UP).await;
    check!(matches!(ended, Ok(Ok(()))), "{ended:?}");
    check!(
        runs.load(Ordering::SeqCst) == 0,
        "cancelled before it was due, and polled after"
    );
}

#[tokio::test(start_paused = true)]
async fn any_period_from_a_nanosecond_to_the_longest_runs_until_cancelled() {
    for every in [Duration::from_nanos(1), Duration::MAX] {
        for behavior in [
            Behavior::AfterStart { every },
            Behavior::AfterEnd { every },
            Behavior::Aligned { every },
        ] {
            for first_run in [FirstRun::Immediately, FirstRun::OnSchedule] {
                let config = Config {
                    behavior,
                    first_run,
                };
                let cancel = CancelToken::new();
                let task = spawn(Runner::new(Idle, config).unwrap(), cancel.clone());
                tokio::time::sleep(Duration::from_millis(5)).await;
                cancel.cancel();
                let ended = task.timeout(GIVE_UP).await;
                check!(
                    matches!(ended, Ok(Ok(Ok(())))),
                    "{behavior:?} {first_run:?}: {ended:?}"
                );
            }
        }
    }
}

/// A job whose runs end in `outcomes` in turn, each a second long on `clock`, and that records
/// `clock`'s time at the end of each.
struct Wall {
    outcomes: Vec<Outcome>,
    clock: SimulatedClock,
    ended_at: Arc<Mutex<Vec<f64>>>,
}

impl Job for Wall {
    fn name(&self) -> &str {
        "wall"
    }

    async fn run_once(&mut self, cancel: &CancelToken) -> anyhow::Result<()> {
        self.clock.advance(Duration::from_secs(1));
        self.ended_at
            .lock()
            .unwrap()
            .push(self.clock.now().unix_seconds());
        let outcome = self.outcomes.remove(0);
        if self.outcomes.is_empty() {
            cancel.cancel();
        }
        match outcome {
            Outcome::Succeed => Ok(()),
            Outcome::Fail => anyhow::bail!("down"),
            Outcome::Panic => panic!("broken"),
        }
    }
}

/// A [`Wall`] of `outcomes` on `clock`, and when each of its runs ends.
fn wall(outcomes: &[Outcome], clock: &SimulatedClock) -> (Wall, Arc<Mutex<Vec<f64>>>) {
    let ended_at = Arc::new(Mutex::new(Vec::new()));
    let wall = Wall {
        outcomes: outcomes.to_vec(),
        clock: clock.clone(),
        ended_at: ended_at.clone(),
    };
    (wall, ended_at)
}

/// The metrics a recorder saw from a [`Wall`].
#[derive(Debug, Default, PartialEq)]
struct Seen {
    started: Option<f64>,
    last_attempt: Option<f64>,
    last_success: Option<f64>,
    /// The runs counted, by their `outcome`.
    runs: BTreeMap<String, u64>,
}

/// What `snapshotter` saw, checking each metric is labelled with the name of a [`Wall`].
fn seen_in(snapshotter: &Snapshotter) -> Seen {
    let mut seen = Seen::default();
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        let key = key.key();
        let mut labels: BTreeMap<_, _> = key
            .labels()
            .map(|label| (label.key().to_owned(), label.value().to_owned()))
            .collect();
        check!(
            labels.remove("job_name").as_deref() == Some("wall"),
            "{key:?}"
        );
        match (key.name(), value) {
            ("job_started_timestamp_seconds", DebugValue::Gauge(at)) => {
                seen.started = Some(at.into_inner());
            }
            ("job_last_attempt_timestamp_seconds", DebugValue::Gauge(at)) => {
                seen.last_attempt = Some(at.into_inner());
            }
            ("job_last_success_timestamp_seconds", DebugValue::Gauge(at)) => {
                seen.last_success = Some(at.into_inner());
            }
            ("job_runs_total", DebugValue::Counter(count)) => {
                let outcome = labels.remove("outcome").expect("a run has an outcome");
                seen.runs.insert(outcome, count);
            }
            (name, value) => panic!("unexpected metric {name}: {value:?}"),
        }
        check!(labels.is_empty(), "{key:?}");
    }
    seen
}

/// Runs counted by outcome, as [`Seen`] holds them.
fn runs(counts: &[(&str, u64)]) -> BTreeMap<String, u64> {
    counts
        .iter()
        .map(|&(outcome, count)| (outcome.to_owned(), count))
        .collect()
}

/// The metrics after runs ending in `outcomes`, the clock's time when the job started, and at the
/// end of each run.
async fn metrics_after(outcomes: &[Outcome]) -> (Seen, f64, Vec<f64>) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let clock = SimulatedClock::new();
    let (wall, ended_at) = wall(outcomes, &clock);
    let config = Config {
        behavior: Behavior::AfterStart { every: EVERY },
        first_run: FirstRun::Immediately,
    };

    let started = clock.now().unix_seconds();
    let _ = Runner::new(wall, config)
        .unwrap()
        .run_on(CancelToken::new(), &clock)
        .timeout(GIVE_UP)
        .await
        .expect("it ends once its last run does");

    let ended_at = ended_at.lock().unwrap().clone();
    (seen_in(&snapshotter), started, ended_at)
}

#[tokio::test(start_paused = true)]
async fn the_metrics_mark_the_start_each_run_and_each_success() {
    let (seen, started, ended_at) = metrics_after(&[Outcome::Fail, Outcome::Panic]).await;
    check!(
        seen == Seen {
            started: Some(started),
            last_attempt: Some(ended_at[1]),
            last_success: None,
            runs: runs(&[("failed", 1), ("panicked", 1)]),
        },
        "a job failing from the start has no success"
    );

    let (seen, started, ended_at) = metrics_after(&[Outcome::Succeed, Outcome::Fail]).await;
    check!(
        seen == Seen {
            started: Some(started),
            last_attempt: Some(ended_at[1]),
            last_success: Some(ended_at[0]),
            runs: runs(&[("succeeded", 1), ("failed", 1)]),
        },
        "a failed run leaves the last success"
    );

    let (seen, _, ended_at) = metrics_after(&[Outcome::Fail, Outcome::Succeed]).await;
    check!(seen.last_success == Some(ended_at[1]));

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let clock = SimulatedClock::new();
    let (wall, _) = wall(&[Outcome::Succeed], &clock);
    let config = Config {
        behavior: Behavior::AfterStart { every: EVERY },
        first_run: FirstRun::OnSchedule,
    };
    let run = pin!(
        Runner::new(wall, config)
            .unwrap()
            .run_on(CancelToken::new(), &clock)
    );
    check!(run.now_or_never().is_none());
    check!(
        seen_in(&snapshotter)
            == Seen {
                started: Some(clock.now().unix_seconds()),
                ..Seen::default()
            },
        "it marks the start before the first run"
    );
}
