//! [`PgContext`], which turns a sqlx error into a [`Failure`] or a port's outcome.

use std::{fmt, io};

use sqlx::error::DatabaseError;
use uima_core::{Failure, FailureKind, PortError, PortResult};

/// Turns a sqlx error into a [`Failure`] of the kind its cause shows, or into an outcome of the
/// port's contract, so a port returns what happened without its callers depending on sqlx.
///
/// ```no_run
/// # #[derive(Debug)]
/// # enum MemberError { NoGroup }
/// # async fn run(pool: &sqlx::PgPool) -> uima_core::PortResult<(), MemberError> {
/// use uima_postgres_sqlx::PgContext as _;
///
/// sqlx::query("DELETE FROM sessions")
///     .execute(pool)
///     .await
///     .pg_context("failed to delete the sessions")?;
///
/// sqlx::query("INSERT INTO members (group_id) VALUES (7)")
///     .execute(pool)
///     .await
///     .pg_contract("failed to add the member", |error| match error.constraint() {
///         Some("members_group_id_fkey") => Some(MemberError::NoGroup),
///         _ => None,
///     })?;
/// # Ok(())
/// # }
/// ```
pub trait PgContext<T> {
    /// The error wrapped in `context`, with the [`FailureKind`] its cause shows.
    fn pg_context<C>(self, context: C) -> Result<T, Failure>
    where
        C: fmt::Display + Send + Sync + 'static;

    /// The outcome `contract` gives for the server's error, or else the error as
    /// [`pg_context`](Self::pg_context) gives it. `contract` sees only an error the server sent.
    fn pg_contract<C, E>(
        self,
        context: C,
        contract: impl FnOnce(&dyn DatabaseError) -> Option<E>,
    ) -> PortResult<T, E>
    where
        C: fmt::Display + Send + Sync + 'static;
}

impl<T> PgContext<T> for Result<T, sqlx::Error> {
    fn pg_context<C>(self, context: C) -> Result<T, Failure>
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|error| failure(error).context(context))
    }

    fn pg_contract<C, E>(
        self,
        context: C,
        contract: impl FnOnce(&dyn DatabaseError) -> Option<E>,
    ) -> PortResult<T, E>
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|error| match error.as_database_error().and_then(contract) {
            Some(outcome) => PortError::Contract(outcome),
            None => failure(error).context(context).into(),
        })
    }
}

/// `error` as a [`Failure`] of the kind its cause shows.
pub(crate) fn failure(error: sqlx::Error) -> Failure {
    Failure::new(kind(&error), error)
}

