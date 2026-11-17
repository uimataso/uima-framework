//! [`PostgresUrl`], a Postgres server's URL.

use std::fmt;
use std::str::FromStr;

use sqlx::ConnectOptions as _;
use sqlx::postgres::PgConnectOptions;

use uima_core::{Invalid, Newtype, Secret};

use crate::DatabaseName;

/// A Postgres server's URL, `postgres://user:password@host:port`, without a database.
///
/// `Debug` and `Display` show it with the password masked, so it can be logged.
///
/// A server across a network takes `?sslmode=verify-full`, since the default, `prefer`, doesn't
/// check the server's certificate and falls back to no TLS.
///
/// sqlx fills what the URL leaves out from the `PG*` variables, such as `PGSSLMODE`, `PGPASSWORD`
/// or `PGOPTIONS`, and from `~/.pgpass`, so set them only when the URL should take them.
#[derive(Clone, PartialEq, Eq)]
pub struct PostgresUrl(url::Url);

impl PostgresUrl {
    /// The query keys a URL can have, which set up TLS or name the client.
    pub const QUERY_KEYS: [&str; 5] = [
        "sslmode",
        "sslrootcert",
        "sslcert",
        "sslkey",
        "application_name",
    ];

    /// Parses a server's URL, which has the `postgres` or `postgresql` scheme, a host, no database
    /// and only [`QUERY_KEYS`](Self::QUERY_KEYS) in its query.
    pub fn new(url: &str) -> Result<Self, Invalid<Self>> {
        let url = url::Url::parse(url)
            .map_err(|error| Invalid::new(PostgresUrlErrorKind::Malformed(error.to_string())))?;

        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(Invalid::new(PostgresUrlErrorKind::Scheme(
                url.scheme().to_owned(),
            )));
        }

        if !matches!(url.path(), "" | "/") {
            return Err(Invalid::new(PostgresUrlErrorKind::HasDatabase));
        }

        if url.host_str().is_none() {
            return Err(Invalid::new(PostgresUrlErrorKind::NoHost));
        }

        if url.fragment().is_some() {
            return Err(Invalid::new(PostgresUrlErrorKind::Fragment));
        }

        let mut seen = Vec::new();
        for (key, _) in url.query_pairs() {
            if !Self::QUERY_KEYS.contains(&key.as_ref()) {
                return Err(Invalid::new(PostgresUrlErrorKind::QueryKey));
            }
            if seen.contains(&key) {
                return Err(Invalid::new(PostgresUrlErrorKind::RepeatedQueryKey));
            }
            seen.push(key);
        }

        // sqlx's error could quote the URL, so it isn't kept.
        if PgConnectOptions::from_url(&url).is_err() {
            return Err(Invalid::new(PostgresUrlErrorKind::Unreadable));
        }

        Ok(Self(url))
    }

    /// The options to connect to `database` on this server, with the password.
    pub(crate) fn connect_options(
        &self,
        database: &DatabaseName,
    ) -> Result<PgConnectOptions, sqlx::Error> {
        let options = PgConnectOptions::from_url(self.with_database(database).expose())?;
        // sqlx keeps an IPv6 host's brackets, which no resolver takes.
        Ok(match self.0.host() {
            Some(url::Host::Ipv6(address)) => options.host(&address.to_string()),
            _ => options,
        })
    }

    /// The URL of `database` on this server, with the password.
    pub(crate) fn with_database(&self, database: &DatabaseName) -> Secret<url::Url> {
        let mut url = self.0.clone();
        url.set_path(database.as_str());
        Secret::new(url)
    }

    /// The URL with the password masked.
    fn masked(&self) -> String {
        let mut url = self.0.clone();
        if url.password().is_some() {
            // A URL with a password has a host, so setting one can't fail.
            let _ = url.set_password(Some("xxxxx"));
        }
        url.set_path("");
        url.into()
    }
}

impl FromStr for PostgresUrl {
    type Err = Invalid<PostgresUrl>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Debug for PostgresUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PostgresUrl").field(&self.masked()).finish()
    }
}

impl fmt::Display for PostgresUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.masked())
    }
}

impl Newtype for PostgresUrl {
    const NAME: &'static str = "PostgresUrl";
    type Kind = PostgresUrlErrorKind;
}

