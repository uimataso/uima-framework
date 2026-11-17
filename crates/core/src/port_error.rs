//! [`PortError`], what a port returns when it fails.

use std::error::Error;
use std::fmt;
use std::time::Duration;

/// The result of a port call; `E` is the error the port's contract defines, none by default.
pub type PortResult<T, E = !> = Result<T, PortError<E>>;

/// Why a port call didn't succeed: an outcome its contract defines, such as not found, which
/// callers handle case by case, or a [`Failure`] of whatever carries it out.
///
/// A port whose contract defines no error uses `!`, so callers match only `Failed`:
///
/// ```
/// use uima_core::{FailureKind, PortError, PortResult};
///
/// fn busy(result: PortResult<()>) -> bool {
///     match result {
///         Ok(()) => false,
///         Err(PortError::Failed(failure)) => failure.kind() == FailureKind::Timeout,
///     }
/// }
/// ```
///
/// Its message and source are those of the error inside.
#[derive(Debug)]
pub enum PortError<E> {
    /// An outcome the port's contract defines.
    Contract(E),
    /// Whatever carries the call out failed, such as a database, an API or a queue.
    Failed(Failure),
}

/// A failure of whatever carries a port call out, such as a database, an API or a queue.
///
/// Its message is the outermost one of its source, the context the adapter added; the rest of
/// the chain is its [`source`](Error::source).
#[derive(Debug)]
pub struct Failure {
    kind: FailureKind,
    retry_after: Option<Duration>,
    source: anyhow::Error,
}

/// What happened, as far as the adapter can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureKind {
    /// Lost to a concurrent operation on the same data, and undone.
    ///
    /// Such as a serialization failure or deadlock, or a version or etag that no longer matches.
    /// Not a problem with the data itself.
    Conflict,
    /// Gave up waiting for a connection, a lock or an answer.
    Timeout,
    /// The other side can't serve now, being unreachable or answering that it is down.
    Unavailable,
    /// The other side refused because of too many requests.
    RateLimited,
    /// None of the above, or the adapter can't tell.
    Other,
}

impl Failure {
    /// A failure of the given kind, with no wait set.
    pub fn new(kind: FailureKind, source: impl Into<anyhow::Error>) -> Self {
        Self {
            kind,
            retry_after: None,
            source: source.into(),
        }
    }

    /// Sets how long the other side asked to wait, such as an HTTP `Retry-After`; `None` when it
    /// didn't say.
    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    /// Wraps the message in `context`, keeping the kind and the wait.
    pub fn context<C>(mut self, context: C) -> Self
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.source = self.source.context(context);
        self
    }

    /// What happened.
    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    /// How long the other side asked to wait, if it said.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

/// Wraps a [`Failure`]'s message in a context and keeps a port failure's kind, where anyhow's
/// `.context` on a port call would turn the failure into an [`Other`](FailureKind::Other) one.
///
/// ```
/// use uima_core::{Failure, FailureContext as _, PortResult};
///
/// fn list_groups(port: impl Fn() -> PortResult<Vec<String>>) -> Result<Vec<String>, Failure> {
///     port().failure_context("failed to list the groups")
/// }
/// ```
pub trait FailureContext<T> {
    /// The failure wrapped in `context`, keeping its kind and wait; an outcome of a port's contract
    /// the caller doesn't handle becomes an [`Other`](FailureKind::Other) failure.
    fn failure_context<C>(self, context: C) -> Result<T, Failure>
    where
        C: fmt::Display + Send + Sync + 'static;
}

/// A port's failure keeps its kind; an outcome of its contract, which the caller doesn't handle,
/// becomes an [`Other`](FailureKind::Other) failure.
impl<T, E> FailureContext<T> for PortResult<T, E>
where
    E: Error + Send + Sync + 'static,
{
    fn failure_context<C>(self, context: C) -> Result<T, Failure>
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|error| {
            match error {
                PortError::Contract(outcome) => Failure::new(FailureKind::Other, outcome),
                PortError::Failed(failure) => failure,
            }
            .context(context)
        })
    }
}

impl<T> FailureContext<T> for Result<T, Failure> {
    fn failure_context<C>(self, context: C) -> Result<T, Failure>
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|failure| failure.context(context))
    }
}

impl<E> From<Failure> for PortError<E> {
    fn from(error: Failure) -> Self {
        Self::Failed(error)
    }
}

/// The failure of a port with no contract error, so `?` keeps its kind.
impl From<PortError<!>> for Failure {
    fn from(PortError::Failed(failure): PortError<!>) -> Self {
        failure
    }
}

/// An unclassified [`Failure`], so `.context(..)?` works in adapters.
impl<E> From<anyhow::Error> for PortError<E> {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error.into())
    }
}

impl<E: fmt::Display> fmt::Display for PortError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(f),
            Self::Failed(error) => error.fmt(f),
        }
    }
}

impl<E: Error> Error for PortError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => error.source(),
            Self::Failed(error) => error.source(),
        }
    }
}

/// An unclassified failure: [`FailureKind::Other`].
impl From<anyhow::Error> for Failure {
    fn from(source: anyhow::Error) -> Self {
        Self::new(FailureKind::Other, source)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.chain().nth(1)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context as _;
    use uima_testing::check;

    use super::*;

    #[derive(Debug)]
    struct NotFound;

    impl fmt::Display for NotFound {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("not found")
        }
    }

    impl Error for NotFound {}

    /// An error with a cause and the context an adapter adds.
    fn with_context() -> anyhow::Error {
        Err::<(), _>(std::io::Error::other("connection reset"))
            .context("couldn't insert the group")
            .unwrap_err()
    }

