//! [`PostgresLockProvider`], a [`LockProvider`] that keeps leases in a Postgres table.
//!
//! # Examples
//!
//! ```no_run
//! # async fn run(postgres: uima_client_postgres_sqlx::PostgresClient) -> anyhow::Result<()> {
//! use uima_lock_postgres_sqlx::{PostgresLockProvider, migrator};
//!
//! postgres.migrate(migrator()).await?;
//!
//! let (locks, driver) = uima_lock::new(PostgresLockProvider::new(postgres));
//! # Ok(())
//! # }
//! ```

use std::collections::HashSet;
use std::sync::LazyLock;

use sqlx::migrate::Migrator;
use uima_client_postgres_sqlx::{PgContext as _, PostgresClient};
use uima_core::{PortResult, Uuid};
use uima_lock::{Extended, Lease, LockKey, LockProvider, TryAcquire, Ttl};

/// The migrations that make the `uima_locks` table, recorded in `_uima_lock_migrations` apart from
/// the app's own, so the app runs them with [`PostgresClient::migrate`] beside its own.
///
/// A later release adds migrations that the release before keeps working with, as instances of
/// both run during a rolling deploy.
pub fn migrator() -> &'static Migrator {
    static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.dangerous_set_table_name("_uima_lock_migrations");
        migrator
    });
    &MIGRATOR
}

/// A [`LockProvider`] on the `uima_locks` table, which [`migrator`] makes.
///
/// A lease never let go stays in the table until its key is taken again.
#[derive(Debug, Clone)]
pub struct PostgresLockProvider {
    postgres: PostgresClient,
}

impl PostgresLockProvider {
    /// A provider on `postgres`, whose database must have run [`migrator`].
    pub fn new(postgres: PostgresClient) -> Self {
        Self { postgres }
    }
}

impl LockProvider for PostgresLockProvider {
    async fn try_acquire(&self, key: &LockKey, ttl: Ttl) -> PortResult<TryAcquire> {
        let token = Uuid::now_v7();
        if !query::acquire(self.postgres.pool(), key, token, ttl).await? {
            return Ok(TryAcquire::Busy);
        }
        Ok(TryAcquire::Acquired(Lease {
            key: key.clone(),
            token,
            ttl,
        }))
    }

    async fn extend(&self, leases: &[Lease]) -> PortResult<Vec<Extended>> {
        let held = query::extend(self.postgres.pool(), leases).await?;
        let extended = leases
            .iter()
            .map(|lease| {
                if held.contains(&lease.token) {
                    Extended::Held
                } else {
                    Extended::Lost
                }
            })
            .collect();
        Ok(extended)
    }

    async fn unlock(&self, lease: &Lease) -> PortResult<()> {
        query::unlock(self.postgres.pool(), lease).await
    }
}

mod query {
    use indoc::indoc;
    use sqlx::PgExecutor;

    use super::*;

    /// Takes `key` for `token` when it is free or its lease has expired, giving whether it did.
    pub async fn acquire<'c>(
        e: impl PgExecutor<'c>,
        key: &LockKey,
        token: Uuid,
        ttl: Ttl,
    ) -> PortResult<bool> {
        // The check's `clock_timestamp()` is after any wait on another acquiring the same key. The
        // new expiry may start before that wait, leaving the lease at most that much shorter.
        let taken: Option<Uuid> = sqlx::query_scalar(indoc! {"
            INSERT INTO uima_locks (key, token, expires_at)
            VALUES ($1, $2, clock_timestamp() + $3 * interval '1 microsecond')
            ON CONFLICT (key) DO UPDATE
              SET token = excluded.token, expires_at = excluded.expires_at
              WHERE uima_locks.expires_at <= clock_timestamp()
            RETURNING token
        "})
        .bind(key.as_str())
        .bind(token)
        .bind(micros(ttl))
        .fetch_optional(e)
        .await
        .pg_context(format!("failed to acquire the lock {key}"))?;
        Ok(taken.is_some())
    }

    /// Extends each of `leases` still held by its own ttl, giving the tokens of those it did.
    pub async fn extend<'c>(e: impl PgExecutor<'c>, leases: &[Lease]) -> PortResult<HashSet<Uuid>> {
        let keys: Vec<&str> = leases.iter().map(|lease| lease.key.as_str()).collect();
        let tokens: Vec<Uuid> = leases.iter().map(|lease| lease.token).collect();
        let ttls: Vec<i64> = leases.iter().map(|lease| micros(lease.ttl)).collect();

        let held: Vec<Uuid> = sqlx::query_scalar(indoc! {"
            UPDATE uima_locks AS lock
            SET expires_at = clock_timestamp() + lease.ttl * interval '1 microsecond'
            FROM unnest($1::text[], $2::uuid[], $3::int8[]) AS lease (key, token, ttl)
            WHERE lock.key = lease.key AND lock.token = lease.token
              AND lock.expires_at > clock_timestamp()
            RETURNING lock.token
        "})
        .bind(keys)
        .bind(tokens)
        .bind(ttls)
        .fetch_all(e)
        .await
        .pg_context(format!("failed to extend {} locks", leases.len()))?;

        Ok(held.into_iter().collect())
    }

    /// Lets `lease` go, if it is still the key's.
    pub async fn unlock<'c>(e: impl PgExecutor<'c>, lease: &Lease) -> PortResult<()> {
        sqlx::query("DELETE FROM uima_locks WHERE key = $1 AND token = $2")
            .bind(lease.key.as_str())
            .bind(lease.token)
            .execute(e)
            .await
            .pg_context(format!("failed to unlock the lock {}", lease.key))?;
        Ok(())
    }

    /// `ttl` in whole microseconds, the finest an `interval` keeps.
    fn micros(ttl: Ttl) -> i64 {
        i64::try_from(ttl.get().as_micros())
            .expect("a ttl of at most a year fits in i64 microseconds")
    }
}

#[cfg(test)]
mod tests {
    use uima_client_postgres_sqlx::testing::TestPostgres;
    use uima_lock::contract;

    use super::*;

    #[tokio::test]
    async fn it_passes_the_contract() {
        // Beside an app's migrations, as an app runs them.
        let app = sqlx::migrate!("./testdata/app-migrations");
        let postgres = TestPostgres::start_with_migrators(&[&app, migrator()]).await;
        contract::run_all(async || PostgresLockProvider::new(postgres.isolated().await)).await;
    }
}
