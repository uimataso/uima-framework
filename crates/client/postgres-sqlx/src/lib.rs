//! Connects a service to Postgres through sqlx.
//!
//! # Examples
//!
//! ```no_run
//! # async fn run() -> anyhow::Result<()> {
//! use sqlx::migrate::Migrator;
//! use uima_config::Env;
//! use uima_client_postgres_sqlx::{PostgresClient, PostgresConfig, PostgresTarget};
//!
//! # static MIGRATOR: Migrator = Migrator::DEFAULT;
//! // static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
//!
//! let env = Env::from_process();
//! let target = PostgresTarget::from_env(&env, "POSTGRES", "app")?;
//! let config = PostgresConfig::default().with_env(&env, "POSTGRES")?;
//!
//! let postgres = PostgresClient::connect(&target, &config).await?;
//! postgres.migrate(&MIGRATOR).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! - `testing`: adds the `testing` module, with `TestPostgres` for a database per test case.

use std::time::Duration;
use std::{fmt, io};

use anyhow::Context as _;
use sqlx::migrate::Migrator;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection, PgPool};
use tracing::info;
use uima_core::Failure;

mod config;
mod database_name;
mod error;
mod max_connections;
mod postgres_url;
mod target;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(test)]
mod tests;
mod transaction;

pub use config::PostgresConfig;
use config::whole_millis;
pub use database_name::DatabaseName;
pub use error::PgContext;
pub use max_connections::{InvalidMaxConnections, MaxConnections};
pub use postgres_url::{PostgresUrl, PostgresUrlErrorKind};
pub use target::PostgresTarget;
pub use transaction::PostgresTransaction;

/// A pool of connections to one database.
#[derive(Clone)]
pub struct PostgresClient {
    pool: PgPool,
}

impl PostgresClient {
    /// Connects, creating the database when it doesn't exist. Fails for a config value its
    /// field's doc doesn't allow.
    ///
    /// Checking the database, creating it and opening the pool each give up after
    /// `acquire_timeout`, so connecting can take up to three times it.
    ///
    /// Connect directly or through a pooler in session mode. A pooler in transaction mode drops
    /// `lock_timeout` and the migration lock with each transaction.
    pub async fn connect(target: &PostgresTarget, config: &PostgresConfig) -> anyhow::Result<Self> {
        let PostgresTarget { url, database } = target;
        config.validate()?;
        let options = url
            .connect_options(database)
            .with_context(|| format!("invalid postgres url {url}"))?;

        if !database_exists(target, &options, config.acquire_timeout).await? {
            create_database(target, &options, config.acquire_timeout).await?;
        }

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections.get())
            .acquire_timeout(config.acquire_timeout)
            .after_connect({
                let lock_timeout = config.lock_timeout;
                move |connection, _| {
                    Box::pin(async move { set_lock_timeout(connection, lock_timeout).await })
                }
            })
            .connect_with(options)
            .await
            .with_context(|| format!("failed to open a pool on database {database} on {url}"))?;

        info!(%url, %database, "connected to postgres");

        Ok(Self { pool })
    }

    /// Runs `migrator` on its own connection, without `lock_timeout`, so instances starting
    /// together can wait on the migration lock for as long as a migration takes. A migration
    /// waiting on a lock blocks every later statement on that table, so running instances' queries
    /// on it fail on their `lock_timeout` until the lock is released.
    pub async fn migrate(&self, migrator: &Migrator) -> anyhow::Result<()> {
        let pool = connect_once(
            &self.pool.connect_options(),
            self.pool.options().get_acquire_timeout(),
        )
        .await
        .context("failed to connect to postgres for migrating")?;

        let migrated = migrator.run(&pool).await;
        pool.close().await;
        migrated.context("failed to migrate postgres")?;

        info!("migrated postgres");

        Ok(())
    }

    /// The pool, whose connections carry `lock_timeout` for their whole session. Session state
    /// stays on a connection for whoever takes it next, so inside a transaction change a setting
    /// with `SET LOCAL` and take an advisory lock with `pg_advisory_xact_lock`.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// A connection from the pool, back in it when dropped, so a repo runs a statement the same
    /// way here and on a [`PostgresTransaction`]. Session state on it follows the rule on
    /// [`pool`](Self::pool).
    pub async fn conn(&self) -> Result<PoolConnection<sqlx::Postgres>, Failure> {
        self.pool
            .acquire()
            .await
            .pg_context("failed to take a pooled connection")
    }
}

