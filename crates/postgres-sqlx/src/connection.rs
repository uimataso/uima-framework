//! [`PostgresConnection`], the server and database to connect to.

use std::str::FromStr;

use uima_config::{Env, EnvError};

use crate::{DatabaseName, PostgresUrl};

/// The server to connect to and the database on it.
#[derive(Debug, Clone)]
pub struct PostgresConnection {
    /// The server, with the user and password to connect as.
    pub url: PostgresUrl,
    /// The database on the server, created when it doesn't exist.
    pub database: DatabaseName,
}

impl PostgresConnection {
    /// Reads `{prefix}_URL` (required) and `{prefix}_DB` (`database` when unset), each checked
    /// like the type it parses into.
    ///
    /// # Panics
    ///
    /// If `database` isn't a valid [`DatabaseName`].
    pub fn from_env(env: &Env, prefix: &str, database: &str) -> Result<Self, EnvError> {
        Ok(Self {
            url: env
                .var(&format!("{prefix}_URL"))
                .parse(PostgresUrl::from_str)
                .required()?,
            database: env
                .var(&format!("{prefix}_DB"))
                .or(database)
                .parse(DatabaseName::from_str)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    #[test]
    fn reads_the_url_and_the_database() {
        let env = Env::from_pairs([
            ("DB_URL", "postgres://app:s3cr3t@db:5432"),
            ("DB_DB", "nova-el"),
        ]);

        let connection = PostgresConnection::from_env(&env, "DB", "app").unwrap();
        check!(connection.url == "postgres://app:s3cr3t@db:5432".parse().unwrap());
        check!(connection.database.as_str() == "nova-el");
    }

    #[test]
    fn takes_the_given_database_when_unset() {
        let env = Env::from_pairs([("DB_URL", "postgres://db")]);

        let connection = PostgresConnection::from_env(&env, "DB", "sowah").unwrap();
        check!(connection.database.as_str() == "sowah");
    }

    #[test]
    fn needs_a_valid_url() {
        let unset = PostgresConnection::from_env(&Env::new(|_| None), "DB", "app").unwrap_err();
        check!(unset.name() == "DB_URL");
        check!(unset.is_unset());

        let invalid =
            PostgresConnection::from_env(&Env::from_pairs([("DB_URL", "http://db")]), "DB", "app")
                .unwrap_err();
        check!(invalid.name() == "DB_URL");
        check!(!invalid.is_unset());
    }

    #[test]
    fn needs_a_valid_database() {
        let env = Env::from_pairs([("DB_URL", "postgres://db"), ("DB_DB", r#"a"b"#)]);

        let error = PostgresConnection::from_env(&env, "DB", "app").unwrap_err();
        check!(error.name() == "DB_DB");
    }

    #[test]
    #[should_panic(expected = "invalid default \"a b\" for DB_DB")]
    fn panics_on_an_invalid_default_database() {
        let _ = PostgresConnection::from_env(
            &Env::from_pairs([("DB_URL", "postgres://db")]),
            "DB",
            "a b",
        );
    }

    #[test]
    fn debug_masks_the_password() {
        let env = Env::from_pairs([("DB_URL", "postgres://app:s3cr3t@db")]);

        let connection = PostgresConnection::from_env(&env, "DB", "app").unwrap();
        assert!(!format!("{connection:?}").contains("s3cr3t"));
    }
}
