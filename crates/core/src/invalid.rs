//! [`Invalid`], the error of every newtype the macros define.

use std::error::Error;
use std::fmt;

/// A newtype the macros define; the macros implement it.
pub trait Newtype {
    /// The type's name, as error messages show it.
    const NAME: &'static str;
    /// What can be wrong with a value, such as [`CountError`](crate::CountError). Its message
    /// follows the type's name: "can't exceed 3, received: 4".
    type Kind: fmt::Debug + fmt::Display;
}

/// A value `T` rejects; [`kind`](Self::kind) tells what was wrong.
///
/// ```
/// use uima_core::{CountError, Invalid};
///
/// uima_core::count!(pub Retries, max = 3);
///
/// let error: Invalid<Retries> = Retries::new(3).unwrap().try_increment().unwrap_err();
/// assert_eq!(error.kind(), &CountError::AboveMax { received: 4, max: 3 });
/// assert_eq!(error.to_string(), "Retries can't exceed 3, received: 4");
/// ```
pub struct Invalid<T: Newtype> {
    kind: T::Kind,
}

impl<T: Newtype> Invalid<T> {
    pub const fn new(kind: T::Kind) -> Self {
        Self { kind }
    }

    pub const fn kind(&self) -> &T::Kind {
        &self.kind
    }

    pub fn into_kind(self) -> T::Kind {
        self.kind
    }
}

impl<T: Newtype> fmt::Debug for Invalid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Invalid")
            .field("type", &T::NAME)
            .field("kind", &self.kind)
            .finish()
    }
}

impl<T: Newtype> fmt::Display for Invalid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", T::NAME, self.kind)
    }
}

impl<T: Newtype> Error for Invalid<T> {}

impl<T: Newtype<Kind: Clone>> Clone for Invalid<T> {
    fn clone(&self) -> Self {
        Self::new(self.kind.clone())
    }
}

impl<T: Newtype<Kind: Copy>> Copy for Invalid<T> {}

impl<T: Newtype<Kind: PartialEq>> PartialEq for Invalid<T> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl<T: Newtype<Kind: Eq>> Eq for Invalid<T> {}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    struct Probe;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ProbeError {
        TooBig,
        TooSmall,
    }

    impl fmt::Display for ProbeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Self::TooBig => "is too big",
                Self::TooSmall => "is too small",
            })
        }
    }

    impl Newtype for Probe {
        const NAME: &'static str = "Probe";
        type Kind = ProbeError;
    }

    fn invalid(kind: ProbeError) -> Invalid<Probe> {
        Invalid::new(kind)
    }

    #[test]
    fn keeps_its_kind() {
        check!(invalid(ProbeError::TooBig).kind() == &ProbeError::TooBig);
        check!(invalid(ProbeError::TooBig).into_kind() == ProbeError::TooBig);
    }

    #[test]
    fn message_is_the_name_then_the_kind() {
        check!(invalid(ProbeError::TooBig).to_string() == "Probe is too big");
    }

    #[test]
    fn debug_shows_the_type_and_kind() {
        let debug = format!("{:?}", invalid(ProbeError::TooBig));
        check!(debug == r#"Invalid { type: "Probe", kind: TooBig }"#);
    }

    #[test]
    fn compares_and_clones_by_kind() {
        let error = invalid(ProbeError::TooBig);
        check!(error.clone() == error);
        check!(error != invalid(ProbeError::TooSmall));
    }

    #[test]
    fn has_no_source() {
        check!(invalid(ProbeError::TooBig).source().is_none());
    }
}
