//! Connects a service to Postgres through sqlx.
//!
//! # Examples
//!
//! ```no_run
//! # async fn run() -> anyhow::Result<()> {
//! use sqlx::migrate::Migrator;
//! use uima_config::Env;
//! use uima_postgres_sqlx::{Postgres, PostgresConfig, PostgresConnection};
//!
//! # static MIGRATOR: Migrator = Migrator::DEFAULT;
//! // static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
//!
//! let env = Env::from_process();
//! let connection = PostgresConnection::from_env(&env, "POSTGRES", "app")?;
//! let config = PostgresConfig::default().with_env(&env, "POSTGRES")?;
//!
//! let postgres = Postgres::connect(&connection, &config).await?;
//! postgres.migrate(&MIGRATOR).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # TLS
//!
//! A server across a network takes `?sslmode=verify-full`, since the default, `prefer`, doesn't
//! check the server's certificate and falls back to no TLS.
//!
//! # Environment
//!
//! sqlx fills what the URL leaves out from the `PG*` variables, such as `PGSSLMODE`, `PGPASSWORD`
//! or `PGOPTIONS`, and from `~/.pgpass`, so set them only when the URL should take them.
//!
//! # Features
//!
//! - `testing`: adds the `testing` module, with `TestPostgres` for a database per test case.

use std::time::Duration;
use std::{fmt, io};

use anyhow::Context as _;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection, PgPool};
use tracing::info;

mod config;
mod connection;
mod database_name;
mod max_connections;
mod postgres_url;
#[cfg(feature = "testing")]
pub mod testing;

pub use config::PostgresConfig;
use config::whole_millis;
pub use connection::PostgresConnection;
pub use database_name::DatabaseName;
pub use max_connections::{InvalidMaxConnections, MaxConnections};
pub use postgres_url::{PostgresUrl, PostgresUrlErrorKind};

/// A pool of connections to one database.
#[derive(Clone)]
pub struct Postgres {
    pool: PgPool,
}

impl Postgres {
    /// Connects, creating the database when it doesn't exist. Fails for a config value its
    /// field's doc doesn't allow.
    ///
    /// Connect directly or through a pooler in session mode. A pooler in transaction mode drops
    /// `lock_timeout` and the migration lock with each transaction.
    pub async fn connect(
        connection: &PostgresConnection,
        config: &PostgresConfig,
    ) -> anyhow::Result<Self> {
        let PostgresConnection { url, database } = connection;
        config.validate()?;
        let options = url
            .connect_options(database)
            .with_context(|| format!("invalid postgres url {url}"))?;

        create_database_if_missing(connection, &options, config.acquire_timeout).await?;

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

    /// The pool, whose connections carry `lock_timeout` for their whole session. Change a setting
    /// with `SET LOCAL`, since a plain `SET`, `RESET` or `DISCARD` stays on the connection for
    /// whoever takes it next.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl fmt::Debug for Postgres {
    /// Shows the database, not the connect options, which hold the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Postgres")
            .field("database", &self.pool.connect_options().get_database())
            .finish_non_exhaustive()
    }
}

/// Creates `connection`'s database when it doesn't exist.
///
/// It tries the database first, so a user that can't reach the `postgres` maintenance database,
/// as on some hosted servers, works when its database already exists.
async fn create_database_if_missing(
    connection: &PostgresConnection,
    options: &PgConnectOptions,
    timeout: Duration,
) -> anyhow::Result<()> {
    /// SQLSTATE `invalid_catalog_name`, for a database that doesn't exist.
    const MISSING_DATABASE: &str = "3D000";
    /// SQLSTATEs for a database another instance created first, `duplicate_database`, or
    /// `unique_violation` when both instances got past Postgres' own check.
    const CREATED_ELSEWHERE: [&str; 2] = ["42P04", "23505"];

    let PostgresConnection { url, database } = connection;
    match connect_within(options, timeout).await {
        Ok(probe) => {
            let _ = probe.close().await;
            return Ok(());
        }
        Err(error) if has_sqlstate(&error, MISSING_DATABASE) => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to connect to database {database} on {url}"));
        }
    }

    let created = async {
        let mut maintenance =
            connect_within(&options.clone().database("postgres"), timeout).await?;
        // A `DatabaseName` holds no `"`, so it can't end the quoted name.
        let create = format!(r#"CREATE DATABASE "{database}""#);
        let created = sqlx::raw_sql(AssertSqlSafe(create))
            .execute(&mut maintenance)
            .await;
        let _ = maintenance.close().await;
        created.map(|_| ())
    };
    match created.await {
        Ok(()) => info!(%database, "created the database"),
        Err(error)
            if CREATED_ELSEWHERE
                .iter()
                .any(|code| has_sqlstate(&error, code)) => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create database {database} on {url}"));
        }
    }
    Ok(())
}

