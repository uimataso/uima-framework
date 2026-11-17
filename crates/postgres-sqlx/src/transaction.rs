//! [`PgTransaction`], a Postgres transaction as a [`UnitOfWork`].

use sqlx::PgConnection;
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use uima_core::{Failure, HasUnitOfWork, UnitOfWork};

use crate::Postgres;
use crate::error::failure;

/// A transaction on one pool connection, which rolls back when dropped without `commit`.
///
/// An app implements its repos on it as on its context, reaching the connection through
/// [`conn`](Self::conn).
#[derive(Debug)]
pub struct PgTransaction {
    tx: Mutex<sqlx::Transaction<'static, sqlx::Postgres>>,
}

impl PgTransaction {
    /// The transaction's connection, held until the guard drops. Another `conn` on the same
    /// transaction waits for that, so drop the guard before calling another of its methods.
    ///
    /// A setting changed on it follows the rule on [`Postgres::pool`]. Take an advisory lock with
    /// `pg_advisory_xact_lock`, since a session lock stays on the connection after the transaction.
    pub async fn conn(&self) -> MappedMutexGuard<'_, PgConnection> {
        MutexGuard::map(self.tx.lock().await, |tx| &mut **tx)
    }
}

impl HasUnitOfWork for Postgres {
    type Uow = PgTransaction;

    async fn begin(&self) -> Result<PgTransaction, Failure> {
        let tx = self.pool.begin().await.map_err(failure)?;
        Ok(PgTransaction { tx: Mutex::new(tx) })
    }
}

impl UnitOfWork for PgTransaction {
    /// Fails when a statement failed earlier, since Postgres then answers `COMMIT` by rolling
    /// back without an error. A deferred constraint fails here as a [`Failure`], so a port whose
    /// contract names it checks it first with `SET CONSTRAINTS ... IMMEDIATE`.
    async fn commit(self) -> Result<(), Failure> {
        let mut tx = self.tx.into_inner();
        sqlx::raw_sql("SELECT 1")
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(failure)
    }

    async fn rollback(self) -> Result<(), Failure> {
        self.tx.into_inner().rollback().await.map_err(failure)
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use uima_core::unit_of_work::contract::{self, Probe, concurrency};
    use uima_core::{FailureKind, ScopedUow, Uuid};
    use uima_testing::check;

    use super::*;
    use crate::testing::{TestPostgres, test_migrator};
    use crate::{DatabaseName, MaxConnections, PostgresConfig, PostgresConnection};

    const WRITE: &str = "INSERT INTO probes (key) VALUES ($1::uuid)";
    const EXISTS: &str = "SELECT EXISTS (SELECT 1 FROM probes WHERE key = $1::uuid)";

    impl Probe for Postgres {
        async fn write(&self, key: Uuid) -> anyhow::Result<()> {
            sqlx::query(WRITE)
                .bind(key.to_string())
                .execute(self.pool())
                .await?;
            Ok(())
        }

        async fn exists(&self, key: Uuid) -> anyhow::Result<bool> {
            Ok(sqlx::query_scalar(EXISTS)
                .bind(key.to_string())
                .fetch_one(self.pool())
                .await?)
        }
    }

    impl Probe for PgTransaction {
        async fn write(&self, key: Uuid) -> anyhow::Result<()> {
            sqlx::query(WRITE)
                .bind(key.to_string())
                .execute(&mut *self.conn().await)
                .await?;
            Ok(())
        }

        async fn exists(&self, key: Uuid) -> anyhow::Result<bool> {
            Ok(sqlx::query_scalar(EXISTS)
                .bind(key.to_string())
                .fetch_one(&mut *self.conn().await)
                .await?)
        }
    }

    #[tokio::test]
    async fn contract() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let isolated = async || server.isolated().await;

        contract::run_all(isolated).await;
        concurrency::run_all(isolated).await;
    }

    #[tokio::test]
    async fn a_deferred_constraint_fails_the_commit() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let uow = server.isolated().await.uow("create group").await.unwrap();
        sqlx::raw_sql(
            "CREATE TEMP TABLE deferred (key INT UNIQUE DEFERRABLE INITIALLY DEFERRED); \
             INSERT INTO deferred VALUES (1), (1)",
        )
        .execute(&mut *uow.conn().await)
        .await
        .unwrap();

        let error = uow.commit().await.unwrap_err();
        check!(error.to_string() == "failed to commit create group");
    }

    #[tokio::test]
    async fn begin_gives_up_on_a_full_pool_as_a_timeout() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let connection = PostgresConnection {
            url: server.url().clone(),
            database: DatabaseName::new("app").unwrap(),
        };
        let config = PostgresConfig {
            max_connections: MaxConnections::new(1).unwrap(),
            ..PostgresConfig::default()
        };
        let postgres = Postgres::connect(&connection, &config).await.unwrap();

        let _held = postgres.pool().acquire().await.unwrap();
        let error = postgres.uow("create group").await.unwrap_err();
        check!(error.to_string() == "failed to begin create group");
        check!(error.kind() == FailureKind::Timeout);
    }

    /// A unit of work whose connection the server has closed.
    async fn cut_off(postgres: &Postgres) -> ScopedUow<PgTransaction> {
        let uow = postgres.uow("create group").await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *uow.conn().await)
            .await
            .unwrap();
        // Waits until the backend has exited.
        sqlx::query("SELECT pg_terminate_backend($1, 5000)")
            .bind(pid)
            .execute(postgres.pool())
            .await
            .unwrap();
        uow
    }

    #[tokio::test]
    async fn ending_a_unit_of_work_on_a_lost_connection_is_unavailable() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let postgres = server.isolated().await;

        let error = cut_off(&postgres).await.commit().await.unwrap_err();
        check!(error.to_string() == "failed to commit create group");
        check!(error.kind() == FailureKind::Unavailable);

        let error = cut_off(&postgres).await.rollback().await.unwrap_err();
        check!(error.to_string() == "failed to roll back create group");
        check!(error.kind() == FailureKind::Unavailable);
    }

    /// A serializable unit of work that has read the probes.
    async fn serializable(postgres: &Postgres) -> ScopedUow<PgTransaction> {
        let uow = postgres.uow("create group").await.unwrap();
        let mut conn = uow.conn().await;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("SELECT count(*) FROM probes")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
        uow
    }

    #[tokio::test]
    async fn a_commit_that_conflicts_is_a_conflict() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let postgres = server.isolated().await;

        // Each reads the probes and then writes one, so one of them must fail.
        let (first, second) = (serializable(&postgres).await, serializable(&postgres).await);
        first.write(Uuid::now_v7()).await.unwrap();
        second.write(Uuid::now_v7()).await.unwrap();
        first.commit().await.unwrap();

        let error = second.commit().await.unwrap_err();
        check!(error.to_string() == "failed to commit create group");
        check!(error.kind() == FailureKind::Conflict);
    }
}
