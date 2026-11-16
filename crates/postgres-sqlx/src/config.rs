//! [`PostgresConfig`], a pool's size and limits.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context as _;

use uima_config::{Env, EnvError};
use uima_time_jiff::{InvalidDuration, parse_duration};

use crate::MaxConnections;

/// The longest timeout, `i32::MAX` milliseconds, which Postgres takes for `lock_timeout`, and
/// which keeps the pool's deadlines far from overflowing the clock.
const MAX_TIMEOUT: Duration = Duration::from_millis(2_147_483_647);

/// The pool's size and limits.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// How many connections the pool opens at most.
    pub max_connections: MaxConnections,
    /// How long a query waits for a free connection, and connecting waits for the server. Longer
    /// than zero and at most `i32::MAX` milliseconds, about 24 days.
    pub acquire_timeout: Duration,
    /// How long a statement waits for a row or table lock, in whole milliseconds rounded up, at
    /// most `i32::MAX`, about 24 days, as Postgres takes it. Zero waits forever, as in Postgres.
    pub lock_timeout: Duration,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            max_connections: MaxConnections::new(10).expect("10 is in range"),
            acquire_timeout: Duration::from_secs(5),
            lock_timeout: Duration::from_secs(2),
        }
    }
}

impl PostgresConfig {
    /// Reads `{prefix}_MAX_CONNECTIONS`, `{prefix}_ACQUIRE_TIMEOUT` and `{prefix}_LOCK_TIMEOUT`,
    /// each falling back to `self`'s value. The timeouts are lengths such as `5s`.
    pub fn with_env(self, env: &Env, prefix: &str) -> Result<Self, EnvError> {
        Ok(Self {
            max_connections: env
                .var(&format!("{prefix}_MAX_CONNECTIONS"))
                .parse(MaxConnections::from_str)
                .or(self.max_connections)?,
            acquire_timeout: env
                .var(&format!("{prefix}_ACQUIRE_TIMEOUT"))
                .parse(parse_acquire_timeout)
                .or(self.acquire_timeout)?,
            lock_timeout: env
                .var(&format!("{prefix}_LOCK_TIMEOUT"))
                .parse(parse_lock_timeout)
                .or(self.lock_timeout)?,
        })
    }

    /// Checks each value against its field's doc, for a config built in code.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        check_acquire_timeout(self.acquire_timeout).context("invalid acquire_timeout")?;
        check_max(self.lock_timeout).context("invalid lock_timeout")?;
        Ok(())
    }
}

/// Reads an acquire timeout and checks it with [`check_acquire_timeout`].
fn parse_acquire_timeout(s: &str) -> Result<Duration, InvalidTimeout> {
    let timeout = parse_duration(s).map_err(InvalidTimeout::Duration)?;
    check_acquire_timeout(timeout)?;
    Ok(timeout)
}

/// Reads a lock timeout and checks it with [`check_max`], since zero waits forever.
fn parse_lock_timeout(s: &str) -> Result<Duration, InvalidTimeout> {
    let timeout = parse_duration(s).map_err(InvalidTimeout::Duration)?;
    check_max(timeout)?;
    Ok(timeout)
}

/// Checks an acquire timeout, which must be longer than zero, or every acquire would fail.
fn check_acquire_timeout(timeout: Duration) -> Result<(), InvalidTimeout> {
    if timeout.is_zero() {
        return Err(InvalidTimeout::Zero);
    }
    check_max(timeout)
}

/// Checks that `timeout` is at most [`MAX_TIMEOUT`].
fn check_max(timeout: Duration) -> Result<(), InvalidTimeout> {
    if timeout > MAX_TIMEOUT {
        return Err(InvalidTimeout::TooLong);
    }
    Ok(())
}

/// `duration` in milliseconds, rounded up so a short timeout doesn't become zero.
pub(crate) fn whole_millis(duration: Duration) -> u128 {
    duration.as_nanos().div_ceil(1_000_000)
}

/// Why a timeout was refused.
#[derive(Debug)]
enum InvalidTimeout {
    Duration(InvalidDuration),
    Zero,
    TooLong,
}

