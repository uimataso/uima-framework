//! Serde helpers for config fields.
//!
//! ```
//! use std::time::Duration;
//!
//! #[derive(serde::Deserialize)]
//! struct Config {
//!     #[serde(deserialize_with = "uima_time_jiff::serde::duration")]
//!     timeout: Duration,
//! }
//!
//! let config: Config = serde_json::from_str(r#"{ "timeout": "30s" }"#).unwrap();
//! assert_eq!(config.timeout, Duration::from_secs(30));
//! ```

use std::time::Duration;

/// Deserializes a [`Duration`] from a string with [`parse_duration`](crate::parse_duration).
pub fn duration<'de, D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let s = <std::borrow::Cow<'de, str> as ::serde::Deserialize>::deserialize(deserializer)?;
    crate::parse_duration(&s).map_err(::serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[derive(Debug, ::serde::Deserialize)]
    struct Config {
        #[serde(deserialize_with = "duration")]
        timeout: Duration,
    }

    #[test]
    fn a_string_parses_like_parse_duration() {
        let config: Config = serde_json::from_str(r#"{ "timeout": "1h 30m" }"#).unwrap();

        check!(config.timeout == Duration::from_secs(5400));
    }

    #[test]
    fn the_feature_turns_on_jiffs_serde() {
        let ts: jiff::Timestamp = serde_json::from_str(r#""2026-01-01T00:00:00Z""#).unwrap();

        check!(ts.as_second() == 1_767_225_600);
    }

    #[test]
    fn a_bad_string_or_a_number_is_an_error() {
        for json in [r#"{ "timeout": "-1s" }"#, r#"{ "timeout": 30 }"#] {
            check!(serde_json::from_str::<Config>(json).is_err(), "{json}");
        }
    }
}
