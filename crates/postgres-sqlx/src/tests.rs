//! Tests of connecting and migrating.

use uima_testing::check;

use super::*;

#[tokio::test]
async fn connect_fails_on_a_server_it_cant_reach() {
    for url in [
        "postgres://app:s3cr3t@127.0.0.1:1",
        "postgres://app:s3cr3t@10.255.255.1",
    ] {
        let target = PostgresTarget {
            url: url.parse().unwrap(),
            database: DatabaseName::new("app").unwrap(),
        };
        let config = PostgresConfig {
            acquire_timeout: Duration::from_millis(300),
            ..PostgresConfig::default()
        };

        let error = tokio::time::timeout(
            Duration::from_secs(5),
            PostgresClient::connect(&target, &config),
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
    let target = PostgresTarget {
        url: "postgres://app@127.0.0.1:1".parse().unwrap(),
        database: DatabaseName::new("app").unwrap(),
    };
    let config = PostgresConfig {
        lock_timeout: Duration::from_secs(30 * 24 * 60 * 60),
        ..PostgresConfig::default()
    };

    let error = PostgresClient::connect(&target, &config).await.unwrap_err();
    let chain = format!("{error:#}");
    check!(chain.contains("lock_timeout"), "{chain}");
    check!(chain.contains("2147483647ms"), "{chain}");
    check!(!chain.contains("failed to connect"), "{chain}");
}

#[tokio::test]
async fn connect_rejects_an_acquire_timeout_it_cant_wait() {
    let target = PostgresTarget {
        url: "postgres://app@127.0.0.1:1".parse().unwrap(),
        database: DatabaseName::new("app").unwrap(),
    };
    for acquire_timeout in [Duration::ZERO, Duration::MAX] {
        let config = PostgresConfig {
            acquire_timeout,
            ..PostgresConfig::default()
        };

        let error = PostgresClient::connect(&target, &config).await.unwrap_err();
        let chain = format!("{error:#}");
        check!(chain.contains("acquire_timeout"), "{chain}");
        check!(!chain.contains("failed to connect"), "{chain}");
    }
}

#[cfg(feature = "testing")]
mod with_a_server {
    use std::sync::atomic::{AtomicU32, Ordering};

    use uima_core::FailureKind;
    use uima_testing::{assert, check, run_case};

    use super::*;
    use crate::testing::{TestPostgres, test_migrator};

    /// One case on a server it shares with the other cases. Databases and roles are the server's,
    /// so a case names its own through `database` and `role` to keep them from the others'.
    struct Case<'a> {
        server: &'a TestPostgres,
        id: u32,
    }

    impl<'a> Case<'a> {
        fn new(server: &'a TestPostgres, created: &AtomicU32) -> Self {
            Self {
                server,
                id: created.fetch_add(1, Ordering::Relaxed),
            }
        }

        /// The database `name`, made this case's own, such as `app-3`.
        fn database(&self, name: &str) -> DatabaseName {
            DatabaseName::new(format!("{name}-{}", self.id)).unwrap()
        }

        /// The role `name`, made this case's own, such as `app_3`.
        fn role(&self, name: &str) -> String {
            format!("{name}_{}", self.id)
        }

        /// This case's database `name`, checked to be missing, for a case that needs `connect` to
        /// create it.
        async fn missing_database(&self, name: &str) -> DatabaseName {
            let database = self.database(name);
            let admin = connect(
                self.server,
                DatabaseName::new("postgres").unwrap(),
                &PostgresConfig::default(),
            )
            .await;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_database WHERE datname = $1)")
                    .bind(database.as_str())
                    .fetch_one(admin.pool())
                    .await
                    .unwrap();
            assert!(!exists, "{database} exists");
            database
        }

        /// Connects as the superuser to this case's database `name`.
        async fn connect(&self, name: &str, config: &PostgresConfig) -> PostgresClient {
            connect(self.server, self.database(name), config).await
        }
    }

    async fn connect(
        server: &TestPostgres,
        database: DatabaseName,
        config: &PostgresConfig,
    ) -> PostgresClient {
        let target = PostgresTarget {
            url: server.url().clone(),
            database,
        };
        PostgresClient::connect(&target, config).await.unwrap()
    }

    /// Runs `statement` as the superuser on the `postgres` database.
    async fn admin(server: &TestPostgres, statement: &str) {
        let admin = connect(
            server,
            DatabaseName::new("postgres").unwrap(),
            &PostgresConfig::default(),
        )
        .await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(admin.pool())
            .await
            .unwrap();
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

    async fn show(postgres: &PostgresClient, query: &'static str) -> String {
        sqlx::query_scalar(query)
            .fetch_one(postgres.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn connect_creates_its_database_or_says_why_not() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let created = AtomicU32::new(0);
        let isolated = async || Case::new(&server, &created);

        run_case(&isolated, connect_creates_a_missing_database_and_reuses_it).await;
        run_case(
            &isolated,
            connect_creates_a_missing_database_as_a_role_that_can_only_create_databases,
        )
        .await;
        run_case(
            &isolated,
            connect_creates_a_missing_database_on_an_ipv6_host,
        )
        .await;
        run_case(
            &isolated,
            connect_creates_a_database_once_when_instances_start_together,
        )
        .await;
        run_case(&isolated, connect_reports_why_an_existing_database_refused).await;
    }

    #[tokio::test]
    async fn the_client_follows_its_config_and_hides_the_password() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let created = AtomicU32::new(0);
        let isolated = async || Case::new(&server, &created);

        run_case(&isolated, connect_sets_the_lock_timeout_on_every_connection).await;
        run_case(&isolated, conn_takes_a_pooled_connection_and_gives_it_back).await;
        run_case(&isolated, the_pool_takes_its_size_and_acquire_timeout).await;
        run_case(
            &isolated,
            every_connection_in_the_pool_carries_the_lock_timeout,
        )
        .await;
        run_case(&isolated, debug_doesnt_show_the_password).await;
    }

    #[tokio::test]
    async fn connect_gives_up_on_a_stalled_server() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let created = AtomicU32::new(0);
        let isolated = async || Case::new(&server, &created);

        run_case(
            &isolated,
            connect_gives_up_on_a_server_that_stalls_after_the_probe,
        )
        .await;
        run_case(
            &isolated,
            connect_gives_up_on_a_server_that_stalls_creating_the_database,
        )
        .await;
        run_case(
            &isolated,
            connect_gives_up_on_a_server_that_stalls_before_the_pool,
        )
        .await;
    }

    #[tokio::test]
    async fn migrate_migrates_a_new_database() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let created = AtomicU32::new(0);
        let isolated = async || Case::new(&server, &created);

        run_case(&isolated, migrate_works_when_instances_start_together).await;
        run_case(&isolated, the_test_migrations_fail_with_a_lock_timeout).await;
        run_case(&isolated, migrate_can_run_on_another_task).await;
        run_case(&isolated, migrate_runs_without_the_lock_timeout).await;
    }

    async fn connect_creates_a_missing_database_and_reuses_it(case: &Case<'_>) {
        let config = PostgresConfig::default();

        let created = connect(case.server, case.missing_database("nova-el").await, &config).await;
        check!(
            show(&created, "SELECT current_database()").await == case.database("nova-el").as_str()
        );
        sqlx::query("CREATE TABLE kept (id INT)")
            .execute(created.pool())
            .await
            .unwrap();

        let again = case.connect("nova-el", &config).await;
        check!(show(&again, "SELECT to_regclass('kept')::text").await == "kept");
    }

    async fn connect_sets_the_lock_timeout_on_every_connection(case: &Case<'_>) {
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
            let postgres = case.connect("app", &config).await;
            check!(
                show(&postgres, "SHOW lock_timeout").await == shown,
                "{lock_timeout:?}"
            );
        }
    }

    async fn conn_takes_a_pooled_connection_and_gives_it_back(case: &Case<'_>) {
        let config = PostgresConfig {
            max_connections: MaxConnections::new(1).unwrap(),
            ..PostgresConfig::default()
        };
        let postgres = case.connect("app", &config).await;

        let held = postgres.conn().await.unwrap();
        let error = postgres.conn().await.unwrap_err();
        check!(error.to_string() == "failed to take a pooled connection");
        check!(error.kind() == FailureKind::Timeout);

        drop(held);
        check!(postgres.conn().await.is_ok());
    }

    async fn connect_creates_a_missing_database_as_a_role_that_can_only_create_databases(
        case: &Case<'_>,
    ) {
        let role = case.role("app");
        admin(
            case.server,
            &format!("CREATE ROLE {role} LOGIN CREATEDB PASSWORD 'app-password'"),
        )
        .await;
        let target = PostgresTarget {
            url: as_role(case.server, &role, "app-password"),
            database: case.missing_database("app").await,
        };

        let postgres = PostgresClient::connect(&target, &PostgresConfig::default())
            .await
            .unwrap();
        check!(show(&postgres, "SELECT current_database()").await == target.database.as_str());
    }

    async fn connect_creates_a_missing_database_on_an_ipv6_host(case: &Case<'_>) {
        let target = PostgresTarget {
            url: on_host(case.server, "[::1]"),
            database: case.missing_database("app").await,
        };

        let postgres = PostgresClient::connect(&target, &PostgresConfig::default())
            .await
            .unwrap();
        check!(show(&postgres, "SELECT current_database()").await == target.database.as_str());
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

    /// A URL for `server` through a proxy that stops forwarding a connection once its client
    /// sends `statement`.
    async fn stalls_on(server: &TestPostgres, statement: &'static [u8]) -> PostgresUrl {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
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
            while let Ok((client, _)) = proxy.accept().await {
                let upstream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                let (mut client_read, mut client_write) = client.into_split();
                let (mut upstream_read, mut upstream_write) = upstream.into_split();
                tokio::spawn(async move {
                    let _ = tokio::io::copy(&mut upstream_read, &mut client_write).await;
                });
                tokio::spawn(async move {
                    let mut buffer = vec![0; 8192];
                    while let Ok(read @ 1..) = client_read.read(&mut buffer).await {
                        let sent = &buffer[..read];
                        if sent.windows(statement.len()).any(|w| w == statement) {
                            // Holds the connection open without answering.
                            std::future::pending::<()>().await;
                        }
                        if upstream_write.write_all(sent).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let mut url = on_host(server, "127.0.0.1")
            .with_database(&DatabaseName::new("postgres").unwrap())
            .into_inner();
        url.set_path("");
        url.set_port(Some(proxy_port)).unwrap();
        PostgresUrl::new(url.as_str()).unwrap()
    }

    /// Whether `connect` gives up within 10s on `database` at `url`, with an acquire timeout of
    /// 500ms.
    async fn gives_up(url: PostgresUrl, database: DatabaseName) -> bool {
        let target = PostgresTarget { url, database };
        let config = PostgresConfig {
            acquire_timeout: Duration::from_millis(500),
            ..PostgresConfig::default()
        };
        let connected = tokio::time::timeout(
            Duration::from_secs(10),
            PostgresClient::connect(&target, &config),
        )
        .await;
        matches!(connected, Ok(Err(_)))
    }

    async fn connect_gives_up_on_a_server_that_stalls_after_the_probe(case: &Case<'_>) {
        let url = stalls_after_the_first_connection(case.server).await;

        // The case's `app` is missing, so the stalled connection is the one creating it.
        check!(gives_up(url, case.missing_database("app").await).await);
    }

    async fn connect_gives_up_on_a_server_that_stalls_creating_the_database(case: &Case<'_>) {
        let url = stalls_on(case.server, b"CREATE DATABASE").await;

        check!(gives_up(url, case.missing_database("app").await).await);
    }

    async fn connect_gives_up_on_a_server_that_stalls_before_the_pool(case: &Case<'_>) {
        case.connect("app", &PostgresConfig::default()).await;
        let url = stalls_after_the_first_connection(case.server).await;

        // The case's `app` exists, so the stalled connection is the pool's first.
        check!(gives_up(url, case.database("app")).await);
    }

    async fn connect_creates_a_database_once_when_instances_start_together(case: &Case<'_>) {
        let target = PostgresTarget {
            url: case.server.url().clone(),
            database: case.missing_database("app").await,
        };
        let config = PostgresConfig::default();

        let mut connects = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let (target, config) = (target.clone(), config.clone());
            connects.spawn(async move { PostgresClient::connect(&target, &config).await });
        }
        while let Some(connected) = connects.join_next().await {
            check!(let Ok(Ok(_)) = connected);
        }
    }

    async fn connect_reports_why_an_existing_database_refused(case: &Case<'_>) {
        let (role, closed) = (case.role("app"), case.database("closed"));
        for statement in [
            format!("CREATE ROLE {role} LOGIN PASSWORD 'app-password'"),
            format!(r#"CREATE DATABASE "{closed}""#),
            format!(r#"REVOKE CONNECT ON DATABASE "{closed}" FROM PUBLIC"#),
        ] {
            admin(case.server, &statement).await;
        }
        let target = PostgresTarget {
            url: as_role(case.server, &role, "app-password"),
            database: closed,
        };

        let error = PostgresClient::connect(&target, &PostgresConfig::default())
            .await
            .unwrap_err();
        let chain = format!("{error:#}");
        check!(chain.contains(&format!(
            "failed to connect to database {}",
            target.database
        )));
        check!(chain.contains("permission denied for database"));
        check!(!chain.contains("create"));
    }

    async fn the_pool_takes_its_size_and_acquire_timeout(case: &Case<'_>) {
        // Long enough that connecting fits in it on a busy machine.
        let acquire_timeout = Duration::from_secs(2);
        let config = PostgresConfig {
            max_connections: MaxConnections::new(1).unwrap(),
            acquire_timeout,
            ..PostgresConfig::default()
        };
        let postgres = case.connect("app", &config).await;

        let _held = postgres.pool().acquire().await.unwrap();
        let started = std::time::Instant::now();
        let waited = tokio::time::timeout(Duration::from_secs(10), postgres.pool().acquire())
            .await
            .expect("the acquire gives up before 10s");
        check!(let Err(sqlx::Error::PoolTimedOut) = waited);
        check!(started.elapsed() >= acquire_timeout);
    }

    async fn every_connection_in_the_pool_carries_the_lock_timeout(case: &Case<'_>) {
        let postgres = case.connect("app", &PostgresConfig::default()).await;

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

    async fn debug_doesnt_show_the_password(case: &Case<'_>) {
        let postgres = case.connect("app", &PostgresConfig::default()).await;

        let debug = format!("{postgres:?}");
        check!(debug.contains(&format!(r#"database: Some("{}")"#, case.database("app"))));
        check!(!debug.contains(crate::testing::PASSWORD));
    }

    async fn migrate_works_when_instances_start_together(case: &Case<'_>) {
        let migrator = std::sync::Arc::new(test_migrator().await);
        let target = PostgresTarget {
            url: case.server.url().clone(),
            database: case.missing_database("app").await,
        };

        let mut instances = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let (target, migrator) = (target.clone(), migrator.clone());
            instances.spawn(async move {
                let postgres = PostgresClient::connect(&target, &PostgresConfig::default()).await?;
                postgres.migrate(&migrator).await
            });
        }
        while let Some(migrated) = instances.join_next().await {
            check!(let Ok(Ok(())) = migrated);
        }
        let postgres = case.connect("app", &PostgresConfig::default()).await;
        check!(show(&postgres, "SELECT count(*)::text FROM _sqlx_migrations").await == "3");
    }

    async fn the_test_migrations_fail_with_a_lock_timeout(case: &Case<'_>) {
        let postgres = connect(
            case.server,
            case.missing_database("app").await,
            &PostgresConfig::default(),
        )
        .await;

        // On the pool, which sets lock_timeout, unlike `migrate`.
        let error = test_migrator()
            .await
            .run(postgres.pool())
            .await
            .unwrap_err();
        check!(error.to_string().contains("lock_timeout"), "{error}");
    }

    async fn migrate_can_run_on_another_task(case: &Case<'_>) {
        let postgres = connect(
            case.server,
            case.missing_database("app").await,
            &PostgresConfig::default(),
        )
        .await;
        let migrator = test_migrator().await;

        tokio::spawn(async move { postgres.migrate(&migrator).await })
            .await
            .unwrap()
            .unwrap();
    }

    async fn migrate_runs_without_the_lock_timeout(case: &Case<'_>) {
        let postgres = connect(
            case.server,
            case.missing_database("app").await,
            &PostgresConfig::default(),
        )
        .await;

        // The second migration fails when lock_timeout is set.
        postgres.migrate(&test_migrator().await).await.unwrap();
        check!(show(&postgres, "SELECT to_regclass('users')::text").await == "users");
    }

    /// On a server of its own, since it revokes connecting to the `postgres` database from every
    /// role but the superuser, which other cases' roles need.
    #[tokio::test]
    async fn connect_works_without_the_maintenance_database_when_its_database_exists() {
        let server = TestPostgres::start(&test_migrator().await).await;
        for statement in [
            "CREATE ROLE app LOGIN PASSWORD 'app-password'",
            "CREATE DATABASE app OWNER app",
            "REVOKE CONNECT ON DATABASE postgres FROM PUBLIC",
        ] {
            admin(&server, statement).await;
        }
        let url = as_role(&server, "app", "app-password");

        let existing = PostgresTarget {
            url: url.clone(),
            database: DatabaseName::new("app").unwrap(),
        };
        check!(let Ok(_) = PostgresClient::connect(&existing, &PostgresConfig::default()).await);

        let missing = PostgresTarget {
            url,
            database: DatabaseName::new("missing").unwrap(),
        };
        let error = PostgresClient::connect(&missing, &PostgresConfig::default())
            .await
            .unwrap_err();
        check!(format!("{error:#}").contains("failed to create database missing"));
    }
}
