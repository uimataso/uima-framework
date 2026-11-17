use std::time::Duration;

use jiff::{Span, SpanRelativeTo};

/// Parses a length of time from config, such as `30s`, `1h 30m`, `30d` or `PT30S`.
///
/// It takes jiff's friendly format and ISO 8601, with a day as 24 hours and a week as 7 days.
/// It ignores whitespace around the length and takes a bare `0`. A negative
/// length, or one in months or years, which have no fixed length, is an error.
///
/// ```
/// use std::time::Duration;
///
/// assert_eq!(uima_time_jiff::parse_duration("1h 30m"), Ok(Duration::from_secs(5400)));
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, InvalidDuration> {
    let s = s.trim();
    if s == "0" {
        return Ok(Duration::ZERO);
    }
    let span: Span = s.parse().map_err(|_| InvalidDuration::Format)?;
    if span.get_years() != 0 || span.get_months() != 0 {
        return Err(InvalidDuration::Calendar);
    }
    let duration = span
        .to_duration(SpanRelativeTo::days_are_24_hours())
        .map_err(|_| InvalidDuration::Format)?;
    // A negative span gives a negative duration, which std's can't hold.
    Duration::try_from(duration).map_err(|_| InvalidDuration::Negative)
}

/// Why [`parse_duration`] refused a string. The message leaves the string out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidDuration {
    /// It isn't a length of time, such as `30s`, `1h 30m` or `PT30S`.
    Format,
    /// It's below zero.
    Negative,
    /// It's in months or years, which have no fixed length.
    Calendar,
}

impl std::fmt::Display for InvalidDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Format => "isn't a length of time such as `30s`, `1h 30m` or `PT30S`",
            Self::Negative => "can't be negative",
            Self::Calendar => "can't be in months or years, which have no fixed length",
        })
    }
}

impl std::error::Error for InvalidDuration {}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    const SECOND: Duration = Duration::from_secs(1);
    const MINUTE: Duration = Duration::from_secs(60);
    const HOUR: Duration = Duration::from_secs(3600);
    const DAY: Duration = Duration::from_secs(86_400);

    #[test]
    fn the_values_services_set_today() {
        for (s, want) in [
            ("1s", SECOND),
            ("3s", 3 * SECOND),
            ("30s", 30 * SECOND),
            ("120s", 120 * SECOND),
            ("1h", HOUR),
            ("30d", 30 * DAY),
        ] {
            check!(parse_duration(s) == Ok(want), "{s}");
        }
    }

    #[test]
    fn units_combine_and_spell_out() {
        for (s, want) in [
            ("1h 30m", HOUR + 30 * MINUTE),
            ("1h30m", HOUR + 30 * MINUTE),
            ("2 hours", 2 * HOUR),
            ("1w", 7 * DAY),
            ("1d 1s", DAY + SECOND),
            ("500ms", Duration::from_millis(500)),
            ("0s", Duration::ZERO),
            ("PT30S", 30 * SECOND),
            ("P1DT1H", DAY + HOUR),
        ] {
            check!(parse_duration(s) == Ok(want), "{s}");
        }
    }

    #[test]
    fn like_humantime_whitespace_around_is_ignored_and_0_is_zero() {
        for (s, want) in [
            (" 30s", 30 * SECOND),
            ("1h\n", HOUR),
            ("0", Duration::ZERO),
            (" 0 ", Duration::ZERO),
        ] {
            check!(parse_duration(s) == Ok(want), "{s:?}");
        }
    }

    #[test]
    fn a_day_is_24_hours() {
        check!(parse_duration("365d") == Ok(365 * DAY));
    }

    #[test]
    fn not_a_duration_is_a_format_error() {
        for s in ["", " ", "00", "30", "abc", "30 parsecs", "1h-", "1.5.5s"] {
            check!(parse_duration(s) == Err(InvalidDuration::Format), "{s:?}");
        }
    }

    #[test]
    fn a_negative_duration_is_refused() {
        for s in ["-1s", "1s ago", "-PT1S"] {
            check!(parse_duration(s) == Err(InvalidDuration::Negative), "{s}");
        }
    }

    #[test]
    fn months_and_years_are_refused() {
        for s in ["1mo", "1y", "P1M", "1y 1d"] {
            check!(parse_duration(s) == Err(InvalidDuration::Calendar), "{s}");
        }
    }

    #[test]
    fn each_error_has_its_own_message() {
        let messages = [
            InvalidDuration::Format,
            InvalidDuration::Negative,
            InvalidDuration::Calendar,
        ]
        .map(|e| e.to_string());

        check!(messages.iter().all(|m| !m.is_empty()));
        check!(messages[0] != messages[1]);
        check!(messages[1] != messages[2]);
        check!(messages[0] != messages[2]);
    }

    #[test]
    fn the_message_leaves_the_input_out() {
        let error = parse_duration("hunter2").unwrap_err();

        check!(!error.to_string().contains("hunter2"));
    }
}

#[cfg(test)]
mod props {
    use jiff::SignedDuration;
    use proptest::prelude::*;
    use proptest::property_test;

    use super::*;

    /// The longest length a span holds, in seconds.
    const MAX_SECS: u64 = 631_107_417_600;

    /// Any length a span holds, often zero, the longest or a whole number of seconds.
    fn duration() -> impl Strategy<Value = Duration> {
        prop_oneof![
            Just(Duration::ZERO),
            Just(Duration::from_secs(MAX_SECS)),
            (0..=MAX_SECS).prop_map(Duration::from_secs),
            (0..MAX_SECS, 0..1_000_000_000_u32)
                .prop_map(|(secs, nanos)| Duration::new(secs, nanos)),
        ]
    }

    /// Up to 2 whitespace characters.
    fn whitespace() -> impl Strategy<Value = String> {
        prop::collection::vec(prop_oneof![Just(' '), Just('\t'), Just('\n')], 0..3)
            .prop_map(|chars| chars.into_iter().collect())
    }

    /// The length in jiff's ISO 8601 and friendly formats.
    fn written(duration: SignedDuration) -> [String; 2] {
        [format!("{duration}"), format!("{duration:#}")]
    }

    #[property_test]
    fn reads_back_a_written_length(
        #[strategy = duration()] duration: Duration,
        #[strategy = whitespace()] before: String,
        #[strategy = whitespace()] after: String,
    ) {
        for written in written(SignedDuration::try_from(duration).unwrap()) {
            let padded = format!("{before}{written}{after}");
            prop_assert_eq!(parse_duration(&padded), Ok(duration), "{:?}", padded);
        }
    }

    #[property_test]
    fn refuses_a_negative_length(
        #[strategy = duration().prop_filter("not zero", |d| !d.is_zero())] duration: Duration,
    ) {
        for written in written(-SignedDuration::try_from(duration).unwrap()) {
            prop_assert_eq!(
                parse_duration(&written),
                Err(InvalidDuration::Negative),
                "{}",
                written
            );
        }
    }

    #[property_test]
    fn refuses_months_and_years(
        #[strategy = 1..=1000_u32] n: u32,
        #[strategy = prop_oneof![Just("mo"), Just("y")]] unit: &'static str,
        #[strategy = 0..=30_u32] days: u32,
    ) {
        let written = format!("{n}{unit} {days}d");
        prop_assert_eq!(
            parse_duration(&written),
            Err(InvalidDuration::Calendar),
            "{}",
            written
        );
    }

    #[property_test]
    fn never_panics(s: String) {
        let _ = parse_duration(&s);
    }
}
