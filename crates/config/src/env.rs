//! [`Env`] and its error.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::str::FromStr;

use tracing::warn;

type Lookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Where configuration values come from.
pub struct Env {
    lookup: Box<Lookup>,
}

impl Env {
    /// Reads `lookup`, a getter from variable name to value.
    pub fn new(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self {
            lookup: Box::new(lookup),
        }
    }

    /// Reads the process environment. A value that isn't UTF-8 counts as unset, with a warning
    /// that names the variable; install the tracing subscriber first to see it.
    pub fn from_process() -> Self {
        Self::new(|name| match std::env::var(name) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                warn!(name, "config variable isn't UTF-8, treating it as unset");
                None
            }
        })
    }

    /// Reads only `pairs`, for tests.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let vars: HashMap<String, String> = pairs
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        Self::new(move |name| vars.get(name).cloned())
    }

    /// The value of `name`, or `None` when it is unset or empty.
    pub fn get(&self, name: &str) -> Option<String> {
        (self.lookup)(name).filter(|value| !value.is_empty())
    }

    /// The value of `name`, or `default` when it is unset or empty.
    pub fn get_or(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or_else(|| default.to_owned())
    }

    /// Parses `name` with [`FromStr`], or gives `None` when it is unset or empty.
    pub fn parse<T>(&self, name: &str) -> Result<Option<T>, EnvError>
    where
        T: FromStr,
        T::Err: Error + Send + Sync + 'static,
    {
        self.parse_with(name, str::parse)
    }

    /// Parses `name` with [`FromStr`], or gives `default` when it is unset or empty.
    pub fn parse_or<T>(&self, name: &str, default: T) -> Result<T, EnvError>
    where
        T: FromStr,
        T::Err: Error + Send + Sync + 'static,
    {
        Ok(self.parse(name)?.unwrap_or(default))
    }

    /// Parses `name` with `parse`, for types without [`FromStr`], or gives `None` when it is unset
    /// or empty.
    pub fn parse_with<T, E>(
        &self,
        name: &str,
        parse: impl FnOnce(&str) -> Result<T, E>,
    ) -> Result<Option<T>, EnvError>
    where
        E: Error + Send + Sync + 'static,
    {
        self.get(name)
            .map(|value| parse(&value))
            .transpose()
            .map_err(|source| EnvError {
                name: name.to_owned(),
                source: Box::new(source),
            })
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env").finish_non_exhaustive()
    }
}

/// A variable didn't parse. The message is the variable's name and the parser's error; it doesn't
/// add the value, which may be a secret, though a parser's own error may quote it (jiff's time
/// zone lookup does), so don't parse secrets with such a parser.
#[derive(Debug)]
pub struct EnvError {
    name: String,
    source: Box<dyn Error + Send + Sync>,
}

impl EnvError {
    /// The variable that didn't parse.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for EnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.source)
    }
}

impl Error for EnvError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use assert2::check;

    use super::*;

    #[test]
    fn debug_shows_no_values() {
        let env = Env::from_pairs([("SECRET", "s3cret")]);
        check!(format!("{env:?}") == "Env { .. }");
    }

    #[test]
    fn get_reads_the_value() {
        let env = Env::from_pairs([("A", "1")]);
        check!(env.get("A") == Some("1".to_owned()));
        check!(env.get("B") == None);
    }

    #[test]
    fn empty_counts_as_unset() {
        let env = Env::from_pairs([("A", "")]);
        check!(env.get("A") == None);
        check!(env.get_or("A", "default") == "default");
        check!(env.parse::<u16>("A").unwrap() == None);
    }

    #[test]
    fn get_or_falls_back_to_the_default() {
        let env = Env::from_pairs([("A", "set")]);
        check!(env.get_or("A", "default") == "set");
        check!(env.get_or("B", "default") == "default");
    }

    #[test]
    fn parse_reads_with_from_str() {
        let env = Env::from_pairs([("ADDR", "127.0.0.1:3000")]);
        let addr: Option<SocketAddr> = env.parse("ADDR").unwrap();
        check!(addr == Some("127.0.0.1:3000".parse().unwrap()));
        check!(env.parse_or("PORT", 8080_u16).unwrap() == 8080);
    }

    #[test]
    fn parse_with_takes_a_custom_parser() {
        let env = Env::from_pairs([("N", "21")]);
        let doubled = env.parse_with("N", |s| s.parse::<u32>().map(|n| n * 2));
        check!(doubled.unwrap() == Some(42));
    }

    #[test]
    fn a_bad_value_names_the_variable_but_not_the_value() {
        let env = Env::from_pairs([("PORT", "s3cret")]);
        let error = env.parse::<u16>("PORT").unwrap_err();
        check!(error.name() == "PORT");
        check!(error.to_string() == "PORT: invalid digit found in string");
        check!(!error.to_string().contains("s3cret"));
        check!(error.source().is_some());
    }
}
