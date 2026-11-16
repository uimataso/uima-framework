//! [`number!`](crate::number), a newtype for an integer within a range.

use std::fmt;

/// Defines a newtype over an integer type for a number within a range, such as an event's
/// capacity or a file's size.
///
/// ```
/// uima_core::number!(
///     /// How many people an event takes.
///     pub EventCapacity, u32, range = 2..=6
/// );
///
/// assert_eq!(EventCapacity::new(4).unwrap().get(), 4);
/// assert_eq!(
///     EventCapacity::new(7).unwrap_err().to_string(),
///     "EventCapacity must be between 2 and 6, received: 7",
/// );
/// ```
///
/// `range` is an inclusive range of const expressions, such as `2..=6`, `-3..=3` or
/// `1..=MAX_MEMBERS`. The type is `Copy`, ordered, and hashable. It has:
///
/// - `MIN`, `MAX`, and `new`, which rejects a value outside `MIN..=MAX`.
/// - `get`.
/// - `try_add` and `try_sub`, which fail outside `MIN..=MAX`.
/// - `TryFrom<i64>`, `From<Name> for i64`, and `Display`.
///
/// Its errors are [`Invalid<Name>`](crate::Invalid), whose kind is a [`NumberErrorKind`].
///
/// # Limits
///
/// The range must fit an `i64`, and the integer type must be at most 64 bits. An empty range
/// doesn't compile either:
///
/// ```compile_fail
/// uima_core::number!(Size, u64, range = 0..=u64::MAX); // error: Size's range must fit an i64
/// ```
///
/// ```compile_fail
/// uima_core::number!(Wide, i128, range = 0..=10); // error: Wide's type must be at most 64 bits
/// ```
///
/// ```compile_fail
/// uima_core::number!(Capacity, u32, range = 6..=2); // error: Capacity's range is empty
/// ```
///
/// Nor does a range that isn't inclusive:
///
/// ```compile_fail
/// uima_core::number!(Capacity, u32, range = 2..7); // error: expected `RangeInclusive`
/// ```
#[macro_export]
macro_rules! number {
    ($(#[$meta:meta])* $vis:vis $name:ident, $t:ty, range = $range:expr $(,)?) => {
        $(#[$meta])*
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            ::core::marker::Copy,
            ::core::cmp::PartialEq,
            ::core::cmp::Eq,
            ::core::cmp::PartialOrd,
            ::core::cmp::Ord,
            ::core::hash::Hash,
        )]
        $vis struct $name($t);

        impl $name {
            /// The lowest number it takes.
            pub const MIN: $t = *::core::ops::RangeInclusive::<$t>::start(&$range);
            /// The highest number it takes.
            pub const MAX: $t = *::core::ops::RangeInclusive::<$t>::end(&$range);

            /// `number`, or `OutOfRange` outside `MIN..=MAX`.
            pub const fn new(number: $t) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                if number < Self::MIN || number > Self::MAX {
                    return ::core::result::Result::Err(Self::out_of_range(
                        number as ::core::primitive::i128,
                    ));
                }
                ::core::result::Result::Ok(Self(number))
            }

            /// The number itself.
            pub const fn get(self) -> $t {
                self.0
            }

            /// `n` more, or `OutOfRange` outside `MIN..=MAX`.
            pub const fn try_add(self, n: $t) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                Self::from_wide(self.0 as ::core::primitive::i128 + n as ::core::primitive::i128)
            }

            /// `n` less, or `OutOfRange` outside `MIN..=MAX`.
            pub const fn try_sub(self, n: $t) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                Self::from_wide(self.0 as ::core::primitive::i128 - n as ::core::primitive::i128)
            }

            const fn from_wide(
                number: ::core::primitive::i128,
            ) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                if number < Self::MIN as ::core::primitive::i128
                    || number > Self::MAX as ::core::primitive::i128
                {
                    return ::core::result::Result::Err(Self::out_of_range(number));
                }
                // Within `MIN..=MAX`, so it fits `$t`.
                ::core::result::Result::Ok(Self(number as $t))
            }

            const fn out_of_range(received: ::core::primitive::i128) -> $crate::Invalid<Self> {
                $crate::Invalid::new($crate::NumberErrorKind::OutOfRange {
                    received,
                    min: Self::MIN as ::core::primitive::i64,
                    max: Self::MAX as ::core::primitive::i64,
                })
            }
        }

        const _: () = ::core::assert!(
            $name::MIN <= $name::MAX,
            ::core::concat!(::core::stringify!($name), "'s range is empty"),
        );
        const _: () = ::core::assert!(
            ::core::mem::size_of::<$t>() <= 8,
            ::core::concat!(::core::stringify!($name), "'s type must be at most 64 bits"),
        );
        const _: () = ::core::assert!(
            $name::MIN as ::core::primitive::i128 >= ::core::primitive::i64::MIN as ::core::primitive::i128
                && $name::MAX as ::core::primitive::i128
                    <= ::core::primitive::i64::MAX as ::core::primitive::i128,
            ::core::concat!(::core::stringify!($name), "'s range must fit an i64"),
        );

        impl ::core::convert::TryFrom<::core::primitive::i64> for $name {
            type Error = $crate::Invalid<Self>;

            fn try_from(number: ::core::primitive::i64) -> ::core::result::Result<Self, Self::Error> {
                match <$t as ::core::convert::TryFrom<::core::primitive::i64>>::try_from(number) {
                    ::core::result::Result::Ok(number) => Self::new(number),
                    ::core::result::Result::Err(_) => {
                        ::core::result::Result::Err(Self::out_of_range(number as ::core::primitive::i128))
                    }
                }
            }
        }

        impl ::core::convert::From<$name> for ::core::primitive::i64 {
            fn from(number: $name) -> Self {
                // The range fits an i64, as asserted above.
                number.0 as ::core::primitive::i64
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl $crate::Newtype for $name {
            const NAME: &'static ::core::primitive::str = ::core::stringify!($name);
            type Kind = $crate::NumberErrorKind;
        }
    };
}