    #[test]
    fn failure_shows_the_context_and_leaves_the_cause_to_source() {
        let error = Failure::new(FailureKind::Unavailable, with_context());
        check!(error.to_string() == "couldn't insert the group");
        let source = error.source().map(ToString::to_string);
        check!(source.as_deref() == Some("connection reset"));
    }

    #[test]
    fn failure_without_a_cause_has_no_source() {
        let error = Failure::new(FailureKind::Timeout, anyhow::anyhow!("pool timed out"));
        check!(error.to_string() == "pool timed out");
        check!(error.source().is_none());
    }

    #[test]
    fn failure_keeps_its_kind_and_retry_after() {
        let error = Failure::new(FailureKind::RateLimited, anyhow::anyhow!("429"));
        check!(error.kind() == FailureKind::RateLimited);
        check!(error.retry_after() == None);
        let error = error.with_retry_after(Some(Duration::from_secs(3)));
        check!(error.retry_after() == Some(Duration::from_secs(3)));
        check!(error.with_retry_after(None).retry_after() == None);
    }

    #[test]
    fn context_wraps_the_message_and_keeps_the_rest() {
        let failure = Failure::new(FailureKind::Conflict, std::io::Error::other("deadlock"))
            .with_retry_after(Some(Duration::from_secs(1)))
            .context("couldn't insert the group");
        check!(failure.to_string() == "couldn't insert the group");
        let source = failure.source().map(ToString::to_string);
        check!(source.as_deref() == Some("deadlock"));
        check!(failure.kind() == FailureKind::Conflict);
        check!(failure.retry_after() == Some(Duration::from_secs(1)));
    }

    #[test]
    fn an_anyhow_error_is_an_unclassified_failure() {
        let error: PortError<NotFound> = with_context().into();
        let PortError::Failed(failure) = error else {
            panic!("expected Failed, got {error:?}");
        };
        check!(failure.kind() == FailureKind::Other);
    }

    #[test]
    fn question_mark_converts_context_errors() {
        fn port() -> PortResult<()> {
            Err(std::io::Error::other("connection reset")).context("couldn't insert the group")?;
            Ok(())
        }
        let PortError::Failed(failure) = port().unwrap_err();
        check!(failure.to_string() == "couldn't insert the group");
    }

    #[test]
    fn port_error_shows_the_error_inside() {
        let contract = PortError::Contract(NotFound);
        check!(contract.to_string() == "not found");
        check!(contract.source().is_none());

        let failed = PortError::<NotFound>::Failed(Failure::from(with_context()));
        check!(failed.to_string() == "couldn't insert the group");
        let source = failed.source().map(ToString::to_string);
        check!(source.as_deref() == Some("connection reset"));
    }

    #[test]
    fn failure_context_keeps_a_port_failures_kind() {
        let result: PortResult<()> = Err(Failure::new(FailureKind::Timeout, with_context()).into());
        let failure = result
            .failure_context("failed to list the groups")
            .unwrap_err();
        check!(failure.to_string() == "failed to list the groups");
        let source = failure.source().map(ToString::to_string);
        check!(source.as_deref() == Some("couldn't insert the group"));
        check!(failure.kind() == FailureKind::Timeout);
    }

    #[test]
    fn failure_context_wraps_a_failure() {
        let result: Result<(), Failure> =
            Err(Failure::new(FailureKind::Unavailable, with_context())
                .with_retry_after(Some(Duration::from_secs(2))));
        let failure = result
            .failure_context("failed to list the groups")
            .unwrap_err();
        check!(failure.to_string() == "failed to list the groups");
        check!(failure.kind() == FailureKind::Unavailable);
        check!(failure.retry_after() == Some(Duration::from_secs(2)));
    }

    #[test]
    fn failure_context_makes_an_unhandled_contract_outcome_an_other_failure() {
        let result: PortResult<(), NotFound> = Err(PortError::Contract(NotFound));
        let failure = result
            .failure_context("failed to get the group")
            .unwrap_err();
        check!(failure.to_string() == "failed to get the group");
        let source = failure.source().map(ToString::to_string);
        check!(source.as_deref() == Some("not found"));
        check!(failure.kind() == FailureKind::Other);
    }

    #[test]
    fn failure_context_keeps_the_kind_of_a_contract_ports_failure() {
        let result: PortResult<(), NotFound> =
            Err(Failure::new(FailureKind::Conflict, with_context()).into());
        let failure = result
            .failure_context("failed to get the group")
            .unwrap_err();
        check!(failure.to_string() == "failed to get the group");
        check!(failure.kind() == FailureKind::Conflict);
    }

    #[test]
    fn failure_context_leaves_a_success_alone() {
        let result: PortResult<u8> = Ok(7);
        check!(result.failure_context("failed").unwrap() == 7);
    }

    #[test]
    fn question_mark_takes_a_port_failure_as_it_is() {
        fn usecase(port: PortResult<()>) -> Result<(), Failure> {
            port?;
            Ok(())
        }
        let failure = Failure::new(FailureKind::Timeout, with_context());
        let failure = usecase(Err(failure.into())).unwrap_err();
        check!(failure.to_string() == "couldn't insert the group");
        check!(failure.kind() == FailureKind::Timeout);
    }

    #[test]
    fn a_failure_converts_into_a_port_error() {
        let error: PortError<NotFound> = Failure::new(FailureKind::Conflict, with_context()).into();
        check!(matches!(error, PortError::Failed(e) if e.kind() == FailureKind::Conflict));
    }
}
