//! [`text!`](crate::text) and [`secret!`](crate::secret), newtypes for checked strings.

use std::fmt;

/// Defines a newtype over `String` for text with a length and the characters it allows, such as
/// a display name or a title.
///
/// ```
/// uima_core::text!(
///     /// What other users see as a user's name.
///     pub DisplayName, len = 1..=30, chars = |c| !c.is_control()
/// );
///
/// let name = DisplayName::new("  uima  ").unwrap();
/// assert_eq!(name.as_str(), "uima");
/// assert_eq!(
///     DisplayName::new("a\tb").unwrap_err().to_string(),
///     r"DisplayName can't contain '\t'",
/// );
/// ```
///
/// `new` trims the text, then checks its length in characters, then each character, then the
/// rule, if there is one. The type is ordered and hashable. It has:
///
/// - `MIN_LENGTH`, `MAX_LENGTH`, and `new`.
/// - `as_str`, `into_string`, `AsRef<str>`, and `From<Name> for String`.
/// - `TryFrom<String>`, `TryFrom<&str>`, and `FromStr`.
/// - `Display`, and `Debug` as `Name("text")`.
///
/// Its errors are [`Invalid<Name>`](crate::Invalid), whose kind is a [`TextErrorKind`].
///
/// # Options
///
/// `len` and `chars` are required, and the options come in any order. `trim = false` keeps the
/// whitespace around the text. `rule: R = f` adds a check of the whole text, where `f` is a
/// `fn(&str) -> Result<(), R>` or a closure that captures nothing. `R` is the
/// [`TextErrorKind::Rule`] it returns; it implements `Debug` and `Display`, and its message
/// follows the type's name:
///
/// ```
/// use std::fmt;
///
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// pub struct DotSegment;
///
/// impl fmt::Display for DotSegment {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         f.write_str("can't be '.' or '..'")
///     }
/// }
///
/// uima_core::text!(
///     pub FileName, len = 1..=255, chars = |c| c != '/' && !c.is_control(),
///     trim = false,
///     rule: DotSegment = |s| if s == "." || s == ".." { Err(DotSegment) } else { Ok(()) },
/// );
///
/// assert_eq!(FileName::new(" a ").unwrap().as_str(), " a ");
/// assert_eq!(FileName::new("..").unwrap_err().to_string(), "FileName can't be '.' or '..'");
/// ```
///
/// An unknown option or an empty `len` range doesn't compile:
///
/// ```compile_fail
/// uima_core::text!(Name, len = 1..=5, chars = |_| true, trimm = false); // error: unknown option
/// ```
///
/// ```compile_fail
/// uima_core::text!(Name, len = 5..=2, chars = |_| true); // error: Name's len range is empty
/// ```
#[macro_export]
macro_rules! text {
    (@options $mode:ident $head:tt [$($len:tt)*] $chars:tt $trim:tt $rule:tt
        len = $value:expr $(, $($rest:tt)*)?
    ) => {
        $crate::text!(@options $mode $head [$($len)* ($value)] $chars $trim $rule $($($rest)*)?);
    };
    (@options $mode:ident $head:tt $len:tt [$($chars:tt)*] $trim:tt $rule:tt
        chars = $value:expr $(, $($rest:tt)*)?
    ) => {
        $crate::text!(@options $mode $head $len [$($chars)* ($value)] $trim $rule $($($rest)*)?);
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt [$($trim:tt)*] $rule:tt
        trim = $value:expr $(, $($rest:tt)*)?
    ) => {
        $crate::text!(@options $mode $head $len $chars [$($trim)* ($value)] $rule $($($rest)*)?);
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt [$($rule:tt)*]
        rule: $rule_type:ty = $value:expr $(, $($rest:tt)*)?
    ) => {
        $crate::text!(
            @options $mode $head $len $chars $trim [$($rule)* ($rule_type = $value)] $($($rest)*)?
        );
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt len $($rest:tt)*) => {
        $crate::text!(@malformed $mode "len = a..=b");
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt chars $($rest:tt)*) => {
        $crate::text!(@malformed $mode "chars = f");
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt trim $($rest:tt)*) => {
        $crate::text!(@malformed $mode "trim = bool");
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt rule $($rest:tt)*) => {
        $crate::text!(@malformed $mode "rule: R = f");
    };
    (@malformed $mode:ident $expected:literal) => {
        ::core::compile_error!(::core::concat!(
            "couldn't read an option of ", ::core::stringify!($mode), "!: expected `", $expected,
            "`, followed by `,` or the end",
        ));
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt $key:tt $($rest:tt)*) => {
        ::core::compile_error!(::core::concat!(
            "unknown option `", ::core::stringify!($key), "` of ", ::core::stringify!($mode),
            "!: expected `len`, `chars`, `trim` or `rule`",
        ));
    };
    (@options $mode:ident [$(#[$meta:meta])* $vis:vis $name:ident]
        [($len:expr)] [($chars:expr)] [$(($trim:expr))?] [$(($rule_type:ty = $rule:expr))?]
    ) => {
        $crate::text!(
            @define $mode $(#[$meta])* $vis $name, $len, $chars,
            $crate::text!(@trim $($trim)?)
            $(, $rule_type = $rule)?
        );
        $crate::text!(@extras $mode $name);
    };
    (@options $mode:ident $head:tt [] $chars:tt $trim:tt $rule:tt) => {
        ::core::compile_error!(::core::concat!(::core::stringify!($mode), "! needs `len`"));
    };
    (@options $mode:ident $head:tt $len:tt [] $trim:tt $rule:tt) => {
        ::core::compile_error!(::core::concat!(::core::stringify!($mode), "! needs `chars`"));
    };
    (@options $mode:ident $head:tt $len:tt $chars:tt $trim:tt $rule:tt) => {
        ::core::compile_error!(::core::concat!(
            ::core::stringify!($mode), "! takes each option once",
        ));
    };

    (@extras text $name:ident) => {
        impl $name {
            /// The text itself.
            pub fn as_str(&self) -> &::core::primitive::str {
                &self.0
            }
        }

        impl ::core::convert::AsRef<::core::primitive::str> for $name {
            fn as_ref(&self) -> &::core::primitive::str {
                &self.0
            }
        }

        impl ::core::convert::From<$name> for ::std::string::String {
            fn from(text: $name) -> Self {
                text.0
            }
        }

        impl ::core::cmp::PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> ::core::option::Option<::core::cmp::Ordering> {
                ::core::option::Option::Some(::core::cmp::Ord::cmp(self, other))
            }
        }

        impl ::core::cmp::Ord for $name {
            fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                ::core::cmp::Ord::cmp(&self.0, &other.0)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.pad(&self.0)
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let mut tuple = ::core::fmt::Formatter::debug_tuple(f, ::core::stringify!($name));
                ::core::fmt::DebugTuple::field(&mut tuple, &self.0);
                ::core::fmt::DebugTuple::finish(&mut tuple)
            }
        }
    };
    (@extras secret $name:ident) => {
        impl $name {
            /// The value itself; keep it out of logs.
            pub fn expose(&self) -> &::core::primitive::str {
                &self.0
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(::core::concat!(::core::stringify!($name), "(***)"))
            }
        }
    };

    (@trim) => { true };
    (@trim $trim:expr) => { $trim };

    (@kind text) => { $crate::TextErrorKind };
    (@kind text $rule_type:ty) => { $crate::TextErrorKind<$rule_type> };
    (@kind secret) => { $crate::SecretErrorKind };
    (@kind secret $rule_type:ty) => { $crate::SecretErrorKind<$rule_type> };

    (@rule) => { |_| ::core::result::Result::<(), !>::Ok(()) };
    (@rule $rule_type:ty = $rule:expr) => {{
        let rule: fn(&::core::primitive::str) -> ::core::result::Result<(), $rule_type> = $rule;
        rule
    }};

    (
        @define $mode:ident $(#[$meta:meta])* $vis:vis $name:ident, $len:expr, $chars:expr,
        $trim:expr $(, $rule_type:ty = $rule:expr)?
    ) => {
        $(#[$meta])*
        #[derive(
            ::core::clone::Clone,
            ::core::cmp::PartialEq,
            ::core::cmp::Eq,
            ::core::hash::Hash,
        )]
        $vis struct $name(::std::string::String);

        impl $name {
            /// The fewest characters it takes.
            pub const MIN_LENGTH: ::core::primitive::usize =
                *::core::ops::RangeInclusive::<::core::primitive::usize>::start(&$len);
            /// The most characters it takes.
            pub const MAX_LENGTH: ::core::primitive::usize =
                *::core::ops::RangeInclusive::<::core::primitive::usize>::end(&$len);

            /// `text`, or the first check it fails.
            pub fn new(
                text: impl ::core::convert::AsRef<::core::primitive::str>,
            ) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                let text = Self::checked(text.as_ref())?;
                ::core::result::Result::Ok(Self(::std::borrow::ToOwned::to_owned(text)))
            }

            /// The text as an owned `String`.
            pub fn into_string(self) -> ::std::string::String {
                self.0
            }

            fn checked(
                text: &::core::primitive::str,
            ) -> ::core::result::Result<&::core::primitive::str, $crate::Invalid<Self>> {
                let text = if $trim { ::core::primitive::str::trim(text) } else { text };
                let chars: fn(::core::primitive::char) -> ::core::primitive::bool = $chars;
                let checked = $crate::__private::check_text(
                    text,
                    Self::MIN_LENGTH,
                    Self::MAX_LENGTH,
                    chars,
                    $crate::text!(@rule $($rule_type = $rule)?),
                );
                match checked {
                    ::core::result::Result::Ok(()) => ::core::result::Result::Ok(text),
                    ::core::result::Result::Err(error) => ::core::result::Result::Err(
                        $crate::Invalid::new(::core::convert::From::from(error)),
                    ),
                }
            }
        }

        const _: () = ::core::assert!(
            $name::MIN_LENGTH <= $name::MAX_LENGTH,
            ::core::concat!(::core::stringify!($name), "'s len range is empty"),
        );

        impl ::core::convert::TryFrom<::std::string::String> for $name {
            type Error = $crate::Invalid<Self>;

            fn try_from(text: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
                let checked = Self::checked(&text)?;
                if checked.len() == text.len() {
                    return ::core::result::Result::Ok(Self(text));
                }
                ::core::result::Result::Ok(Self(::std::borrow::ToOwned::to_owned(checked)))
            }
        }

        impl ::core::convert::TryFrom<&::core::primitive::str> for $name {
            type Error = $crate::Invalid<Self>;

            fn try_from(text: &::core::primitive::str) -> ::core::result::Result<Self, Self::Error> {
                Self::new(text)
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = $crate::Invalid<Self>;

            fn from_str(text: &::core::primitive::str) -> ::core::result::Result<Self, Self::Err> {
                Self::new(text)
            }
        }

        impl $crate::Newtype for $name {
            const NAME: &'static ::core::primitive::str = ::core::stringify!($name);
            type Kind = $crate::text!(@kind $mode $($rule_type)?);
        }
    };
    ($(#[$meta:meta])* $vis:vis $name:ident, $($options:tt)*) => {
        $crate::text!(@options text [$(#[$meta])* $vis $name] [] [] [] [] $($options)*);
    };
}

/// Like [`text!`](crate::text), for a value that must stay out of logs and error messages, such
/// as a password or an API key.
///
/// ```
/// use std::str::FromStr;
///
/// uima_core::secret!(pub ApiKey, len = 16..=64, chars = |c| c.is_ascii_alphanumeric());
///
/// let key = ApiKey::from_str("0123456789abcdef").unwrap();
/// assert_eq!(key.expose(), "0123456789abcdef");
/// assert_eq!(format!("{key:?}"), "ApiKey(***)");
/// assert_eq!(
///     ApiKey::new("0123456789abcde!").unwrap_err().to_string(),
///     "ApiKey has a character that isn't allowed",
/// );
/// ```
///
/// It takes the same options and runs the same checks, but reading it takes `expose` or
/// `into_string`: it has no `Display`, `AsRef<str>`, `as_str`, ordering, or conversion into a
/// `String`, and `Debug` prints `Name(***)`. Its errors' kind is a [`SecretErrorKind`], which leaves
/// out the value and its length; a `rule`'s error type mustn't hold the value either. Comparing two
/// with `==` isn't constant-time. With serde, derive only `Deserialize`, with
/// `#[serde(try_from = "String")]`; serializing would write the secret out.
#[macro_export]
macro_rules! secret {
    ($(#[$meta:meta])* $vis:vis $name:ident, $($options:tt)*) => {
        $crate::text!(@options secret [$(#[$meta])* $vis $name] [] [] [] [] $($options)*);
    };
}

/// What is wrong with a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextErrorKind<R = !> {
    /// Fewer than `min` characters.
    TooShort { min: usize, received: usize },
    /// More than `max` characters.
    TooLong { max: usize },
    /// Has a character `chars` doesn't allow.
    Char { received: char },
    /// Fails the type's `rule`.
    Rule(R),
}

/// What is wrong with a secret, leaving out the value and its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretErrorKind<R = !> {
    /// Fewer than `min` characters.
    TooShort { min: usize },
    /// More than `max` characters.
    TooLong { max: usize },
    /// Has a character `chars` doesn't allow.
    Char,
    /// Fails the type's `rule`.
    Rule(R),
}

impl<R: fmt::Display> fmt::Display for TextErrorKind<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { min, received } => {
                write!(
                    f,
                    "must have at least {}, received: {received}",
                    characters(*min)
                )
            }
            Self::TooLong { max } => write!(f, "must have at most {}", characters(*max)),
            Self::Char { received } => write!(f, "can't contain {received:?}"),
            Self::Rule(rule) => rule.fmt(f),
        }
    }
}

impl<R: fmt::Display> fmt::Display for SecretErrorKind<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { min } => write!(f, "must have at least {}", characters(*min)),
            Self::TooLong { max } => write!(f, "must have at most {}", characters(*max)),
            Self::Char => f.write_str("has a character that isn't allowed"),
            Self::Rule(rule) => rule.fmt(f),
        }
    }
}

impl<R> From<TextErrorKind<R>> for SecretErrorKind<R> {
    fn from(error: TextErrorKind<R>) -> Self {
        match error {
            TextErrorKind::TooShort { min, .. } => Self::TooShort { min },
            TextErrorKind::TooLong { max } => Self::TooLong { max },
            TextErrorKind::Char { .. } => Self::Char,
            TextErrorKind::Rule(rule) => Self::Rule(rule),
        }
    }
}

fn characters(n: usize) -> String {
    match n {
        1 => "1 character".to_owned(),
        n => format!("{n} characters"),
    }
}

/// The checks of [`text!`](crate::text) and [`secret!`](crate::secret), in order.
#[doc(hidden)]
pub fn check<R>(
    text: &str,
    min: usize,
    max: usize,
    chars: fn(char) -> bool,
    rule: fn(&str) -> Result<(), R>,
) -> Result<(), TextErrorKind<R>> {
    // Counts at most one past `max`, so a long text costs no more than a valid one.
    let length = text.chars().take(max.saturating_add(1)).count();
    if length < min {
        return Err(TextErrorKind::TooShort {
            min,
            received: length,
        });
    }
    if length > max {
        return Err(TextErrorKind::TooLong { max });
    }
    if let Some(received) = text.chars().find(|&c| !chars(c)) {
        return Err(TextErrorKind::Char { received });
    }
    rule(text).map_err(TextErrorKind::Rule)
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "a type needn't use every method the macro defines"
)]
mod tests {
    use std::str::FromStr;

