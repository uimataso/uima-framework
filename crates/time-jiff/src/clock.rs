use jiff::{RoundMode, Timestamp, TimestampRound, Unit};

/// Where the app gets the current time.
///
/// The app reads it from its context, so a test can set the time with a
/// `testing::SimulatedClock`.
///
/// ```
/// use uima_time_jiff::jiff::Timestamp;
/// use uima_time_jiff::{Clock as _, HasClock, SystemClock};
///
/// #[derive(uima_port_macro::HasPort)]
/// struct Ctx {
///     #[has_port(Clock)]
///     clock: SystemClock,
/// }
///
/// fn stamp(ctx: &impl HasClock) -> Timestamp {
///     ctx.clock().now()
/// }
///
/// assert!(stamp(&Ctx { clock: SystemClock }) <= Timestamp::now());
/// ```
///
/// `now` is to the microsecond, the precision Postgres stores, so a time saved and read back is
/// equal to the one the app has.
#[uima_port_macro::has_port]
pub trait Clock: Send + Sync {
    /// The current time, to the microsecond.
    fn now(&self) -> Timestamp;
}

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        to_micros(Timestamp::now())
    }
}

/// `ts` without what's below a microsecond, toward zero: before 1970 that's later.
pub(crate) fn to_micros(ts: Timestamp) -> Timestamp {
    let micros = TimestampRound::new()
        .smallest(Unit::Microsecond)
        .mode(RoundMode::Trunc);
    ts.round(micros)
        .expect("truncating toward zero stays in range")
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn a_clock_can_be_shared_across_threads() {
        fn shareable<T: Send + Sync + ?Sized>() {}

        shareable::<dyn Clock>();
    }

    #[test]
    fn system_now_is_to_the_microsecond() {
        let now = SystemClock.now();

        check!(now.subsec_nanosecond() % 1000 == 0);
        check!(now.duration_until(Timestamp::now()).as_secs() < 5);
    }

    #[test]
    fn to_micros_drops_only_the_nanoseconds() {
        let ts = Timestamp::new(1_767_225_600, 123_456_789).unwrap();

        check!(to_micros(ts) == Timestamp::new(1_767_225_600, 123_456_000).unwrap());
        check!(to_micros(Timestamp::MIN) == Timestamp::MIN);
        check!(
            to_micros(Timestamp::new(-1, -1_999).unwrap()) == Timestamp::new(-1, -1_000).unwrap()
        );
        check!(
            to_micros(Timestamp::MAX)
                == Timestamp::new(Timestamp::MAX.as_second(), 999_999_000).unwrap()
        );
    }

    #[test]
    fn a_context_gets_the_clock_through_has_clock() {
        #[derive(uima_port_macro::HasPort)]
        struct Ctx {
            #[has_port(Clock)]
            clock: SystemClock,
        }
        fn now_of(ctx: &impl HasClock) -> Timestamp {
            ctx.clock().now()
        }

        check!(now_of(&Ctx { clock: SystemClock }).subsec_nanosecond() % 1000 == 0);
    }
}