/// One connection to `options`, given up after `timeout`.
async fn connect_within(
    options: &PgConnectOptions,
    timeout: Duration,
) -> Result<PgConnection, sqlx::Error> {
    tokio::time::timeout(timeout, PgConnection::connect_with(options))
        .await
        .map_err(|_| {
            sqlx::Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "the server didn't answer within the acquire timeout",
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

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[tokio::test]
    async fn connect_fails_on_a_server_it_cant_reach() {
        for url in [
            "postgres://app:s3cr3t@127.0.0.1:1",
            "postgres://app:s3cr3t@10.255.255.1",
        ] {
            let connection = PostgresConnection {
                url: url.parse().unwrap(),
                database: DatabaseName::new("app").unwrap(),
            };
            let config = PostgresConfig {
                acquire_timeout: Duration::from_millis(300),
                ..PostgresConfig::default()
            };

            let error = tokio::time::timeout(
                Duration::from_secs(5),
                Postgres::connect(&connection, &config),
            )
            .await
            .expect("connect gives up within its acquire timeout")
            .unwrap_err();
            let chain = format!("{error:#}");
            check!(
                chain.contains("failed to connect to database app"),
                "{chain}"
            );
            check!(chain.contains(&url.replace("s3cr3t", "xxxxx")), "{chain}");
            check!(!chain.contains("s3cr3t"), "{chain}");
        }
    }

    #[tokio::test]
    async fn connect_rejects_a_lock_timeout_postgres_cant_take() {
        let connection = PostgresConnection {
            url: "postgres://app@127.0.0.1:1".parse().unwrap(),
            database: DatabaseName::new("app").unwrap(),
        };
        let config = PostgresConfig {
            lock_timeout: Duration::from_secs(30 * 24 * 60 * 60),
            ..PostgresConfig::default()
        };

        let error = Postgres::connect(&connection, &config).await.unwrap_err();
        let chain = format!("{error:#}");
        check!(chain.contains("lock_timeout"), "{chain}");
        check!(chain.contains("2147483647ms"), "{chain}");
        check!(!chain.contains("failed to connect"), "{chain}");
    }

    #[tokio::test]
    async fn connect_rejects_an_acquire_timeout_it_cant_wait() {
        let connection = PostgresConnection {
            url: "postgres://app@127.0.0.1:1".parse().unwrap(),
            database: DatabaseName::new("app").unwrap(),
        };
        for acquire_timeout in [Duration::ZERO, Duration::MAX] {
            let config = PostgresConfig {
                acquire_timeout,
                ..PostgresConfig::default()
            };

            let error = Postgres::connect(&connection, &config).await.unwrap_err();
            let chain = format!("{error:#}");
            check!(chain.contains("acquire_timeout"), "{chain}");
            check!(!chain.contains("failed to connect"), "{chain}");
        }
    }

    #[cfg(feature = "testing")]
    mod with_a_server {
        use uima_testing::check;

        use super::*;
        use crate::testing::{TestPostgres, test_migrator};

        async fn connect(
            postgres: &TestPostgres,
            database: &str,
            config: &PostgresConfig,
        ) -> Postgres {
            let connection = PostgresConnection {
                url: postgres.url().clone(),
                database: DatabaseName::new(database).unwrap(),
            };
            Postgres::connect(&connection, config).await.unwrap()
        }

        /// The server's URL with `host` in place of its own.
        fn on_host(server: &TestPostgres, host: &str) -> PostgresUrl {
            let mut url = server
                .url()
                .with_database(&DatabaseName::new("postgres").unwrap())
                .into_inner();
            url.set_path("");
            url.set_host(Some(host)).unwrap();
            PostgresUrl::new(url.as_str()).unwrap()
        }

        /// The server's URL as `role`.
        fn as_role(server: &TestPostgres, role: &str, password: &str) -> PostgresUrl {
            let mut url = server
                .url()
                .with_database(&DatabaseName::new("postgres").unwrap())
                .into_inner();
            url.set_path("");
            url.set_username(role).unwrap();
            url.set_password(Some(password)).unwrap();
            PostgresUrl::new(url.as_str()).unwrap()
        }

        async fn show(postgres: &Postgres, query: &'static str) -> String {
            sqlx::query_scalar(query)
                .fetch_one(postgres.pool())
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn connect_creates_a_missing_database_and_reuses_it() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let config = PostgresConfig::default();

            let created = connect(&server, "nova-el", &config).await;
            check!(show(&created, "SELECT current_database()").await == "nova-el");
            sqlx::query("CREATE TABLE kept (id INT)")
                .execute(created.pool())
                .await
                .unwrap();

            let again = connect(&server, "nova-el", &config).await;
            check!(show(&again, "SELECT to_regclass('kept')::text").await == "kept");
        }

        #[tokio::test]
        async fn connect_sets_the_lock_timeout_on_every_connection() {
            let server = TestPostgres::start(&test_migrator().await).await;

            for (lock_timeout, shown) in [
                (Duration::from_millis(1500), "1500ms"),
                (Duration::from_micros(1500), "2ms"),
                (Duration::from_micros(500), "1ms"),
                (Duration::ZERO, "0"),
            ] {
                let config = PostgresConfig {
                    lock_timeout,
                    ..PostgresConfig::default()
                };
                let postgres = connect(&server, "app", &config).await;
                check!(
                    show(&postgres, "SHOW lock_timeout").await == shown,
                    "{lock_timeout:?}"
                );
            }
        }

        #[tokio::test]
        async fn connect_creates_a_missing_database_as_a_role_that_can_only_create_databases() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let admin = connect(&server, "postgres", &PostgresConfig::default()).await;
            sqlx::raw_sql("CREATE ROLE app LOGIN CREATEDB PASSWORD 'app-password'")
                .execute(admin.pool())
                .await
                .unwrap();
            let connection = PostgresConnection {
                url: as_role(&server, "app", "app-password"),
                database: DatabaseName::new("app").unwrap(),
            };

            let postgres = Postgres::connect(&connection, &PostgresConfig::default())
                .await
                .unwrap();
            check!(show(&postgres, "SELECT current_database()").await == "app");
        }

        #[tokio::test]
        async fn connect_creates_a_missing_database_on_an_ipv6_host() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let connection = PostgresConnection {
                url: on_host(&server, "[::1]"),
                database: DatabaseName::new("app").unwrap(),
            };

            let postgres = Postgres::connect(&connection, &PostgresConfig::default())
                .await
                .unwrap();
            check!(show(&postgres, "SELECT current_database()").await == "app");
        }

        /// A URL for `server` through a proxy that forwards the first connection and leaves every
        /// later one unanswered.
        async fn stalls_after_the_first_connection(server: &TestPostgres) -> PostgresUrl {
            use tokio::net::{TcpListener, TcpStream};

            let port = server
                .url()
                .with_database(&DatabaseName::new("postgres").unwrap())
                .expose()
                .port()
                .unwrap();
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_port = proxy.local_addr().unwrap().port();
            tokio::spawn(async move {
                let (mut first, _) = proxy.accept().await.unwrap();
                let mut upstream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut first, &mut upstream).await;
                });
                let mut stalled = Vec::new();
                while let Ok((connection, _)) = proxy.accept().await {
                    stalled.push(connection);
                }
            });
            let mut url = on_host(server, "127.0.0.1")
                .with_database(&DatabaseName::new("postgres").unwrap())
                .into_inner();
            url.set_path("");
            url.set_port(Some(proxy_port)).unwrap();
            PostgresUrl::new(url.as_str()).unwrap()
        }

        /// Whether `connect` gives up within 10s on `url`, with an acquire timeout of 500ms.
        async fn gives_up(url: PostgresUrl) -> bool {
            let connection = PostgresConnection {
                url,
                database: DatabaseName::new("app").unwrap(),
            };
            let config = PostgresConfig {
                acquire_timeout: Duration::from_millis(500),
                ..PostgresConfig::default()
            };
            let connected = tokio::time::timeout(
                Duration::from_secs(10),
                Postgres::connect(&connection, &config),
            )
            .await;
            matches!(connected, Ok(Err(_)))
        }

        #[tokio::test]
        async fn connect_gives_up_on_a_server_that_stalls_after_the_probe() {
            let server = TestPostgres::start(&test_migrator().await).await;

            // `app` is missing, so the stalled connection is the one creating it.
            check!(gives_up(stalls_after_the_first_connection(&server).await).await);
        }

        #[tokio::test]
        async fn connect_gives_up_on_a_server_that_stalls_before_the_pool() {
            let server = TestPostgres::start(&test_migrator().await).await;
            connect(&server, "app", &PostgresConfig::default()).await;

            // `app` exists, so the stalled connection is the pool's first.
            check!(gives_up(stalls_after_the_first_connection(&server).await).await);
        }

        #[tokio::test]
        async fn connect_creates_a_database_once_when_instances_start_together() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let connection = PostgresConnection {
                url: server.url().clone(),
                database: DatabaseName::new("app").unwrap(),
            };
            let config = PostgresConfig::default();

            let mut connects = tokio::task::JoinSet::new();
            for _ in 0..8 {
                let (connection, config) = (connection.clone(), config.clone());
                connects.spawn(async move { Postgres::connect(&connection, &config).await });
            }
            while let Some(connected) = connects.join_next().await {
                check!(let Ok(Ok(_)) = connected);
            }
        }

        #[tokio::test]
        async fn connect_works_without_the_maintenance_database_when_its_database_exists() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let admin = connect(&server, "postgres", &PostgresConfig::default()).await;
            for statement in [
                "CREATE ROLE app LOGIN PASSWORD 'app-password'",
                "CREATE DATABASE app OWNER app",
                "REVOKE CONNECT ON DATABASE postgres FROM PUBLIC",
            ] {
                sqlx::raw_sql(statement)
                    .execute(admin.pool())
                    .await
                    .unwrap();
            }
            let url = as_role(&server, "app", "app-password");

            let existing = PostgresConnection {
                url: url.clone(),
                database: DatabaseName::new("app").unwrap(),
            };
            check!(let Ok(_) = Postgres::connect(&existing, &PostgresConfig::default()).await);

            let missing = PostgresConnection {
                url,
                database: DatabaseName::new("missing").unwrap(),
            };
            let error = Postgres::connect(&missing, &PostgresConfig::default())
                .await
                .unwrap_err();
            check!(format!("{error:#}").contains("failed to create database missing"));
        }

        #[tokio::test]
        async fn connect_reports_why_an_existing_database_refused() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let admin = connect(&server, "postgres", &PostgresConfig::default()).await;
            for statement in [
                "CREATE ROLE app LOGIN PASSWORD 'app-password'",
                "CREATE DATABASE closed",
                "REVOKE CONNECT ON DATABASE closed FROM PUBLIC",
            ] {
                sqlx::raw_sql(statement)
                    .execute(admin.pool())
                    .await
                    .unwrap();
            }
            let connection = PostgresConnection {
                url: as_role(&server, "app", "app-password"),
                database: DatabaseName::new("closed").unwrap(),
            };

            let error = Postgres::connect(&connection, &PostgresConfig::default())
                .await
                .unwrap_err();
            let chain = format!("{error:#}");
            check!(chain.contains("failed to connect to database closed"));
            check!(chain.contains("permission denied for database"));
            check!(!chain.contains("create"));
        }

        #[tokio::test]
        async fn the_pool_takes_its_size_and_acquire_timeout() {
            let server = TestPostgres::start(&test_migrator().await).await;
            // Long enough that connecting fits in it on a busy machine.
            let acquire_timeout = Duration::from_secs(2);
            let config = PostgresConfig {
                max_connections: MaxConnections::new(1).unwrap(),
                acquire_timeout,
                ..PostgresConfig::default()
            };
            let postgres = connect(&server, "app", &config).await;

            let _held = postgres.pool().acquire().await.unwrap();
            let started = std::time::Instant::now();
            let waited = tokio::time::timeout(Duration::from_secs(10), postgres.pool().acquire())
                .await
                .expect("the acquire gives up before 10s");
            check!(let Err(sqlx::Error::PoolTimedOut) = waited);
            check!(started.elapsed() >= acquire_timeout);
        }

        #[tokio::test]
        async fn every_connection_in_the_pool_carries_the_lock_timeout() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;

            let mut first = postgres.pool().acquire().await.unwrap();
            let mut second = postgres.pool().acquire().await.unwrap();
            for connection in [&mut first, &mut second] {
                let shown: String = sqlx::query_scalar("SHOW lock_timeout")
                    .fetch_one(&mut **connection)
                    .await
                    .unwrap();
                check!(shown == "2s");
            }
        }

        #[tokio::test]
        async fn migrate_works_when_instances_start_together() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let migrator = std::sync::Arc::new(test_migrator().await);
            let connection = PostgresConnection {
                url: server.url().clone(),
                database: DatabaseName::new("app").unwrap(),
            };

            let mut instances = tokio::task::JoinSet::new();
            for _ in 0..4 {
                let (connection, migrator) = (connection.clone(), migrator.clone());
                instances.spawn(async move {
                    let postgres =
                        Postgres::connect(&connection, &PostgresConfig::default()).await?;
                    postgres.migrate(&migrator).await
                });
            }
            while let Some(migrated) = instances.join_next().await {
                check!(let Ok(Ok(())) = migrated);
            }
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;
            check!(show(&postgres, "SELECT count(*)::text FROM _sqlx_migrations").await == "2");
        }

        #[tokio::test]
        async fn the_test_migrations_fail_with_a_lock_timeout() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;

            // On the pool, which sets lock_timeout, unlike `migrate`.
            let error = test_migrator()
                .await
                .run(postgres.pool())
                .await
                .unwrap_err();
            check!(error.to_string().contains("lock_timeout"), "{error}");
        }

        #[tokio::test]
        async fn migrate_can_run_on_another_task() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;
            let migrator = test_migrator().await;

            tokio::spawn(async move { postgres.migrate(&migrator).await })
                .await
                .unwrap()
                .unwrap();
        }

        #[tokio::test]
        async fn migrate_runs_without_the_lock_timeout() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;

            // The second migration fails when lock_timeout is set.
            postgres.migrate(&test_migrator().await).await.unwrap();
            check!(show(&postgres, "SELECT to_regclass('users')::text").await == "users");
        }

        #[tokio::test]
        async fn debug_doesnt_show_the_password() {
            let server = TestPostgres::start(&test_migrator().await).await;
            let postgres = connect(&server, "app", &PostgresConfig::default()).await;

            let debug = format!("{postgres:?}");
            check!(debug.contains(r#"database: Some("app")"#));
            check!(!debug.contains(crate::testing::PASSWORD));
        }
    }
}