    use assert2::check;

    use super::*;
    use crate::{Invalid, Newtype};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Digits {
        LeadingZero,
    }

    impl fmt::Display for Digits {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("can't start with 0")
        }
    }

    fn no_control(c: char) -> bool {
        !c.is_control()
    }

    fn no_leading_zero(s: &str) -> Result<(), Digits> {
        match s.starts_with('0') {
            true => Err(Digits::LeadingZero),
            false => Ok(()),
        }
    }

    crate::text!(Name, len = 1..=5, chars = no_control);
    crate::text!(Note, len = 0..=5, chars = |c| c == '\n' || !c.is_control());
    crate::text!(Spaced, len = 1..=5, chars = no_control, trim = false);
    crate::text!(Code, len = 1..=5, chars = |c| c.is_ascii_digit(), rule: Digits = no_leading_zero);
    crate::text!(
        Lower,
        len = 1..=5,
        chars = |c| c.is_ascii_lowercase(),
        rule: Digits = |s| if s == "no" { Err(Digits::LeadingZero) } else { Ok(()) },
    );
    crate::text!(Single, len = 1..=1, chars = no_control);
    crate::text!(
        Reordered,
        rule: Digits = no_leading_zero,
        trim = false,
        chars = |c| c.is_ascii_digit(),
        len = 1..=3,
    );
    crate::secret!(Token, len = 2..=5, chars = |c| c.is_ascii_graphic());
    crate::secret!(Pin, len = 1..=4, chars = |c| c.is_ascii_digit(), trim = false, rule: Digits = no_leading_zero);

    fn rejected<T, N>(result: Result<T, Invalid<N>>) -> N::Kind
    where
        T: fmt::Debug,
        N: Newtype,
    {
        result.unwrap_err().into_kind()
    }

    #[test]
    fn trims_unless_told_not_to() {
        check!(Name::new("  ab  ").unwrap().as_str() == "ab");
        check!(Spaced::new(" ab ").unwrap().as_str() == " ab ");
        check!(Token::new(" ab ").unwrap().expose() == "ab");
        check!(Pin::new(" 1").is_err());
    }

    #[test]
    fn checks_the_length_once_trimmed() {
        check!(
            rejected(Name::new("   "))
                == TextErrorKind::TooShort {
                    min: 1,
                    received: 0
                }
        );
        check!(rejected(Spaced::new("      ")) == TextErrorKind::TooLong { max: 5 });
    }

    #[test]
    fn counts_the_length_in_characters() {
        check!(Name::new("測試測試測").unwrap().as_str() == "測試測試測");
        check!(rejected(Name::new("測試測試測試")) == TextErrorKind::TooLong { max: 5 });
    }

    #[test]
    fn accepts_empty_text_with_a_zero_min() {
        check!(Note::new("").unwrap().as_str() == "");
    }

    #[test]
    fn exposes_its_bounds() {
        check!((Name::MIN_LENGTH, Name::MAX_LENGTH) == (1, 5));
        check!((Note::MIN_LENGTH, Note::MAX_LENGTH) == (0, 5));
    }

    #[test]
    fn rejects_characters_it_doesnt_allow() {
        check!(rejected(Name::new("a\nb")) == TextErrorKind::Char { received: '\n' });
        check!(Name::new("a b").is_ok());
        check!(Note::new("a\nb").unwrap().as_str() == "a\nb");
        check!(rejected(Note::new("a\tb")) == TextErrorKind::Char { received: '\t' });
    }

    #[test]
    fn runs_its_rule() {
        check!(Code::new("120").unwrap().as_str() == "120");
        check!(rejected(Code::new("012")) == TextErrorKind::Rule(Digits::LeadingZero));
        check!(Lower::new("ab").is_ok());
        check!(rejected(Lower::new("no")) == TextErrorKind::Rule(Digits::LeadingZero));
    }

    #[test]
    fn takes_its_options_in_any_order() {
        check!((Reordered::MIN_LENGTH, Reordered::MAX_LENGTH) == (1, 3));
        check!(Reordered::new("12").is_ok());
        check!(rejected(Reordered::new(" 12")) == TextErrorKind::Char { received: ' ' });
        check!(rejected(Reordered::new("012")) == TextErrorKind::Rule(Digits::LeadingZero));
    }

    #[test]
    fn checks_length_then_characters_then_rule() {
        check!(rejected(Code::new("0a0a0a")) == TextErrorKind::TooLong { max: 5 });
        check!(rejected(Code::new("0a")) == TextErrorKind::Char { received: 'a' });
    }

    #[test]
    fn converts_from_and_into_strings() {
        check!(Name::try_from(String::from("ab")).unwrap().into_string() == "ab");
        check!(Name::try_from("ab").unwrap().as_ref() == "ab");
        check!(Name::from_str(" ab ").unwrap().to_string() == "ab");
        check!(String::from(Name::new("ab").unwrap()) == "ab");
        check!(Name::new("ab").unwrap() < Name::new("b").unwrap());
    }

    #[test]
    fn try_from_a_string_keeps_it_unless_trimmed() {
        let text = String::from("ab");
        let address = text.as_ptr();
        let kept = Name::try_from(text).unwrap().into_string();
        check!(kept.as_ptr() == address);
        check!(Name::try_from(String::from(" ab ")).unwrap().as_str() == "ab");
        check!(
            rejected(Name::try_from(String::from("  ")))
                == TextErrorKind::TooShort {
                    min: 1,
                    received: 0
                }
        );
    }

    #[test]
    fn display_pads_like_a_str() {
        check!(format!("[{:>4}]", Name::new("ab").unwrap()) == "[  ab]");
    }

    #[test]
    fn debug_shows_the_text() {
        check!(format!("{:?}", Name::new("ab").unwrap()) == r#"Name("ab")"#);
    }

    #[test]
    fn error_names_its_type() {
        check!(
            Name::new("").unwrap_err().to_string()
                == "Name must have at least 1 character, received: 0"
        );
        check!(
            Single::new("ab").unwrap_err().to_string() == "Single must have at most 1 character"
        );
        check!(
            Name::new("abcdef").unwrap_err().to_string() == "Name must have at most 5 characters"
        );
        check!(Name::new("a\tb").unwrap_err().to_string() == r"Name can't contain '\t'");
        check!(Code::new("012").unwrap_err().to_string() == "Code can't start with 0");
    }

    #[test]
    fn secret_hides_its_value() {
        let token = Token::new(" abc ").unwrap();
        check!(format!("{token:?}") == "Token(***)");
        check!(token.expose() == "abc");
        check!(token.into_string() == "abc");
        check!(Token::from_str("abc").is_ok());
        check!(Token::try_from("abc").is_ok());
        check!(Token::try_from(String::from("abc")).is_ok());
    }

    #[test]
    fn secret_errors_leave_out_the_value() {
        check!(rejected(Token::new("a")) == SecretErrorKind::TooShort { min: 2 });
        check!(rejected(Token::new("abcdef")) == SecretErrorKind::TooLong { max: 5 });
        check!(rejected(Token::new("a\u{1}b")) == SecretErrorKind::Char);
        check!(rejected(Pin::new("012")) == SecretErrorKind::Rule(Digits::LeadingZero));
        check!(Token::new("a").unwrap_err().to_string() == "Token must have at least 2 characters");
        check!(
            Token::new("abcdef").unwrap_err().to_string() == "Token must have at most 5 characters"
        );
        check!(
            Token::new("a b").unwrap_err().to_string()
                == "Token has a character that isn't allowed"
        );
        check!(Pin::new("012").unwrap_err().to_string() == "Pin can't start with 0");
        let debug = format!("{:?}", Token::new("x\u{1}y").unwrap_err());
        check!(!debug.contains('x'));
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "a type needn't use every method the macro defines"
)]
mod props {
    use proptest::prelude::*;

