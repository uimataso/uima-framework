//! Test helpers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::{AssertSqlSafe, PgPool};
use testcontainers::bollard::Docker;
use testcontainers::bollard::errors::Error::DockerResponseServerError;
use testcontainers::bollard::query_parameters::RemoveContainerOptionsBuilder;
use testcontainers::core::client::docker_client_instance;
use testcontainers::core::{Mount, WaitFor};
use testcontainers::runners::AsyncRunner as _;
use testcontainers::{ContainerAsync, ContainerRequest, GenericImage, ImageExt as _};

use uima_core::Uuid;

use crate::{DatabaseName, PostgresClient, PostgresConfig, PostgresTarget, PostgresUrl};

/// The superuser's password, distinct so a test can check it isn't shown.
pub(crate) const PASSWORD: &str = "pg-test-password";

/// The migrated database every case's database is copied from.
const TEMPLATE: &str = "template_migrated";

/// How long a container runs at most.
const LIFETIME: Duration = Duration::from_secs(600);

// Longer than the 5 × 60 s after which the CI profile in `.config/nextest.toml` kills a test.
const _: () = assert!(LIFETIME.as_secs() > 300);

/// A Postgres container for one test function, where each case takes a database of its own.
///
/// The container keeps its data in memory without syncing it to disk, and the migrations run once
/// into a template that every case's database is copied from. It stops and is removed 10 minutes
/// after it starts, so a test that runs longer loses its server, and a killed test's server doesn't
/// keep running. A test killed between creating the container and starting it leaves one that never
/// runs, which `docker container prune` removes.
///
/// ```no_run
/// # async fn run() {
/// use uima_client_postgres_sqlx::testing::TestPostgres;
///
/// # static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator::DEFAULT;
/// // static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
///
/// let postgres = TestPostgres::start(&MIGRATOR).await;
/// let first = postgres.isolated().await;
/// let second = postgres.isolated().await; // doesn't see what `first` wrote
/// # }
/// ```
pub struct TestPostgres {
    url: PostgresUrl,
    /// A pool on the `postgres` database, which creates the cases' databases.
    admin: PgPool,
    created: AtomicU32,
    _container: ContainerAsync<GenericImage>,
}

impl TestPostgres {
    /// Starts the container and migrates the template database.
    ///
    /// # Panics
    ///
    /// If the container doesn't start, its server can't be reached, or a migration fails.
    pub async fn start(migrator: &Migrator) -> Self {
        Self::start_with_migrators(&[migrator]).await
    }

    /// Starts the container and runs each of `migrators` in turn on the template database, such
    /// as the app's and those of the framework crates that keep tables of their own.
    ///
    /// # Panics
    ///
    /// As [`start`](Self::start) does, and if two of `migrators` record what they ran in one table,
    /// where each would take the other's migrations for missing.
    pub async fn start_with_migrators(migrators: &[&Migrator]) -> Self {
        let mut tables = HashSet::new();
        for migrator in migrators {
            let table = &*migrator.table_name;
            if !tables.insert(table) {
                panic!("two migrators record what they ran in {table}");
            }
        }

        let name = container_name();
        let container = start_container(image(&name, LIFETIME), &name).await;
        let url = server_url(&container).await;
        migrate_template(&url, migrators).await;

        let admin = PostgresClient::connect(&target(&url, "postgres"), &PostgresConfig::default())
            .await
            .expect("the postgres database is reachable");
        Self {
            url,
            admin: admin.pool().clone(),
            created: AtomicU32::new(0),
            _container: container,
        }
    }

    /// A new database copied from the template, connected with the default config.
    ///
    /// # Panics
    ///
    /// If the database can't be created or connected to.
    pub async fn isolated(&self) -> PostgresClient {
        self.isolated_with(&PostgresConfig::default()).await
    }