/// What is wrong with a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberErrorKind {
    /// Outside `min..=max`.
    OutOfRange { received: i128, min: i64, max: i64 },
}

impl fmt::Display for NumberErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { received, min, max } => {
                write!(f, "must be between {min} and {max}, received: {received}")
            }
        }
    }
}

#[cfg(test)]
#[expect(
    dead_code,
    reason = "a type needn't use every method the macro defines"
)]
mod tests {
    use uima_testing::check;

    use super::*;
    use crate::{Invalid, Newtype};

    const LIMIT: u32 = 9;

    crate::number!(Capacity, u32, range = 2..=6);
    crate::number!(Offset, i16, range = -3..=3);
    crate::number!(Below, i16, range = -9..=-1);
    crate::number!(Big, i64, range = 0..=i64::MAX);
    crate::number!(Size, u64, range = 0..=10);
    crate::number!(Wide, u32, range = 1..=u32::MAX);
    crate::number!(Limit, u32, range = 1..=LIMIT);

    fn rejected<T, N>(result: Result<T, Invalid<N>>) -> NumberErrorKind
    where
        T: fmt::Debug,
        N: Newtype<Kind = NumberErrorKind>,
    {
        result.unwrap_err().into_kind()
    }

    #[test]
    fn takes_its_bounds_from_the_range() {
        check!((Capacity::MIN, Capacity::MAX) == (2, 6));
        check!((Offset::MIN, Offset::MAX) == (-3, 3));
        check!((Limit::MIN, Limit::MAX) == (1, LIMIT));
    }

