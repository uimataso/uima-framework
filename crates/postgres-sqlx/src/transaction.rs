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
pub struct PgTransaction {
    tx: Mutex<sqlx::Transaction<'static, sqlx::Postgres>>,
}

impl PgTransaction {
    /// The transaction's connection, held until the guard drops.
    pub async fn conn(&self) -> MappedMutexGuard<'_, PgConnection> {
        MutexGuard::map(self.tx.lock().await, |tx| &mut **tx)
    }
}

impl HasUnitOfWork for Postgres {
    type UoW = PgTransaction;

    async fn begin(&self) -> Result<PgTransaction, Failure> {
        let tx = self.pool.begin().await.map_err(failure)?;
        Ok(PgTransaction { tx: Mutex::new(tx) })
    }
}

impl UnitOfWork for PgTransaction {
    async fn commit(self) -> Result<(), Failure> {
        self.tx.into_inner().commit().await.map_err(failure)
    }

    async fn rollback(self) -> Result<(), Failure> {
        self.tx.into_inner().rollback().await.map_err(failure)
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::time::Duration;

    use uima_core::unit_of_work::contract::{self, Probe, concurrency};
    use uima_core::{FailureKind, Uuid};
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
    async fn begin_gives_up_on_a_full_pool_as_a_timeout() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let connection = PostgresConnection {
            url: server.url().clone(),
            database: DatabaseName::new("app").unwrap(),
        };
        let config = PostgresConfig {
            max_connections: MaxConnections::new(1).unwrap(),
            acquire_timeout: Duration::from_millis(300),
            ..PostgresConfig::default()
        };
        let postgres = Postgres::connect(&connection, &config).await.unwrap();

        let _held = postgres.pool().acquire().await.unwrap();
        let Err(error) = postgres.uow("create group").await else {
            panic!("the pool is full");
        };
        check!(error.to_string() == "failed to begin create group");
        check!(error.kind() == FailureKind::Timeout);
    }
}