    /// A new database copied from the template, connected with `config`, so a case runs with
    /// the app's own settings.
    ///
    /// # Panics
    ///
    /// If the database can't be created or connected to, or `config` is invalid.
    pub async fn isolated_with(&self, config: &PostgresConfig) -> PostgresClient {
        let name = format!("case_{}", self.created.fetch_add(1, Ordering::Relaxed));
        // `name` and `TEMPLATE` are fixed identifiers, never input.
        let create = format!(r#"CREATE DATABASE "{name}" TEMPLATE "{TEMPLATE}""#);
        sqlx::raw_sql(AssertSqlSafe(create))
            .execute(&self.admin)
            .await
            .expect("the case's database is created from the template");
        PostgresClient::connect(&target(&self.url, &name), config)
            .await
            .expect("the case's database is reachable")
    }

    /// The server's URL, for connecting to it some other way.
    pub fn url(&self) -> &PostgresUrl {
        &self.url
    }
}

/// `request` started, or, when it fails, the container named `name` removed before the panic,
/// since one created but never started has nothing that removes it.
///
/// # Panics
///
/// If the container doesn't start.
async fn start_container(
    request: ContainerRequest<GenericImage>,
    name: &str,
) -> ContainerAsync<GenericImage> {
    let error = match request.start().await {
        Ok(container) => return container,
        Err(error) => error,
    };
    match remove_container(name).await {
        Ok(()) => panic!("the postgres container starts: {error}"),
        Err(removing) => {
            panic!("the postgres container starts: {error}; removing {name} failed: {removing:#}")
        }
    }
}

/// Removes the container named `name`, if there is one, waiting for a removal Docker already
/// started.
async fn remove_container(name: &str) -> anyhow::Result<()> {
    let options = RemoveContainerOptionsBuilder::new()
        .force(true)
        .v(true)
        .build();
    // The client testcontainers starts containers with, so both find the same daemon.
    let docker = docker_client_instance().await?;
    match docker.remove_container(name, Some(options)).await {
        Ok(())
        | Err(DockerResponseServerError {
            status_code: 404, ..
        }) => Ok(()),
        // Docker is already removing it, for another caller or as an auto-removed container that
        // stopped.
        Err(DockerResponseServerError {
            status_code: 409, ..
        }) => wait_until_gone(&docker, name).await,
        Err(error) => Err(error.into()),
    }
}

/// Waits for the container named `name` to be gone, for about 30 seconds.
async fn wait_until_gone(docker: &Docker, name: &str) -> anyhow::Result<()> {
    for _ in 0..300 {
        match docker.inspect_container(name, None).await {
            Err(DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    anyhow::bail!("{name} is still being removed after about 30 seconds")
}

/// Postgres in memory named `name`, without syncing to disk, since a test's data needn't survive a
/// crash. The container stops after `lifetime` and Docker removes it then, so one whose test was
/// killed before its drop removed it doesn't stay.
fn image(name: &str, lifetime: Duration) -> ContainerRequest<GenericImage> {
    let lifetime = lifetime.as_secs_f64().to_string();
    GenericImage::new("postgres", "17.9")
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_entrypoint("timeout")
        .with_env_var("POSTGRES_PASSWORD", PASSWORD)
        .with_container_name(name)
        .with_mount(Mount::tmpfs_mount("/var/lib/postgresql/data"))
        .with_host_config_modifier(|config| config.auto_remove = Some(true))
        .with_cmd([
            "--signal=KILL",
            &lifetime,
            "docker-entrypoint.sh",
            "postgres",
            "-c",
            "fsync=off",
            "-c",
            "synchronous_commit=off",
            "-c",
            "full_page_writes=off",
        ])
}

/// A container name Docker accepts that names the package and the test starting it, with a random
/// end so it differs from every other container, such as
/// `uima-test-postgres_app_tests.saves_a_user_1a2b3c4d`.
fn container_name() -> String {
    // cargo and nextest set the package's name, and name each test's thread after the test.
    let package = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "app".to_owned());
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("test").replace("::", ".");
    // The end of a v7 UUID is random; its start is the time.
    let uuid = Uuid::now_v7().simple().to_string();
    let random = &uuid[uuid.len() - 8..];
    format!(
        "uima-test-postgres_{}_{}_{random}",
        docker_safe(&package, 25),
        docker_safe(&test, 70),
    )
}

/// `text` cut to `max` characters, each one Docker doesn't take in a name replaced by `-`.
fn docker_safe(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(max)
        .collect()
}

/// The URL of the server in `container`, as the superuser.
async fn server_url(container: &ContainerAsync<GenericImage>) -> PostgresUrl {
    let host = container
        .get_host()
        .await
        .expect("the container has a host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container maps postgres' port");
    PostgresUrl::new(&format!("postgres://postgres:{PASSWORD}@{host}:{port}"))
        .expect("the container's url is valid")
}

/// Creates the template database and runs each of `migrators` on it.
async fn migrate_template(url: &PostgresUrl, migrators: &[&Migrator]) {
    let template = PostgresClient::connect(&target(url, TEMPLATE), &PostgresConfig::default())
        .await
        .expect("the template database is created");
    for migrator in migrators {
        template
            .migrate(migrator)
            .await
            .expect("the migrations run on the template database");
    }
    // `CREATE DATABASE ... TEMPLATE` needs the template to have no sessions.
    template.pool().close().await;
}

/// `database` on the server at `url`.
fn target(url: &PostgresUrl, database: &str) -> PostgresTarget {
    PostgresTarget {
        url: url.clone(),
        database: DatabaseName::new(database).expect("a test database's name is valid"),
    }
}

/// The migrations this crate's tests run.
#[cfg(test)]
pub(crate) async fn test_migrator() -> Migrator {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/migrations");
    Migrator::new(std::path::Path::new(dir)).await.unwrap()
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    async fn names(postgres: &PostgresClient) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM users ORDER BY id")
            .fetch_all(postgres.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_server_gives_each_case_its_own_migrated_database_in_memory() {
        let postgres = TestPostgres::start(&test_migrator().await).await;

        isolated_databases_are_migrated_and_dont_share_rows(&postgres).await;
        the_server_doesnt_sync_to_disk(&postgres).await;
        isolated_databases_have_their_own_names_even_at_once(&postgres).await;
        an_isolated_database_connects_with_the_given_config(&postgres).await;
    }

    async fn isolated_databases_are_migrated_and_dont_share_rows(postgres: &TestPostgres) {
        let first = postgres.isolated().await;
        let second = postgres.isolated().await;

        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'uima')")
            .execute(first.pool())
            .await
            .unwrap();

        check!(names(&first).await == ["uima"]);
        check!(names(&second).await.is_empty());
    }

    async fn the_server_doesnt_sync_to_disk(postgres: &TestPostgres) {
        let isolated = postgres.isolated().await;

        for setting in ["fsync", "synchronous_commit", "full_page_writes"] {
            let shown: String = sqlx::query_scalar(AssertSqlSafe(format!("SHOW {setting}")))
                .fetch_one(isolated.pool())
                .await
                .unwrap();
            check!(shown == "off", "{setting}");
        }
    }

    async fn isolated_databases_have_their_own_names_even_at_once(postgres: &TestPostgres) {
        let (first, second, third) = tokio::join!(
            postgres.isolated(),
            postgres.isolated(),
            postgres.isolated()
        );

        let mut databases = std::collections::HashSet::new();
        for isolated in [first, second, third] {
            let database: String = sqlx::query_scalar("SELECT current_database()")
                .fetch_one(isolated.pool())
                .await
                .unwrap();
            databases.insert(database);
        }
        check!(databases.len() == 3);
    }

    async fn an_isolated_database_connects_with_the_given_config(postgres: &TestPostgres) {
        let config = PostgresConfig {
            lock_timeout: Duration::from_millis(750),
            ..PostgresConfig::default()
        };
        let isolated = postgres.isolated_with(&config).await;

        let lock_timeout: String = sqlx::query_scalar("SHOW lock_timeout")
            .fetch_one(isolated.pool())
            .await
            .unwrap();
        check!(lock_timeout == "750ms");
        check!(names(&isolated).await.is_empty());
    }

    #[tokio::test]
    async fn the_template_runs_each_migrator() {
        let app = test_migrator().await;
        let mut other = other_migrator().await;
        other.dangerous_set_table_name("_other_migrations");
        let postgres = TestPostgres::start_with_migrators(&[&app, &other]).await;

        let isolated = postgres.isolated().await;

        check!(names(&isolated).await.is_empty());
        let others: i64 = sqlx::query_scalar("SELECT count(*) FROM others")
            .fetch_one(isolated.pool())
            .await
            .unwrap();
        check!(others == 0);
    }

    #[tokio::test]
    async fn migrators_recording_in_one_table_dont_start() {
        let (app, other) = (test_migrator().await, other_migrator().await);

        let started = tokio::spawn(async move {
            TestPostgres::start_with_migrators(&[&app, &other]).await;
        });

        let panic = started.await.unwrap_err().into_panic();
        assert!(let Some(message) = panic.downcast_ref::<String>());
        check!(message == "two migrators record what they ran in _sqlx_migrations");
    }

    async fn other_migrator() -> Migrator {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/other-migrations");
        Migrator::new(std::path::Path::new(dir)).await.unwrap()
    }

    #[tokio::test]
    async fn a_container_that_doesnt_start_is_removed() {
        let name = container_name();
        let request = image(&name, LIFETIME).with_startup_timeout(Duration::from_millis(1));

        let starting = name.clone();
        let started = tokio::spawn(async move { start_container(request, &starting).await });
        check!(started.await.unwrap_err().is_panic());

        let docker = docker_client_instance().await.unwrap();
        let found = docker.inspect_container(&name, None).await;
        check!(let Err(DockerResponseServerError { status_code: 404, .. }) = found);
    }

    #[tokio::test]
    async fn a_container_nothing_removes_removes_itself_after_its_lifetime() {
        let name = container_name();
        let container = start_container(image(&name, Duration::from_secs(20)), &name).await;
        let url = server_url(&container).await;
        // As when the test's process is killed, no drop removes it.
        std::mem::forget(container);
        // A session still open, as a killed test's would be, keeps a server that's only asked to
        // stop running.
        let client = PostgresClient::connect(&target(&url, "postgres"), &PostgresConfig::default())
            .await
            .unwrap();
        let _session = client.pool().acquire().await.unwrap();

        let docker = docker_client_instance().await.unwrap();
        let mut found = docker.inspect_container(&name, None).await;
        for _ in 0..80 {
            if found.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            found = docker.inspect_container(&name, None).await;
        }
        remove_container(&name).await.unwrap();
        check!(let Err(DockerResponseServerError { status_code: 404, .. }) = found);
    }

    #[tokio::test]
    async fn a_container_two_callers_remove_at_once_is_removed_for_both() {
        let name = container_name();
        let container = start_container(image(&name, LIFETIME), &name).await;
        std::mem::forget(container);
        let docker = docker_client_instance().await.unwrap();
        // Each caller looks right after its own removal returns, before the other's ends.
        let remove = || async {
            let removed = remove_container(&name).await;
            (removed, docker.inspect_container(&name, None).await)
        };

        let ((first, first_found), (second, second_found)) = tokio::join!(remove(), remove());

        check!(first.is_ok());
        check!(second.is_ok());
        check!(let Err(DockerResponseServerError { status_code: 404, .. }) = first_found);
        check!(let Err(DockerResponseServerError { status_code: 404, .. }) = second_found);
    }

    #[tokio::test]
    async fn a_lifetime_under_a_second_still_ends() {
        let name = container_name();
        // The server may stop before it's ready, failing the start.
        if let Ok(container) = image(&name, Duration::from_millis(500)).start().await {
            std::mem::forget(container);
        }

        let docker = docker_client_instance().await.unwrap();
        let mut found = docker.inspect_container(&name, None).await;
        for _ in 0..30 {
            if found.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            found = docker.inspect_container(&name, None).await;
        }
        remove_container(&name).await.unwrap();
        check!(let Err(DockerResponseServerError { status_code: 404, .. }) = found);
    }

    /// Whether `name` is one Docker takes.
    fn docker_takes(name: &str) -> bool {
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    }

    #[test]
    fn container_names_name_the_package_and_the_test_and_differ() {
        let first = container_name();
        let second = container_name();

        let (named, random) = first.rsplit_once('_').unwrap();
        check!(
            named
                == "uima-test-postgres_uima-client-postgres-sqlx_\
                    testing.tests.container_names_name_the_package_and_the_test_and_differ"
        );
        check!(random.len() == 8);
        check!(random.chars().all(|c| c.is_ascii_hexdigit()));
        check!(first != second);
        check!(docker_takes(&first));
    }

    #[test]
    fn container_names_keep_only_what_docker_accepts() {
        let name = std::thread::Builder::new()
            .name("a::b c/d".repeat(30))
            .spawn(container_name)
            .unwrap()
            .join()
            .unwrap();

        check!(name.starts_with("uima-test-postgres_uima-client-postgres-sqlx_a.b-c-d"));
        check!(name.len() <= 128);
        check!(docker_takes(&name));
    }
}
