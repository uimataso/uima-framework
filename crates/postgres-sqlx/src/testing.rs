//! Test helpers.

use std::sync::atomic::{AtomicU32, Ordering};

use sqlx::migrate::Migrator;
use sqlx::{AssertSqlSafe, PgPool};
use testcontainers::core::{Mount, WaitFor};
use testcontainers::runners::AsyncRunner as _;
use testcontainers::{ContainerAsync, ContainerRequest, GenericImage, ImageExt as _};

use uima_core::Uuid;

use crate::{DatabaseName, Postgres, PostgresConfig, PostgresConnection, PostgresUrl};

/// The superuser's password, distinct so a test can check it isn't shown.
pub(crate) const PASSWORD: &str = "pg-test-password";

/// The migrated database every case's database is copied from.
const TEMPLATE: &str = "template_migrated";

/// A Postgres container for one test function, where each case takes a database of its own.
///
/// The container keeps its data in memory without syncing it to disk, and the migrations run once
/// into a template that every case's database is copied from.
///
/// ```no_run
/// # async fn run() {
/// use uima_postgres_sqlx::testing::TestPostgres;
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
        let container = image()
            .start()
            .await
            .expect("the postgres container starts");
        let url = server_url(&container).await;
        migrate_template(&url, migrator).await;

        let admin = Postgres::connect(&connection(&url, "postgres"), &PostgresConfig::default())
            .await
            .expect("the postgres database is reachable");
        Self {
            url,
            admin: admin.pool().clone(),
            created: AtomicU32::new(0),
            _container: container,
        }
    }

    /// A new database copied from the template.
    ///
    /// # Panics
    ///
    /// If the database can't be created or connected to.
    pub async fn isolated(&self) -> Postgres {
        let name = format!("case_{}", self.created.fetch_add(1, Ordering::Relaxed));
        // `name` and `TEMPLATE` are fixed identifiers, never input.
        let create = format!(r#"CREATE DATABASE "{name}" TEMPLATE "{TEMPLATE}""#);
        sqlx::raw_sql(AssertSqlSafe(create))
            .execute(&self.admin)
            .await
            .expect("the case's database is created from the template");
        Postgres::connect(&connection(&self.url, &name), &PostgresConfig::default())
            .await
            .expect("the case's database is reachable")
    }

    /// The server's URL, for connecting to it some other way.
    pub fn url(&self) -> &PostgresUrl {
        &self.url
    }
}

/// Postgres in memory, without syncing to disk, since a test's data needn't survive a crash.
fn image() -> ContainerRequest<GenericImage> {
    GenericImage::new("postgres", "17.9")
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_PASSWORD", PASSWORD)
        .with_container_name(container_name())
        .with_mount(Mount::tmpfs_mount("/var/lib/postgresql/data"))
        .with_cmd([
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

/// Creates the template database and runs `migrator` on it.
async fn migrate_template(url: &PostgresUrl, migrator: &Migrator) {
    let template = Postgres::connect(&connection(url, TEMPLATE), &PostgresConfig::default())
        .await
        .expect("the template database is created");
    template
        .migrate(migrator)
        .await
        .expect("the migrations run on the template database");
    // `CREATE DATABASE ... TEMPLATE` needs the template to have no sessions.
    template.pool().close().await;
}

fn connection(url: &PostgresUrl, database: &str) -> PostgresConnection {
    PostgresConnection {
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
    use uima_testing::check;

    use super::*;

    async fn names(postgres: &Postgres) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM users ORDER BY id")
            .fetch_all(postgres.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn isolated_databases_are_migrated_and_dont_share_rows() {
        let postgres = TestPostgres::start(&test_migrator().await).await;
        let first = postgres.isolated().await;
        let second = postgres.isolated().await;

        sqlx::query("INSERT INTO users (id, name) VALUES (1, 'uima')")
            .execute(first.pool())
            .await
            .unwrap();

        check!(names(&first).await == ["uima"]);
        check!(names(&second).await.is_empty());
    }

    #[tokio::test]
    async fn the_server_doesnt_sync_to_disk() {
        let postgres = TestPostgres::start(&test_migrator().await).await;
        let isolated = postgres.isolated().await;

        for setting in ["fsync", "synchronous_commit", "full_page_writes"] {
            let shown: String = sqlx::query_scalar(AssertSqlSafe(format!("SHOW {setting}")))
                .fetch_one(isolated.pool())
                .await
                .unwrap();
            check!(shown == "off", "{setting}");
        }
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
                == "uima-test-postgres_uima-postgres-sqlx_\
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

        check!(name.starts_with("uima-test-postgres_uima-postgres-sqlx_a.b-c-d"));
        check!(name.len() <= 128);
        check!(docker_takes(&name));
    }

    #[tokio::test]
    async fn isolated_databases_have_their_own_names_even_at_once() {
        let postgres = TestPostgres::start(&test_migrator().await).await;
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
}