    #[test]
    fn new_accepts_the_bounds_and_rejects_past_them() {
        check!(Capacity::new(2).unwrap().get() == 2);
        check!(Capacity::new(6).unwrap().get() == 6);
        check!(
            rejected(Capacity::new(1))
                == NumberErrorKind::OutOfRange {
                    received: 1,
                    min: 2,
                    max: 6
                }
        );
        check!(
            rejected(Capacity::new(7))
                == NumberErrorKind::OutOfRange {
                    received: 7,
                    min: 2,
                    max: 6
                }
        );
        check!(
            rejected(Below::new(0))
                == NumberErrorKind::OutOfRange {
                    received: 0,
                    min: -9,
                    max: -1
                }
        );
    }

    #[test]
    fn reads_an_i64_and_converts_back() {
        check!(i64::from(Offset::try_from(-3_i64).unwrap()) == -3);
        check!(i64::from(Big::try_from(i64::MAX).unwrap()) == i64::MAX);
        check!(i64::from(Wide::try_from(i64::from(u32::MAX)).unwrap()) == i64::from(u32::MAX));
    }

    #[test]
    fn try_from_rejects_an_i64_the_type_cant_hold() {
        check!(
            rejected(Capacity::try_from(-1_i64))
                == NumberErrorKind::OutOfRange {
                    received: -1,
                    min: 2,
                    max: 6
                }
        );
        check!(
            rejected(Size::try_from(i64::MIN))
                == NumberErrorKind::OutOfRange {
                    received: i64::MIN.into(),
                    min: 0,
                    max: 10
                }
        );
    }

    #[test]
    fn get_returns_the_number_as_its_type() {
        check!(Offset::new(-3).unwrap().get() == -3_i16);
        check!(Below::new(-1).unwrap().get() == -1_i16);
        check!(Big::new(i64::MAX).unwrap().get() == i64::MAX);
        check!(Size::new(10).unwrap().get() == 10_u64);
        check!(Wide::new(u32::MAX).unwrap().get() == u32::MAX);
        check!(Limit::new(LIMIT).unwrap().get() == LIMIT);
    }

    #[test]
    fn adds_and_subtracts_within_the_range() {
        let three = Capacity::new(3).unwrap();
        check!(three.try_add(3) == Capacity::new(6));
        check!(three.try_sub(1) == Capacity::new(2));
        check!(
            rejected(three.try_add(4))
                == NumberErrorKind::OutOfRange {
                    received: 7,
                    min: 2,
                    max: 6
                }
        );
        check!(
            rejected(three.try_sub(5))
                == NumberErrorKind::OutOfRange {
                    received: -2,
                    min: 2,
                    max: 6
                }
        );
        let top = Big::new(i64::MAX).unwrap();
        check!(
            rejected(top.try_add(i64::MAX))
                == NumberErrorKind::OutOfRange {
                    received: 2 * i128::from(i64::MAX),
                    min: 0,
                    max: i64::MAX
                }
        );
        check!(Offset::new(-3).unwrap().try_sub(-6) == Offset::new(3));
    }

    #[test]
    fn displays_as_the_number() {
        check!(Offset::new(-2).unwrap().to_string() == "-2");
    }

    #[test]
    fn error_names_its_type() {
        check!(
            Capacity::new(7).unwrap_err().to_string()
                == "Capacity must be between 2 and 6, received: 7"
        );
        check!(
            Size::new(u64::MAX).unwrap_err().to_string()
                == format!("Size must be between 0 and 10, received: {}", u64::MAX)
        );
    }
}

#[cfg(test)]
mod props {
    use super::NumberErrorKind;

    crate::number!(Capacity, u32, range = 2..=6);
    crate::number!(Offset, i16, range = -3..=3);
    crate::number!(Big, i64, range = 0..=i64::MAX);

