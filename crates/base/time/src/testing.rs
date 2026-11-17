//! Test helpers.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::Timestamp;

use crate::Clock;
use crate::clock::to_micros;

/// A clock for tests that stands still until [`advance`](Self::advance) or
/// [`advance_to`](Self::advance_to) moves it. Clones share the time, so a test keeps one and gives
/// another to the code under test.
///
/// It holds its time to the microsecond, as `now` gives it, so a step below a microsecond doesn't
/// move it.
///
/// ```
/// use std::time::Duration;
///
/// use uima_time::Clock as _;
/// use uima_time::testing::SimulatedClock;
///
/// let clock = SimulatedClock::new();
/// let app = clock.clone();
/// let start = app.now();
/// clock.advance(Duration::from_secs(60));
/// assert_eq!(app.now(), start + Duration::from_secs(60));
/// ```
#[derive(Debug, Clone)]
pub struct SimulatedClock(Arc<Mutex<Timestamp>>);

impl SimulatedClock {
    /// The time a new clock starts at: 2026-01-01T00:00:00Z.
    pub const START: Timestamp = Timestamp::constant(1_767_225_600, 0);

    /// A clock at [`START`](Self::START).
    pub fn new() -> Self {
        Self::new_at(Self::START)
    }

    /// A clock at `start`.
    pub fn new_at(start: Timestamp) -> Self {
        Self(Arc::new(Mutex::new(to_micros(start))))
    }

    /// Moves the clock forward to `to`.
    ///
    /// # Panics
    ///
    /// If `to` is before the clock's time, to the microsecond.
    pub fn advance_to(&self, to: Timestamp) {
        let to = to_micros(to);
        let mut now = self.lock();
        if to < *now {
            let from = *now;
            drop(now);
            panic!("can't move the clock back from {from} to {to}");
        }
        *now = to;
    }

    /// Moves the clock forward by `by`.
    ///
    /// # Panics
    ///
    /// If that passes the latest time jiff can hold.
    pub fn advance(&self, by: Duration) {
        let mut now = self.lock();
        let Ok(to) = now.checked_add(by) else {
            drop(now);
            panic!("advanced the clock past the latest time jiff can hold");
        };
        *now = to_micros(to);
    }

    fn lock(&self) -> MutexGuard<'_, Timestamp> {
        self.0.lock().expect("a clock's lock isn't poisoned")
    }
}

impl Default for SimulatedClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SimulatedClock {
    fn now(&self) -> Timestamp {
        *self.lock()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uima_testing::check;

    use super::*;

    #[test]
    fn default_starts_at_start() {
        check!(SimulatedClock::default().now() == SimulatedClock::START);
    }

    #[test]
    fn starts_at_start_and_stands_still() {
        let clock = SimulatedClock::new();

        check!(clock.now() == SimulatedClock::START);
        check!(clock.now() == SimulatedClock::START);
        check!(SimulatedClock::START.to_string() == "2026-01-01T00:00:00Z");
    }

    #[test]
    fn advance_moves_every_clone() {
        let clock = SimulatedClock::new();
        let app = clock.clone();

        clock.advance(Duration::from_secs(90));
        app.advance(Duration::from_millis(500));

        check!(app.now() == SimulatedClock::START + Duration::from_millis(90_500));
        check!(clock.now() == app.now());
    }

    #[test]
    fn now_is_to_the_microsecond() {
        let start = SimulatedClock::START;
        let clock = SimulatedClock::new_at(start + Duration::from_nanos(1_999));

        check!(clock.now() == start + Duration::from_micros(1));
        clock.advance(Duration::from_nanos(999));
        check!(clock.now() == start + Duration::from_micros(1));
    }

    #[test]
    fn advance_to_within_the_same_microsecond_stays() {
        let start = SimulatedClock::START;
        let clock = SimulatedClock::new_at(start + Duration::from_nanos(1_500));

        clock.advance_to(start + Duration::from_nanos(1_499));

        check!(clock.now() == start + Duration::from_micros(1));
    }

    #[test]
    #[should_panic(expected = "past the latest time")]
    fn advancing_past_the_end_panics() {
        SimulatedClock::new_at(Timestamp::MAX).advance(Duration::from_secs(1));
    }

    #[test]
    fn advance_to_moves_every_clone_to_that_time() {
        let clock = SimulatedClock::new();
        let app = clock.clone();
        let later = SimulatedClock::START + Duration::from_secs(3600);

        clock.advance_to(SimulatedClock::START);
        check!(app.now() == SimulatedClock::START);
        clock.advance_to(later);
        check!(app.now() == later);
    }

    #[test]
    #[should_panic(expected = "can't move the clock back")]
    fn advance_to_an_earlier_time_panics() {
        SimulatedClock::new().advance_to(SimulatedClock::START - Duration::from_nanos(1));
    }

    #[test]
    fn a_failed_advance_leaves_the_clock_usable() {
        let clock = SimulatedClock::new_at(Timestamp::MAX);
        let app = clock.clone();

        let advanced = std::panic::catch_unwind(|| clock.advance(Duration::from_secs(1)));
        let moved_back = std::panic::catch_unwind(|| clock.advance_to(Timestamp::MIN));

        check!(advanced.is_err());
        check!(moved_back.is_err());
        check!(app.now() == crate::clock::to_micros(Timestamp::MAX));
        app.advance(Duration::ZERO);
    }
}