    use super::*;

    crate::text!(Name, len = 1..=5, chars = |c| !c.is_control());
    crate::secret!(
        Token,
        len = 1..=5,
        chars = |c| !c.is_control(),
        trim = false
    );

    /// Short text, often with whitespace and control characters.
    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just(' '),
                Just('\t'),
                Just('\u{1}'),
                Just('a'),
                Just('測'),
                any::<char>()
            ],
            0..9,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    /// The checks, spelled out.
    fn expected(text: &str) -> Result<String, TextErrorKind> {
        let length = text.chars().count();
        if length < 1 {
            return Err(TextErrorKind::TooShort {
                min: 1,
                received: length,
            });
        }
        if length > 5 {
            return Err(TextErrorKind::TooLong { max: 5 });
        }
        match text.chars().find(|c| c.is_control()) {
            Some(received) => Err(TextErrorKind::Char { received }),
            None => Ok(text.to_owned()),
        }
    }

    proptest! {
        #[test]
        fn text_checks_the_trimmed_text(raw in text()) {
            let got = Name::new(&raw).map(Name::into_string).map_err(|e| e.into_kind());
            prop_assert_eq!(got, expected(raw.trim()));
        }

        #[test]
        fn secret_checks_like_text_without_the_value(raw in text()) {
            let got = Token::new(&raw).map(Token::into_string).map_err(|e| e.into_kind());
            prop_assert_eq!(got, expected(&raw).map_err(SecretErrorKind::from));
        }
    }
}