impl fmt::Debug for PostgresClient {
    /// Shows the database, not the connect options, which hold the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresClient")
            .field("database", &self.pool.connect_options().get_database())
            .finish_non_exhaustive()
    }
}

/// Whether `target`'s database exists, found by connecting to it, so a user that can't reach
/// the `postgres` maintenance database, as on some hosted servers, works when its database
/// already exists.
async fn database_exists(
    target: &PostgresTarget,
    options: &PgConnectOptions,
    timeout: Duration,
) -> anyhow::Result<bool> {
    /// SQLSTATE `invalid_catalog_name`, for a database that doesn't exist.
    const MISSING_DATABASE: &str = "3D000";

    let PostgresTarget { url, database } = target;
    match connect_within(options, timeout).await {
        Ok(probe) => {
            let _ = probe.close().await;
            Ok(true)
        }
        Err(error) if has_sqlstate(&error, MISSING_DATABASE) => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("failed to connect to database {database} on {url}"))
        }
    }
}

/// Creates `target`'s database from the `postgres` maintenance database, taking one another
/// instance created first as created, and gives up after `timeout`.
async fn create_database(
    target: &PostgresTarget,
    options: &PgConnectOptions,
    timeout: Duration,
) -> anyhow::Result<()> {
    /// SQLSTATEs for a database another instance created first, `duplicate_database`, or
    /// `unique_violation` when both instances got past Postgres' own check.
    const CREATED_ELSEWHERE: [&str; 2] = ["42P04", "23505"];

    let PostgresTarget { url, database } = target;
    let created = within(timeout, async {
        let mut maintenance =
            PgConnection::connect_with(&options.clone().database("postgres")).await?;
        // A `DatabaseName` holds no `"`, so it can't end the quoted name.
        let create = format!(r#"CREATE DATABASE "{database}""#);
        let created = sqlx::raw_sql(AssertSqlSafe(create))
            .execute(&mut maintenance)
            .await;
        let _ = maintenance.close().await;
        created.map(|_| ())
    });
    match created.await {
        Ok(()) => {
            info!(%database, "created the database");
            Ok(())
        }
        Err(error)
            if CREATED_ELSEWHERE
                .iter()
                .any(|code| has_sqlstate(&error, code)) =>
        {
            Ok(())
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to create database {database} on {url}"))
        }
    }
}

/// One connection to `options`, given up after `timeout`.
async fn connect_within(
    options: &PgConnectOptions,
    timeout: Duration,
) -> Result<PgConnection, sqlx::Error> {
    within(timeout, PgConnection::connect_with(options)).await
}

/// `work`'s result, or a timeout error once `timeout` passes.
async fn within<T>(
    timeout: Duration,
    work: impl Future<Output = Result<T, sqlx::Error>>,
) -> Result<T, sqlx::Error> {
    tokio::time::timeout(timeout, work).await.map_err(|_| {
        sqlx::Error::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "the server didn't finish within the acquire timeout",
        ))
    })?
}

/// Sets `lock_timeout` on a new pool connection, before any query runs on it. Not on the
/// connection's startup options, which PgBouncer refuses, and through `set_config`, which binds
/// the value instead of writing it into the SQL.
async fn set_lock_timeout(
    connection: &mut PgConnection,
    lock_timeout: Duration,
) -> sqlx::Result<()> {
    sqlx::query("SELECT set_config('lock_timeout', $1, false)")
        .bind(format!("{}ms", whole_millis(lock_timeout)))
        .execute(connection)
        .await?;
    Ok(())
}

/// A pool of one connection to `options`, which gives up connecting after `timeout`. A pool,
/// not a bare connection, so the migrator's future is `Send`.
async fn connect_once(
    options: &PgConnectOptions,
    timeout: Duration,
) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(timeout)
        .connect_with(options.clone())
        .await
}

/// Whether `error` is a database error with the SQLSTATE `code`.
fn has_sqlstate(error: &sqlx::Error, code: &str) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|actual| actual == code)
}
