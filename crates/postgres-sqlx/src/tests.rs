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
    use uima_core::FailureKind;
    use uima_testing::check;

    use super::*;
    use crate::testing::{TestPostgres, test_migrator};

    async fn connect(
        server: &TestPostgres,
        database: &str,
        config: &PostgresConfig,
    ) -> PostgresClient {
        let target = PostgresTarget {
            url: server.url().clone(),
            database: DatabaseName::new(database).unwrap(),
        };
        PostgresClient::connect(&target, config).await.unwrap()
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
    async fn conn_takes_a_pooled_connection_and_gives_it_back() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let config = PostgresConfig {
            max_connections: MaxConnections::new(1).unwrap(),
            ..PostgresConfig::default()
        };
        let postgres = connect(&server, "app", &config).await;

        let held = postgres.conn().await.unwrap();
        let error = postgres.conn().await.unwrap_err();
        check!(error.to_string() == "failed to take a pooled connection");
        check!(error.kind() == FailureKind::Timeout);

        drop(held);
        check!(postgres.conn().await.is_ok());
    }

    #[tokio::test]
    async fn connect_creates_a_missing_database_as_a_role_that_can_only_create_databases() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let admin = connect(&server, "postgres", &PostgresConfig::default()).await;
        sqlx::raw_sql("CREATE ROLE app LOGIN CREATEDB PASSWORD 'app-password'")
            .execute(admin.pool())
            .await
            .unwrap();
        let target = PostgresTarget {
            url: as_role(&server, "app", "app-password"),
            database: DatabaseName::new("app").unwrap(),
        };

        let postgres = PostgresClient::connect(&target, &PostgresConfig::default())
            .await
            .unwrap();
        check!(show(&postgres, "SELECT current_database()").await == "app");
    }

    #[tokio::test]
    async fn connect_creates_a_missing_database_on_an_ipv6_host() {
        let server = TestPostgres::start(&test_migrator().await).await;
        let target = PostgresTarget {
            url: on_host(&server, "[::1]"),
            database: DatabaseName::new("app").unwrap(),
        };

        let postgres = PostgresClient::connect(&target, &PostgresConfig::default())
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

    /// Whether `connect` gives up within 10s on `url`, with an acquire timeout of 500ms.
    async fn gives_up(url: PostgresUrl) -> bool {
        let target = PostgresTarget {
            url,
            database: DatabaseName::new("app").unwrap(),
        };
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

    #[tokio::test]
    async fn connect_gives_up_on_a_server_that_stalls_after_the_probe() {
        let server = TestPostgres::start(&test_migrator().await).await;

        // `app` is missing, so the stalled connection is the one creating it.
        check!(gives_up(stalls_after_the_first_connection(&server).await).await);
    }

    #[tokio::test]
    async fn connect_gives_up_on_a_server_that_stalls_creating_the_database() {
        let server = TestPostgres::start(&test_migrator().await).await;

        check!(gives_up(stalls_on(&server, b"CREATE DATABASE").await).await);
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
        let target = PostgresTarget {
            url: server.url().clone(),
            database: DatabaseName::new("app").unwrap(),
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
        let target = PostgresTarget {
            url: as_role(&server, "app", "app-password"),
            database: DatabaseName::new("closed").unwrap(),
        };

        let error = PostgresClient::connect(&target, &PostgresConfig::default())
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
        let target = PostgresTarget {
            url: server.url().clone(),
            database: DatabaseName::new("app").unwrap(),
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
        let postgres = connect(&server, "app", &PostgresConfig::default()).await;
        check!(show(&postgres, "SELECT count(*)::text FROM _sqlx_migrations").await == "3");
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
