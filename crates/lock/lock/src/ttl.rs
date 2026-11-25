//! [`Ttl`], how long a lock is held unless extended.

use std::fmt;
use std::time::Duration;

use uima_core::{Invalid, Newtype};

/// How long a lock is held unless extended, within [`Ttl::MIN`]`..=`[`Ttl::MAX`].
///
/// A constant one comes from [`from_secs`](Self::from_secs), and one read from config from
/// [`new`](Self::new).
///
/// ```
/// use std::time::Duration;
///
/// use uima_lock::Ttl;
///
/// const PURGE_TTL: Ttl = Ttl::from_secs(30);
///
/// assert_eq!(PURGE_TTL.get(), Duration::from_secs(30));
/// assert!(Ttl::new(Duration::from_millis(10)).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ttl(Duration);

impl Ttl {
    /// Long enough to extend a lease, through a slow store, before most of it passes.
    pub const MIN: Self = Self(Duration::from_secs(1));
    /// A year, so a lease's expiry can't overflow.
    pub const MAX: Self = Self(Duration::from_secs(365 * 24 * 60 * 60));

    /// Fails outside [`MIN`](Self::MIN)`..=`[`MAX`](Self::MAX).
    pub const fn new(ttl: Duration) -> Result<Self, Invalid<Self>> {
        let nanos = ttl.as_nanos();
        if nanos < Self::MIN.0.as_nanos() || nanos > Self::MAX.0.as_nanos() {
            return Err(Invalid::new(TtlErrorKind::OutOfRange { received: ttl }));
        }
        Ok(Self(ttl))
    }

    /// `secs` seconds.
    ///
    /// Panics outside [`MIN`](Self::MIN)`..=`[`MAX`](Self::MAX), so a constant out of range
    /// doesn't compile.
    pub const fn from_secs(secs: u64) -> Self {
        match Self::new(Duration::from_secs(secs)) {
            Ok(ttl) => ttl,
            Err(_) => panic!("a lock's ttl must be between 1s and 365 days"),
        }
    }

    pub const fn get(self) -> Duration {
        self.0
    }
}

impl Newtype for Ttl {
    const NAME: &'static str = "Ttl";
    type Kind = TtlErrorKind;
}

/// What can be wrong with a [`Ttl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtlErrorKind {
    /// Outside [`Ttl::MIN`]`..=`[`Ttl::MAX`].
    OutOfRange { received: Duration },
}

impl fmt::Display for TtlErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { received } => write!(
                f,
                "must be between {:?} and {:?}, received: {received:?}",
                Ttl::MIN.0,
                Ttl::MAX.0
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    #[test]
    fn new_takes_a_ttl_within_its_range_only() {
        let nanosecond = Duration::from_nanos(1);
        for ttl in [Ttl::MIN.get(), Ttl::MAX.get()] {
            assert!(let Ok(_) = Ttl::new(ttl), "{ttl:?}");
        }
        for ttl in [
            Duration::ZERO,
            Ttl::MIN.get() - nanosecond,
            Ttl::MAX.get() + nanosecond,
            Duration::MAX,
        ] {
            let error = Ttl::new(ttl).unwrap_err();
            check!(error.kind() == &TtlErrorKind::OutOfRange { received: ttl });
        }

        check!(
            Ttl::new(Duration::ZERO).unwrap_err().to_string()
                == "Ttl must be between 1s and 31536000s, received: 0ns"
        );
    }

    #[test]
    fn from_secs_takes_a_ttl_within_its_range_only() {
        const DAY: Ttl = Ttl::from_secs(24 * 60 * 60);
        check!(DAY.get() == Duration::from_secs(24 * 60 * 60));

        check!(std::panic::catch_unwind(|| Ttl::from_secs(0)).is_err());
    }
}
