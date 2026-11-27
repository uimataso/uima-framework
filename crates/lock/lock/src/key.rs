//! [`LockKey`], what a lock is taken on.

use std::borrow::Cow;
use std::fmt;

use uima_core::{Invalid, Newtype};

/// What a lock is taken on, 1 to [`LockKey::MAX_LEN`] lowercase ASCII letters, digits and `-`,
/// starting and ending with a letter or digit.
///
/// That is a DNS label, so a [`LockProvider`](crate::LockProvider) can name the lock in its store
/// by the key as it is.
///
/// A constant one comes from [`from_static`](Self::from_static), and one made at run time, such
/// as a key per device, from [`new`](Self::new).
///
/// ```
/// use uima_lock::LockKey;
///
/// const PURGE: LockKey = LockKey::from_static("purge");
///
/// let device = LockKey::new(format!("device-{}", 7))?;
/// assert_eq!(device.as_str(), "device-7");
/// assert!(LockKey::new("device:7").is_err());
/// # Ok::<(), uima_core::Invalid<LockKey>>(())
/// ```
// Written by hand, not with `uima_core::text!`, so `from_static` can be `const`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LockKey(Cow<'static, str>);

impl LockKey {
    /// The longest key, as a DNS label is.
    pub const MAX_LEN: usize = 63;

    /// A key of `key`.
    ///
    /// Fails for a key the type's rule doesn't allow.
    pub fn new(key: impl Into<String>) -> Result<Self, Invalid<Self>> {
        let key = key.into();
        Self::check(&key)?;
        Ok(Self(Cow::Owned(key)))
    }

    /// `key`.
    ///
    /// Panics for a key [`new`](Self::new) fails, so a constant one doesn't compile:
    ///
    /// ```compile_fail
    /// const KEY: uima_lock::LockKey = uima_lock::LockKey::from_static("Purge");
    /// # let _ = KEY;
    /// ```
    pub const fn from_static(key: &'static str) -> Self {
        match Self::check(key) {
            Ok(()) => Self(Cow::Borrowed(key)),
            Err(_) => panic!(
                "a lock's key must be 1 to 63 of a-z, 0-9 and -, starting and ending with a letter or digit"
            ),
        }
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    const fn check(key: &str) -> Result<(), Invalid<Self>> {
        let bytes = key.as_bytes();
        let len = bytes.len();
        if len == 0 {
            return Err(Invalid::new(LockKeyErrorKind::Empty));
        }

        let mut at = 0;
        while at < len {
            let byte = bytes[at];
            if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
                return Err(Invalid::new(LockKeyErrorKind::Char { at }));
            }
            at += 1;
        }

        // Checked after the characters, so each byte here is one character.
        if len > Self::MAX_LEN {
            return Err(Invalid::new(LockKeyErrorKind::TooLong { received: len }));
        }

        if bytes[0] == b'-' || bytes[len - 1] == b'-' {
            return Err(Invalid::new(LockKeyErrorKind::HyphenAtEnd));
        }

        Ok(())
    }
}

impl fmt::Display for LockKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Newtype for LockKey {
    const NAME: &'static str = "LockKey";
    type Kind = LockKeyErrorKind;
}

/// What can be wrong with a [`LockKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockKeyErrorKind {
    Empty,
    /// Over [`LockKey::MAX_LEN`] characters.
    TooLong {
        received: usize,
    },
    /// A character other than a lowercase ASCII letter, a digit or `-`, starting at byte `at`.
    Char {
        at: usize,
    },
    /// Starts or ends with `-`.
    HyphenAtEnd,
}

impl fmt::Display for LockKeyErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("must not be empty"),
            Self::TooLong { received } => write!(
                f,
                "must be at most {} characters, received: {received}",
                LockKey::MAX_LEN
            ),
            Self::Char { at } => write!(
                f,
                "must hold only a-z, 0-9 and -, received another character at byte {at}"
            ),
            Self::HyphenAtEnd => f.write_str("must not start or end with -"),
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    #[test]
    fn new_takes_a_dns_label_only() {
        let longest = "k".repeat(LockKey::MAX_LEN);
        for key in ["k", "7", "purge", "device-7", "a--b", &longest] {
            assert!(let Ok(made) = LockKey::new(key), "{key}");
            check!(made.as_str() == key);
        }

        let too_long = "k".repeat(LockKey::MAX_LEN + 1);
        let too_long_unicode = "鎖".repeat(LockKey::MAX_LEN / 3 + 1);
        for (key, kind) in [
            ("", LockKeyErrorKind::Empty),
            (&*too_long, LockKeyErrorKind::TooLong { received: 64 }),
            ("Purge", LockKeyErrorKind::Char { at: 0 }),
            ("device:7", LockKeyErrorKind::Char { at: 6 }),
            ("device_7", LockKeyErrorKind::Char { at: 6 }),
            ("a.b", LockKeyErrorKind::Char { at: 1 }),
            ("a b", LockKeyErrorKind::Char { at: 1 }),
            ("鎖", LockKeyErrorKind::Char { at: 0 }),
            (&*too_long_unicode, LockKeyErrorKind::Char { at: 0 }),
            ("a\0", LockKeyErrorKind::Char { at: 1 }),
            ("-purge", LockKeyErrorKind::HyphenAtEnd),
            ("purge-", LockKeyErrorKind::HyphenAtEnd),
            ("-", LockKeyErrorKind::HyphenAtEnd),
        ] {
            let error = LockKey::new(key).unwrap_err();
            check!(error.kind() == &kind, "{key:?}");
        }

        check!(
            LockKey::new("purge-").unwrap_err().to_string()
                == "LockKey must not start or end with -"
        );
    }

    #[test]
    fn from_static_takes_a_dns_label_only() {
        const PURGE: LockKey = LockKey::from_static("purge");
        check!(PURGE == LockKey::new("purge").unwrap());

        for key in ["", "Purge", "purge-"] {
            check!(
                std::panic::catch_unwind(|| LockKey::from_static(key)).is_err(),
                "{key:?}"
            );
        }
    }
}