/// Compiles only if the macros use none of the caller's names.
#[cfg(test)]
#[allow(
    dead_code,
    unused_macros,
    non_camel_case_types,
    reason = "only here to shadow names"
)]
mod shadowed {
    struct String;
    struct str;
    struct char;
    struct bool;
    struct usize;
    struct Option;
    struct Invalid;
    struct TextErrorKind;
    struct SecretErrorKind;
    trait Newtype {}
    trait ToOwned {}
    trait Methods {
        fn field(&self, _: &i32) {}
        fn finish(&self) {}
        fn len(&self) {}
        fn trim(&self) {}
        fn pad(&self, _: &i32) {}
        fn as_ref(&self) {}
    }
    impl<T: ?Sized> Methods for T {}
    type Result<T> = core::result::Result<T, ()>;
    macro_rules! write {
        ($($t:tt)*) => {
            compile_error!("the macros must not use the caller's write!")
        };
    }
    macro_rules! assert {
        ($($t:tt)*) => {
            compile_error!("the macros must not use the caller's assert!")
        };
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct Rule;

    impl core::fmt::Display for Rule {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("breaks the rule")
        }
    }

    crate::text!(pub(crate) ShadowText, len = 1..=3, chars = |c| c.is_ascii());
    crate::text!(pub(crate) ShadowRule, len = 1..=3, chars = |c| c.is_ascii(), trim = false, rule: Rule = |_| Ok(()));
    crate::secret!(pub(crate) ShadowSecret, len = 1..=3, chars = |c| c.is_ascii());
}