/// What `error` shows happened, by its SQLSTATE when the server sent one.
fn kind(error: &sqlx::Error) -> FailureKind {
    match error {
        sqlx::Error::PoolTimedOut => FailureKind::Timeout,
        sqlx::Error::Io(error) if error.kind() == io::ErrorKind::TimedOut => FailureKind::Timeout,
        sqlx::Error::Io(_) => FailureKind::Unavailable,
        sqlx::Error::Database(error) => match error.code().as_deref() {
            // serialization_failure, deadlock_detected
            Some("40001" | "40P01") => FailureKind::Conflict,
            // lock_not_available, query_canceled (statement_timeout)
            Some("55P03" | "57014") => FailureKind::Timeout,
            // too_many_connections, admin_shutdown, crash_shutdown, cannot_connect_now
            Some("53300" | "57P01" | "57P02" | "57P03") => FailureKind::Unavailable,
            // connection_exception and its subclasses
            Some(code) if code.starts_with("08") => FailureKind::Unavailable,
            _ => FailureKind::Other,
        },
        _ => FailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use sqlx::Connection as _;
    use uima_testing::check;

    use super::*;

    #[test]
    fn waiting_too_long_is_a_timeout() {
        let timed_out = sqlx::Error::Io(io::Error::new(io::ErrorKind::TimedOut, "slow"));
        for error in [sqlx::Error::PoolTimedOut, timed_out] {
            check!(failure(error).kind() == FailureKind::Timeout);
        }
    }

    #[tokio::test]
    async fn a_server_it_cant_reach_is_unavailable() {
        let error = sqlx::PgConnection::connect("postgres://app@127.0.0.1:1")
            .await
            .unwrap_err();

        check!(failure(error).kind() == FailureKind::Unavailable);
    }

    #[test]
    fn a_contract_leaves_an_error_the_server_didnt_send_as_a_failure() {
        let mut asked = false;
        let result = Err::<(), _>(sqlx::Error::PoolTimedOut).pg_contract("failed", |_| {
            asked = true;
            Some(())
        });

        check!(!asked);
        let Err(PortError::Failed(error)) = result else {
            panic!("the error stays a failure, got {result:?}");
        };
        check!(error.kind() == FailureKind::Timeout);
    }

    #[test]
    fn the_context_wraps_the_cause() {
        let failure = Err::<(), _>(sqlx::Error::PoolTimedOut)
            .pg_context("failed to delete the sessions")
            .unwrap_err();

        check!(failure.to_string() == "failed to delete the sessions");
        check!(failure.kind() == FailureKind::Timeout);
        let cause = std::error::Error::source(&failure).unwrap().to_string();
        check!(cause.contains("pool timed out"));
    }

    #[cfg(feature = "testing")]
    mod with_a_server {
        use std::time::Duration;

        use uima_testing::check;

        use super::*;
        use crate::testing::{TestPostgres, test_migrator};
        use crate::{Postgres, PostgresConfig, PostgresConnection};

        #[tokio::test]
        async fn each_sqlstate_takes_its_kind() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = server.isolated().await;

            for (code, kind) in [
                ("40001", FailureKind::Conflict),
                ("40P01", FailureKind::Conflict),
                ("55P03", FailureKind::Timeout),
                ("57014", FailureKind::Timeout),
                ("08006", FailureKind::Unavailable),
                ("53300", FailureKind::Unavailable),
                ("57P01", FailureKind::Unavailable),
                ("57P03", FailureKind::Unavailable),
                ("23505", FailureKind::Other),
                ("42601", FailureKind::Other),
            ] {
                let raise = format!("DO $$ BEGIN RAISE EXCEPTION USING ERRCODE = '{code}'; END $$");
                let error = sqlx::raw_sql(sqlx::AssertSqlSafe(raise))
                    .execute(postgres.pool())
                    .await
                    .unwrap_err();
                check!(failure(error).kind() == kind, "{code}");
            }
        }

        #[derive(Debug, PartialEq)]
        enum MemberError {
            NoGroup,
        }

        async fn add_member(postgres: &Postgres) -> PortResult<(), MemberError> {
            sqlx::query("INSERT INTO members (group_id) VALUES (7)")
                .execute(postgres.pool())
                .await
                .pg_contract("failed to add the member", |error| {
                    match error.constraint() {
                        Some("members_group_id_fkey") => Some(MemberError::NoGroup),
                        _ => None,
                    }
                })?;
            Ok(())
        }

        async fn with_members(server: &TestPostgres) -> Postgres {
            let postgres = server.isolated().await;
            sqlx::raw_sql(
                "CREATE TABLE groups (id INT PRIMARY KEY); \
                 CREATE TABLE members (group_id INT REFERENCES groups, UNIQUE (group_id))",
            )
            .execute(postgres.pool())
            .await
            .unwrap();
            postgres
        }

        #[tokio::test]
        async fn a_contract_maps_the_errors_it_names() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = with_members(&server).await;

            check!(let Err(PortError::Contract(MemberError::NoGroup)) = add_member(&postgres).await);
        }

        #[tokio::test]
        async fn a_contract_leaves_the_errors_it_doesnt_name_as_failures() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = with_members(&server).await;
            sqlx::raw_sql("INSERT INTO groups VALUES (7); INSERT INTO members VALUES (7)")
                .execute(postgres.pool())
                .await
                .unwrap();

            let result = add_member(&postgres).await;
            let Err(PortError::Failed(error)) = result else {
                panic!("the error stays a failure, got {result:?}");
            };
            check!(error.to_string() == "failed to add the member");
            check!(error.kind() == FailureKind::Other);
        }

        #[tokio::test]
        async fn a_lock_held_past_the_lock_timeout_is_a_timeout() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let connection = PostgresConnection {
                url: server.url().clone(),
                database: crate::DatabaseName::new("app").unwrap(),
            };
            let config = PostgresConfig {
                lock_timeout: Duration::from_millis(200),
                ..PostgresConfig::default()
            };
            let postgres = Postgres::connect(&connection, &config).await.unwrap();
            sqlx::raw_sql(
                "CREATE TABLE locked (id INT PRIMARY KEY); INSERT INTO locked VALUES (1)",
            )
            .execute(postgres.pool())
            .await
            .unwrap();

            let mut holder = postgres.pool().begin().await.unwrap();
            sqlx::query("SELECT id FROM locked WHERE id = 1 FOR UPDATE")
                .execute(&mut *holder)
                .await
                .unwrap();
            let error = sqlx::query("UPDATE locked SET id = 2 WHERE id = 1")
                .execute(postgres.pool())
                .await
                .unwrap_err();

            check!(failure(error).kind() == FailureKind::Timeout);
        }
    }
}
