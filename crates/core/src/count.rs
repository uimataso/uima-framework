//! [`count!`](crate::count), a newtype for a count that can't go negative.

use std::fmt;

/// Defines a newtype over `u64` for a count that can't go negative, such as a post's comments or
/// a user's failed logins.
///
/// ```
/// uima_core::count!(
///     /// A post's comments.
///     pub CommentCount
/// );
///
/// let comments = CommentCount::ZERO.increment();
/// assert_eq!(comments.get(), 1);
/// assert_eq!(comments.decrement().decrement(), CommentCount::ZERO);
/// ```
///
/// The type is `Copy`, ordered, hashable, and `Default` to zero. It has:
///
/// - `ZERO` and `MAX`.
/// - `new`, which rejects a value above `MAX`.
/// - `get` and `is_zero`.
/// - `remaining`, how many more fit before `MAX`.
/// - `increment`, `decrement`, `saturating_add` and `saturating_sub`, which stop at `MAX` and at
///   zero.
/// - `try_increment`, `try_decrement`, `try_add` and `try_sub`, which fail there instead.
/// - `TryFrom<i64>`, `From<Name> for i64`, and `Display`.
///
/// Its errors are [`Invalid<Name>`](crate::Invalid), whose kind is a [`CountErrorKind`].
///
/// # Max
///
/// `max = N` caps the count at `N`; without it, the cap is `i64::MAX`:
///
/// ```
/// use uima_core::CountErrorKind;
///
/// uima_core::count!(pub Retries, max = 3);
///
/// let retries = Retries::new(3).unwrap();
/// let error = retries.try_increment().unwrap_err();
/// assert_eq!(error.kind(), &CountErrorKind::AboveMax { received: 4, max: 3 });
/// assert_eq!(error.to_string(), "Retries can't exceed 3, received: 4");
/// ```
///
/// Every count fits an `i64`, so a `max` above `i64::MAX` doesn't compile:
///
/// ```compile_fail
/// uima_core::count!(Huge, max = u64::MAX); // error: Huge's max must fit an i64
/// ```
#[macro_export]
macro_rules! count {
    ($(#[$meta:meta])* $vis:vis $name:ident $(,)?) => {
        $crate::count!(
            @define $(#[$meta])* $vis $name,
            ::core::primitive::i64::MAX as ::core::primitive::u64
        );
    };

    ($(#[$meta:meta])* $vis:vis $name:ident, max = $max:expr $(,)?) => {
        $crate::count!(@define $(#[$meta])* $vis $name, $max);
    };

    (@define $(#[$meta:meta])* $vis:vis $name:ident, $max:expr) => {
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
            ::core::default::Default,
        )]
        $vis struct $name(::core::primitive::u64);

        impl $name {
            /// A count of zero.
            pub const ZERO: Self = Self(0);
            /// The highest count: the `max` given, else `i64::MAX`.
            pub const MAX: ::core::primitive::u64 = $max;

            /// `count`, or `AboveMax` above `MAX`.
            pub const fn new(count: ::core::primitive::u64) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                if count > Self::MAX {
                    return ::core::result::Result::Err($crate::Invalid::new($crate::CountErrorKind::AboveMax {
                        received: count as ::core::primitive::u128,
                        max: Self::MAX,
                    }));
                }
                ::core::result::Result::Ok(Self(count))
            }

            /// The count as a `u64`.
            pub const fn get(self) -> ::core::primitive::u64 {
                self.0
            }

            /// Whether the count is zero.
            pub const fn is_zero(self) -> ::core::primitive::bool {
                self.0 == 0
            }

            /// One more, or `AboveMax` at `MAX`.
            pub const fn try_increment(self) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                self.try_add(1)
            }

            /// One fewer, or `Negative` at zero.
            pub const fn try_decrement(self) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                self.try_sub(1)
            }

            /// `n` more, or `AboveMax` past `MAX`.
            pub const fn try_add(
                self,
                n: ::core::primitive::u64,
            ) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                let sum = self.0 as ::core::primitive::u128 + n as ::core::primitive::u128;
                if sum > Self::MAX as ::core::primitive::u128 {
                    return ::core::result::Result::Err($crate::Invalid::new($crate::CountErrorKind::AboveMax {
                        received: sum,
                        max: Self::MAX,
                    }));
                }
                ::core::result::Result::Ok(Self(sum as ::core::primitive::u64))
            }

            /// `n` fewer, or `Negative` below zero.
            pub const fn try_sub(
                self,
                n: ::core::primitive::u64,
            ) -> ::core::result::Result<Self, $crate::Invalid<Self>> {
                if n > self.0 {
                    return ::core::result::Result::Err($crate::Invalid::new($crate::CountErrorKind::Negative {
                        received: self.0 as ::core::primitive::i128 - n as ::core::primitive::i128,
                    }));
                }
                ::core::result::Result::Ok(Self(self.0 - n))
            }

            /// One more, stopping at `MAX`.
            #[must_use]
            pub const fn increment(self) -> Self {
                self.saturating_add(1)
            }

            /// One fewer, stopping at zero.
            #[must_use]
            pub const fn decrement(self) -> Self {
                self.saturating_sub(1)
            }

            /// `n` more, stopping at `MAX`.
            #[must_use]
            pub const fn saturating_add(self, n: ::core::primitive::u64) -> Self {
                if n >= Self::MAX - self.0 {
                    return Self(Self::MAX);
                }
                Self(self.0 + n)
            }

            /// `n` fewer, stopping at zero.
            #[must_use]
            pub const fn saturating_sub(self, n: ::core::primitive::u64) -> Self {
                Self(self.0.saturating_sub(n))
            }

            /// How many more until `MAX`.
            pub const fn remaining(self) -> ::core::primitive::u64 {
                Self::MAX - self.0
            }
        }

        const _: () = ::core::assert!(
            $name::MAX <= ::core::primitive::i64::MAX as ::core::primitive::u64,
            ::core::concat!(::core::stringify!($name), "'s max must fit an i64"),
        );

        impl ::core::convert::TryFrom<::core::primitive::i64> for $name {
            type Error = $crate::Invalid<Self>;

            fn try_from(count: ::core::primitive::i64) -> ::core::result::Result<Self, Self::Error> {
                match <::core::primitive::u64 as ::core::convert::TryFrom<_>>::try_from(count) {
                    ::core::result::Result::Ok(count) => Self::new(count),
                    ::core::result::Result::Err(_) => {
                        ::core::result::Result::Err($crate::Invalid::new($crate::CountErrorKind::Negative {
                            received: count as ::core::primitive::i128,
                        }))
                    }
                }
            }
        }

        impl ::core::convert::From<$name> for ::core::primitive::i64 {
            fn from(count: $name) -> Self {
                // `MAX` fits an i64, as asserted above.
                count.0 as ::core::primitive::i64
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl $crate::Newtype for $name {
            const NAME: &'static ::core::primitive::str = ::core::stringify!($name);
            type Kind = $crate::CountErrorKind;
        }
    };
}

/// What is wrong with a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountErrorKind {
    /// Below zero, read back from a negative number or with more taken away than there is.
    Negative { received: i128 },
    /// Above the max, given a larger number or with more added than fits.
    AboveMax { received: u128, max: u64 },
}

impl fmt::Display for CountErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Negative { received } => write!(f, "can't be negative, received: {received}"),
            Self::AboveMax { received, max } => {
                write!(f, "can't exceed {max}, received: {received}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;
    use crate::{Invalid, Newtype};

    crate::count!(TestCount);
    crate::count!(TestCapped, max = 3);

    fn count(n: u64) -> TestCount {
        TestCount::new(n).unwrap()
    }

    fn capped(n: u64) -> TestCapped {
        TestCapped::new(n).unwrap()
    }

    fn rejected<T, N>(result: Result<T, Invalid<N>>) -> CountErrorKind
    where
        T: fmt::Debug,
        N: Newtype<Kind = CountErrorKind>,
    {
        result.unwrap_err().into_kind()
    }

    #[test]
    fn starts_at_zero() {
        check!(TestCount::ZERO.get() == 0);
        check!(TestCount::default() == TestCount::ZERO);
        check!(TestCount::ZERO.is_zero());
        check!(!count(1).is_zero());
        check!(TestCapped::ZERO.get() == 0);
        check!(TestCapped::ZERO.is_zero());
        check!(!capped(1).is_zero());
    }

    #[test]
    fn max_is_i64_max_unless_given() {
        check!(TestCount::MAX == i64::MAX as u64);
        check!(TestCapped::MAX == 3);
    }

    #[test]
    fn new_rejects_a_value_above_max() {
        check!(TestCount::new(TestCount::MAX).is_ok());
        check!(
            rejected(TestCount::new(TestCount::MAX + 1))
                == CountErrorKind::AboveMax {
                    received: u128::from(TestCount::MAX) + 1,
                    max: TestCount::MAX,
                }
        );
        check!(TestCapped::new(3) == Ok(capped(3)));
        check!(
            rejected(TestCapped::new(4))
                == CountErrorKind::AboveMax {
                    received: 4,
                    max: 3
                }
        );
    }

    #[test]
    fn increments_and_decrements() {
        check!(count(4).increment() == count(5));
        check!(count(4).decrement() == count(3));
        check!(capped(2).decrement() == capped(1));
    }

    #[test]
    fn increment_stops_at_max() {
        check!(count(TestCount::MAX).increment() == count(TestCount::MAX));
        check!(capped(2).increment() == capped(3));
        check!(capped(3).increment() == capped(3));
    }

    #[test]
    fn decrement_stops_at_zero() {
        check!(TestCount::ZERO.decrement() == TestCount::ZERO);
        check!(TestCapped::ZERO.decrement() == TestCapped::ZERO);
    }

    #[test]
    fn try_increment_refuses_to_pass_max() {
        check!(count(4).try_increment() == Ok(count(5)));
        check!(
            rejected(count(TestCount::MAX).try_increment())
                == CountErrorKind::AboveMax {
                    received: u128::from(TestCount::MAX) + 1,
                    max: TestCount::MAX,
                }
        );
        check!(capped(2).try_increment() == Ok(capped(3)));
        check!(
            rejected(capped(3).try_increment())
                == CountErrorKind::AboveMax {
                    received: 4,
                    max: 3
                }
        );
    }

    #[test]
    fn try_decrement_refuses_to_go_below_zero() {
        check!(count(4).try_decrement() == Ok(count(3)));
        check!(count(1).try_decrement() == Ok(TestCount::ZERO));
        check!(
            rejected(TestCount::ZERO.try_decrement()) == CountErrorKind::Negative { received: -1 }
        );
        check!(
            rejected(TestCapped::ZERO.try_decrement()) == CountErrorKind::Negative { received: -1 }
        );
    }

    #[test]
    fn adds_and_subtracts() {
        check!(count(4).saturating_add(3) == count(7));
        check!(count(4).saturating_sub(3) == count(1));
        check!(count(4).try_add(3) == Ok(count(7)));
        check!(count(4).try_sub(4) == Ok(TestCount::ZERO));
        check!(capped(1).saturating_add(2) == capped(3));
        check!(capped(1).try_add(2) == Ok(capped(3)));
        check!(capped(3).try_sub(2) == Ok(capped(1)));
    }

    #[test]
    fn add_and_sub_stop_at_the_ends() {
        check!(capped(1).saturating_add(5) == capped(3));
        check!(count(TestCount::MAX).saturating_add(u64::MAX) == count(TestCount::MAX));
        check!(count(4).saturating_sub(5) == TestCount::ZERO);
        check!(count(4).saturating_sub(u64::MAX) == TestCount::ZERO);
    }

    #[test]
    fn try_add_and_try_sub_refuse_to_pass_the_ends() {
        check!(
            rejected(capped(1).try_add(5))
                == CountErrorKind::AboveMax {
                    received: 6,
                    max: 3
                }
        );
        check!(
            rejected(count(TestCount::MAX).try_add(u64::MAX))
                == CountErrorKind::AboveMax {
                    received: u128::from(TestCount::MAX) + u128::from(u64::MAX),
                    max: TestCount::MAX,
                }
        );
        check!(rejected(count(4).try_sub(5)) == CountErrorKind::Negative { received: -1 });
        check!(
            rejected(TestCount::ZERO.try_sub(u64::MAX))
                == CountErrorKind::Negative {
                    received: -i128::from(u64::MAX)
                }
        );
    }

    #[test]
    fn knows_what_is_left() {
        check!(count(5).remaining() == TestCount::MAX - 5);
        check!(TestCapped::ZERO.remaining() == 3);
        check!(capped(1).remaining() == 2);
        check!(capped(3).remaining() == 0);
    }

    #[test]
    fn reads_a_non_negative_i64() {
        check!(TestCount::try_from(0_i64) == Ok(TestCount::ZERO));
        check!(TestCount::try_from(i64::MAX) == Ok(count(i64::MAX as u64)));
        check!(rejected(TestCount::try_from(-1_i64)) == CountErrorKind::Negative { received: -1 });
        check!(
            rejected(TestCapped::try_from(4_i64))
                == CountErrorKind::AboveMax {
                    received: 4,
                    max: 3
                }
        );
    }

    #[test]
    fn converts_into_an_i64() {
        check!(i64::from(count(42)) == 42);
        check!(i64::from(count(TestCount::MAX)) == i64::MAX);
    }

    #[test]
    fn displays_as_the_number() {
        check!(count(42).to_string() == "42");
    }

    #[test]
    fn error_names_its_type() {
        let negative = TestCount::try_from(-1_i64).unwrap_err();
        check!(negative.to_string() == "TestCount can't be negative, received: -1");
        let above = TestCapped::new(4).unwrap_err();
        check!(above.to_string() == "TestCapped can't exceed 3, received: 4");
    }
}

#[cfg(test)]
mod props {
    use proptest::prelude::*;

    use super::CountErrorKind;

    crate::count!(TestCount);
    crate::count!(TestCapped, max = 3);

    /// Any `u64`, often at the edges: zero, `max`, and just past it.
    fn value(max: u64) -> impl Strategy<Value = u64> {
        prop_oneof![
            Just(0),
            Just(max),
            Just(max.saturating_add(1)),
            0..=max,
            any::<u64>(),
        ]
    }

    /// Any `i64`, often at the edges: zero, -1, `max`, and just past it.
    fn signed(max: u64) -> impl Strategy<Value = i64> {
        let max = max as i64;
        prop_oneof![
            Just(0),
            Just(-1),
            Just(max),
            Just(max.saturating_add(1)),
            any::<i64>()
        ]
    }

    macro_rules! count_props {
        ($mod:ident, $count:ident) => {
            mod $mod {
                use proptest::prelude::*;

                use super::*;

                proptest! {
                    #[test]
                    fn new_accepts_exactly_up_to_max(n in value($count::MAX)) {
                        match $count::new(n) {
                            Ok(count) => {
                                prop_assert!(n <= $count::MAX);
                                prop_assert_eq!(count.get(), n);
                            }
                            Err(error) => prop_assert_eq!(
                                error.into_kind(),
                                CountErrorKind::AboveMax { received: n.into(), max: $count::MAX }
                            ),
                        }
                    }

                    #[test]
                    fn round_trips_through_i64(n in signed($count::MAX)) {
                        match $count::try_from(n) {
                            Ok(count) => prop_assert_eq!(i64::from(count), n),
                            Err(error) if n < 0 => prop_assert_eq!(
                                error.into_kind(),
                                CountErrorKind::Negative { received: n.into() }
                            ),
                            Err(error) => prop_assert_eq!(
                                error.into_kind(),
                                CountErrorKind::AboveMax { received: n as u128, max: $count::MAX }
                            ),
                        }
                    }

                    #[test]
                    fn increment_adds_one_up_to_max(n in 0..=$count::MAX) {
                        let count = $count::new(n).unwrap();
                        prop_assert_eq!(count.increment().get(), (n + 1).min($count::MAX));
                        match count.try_increment() {
                            Ok(next) => prop_assert_eq!(next.get(), n + 1),
                            Err(error) => {
                                prop_assert_eq!(n, $count::MAX);
                                prop_assert_eq!(
                                    error.into_kind(),
                                    CountErrorKind::AboveMax {
                                        received: u128::from($count::MAX) + 1,
                                        max: $count::MAX,
                                    }
                                );
                            }
                        }
                    }

                    #[test]
                    fn decrement_takes_one_down_to_zero(n in 0..=$count::MAX) {
                        let count = $count::new(n).unwrap();
                        prop_assert_eq!(count.decrement().get(), n.saturating_sub(1));
                        match count.try_decrement() {
                            Ok(previous) => prop_assert_eq!(previous.get(), n - 1),
                            Err(error) => {
                                prop_assert_eq!(n, 0);
                                prop_assert_eq!(error.into_kind(), CountErrorKind::Negative { received: -1 });
                            }
                        }
                    }

                    #[test]
                    fn add_and_sub_match_the_wide_arithmetic(
                        n in 0..=$count::MAX,
                        by in value($count::MAX),
                    ) {
                        let count = $count::new(n).unwrap();
                        let sum = u128::from(n) + u128::from(by);
                        let difference = i128::from(n) - i128::from(by);
                        let max = u128::from($count::MAX);
                        prop_assert_eq!(u128::from(count.saturating_add(by).get()), sum.min(max));
                        prop_assert_eq!(i128::from(count.saturating_sub(by).get()), difference.max(0));
                        match count.try_add(by) {
                            Ok(next) => prop_assert_eq!(u128::from(next.get()), sum),
                            Err(error) => prop_assert_eq!(
                                error.into_kind(),
                                CountErrorKind::AboveMax { received: sum, max: $count::MAX }
                            ),
                        }
                        prop_assert_eq!(count.try_add(by).is_ok(), sum <= max);
                        match count.try_sub(by) {
                            Ok(previous) => prop_assert_eq!(i128::from(previous.get()), difference),
                            Err(error) => prop_assert_eq!(
                                error.into_kind(),
                                CountErrorKind::Negative { received: difference }
                            ),
                        }
                        prop_assert_eq!(count.try_sub(by).is_ok(), difference >= 0);
                    }

                    #[test]
                    fn remaining_and_get_add_up_to_max(n in 0..=$count::MAX) {
                        let count = $count::new(n).unwrap();
                        prop_assert_eq!(count.get() + count.remaining(), $count::MAX);
                        prop_assert_eq!(count.is_zero(), n == 0);
                        prop_assert_eq!(count == $count::ZERO, n == 0);
                    }

                    #[test]
                    fn orders_and_displays_as_the_number(
                        a in 0..=$count::MAX,
                        b in 0..=$count::MAX,
                    ) {
                        let (x, y) = ($count::new(a).unwrap(), $count::new(b).unwrap());
                        prop_assert_eq!(x.cmp(&y), a.cmp(&b));
                        prop_assert_eq!(x.to_string(), a.to_string());
                    }
                }
            }
        };
    }

    count_props!(uncapped, TestCount);
    count_props!(capped, TestCapped);
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
    struct u64;
    struct i64;
    struct bool;
    struct Option;
    struct Invalid;
    struct CountErrorKind;
    trait Newtype {}
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

    crate::count!(pub(crate) ShadowCount);
    crate::count!(pub(crate) ShadowCapped, max = 3);
}