/// What is wrong with a Postgres server's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostgresUrlErrorKind {
    /// Not a URL at all, with the parser's reason.
    Malformed(String),
    /// A scheme other than `postgres` or `postgresql`.
    Scheme(String),
    /// A path, which would name a database; [`DatabaseName`] does that.
    HasDatabase,
    /// No host to connect to.
    NoHost,
    /// A query key other than [`QUERY_KEYS`](PostgresUrl::QUERY_KEYS), such as `password` or
    /// `dbname`, which would override the URL or hide a password from the mask. It leaves the key
    /// out, since a password with an unencoded `?` reads as a query.
    QueryKey,
    /// A fragment, which a password with an unencoded `#` turns into, where the mask doesn't
    /// reach it.
    Fragment,
    /// A query key set twice, where sqlx would take the last value, though a reader of the URL
    /// may take the first.
    RepeatedQueryKey,
    /// A URL sqlx can't read, such as one with an `sslmode` it doesn't know, or a user or password
    /// that isn't UTF-8 once decoded.
    Unreadable,
}

impl fmt::Display for PostgresUrlErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "isn't a URL: {reason}"),
            Self::Scheme(scheme) => write!(
                f,
                "can't have the scheme '{scheme}', only postgres or postgresql"
            ),
            Self::HasDatabase => write!(f, "can't name a database"),
            Self::NoHost => write!(f, "needs a host"),
            Self::QueryKey => write!(
                f,
                "can't have a query key other than {}",
                PostgresUrl::QUERY_KEYS.join(", "),
            ),
            Self::Fragment => write!(f, "can't have a fragment"),
            Self::RepeatedQueryKey => write!(f, "can't set a query key twice"),
            Self::Unreadable => write!(f, "isn't one sqlx can read"),
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    fn kind(s: &str) -> PostgresUrlErrorKind {
        s.parse::<PostgresUrl>().unwrap_err().kind().clone()
    }

    #[test]
    fn parses_a_server_url_with_either_scheme() {
        for url in [
            "postgres://app:s3cr3t@db:5432",
            "postgresql://app:s3cr3t@db:5432",
            "postgres://db",
            "postgres://db:5432/",
            "postgres://app:s3cr3t@db:5432?sslmode=require",
        ] {
            assert!(let Ok(_) = url.parse::<PostgresUrl>(), "{url}");
        }
    }

    #[test]
    fn shows_the_url_with_the_password_masked() {
        let url: PostgresUrl = "postgres://app:s3cr3t@db:5432?sslmode=require"
            .parse()
            .unwrap();

        check!(url.to_string() == "postgres://app:xxxxx@db:5432?sslmode=require");
        check!(
            format!("{url:?}") == r#"PostgresUrl("postgres://app:xxxxx@db:5432?sslmode=require")"#
        );
    }

    #[test]
    fn shows_the_url_without_a_slash_before_its_query() {
        let url: PostgresUrl = "postgres://app:s3cr3t@db:5432/?sslmode=require"
            .parse()
            .unwrap();

        check!(url.to_string() == "postgres://app:xxxxx@db:5432?sslmode=require");
    }

    #[test]
    fn shows_a_url_without_a_password_as_it_is() {
        let url: PostgresUrl = "postgres://app@db:5432".parse().unwrap();

        check!(url.to_string() == "postgres://app@db:5432");
    }

    #[test]
    fn shows_a_url_without_its_trailing_slash() {
        let url: PostgresUrl = "postgres://db:5432/".parse().unwrap();

        check!(url.to_string() == "postgres://db:5432");
    }

    #[test]
    fn masks_an_encoded_password() {
        let url: PostgresUrl = "postgres://app:p%40ss%2Fw0rd@db".parse().unwrap();

        check!(!url.to_string().contains("p%40ss"));
        check!(!format!("{url:?}").contains("p%40ss"));
    }

    #[test]
    fn connects_to_an_ipv6_host_without_its_brackets() {
        let url: PostgresUrl = "postgres://app@[::1]:5432".parse().unwrap();

        let options = url
            .connect_options(&DatabaseName::new("app").unwrap())
            .unwrap();
        check!(options.get_host() == "::1");
        check!(options.get_port() == 5432);
    }

    #[test]
    fn rejects_what_isnt_a_url() {
        assert!(let PostgresUrlErrorKind::Malformed(_) = kind("not a url"));
        assert!(let PostgresUrlErrorKind::Malformed(_) = kind(""));
        assert!(let PostgresUrlErrorKind::Malformed(_) = kind("postgres://db:notaport"));
        check!(
            "postgres://db:notaport"
                .parse::<PostgresUrl>()
                .unwrap_err()
                .to_string()
                == "PostgresUrl isn't a URL: invalid port number"
        );
    }

    #[test]
    fn rejects_a_query_key_set_twice() {
        check!(
            kind("postgres://db?sslmode=verify-full&sslmode=disable")
                == PostgresUrlErrorKind::RepeatedQueryKey
        );
        check!(
            "postgres://db?sslmode=verify-full&sslmode=disable"
                .parse::<PostgresUrl>()
                .unwrap_err()
                .to_string()
                == "PostgresUrl can't set a query key twice"
        );
    }

    #[test]
    fn rejects_a_user_or_password_sqlx_cant_read() {
        for url in ["postgres://app:%FF@db", "postgres://%FF@db"] {
            check!(kind(url) == PostgresUrlErrorKind::Unreadable, "{url}");
        }
    }

    #[test]
    fn rejects_another_scheme() {
        check!(kind("mysql://app@db") == PostgresUrlErrorKind::Scheme("mysql".to_owned()));
        check!(
            "http://db".parse::<PostgresUrl>().unwrap_err().to_string()
                == "PostgresUrl can't have the scheme 'http', only postgres or postgresql"
        );
    }

    #[test]
    fn rejects_a_database_in_the_url() {
        check!(kind("postgres://db:5432/app") == PostgresUrlErrorKind::HasDatabase);
        check!(
            "postgres://db/app"
                .parse::<PostgresUrl>()
                .unwrap_err()
                .to_string()
                == "PostgresUrl can't name a database"
        );
    }

    #[test]
    fn takes_the_query_keys_for_tls_and_the_client_name() {
        let url = "postgres://db?sslmode=verify-full&sslrootcert=/ca.pem&sslcert=/c.pem\
                   &sslkey=/c.key&application_name=api";

        assert!(let Ok(_) = url.parse::<PostgresUrl>());
    }

    #[test]
    fn rejects_a_query_key_that_overrides_the_url() {
        for key in [
            "password", "dbname", "user", "host", "hostaddr", "port", "passwd",
        ] {
            check!(
                kind(&format!("postgres://app@db?{key}=s3cr3t")) == PostgresUrlErrorKind::QueryKey,
                "{key}"
            );
        }
        check!(
            "postgres://db?sslmode=require&dbname=other"
                .parse::<PostgresUrl>()
                .unwrap_err()
                .to_string()
                == "PostgresUrl can't have a query key other than sslmode, sslrootcert, sslcert, \
                    sslkey, application_name"
        );
    }

    #[test]
    fn rejects_an_sslmode_sqlx_doesnt_know() {
        check!(kind("postgres://db?sslmode=bogus") == PostgresUrlErrorKind::Unreadable);
    }

    #[test]
    fn rejects_a_fragment() {
        check!(kind("postgres://db#frag") == PostgresUrlErrorKind::Fragment);
    }

    #[test]
    fn rejects_a_url_without_a_host() {
        for url in ["postgres://", "postgres:///", "postgres:"] {
            check!(kind(url) == PostgresUrlErrorKind::NoHost, "{url}");
        }
        check!(
            "postgres://"
                .parse::<PostgresUrl>()
                .unwrap_err()
                .to_string()
                == "PostgresUrl needs a host"
        );
    }

    #[test]
    fn errors_never_show_the_password() {
        for url in [
            "postgres://app:s3cr3t@db/app",
            "mysql://app:s3cr3t@db",
            "postgres://app:s3cr3t@db:notaport",
            "postgres://app@db?password=s3cr3t",
            "postgres://app:?s3cr3t@db",
            "postgres://app:s3cr3t@db#s3cr3t",
        ] {
            let error = url.parse::<PostgresUrl>().unwrap_err();
            check!(!error.to_string().contains("s3cr3t"), "{url}");
            check!(!format!("{error:?}").contains("s3cr3t"), "{url}");
        }
    }
}

#[cfg(test)]
mod props {
    use proptest::prelude::*;
    use proptest::property_test;

    use super::*;

    /// Up to 3 characters, often ones a URL reads as a delimiter, as an unencoded password has.
    fn delimiters() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('?'),
                Just('#'),
                Just('@'),
                Just('/'),
                Just(':'),
                Just('%'),
                Just('&'),
                Just('='),
                Just(' '),
                Just('['),
                any::<char>()
            ],
            0..4,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    #[property_test]
    fn never_shows_the_password(
        #[strategy = delimiters()] before: String,
        #[strategy = delimiters()] after: String,
        #[strategy = prop_oneof![Just(""), Just("/"), Just("?sslmode=require"), Just("/app")]]
        rest: &'static str,
    ) {
        // Lowercase, so it reads the same if the URL takes it for the host.
        let url = format!("postgres://app:{before}s3cr3t{after}@db:5432{rest}");

        let shown = match url.parse::<PostgresUrl>() {
            Ok(url) => format!("{url} {url:?}"),
            Err(error) => format!("{error} {error:?}"),
        };
        prop_assert!(!shown.contains("s3cr3t"), "{}", shown);
    }
}
