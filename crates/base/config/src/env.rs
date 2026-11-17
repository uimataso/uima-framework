//! [`Env`], the variables read from it, and their error.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use tracing::warn;

type Lookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Where configuration values come from. An empty value counts as unset.
pub struct Env {
    lookup: Box<Lookup>,
}

impl Env {
    /// Reads variables through `lookup`.
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

    /// The variable `name`, to read with [`Var::optional`], [`Var::required`], [`Var::or`] or
    /// [`Var::parse`].
    pub fn var<'a>(&'a self, name: &'a str) -> Var<'a> {
        Var { env: self, name }
    }

    /// The value of `name`, or `None` when it is unset.
    fn value(&self, name: &str) -> Option<String> {
        (self.lookup)(name).filter(|value| !value.is_empty())
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env").finish_non_exhaustive()
    }
}

/// A variable of an [`Env`], from [`Env::var`].
#[derive(Debug, Clone, Copy)]
pub struct Var<'a> {
    env: &'a Env,
    name: &'a str,
}

impl<'a> Var<'a> {
    /// Falls back to `default` when the variable is unset.
    pub fn or(self, default: &'a str) -> VarOr<'a> {
        VarOr { var: self, default }
    }

    /// The value, or `None` when unset.
    pub fn optional(self) -> Option<String> {
        self.env.value(self.name)
    }

    /// The value, or an error when unset.
    pub fn required(self) -> Result<String, EnvError> {
        self.optional().ok_or_else(|| EnvError::unset(self.name))
    }

    /// Parses the value with `parse`, such as `u16::from_str` or `TimeZone::get`. Finish with
    /// [`Parsed::or`], [`Parsed::or_else`], [`Parsed::optional`] or [`Parsed::required`].
    pub fn parse<T, E>(self, parse: impl FnOnce(&str) -> Result<T, E>) -> Parsed<'a, T>
    where
        E: Error + Send + Sync + 'static,
    {
        let parsed = self.optional().map(|value| parse(&value)).transpose();
        Parsed {
            name: self.name,
            parsed: parsed.map_err(|source| EnvError {
                name: self.name.to_owned(),
                source: Some(Box::new(source)),
            }),
        }
    }
}

/// A variable with a default, from [`Var::or`].
#[derive(Debug, Clone, Copy)]
pub struct VarOr<'a> {
    var: Var<'a>,
    default: &'a str,
}

impl VarOr<'_> {
    /// The value, or the default when unset.
    pub fn get(self) -> String {
        self.var
            .optional()
            .unwrap_or_else(|| self.default.to_owned())
    }

    /// Parses the value, or the default when unset, with `parse`.
    ///
    /// # Panics
    ///
    /// If the default doesn't parse, even when the variable is set.
    pub fn parse<T, E>(self, parse: impl Fn(&str) -> Result<T, E>) -> Result<T, EnvError>
    where
        E: Error + Send + Sync + 'static,
    {
        let default = parse(self.default).unwrap_or_else(|error| {
            panic!(
                "invalid default {:?} for {}: {error}",
                self.default, self.var.name
            )
        });
        self.var.parse(parse).or(default)
    }
}

/// A parsed variable that may be unset, from [`Var::parse`].
#[derive(Debug)]
#[must_use = "finish with `.or(..)`, `.or_else(..)`, `.optional()` or `.required()` to get the value"]
pub struct Parsed<'a, T> {
    name: &'a str,
    parsed: Result<Option<T>, EnvError>,
}

impl<T> Parsed<'_, T> {
    /// The value, or `default` when unset.
    pub fn or(self, default: T) -> Result<T, EnvError> {
        self.parsed.map(|value| value.unwrap_or(default))
    }

    /// The value, or `default()` when unset, for a default that is costly to build or only valid
    /// to build when needed.
    pub fn or_else(self, default: impl FnOnce() -> T) -> Result<T, EnvError> {
        self.parsed.map(|value| value.unwrap_or_else(default))
    }

    /// The value, or `None` when unset.
    pub fn optional(self) -> Result<Option<T>, EnvError> {
        self.parsed
    }

    /// The value, or an error when unset.
    pub fn required(self) -> Result<T, EnvError> {
        self.parsed?.ok_or_else(|| EnvError::unset(self.name))
    }
}

/// A required variable is unset, or a variable didn't parse.
///
/// The message names the variable but not its value; the parse error is its
/// [`source`](Error::source), so print the error chain to see it. A parser's own error may quote
/// the value, so don't parse a secret with a parser whose errors do.
#[derive(Debug)]
pub struct EnvError {
    name: String,
    /// The parse error, or `None` when the variable is unset.
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl EnvError {
    fn unset(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            source: None,
        }
    }