impl fmt::Display for InvalidTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duration(error) => error.fmt(f),
            Self::Zero => write!(f, "must be longer than zero"),
            Self::TooLong => write!(
                f,
                "is longer than the limit of {}ms",
                MAX_TIMEOUT.as_millis()
            ),
        }
    }
}

impl std::error::Error for InvalidTimeout {}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn keeps_its_values_when_unset() {
        let start = PostgresConfig {
            max_connections: MaxConnections::new(3).unwrap(),
            acquire_timeout: Duration::from_secs(9),
            lock_timeout: Duration::from_secs(7),
        };

        let config = start.clone().with_env(&Env::new(|_| None), "DB").unwrap();
        check!(config.max_connections == start.max_connections);
        check!(config.acquire_timeout == start.acquire_timeout);
        check!(config.lock_timeout == start.lock_timeout);
    }

    #[test]
    fn reads_each_value() {
        let env = Env::from_pairs([
            ("DB_MAX_CONNECTIONS", "20"),
            ("DB_ACQUIRE_TIMEOUT", "30s"),
            ("DB_LOCK_TIMEOUT", "500ms"),
        ]);

        let config = PostgresConfig::default().with_env(&env, "DB").unwrap();
        check!(config.max_connections.get() == 20);
        check!(config.acquire_timeout == Duration::from_secs(30));
        check!(config.lock_timeout == Duration::from_millis(500));
    }

    #[test]
    fn takes_a_zero_lock_timeout() {
        let env = Env::from_pairs([("DB_LOCK_TIMEOUT", "0")]);

        let config = PostgresConfig::default().with_env(&env, "DB").unwrap();
        check!(config.lock_timeout == Duration::ZERO);
    }

    #[test]
    fn rejects_a_timeout_past_the_limit() {
        for name in ["DB_ACQUIRE_TIMEOUT", "DB_LOCK_TIMEOUT"] {
            for value in ["24d 20h 31m 23s 648ms", "25d"] {
                let error = PostgresConfig::default()
                    .with_env(&Env::from_pairs([(name, value)]), "DB")
                    .unwrap_err();
                check!(error.name() == name, "{value}");
                let reason = std::error::Error::source(&error).unwrap().to_string();
                check!(
                    reason == "is longer than the limit of 2147483647ms",
                    "{name} {value}"
                );
            }
        }

        let env = Env::from_pairs([
            ("DB_ACQUIRE_TIMEOUT", "24d 20h 31m 23s 647ms"),
            ("DB_LOCK_TIMEOUT", "24d 20h 31m 23s 647ms"),
        ]);
        let config = PostgresConfig::default().with_env(&env, "DB").unwrap();
        check!(config.acquire_timeout == Duration::from_millis(2_147_483_647));
        check!(config.lock_timeout == Duration::from_millis(2_147_483_647));
    }

    #[test]
    fn validate_checks_a_config_built_in_code() {
        check!(PostgresConfig::default().validate().is_ok());
        for config in [
            PostgresConfig {
                acquire_timeout: Duration::ZERO,
                ..PostgresConfig::default()
            },
            PostgresConfig {
                acquire_timeout: Duration::from_secs(25 * 24 * 60 * 60),
                ..PostgresConfig::default()
            },
            PostgresConfig {
                lock_timeout: Duration::from_secs(25 * 24 * 60 * 60),
                ..PostgresConfig::default()
            },
        ] {
            check!(config.validate().is_err(), "{config:?}");
        }
    }

    #[test]
    fn rejects_a_value_that_doesnt_parse() {
        for (name, value) in [
            ("DB_MAX_CONNECTIONS", "many"),
            ("DB_MAX_CONNECTIONS", "0"),
            ("DB_ACQUIRE_TIMEOUT", "soon"),
            ("DB_ACQUIRE_TIMEOUT", "0"),
            ("DB_LOCK_TIMEOUT", "-1s"),
        ] {
            let error = PostgresConfig::default()
                .with_env(&Env::from_pairs([(name, value)]), "DB")
                .unwrap_err();
            check!(error.name() == name);
        }
    }

    #[test]
    fn defaults() {
        let config = PostgresConfig::default();
        check!(config.max_connections.get() == 10);
        check!(config.acquire_timeout == Duration::from_secs(5));
        check!(config.lock_timeout == Duration::from_secs(2));
    }
}
