use std::time::Duration;

use jiff::Timestamp;

/// The [`Duration`] between two timestamps, named after [`std::time::Instant`]'s methods.
pub trait TimestampExt {
    /// How long after `earlier` this is, or `None` when it's before `earlier`.
    fn checked_duration_since(self, earlier: Timestamp) -> Option<Duration>;

    /// How long after `earlier` this is, or zero when it's before `earlier`.
    fn saturating_duration_since(self, earlier: Timestamp) -> Duration;

    /// How far apart this and `other` are, in either order.
    fn abs_duration_since(self, other: Timestamp) -> Duration;
}

impl TimestampExt for Timestamp {
    fn checked_duration_since(self, earlier: Timestamp) -> Option<Duration> {
        Duration::try_from(self.duration_since(earlier)).ok()
    }

    fn saturating_duration_since(self, earlier: Timestamp) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }

    fn abs_duration_since(self, other: Timestamp) -> Duration {
        self.duration_since(other).unsigned_abs()
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    const NANO: Duration = Duration::from_nanos(1);

    #[test]
    fn later_minus_earlier_is_the_gap() {
        let earlier = at(100);
        let later = earlier + Duration::from_millis(1500);

        check!(later.checked_duration_since(earlier) == Some(Duration::from_millis(1500)));
        check!(later.saturating_duration_since(earlier) == Duration::from_millis(1500));
        check!(later.abs_duration_since(earlier) == Duration::from_millis(1500));
    }

    #[test]
    fn the_same_timestamp_is_zero() {
        check!(at(5).checked_duration_since(at(5)) == Some(Duration::ZERO));
        check!(at(5).saturating_duration_since(at(5)) == Duration::ZERO);
        check!(at(5).abs_duration_since(at(5)) == Duration::ZERO);
    }

    #[test]
    fn earlier_minus_later_is_none_zero_or_the_gap() {
        let earlier = at(100);
        let later = earlier + NANO;

        check!(earlier.checked_duration_since(later) == None);
        check!(earlier.saturating_duration_since(later) == Duration::ZERO);
        check!(earlier.abs_duration_since(later) == NANO);
    }

    #[test]
    fn the_whole_range_fits() {
        let span = Timestamp::MAX.duration_since(Timestamp::MIN).unsigned_abs();

        check!(Timestamp::MAX.checked_duration_since(Timestamp::MIN) == Some(span));
        check!(Timestamp::MIN.abs_duration_since(Timestamp::MAX) == span);
        check!(Timestamp::MIN.saturating_duration_since(Timestamp::MAX) == Duration::ZERO);
    }
}