    /// The variable that is unset or didn't parse.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the variable is unset, rather than set to a value that didn't parse.
    pub fn is_unset(&self) -> bool {
        self.source.is_none()
    }
}

impl fmt::Display for EnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(_) => write!(f, "environment variable {} is invalid", self.name),
            None => write!(f, "environment variable {} is not set", self.name),
        }
    }
}

impl Error for EnvError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::str::FromStr;

    use uima_testing::check;

    use super::*;

    #[test]
    fn debug_shows_no_values() {
        let env = Env::from_pairs([("SECRET", "s3cret")]);
        check!(format!("{env:?}") == "Env { .. }");
    }

    #[test]
    fn reads_the_value_or_the_default() {
        let env = Env::from_pairs([("A", "set")]);
        check!(env.var("A").optional() == Some("set".to_owned()));
        check!(env.var("B").optional() == None);
        check!(env.var("A").or("default").get() == "set");
        check!(env.var("B").or("default").get() == "default");
    }

    #[test]
    fn empty_counts_as_unset() {
        let env = Env::from_pairs([("A", "")]);
        check!(env.var("A").optional() == None);
        check!(env.var("A").or("default").get() == "default");
        check!(env.var("A").parse(u16::from_str).optional().unwrap() == None);
    }

    #[test]
    fn parse_then_or_falls_back_to_a_typed_default() {
        let env = Env::from_pairs([("PORT", "3000")]);
        check!(env.var("PORT").parse(u16::from_str).or(8080).unwrap() == 3000);
        check!(env.var("NONE").parse(u16::from_str).or(8080).unwrap() == 8080);
    }

    #[test]
    fn or_then_parse_parses_a_string_default() {
        let env = Env::from_pairs([("ADDR", "127.0.0.1:3000")]);
        let addr = env.var("ADDR").or("0.0.0.0:80").parse(SocketAddr::from_str);
        check!(addr.unwrap() == SocketAddr::from(([127, 0, 0, 1], 3000)));
        let addr = env.var("NONE").or("0.0.0.0:80").parse(SocketAddr::from_str);
        check!(addr.unwrap() == SocketAddr::from(([0, 0, 0, 0], 80)));
    }

    #[test]
    fn or_else_builds_the_default_only_when_unset() {
        let env = Env::from_pairs([("PORT", "3000")]);
        let set = env
            .var("PORT")
            .parse(u16::from_str)
            .or_else(|| unreachable!());
        check!(set.unwrap() == 3000);
        let unset = env.var("NONE").parse(u16::from_str).or_else(|| 8080);
        check!(unset.unwrap() == 8080);
    }

    #[test]
    fn optional_gives_none_when_unset() {
        let env = Env::from_pairs([("N", "21")]);
        check!(env.var("N").parse(u32::from_str).optional().unwrap() == Some(21));
        check!(env.var("NONE").parse(u32::from_str).optional().unwrap() == None);
    }

    #[test]
    fn required_errors_when_unset() {
        let env = Env::from_pairs([("N", "21"), ("EMPTY", "")]);
        check!(env.var("N").required().unwrap() == "21");
        check!(env.var("N").parse(u32::from_str).required().unwrap() == 21);

        for error in [
            env.var("EMPTY").required().unwrap_err(),
            env.var("NONE").parse(u32::from_str).required().unwrap_err(),
        ] {
            check!(error.is_unset());
            check!(error.source().is_none());
        }
        let error = env.var("NONE").required().unwrap_err();
        check!(error.to_string() == "environment variable NONE is not set");
    }

    #[test]
    fn required_still_reports_a_bad_value() {
        let env = Env::from_pairs([("N", "x")]);
        let error = env.var("N").parse(u32::from_str).required().unwrap_err();
        check!(!error.is_unset());
    }

    #[test]
    #[should_panic(expected = "invalid default \"nope\" for PORT")]
    fn a_bad_default_panics_even_when_the_variable_is_set() {
        let env = Env::from_pairs([("PORT", "3000")]);
        let _ = env.var("PORT").or("nope").parse(u16::from_str);
    }

    #[test]
    fn a_bad_value_names_the_variable_but_not_the_value() {
        let env = Env::from_pairs([("PORT", "s3cret")]);
        let error = env.var("PORT").or("80").parse(u16::from_str).unwrap_err();
        check!(error.name() == "PORT");
        check!(error.to_string() == "environment variable PORT is invalid");
        let source = error.source().map(ToString::to_string);
        check!(source.as_deref() == Some("invalid digit found in string"));
        check!(!error.to_string().contains("s3cret"));
        check!(!error.is_unset());
    }
}