    macro_rules! number_props {
        ($mod:ident, $number:ident, $t:ty) => {
            mod $mod {
                use proptest::prelude::*;

                use super::*;

                fn edge() -> impl Strategy<Value = $t> {
                    prop_oneof![
                        Just($number::MIN),
                        Just($number::MAX),
                        Just($number::MIN.saturating_sub(1)),
                        Just($number::MAX.saturating_add(1)),
                        any::<$t>(),
                    ]
                }

                fn signed() -> impl Strategy<Value = i64> {
                    prop_oneof![
                        Just(i64::from($number::MIN)),
                        Just(i64::from($number::MAX)),
                        Just(i64::from($number::MIN).saturating_sub(1)),
                        Just(i64::from($number::MAX).saturating_add(1)),
                        any::<i64>(),
                    ]
                }

                proptest! {
                    #[test]
                    fn new_accepts_exactly_the_range(n in edge()) {
                        match $number::new(n) {
                            Ok(number) => {
                                prop_assert!(($number::MIN..=$number::MAX).contains(&n));
                                prop_assert_eq!(number.get(), n);
                            }
                            Err(error) => {
                                prop_assert!(!($number::MIN..=$number::MAX).contains(&n));
                                prop_assert_eq!(
                                    error.into_kind(),
                                    NumberErrorKind::OutOfRange {
                                        received: n.into(),
                                        min: $number::MIN.into(),
                                        max: $number::MAX.into(),
                                    }
                                );
                            }
                        }
                    }

                    #[test]
                    fn round_trips_through_i64(n in signed()) {
                        let in_range =
                            (i64::from($number::MIN)..=i64::from($number::MAX)).contains(&n);
                        match $number::try_from(n) {
                            Ok(number) => prop_assert_eq!(i64::from(number), n),
                            Err(error) => {
                                prop_assert!(!in_range);
                                prop_assert_eq!(
                                    error.into_kind(),
                                    NumberErrorKind::OutOfRange {
                                        received: n.into(),
                                        min: $number::MIN.into(),
                                        max: $number::MAX.into(),
                                    }
                                );
                            }
                        }
                    }

                    #[test]
                    fn try_add_and_try_sub_match_the_wide_arithmetic(
                        n in $number::MIN..=$number::MAX,
                        by in edge(),
                    ) {
                        let number = $number::new(n).unwrap();
                        let range = i128::from($number::MIN)..=i128::from($number::MAX);
                        for (result, wide) in [
                            (number.try_add(by), i128::from(n) + i128::from(by)),
                            (number.try_sub(by), i128::from(n) - i128::from(by)),
                        ] {
                            match result {
                                Ok(next) => prop_assert_eq!(i128::from(next.get()), wide),
                                Err(error) => {
                                    prop_assert!(!range.contains(&wide));
                                    prop_assert_eq!(
                                        error.into_kind(),
                                        NumberErrorKind::OutOfRange {
                                            received: wide,
                                            min: $number::MIN.into(),
                                            max: $number::MAX.into(),
                                        }
                                    );
                                }
                            }
                        }
                    }

                    #[test]
                    fn orders_and_displays_as_the_number(
                        a in $number::MIN..=$number::MAX,
                        b in $number::MIN..=$number::MAX,
                    ) {
                        let (x, y) = ($number::new(a).unwrap(), $number::new(b).unwrap());
                        prop_assert_eq!(x.cmp(&y), a.cmp(&b));
                        prop_assert_eq!(x.to_string(), a.to_string());
                    }
                }
            }
        };
    }

    number_props!(unsigned, Capacity, u32);
    number_props!(negative, Offset, i16);
    number_props!(whole_i64, Big, i64);
}

/// Compiles only if the macro uses none of the caller's names.
#[cfg(test)]
#[expect(
    dead_code,
    unused_macros,
    non_camel_case_types,
    reason = "only here to shadow names"
)]
mod shadowed {
    struct i64;
    struct i128;
    struct str;
    struct Invalid;
    struct NumberErrorKind;
    trait Newtype {}
    type Result<T> = core::result::Result<T, ()>;
    macro_rules! assert {
        ($($t:tt)*) => {
            compile_error!("the macros must not use the caller's assert!")
        };
    }

    crate::number!(pub(crate) ShadowNumber, u32, range = 1..=3);
}
