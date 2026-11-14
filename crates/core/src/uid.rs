//! [`uid!`](crate::uid), a newtype for an entity's uid.

/// Defines a `Copy` newtype over [`uuid::Uuid`] for one entity's uid.
///
/// ```
/// uima_core::uid!(
///     /// A post.
///     pub PostUid
/// );
///
/// let post = PostUid::generate();
/// let parsed: PostUid = post.to_string().parse().unwrap();
/// assert_eq!(parsed, post);
/// ```
///
/// Two uid types don't mix, even though both wrap a `Uuid`:
///
/// ```compile_fail
/// uima_core::uid!(UserUid);
/// uima_core::uid!(PostUid);
///
/// fn author(_: UserUid) {}
/// author(PostUid::generate()); // error: expected `UserUid`, found `PostUid`
/// ```
///
/// The type converts from and into `Uuid`, displays and parses as the hyphenated uuid, and has no
/// `Default`: a uid is either generated or read back. Attributes, such as doc comments or serde
/// derives, go on the struct.
#[macro_export]
macro_rules! uid {
    ($(#[$meta:meta])* $vis:vis $name:ident $(,)?) => {
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
        $vis struct $name($crate::__private::Uuid);

        impl $name {
            /// A new uid, ordered by when it was generated (UUID v7).
            pub fn generate() -> Self {
                Self($crate::__private::Uuid::now_v7())
            }

            /// The uid `uuid`, usable in a `const`.
            pub const fn from_uuid(uuid: $crate::__private::Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(self) -> $crate::__private::Uuid {
                self.0
            }
        }

        impl ::core::convert::From<$crate::__private::Uuid> for $name {
            fn from(uuid: $crate::__private::Uuid) -> Self {
                Self(uuid)
            }
        }

        impl ::core::convert::From<$name> for $crate::__private::Uuid {
            fn from(uid: $name) -> Self {
                uid.0
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = $crate::__private::uuid::Error;

            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                ::core::result::Result::Ok(Self(s.parse()?))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use uuid::Uuid;

    crate::uid!(TestUid);

    #[test]
    fn round_trips_through_uuid() {
        let raw = Uuid::from_u128(7);
        let uid = TestUid::from(raw);
        check!(uid.as_uuid() == raw);
        check!(Uuid::from(uid) == raw);
    }

    #[test]
    fn from_uuid_works_in_a_const() {
        const RAW: Uuid = Uuid::from_u128(7);
        const UID: TestUid = TestUid::from_uuid(RAW);
        check!(UID.as_uuid() == RAW);
    }

    #[test]
    fn generates_distinct_v7_uids_in_order() {
        let first = TestUid::generate();
        let second = TestUid::generate();
        check!(first.as_uuid().get_version_num() == 7);
        check!(first < second);
    }

    #[test]
    fn displays_and_parses_as_the_hyphenated_uuid() {
        let raw = Uuid::from_u128(7);
        let text = "00000000-0000-0000-0000-000000000007";
        check!(TestUid::from(raw).to_string() == text);
        check!(text.parse::<TestUid>().ok() == Some(TestUid::from(raw)));
    }

    #[test]
    fn accepts_a_v4_uuid() {
        let text = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        let raw: Uuid = text.parse().unwrap();
        check!(raw.get_version_num() == 4);
        let uid = TestUid::from(raw);
        check!(uid.as_uuid() == raw);
        check!(text.parse::<TestUid>().ok() == Some(uid));
        check!(uid.to_string() == text);
    }

    #[test]
    fn rejects_text_that_is_not_a_uuid() {
        check!("not a uuid".parse::<TestUid>().is_err());
    }
}

/// Compiles only if the macro uses none of the caller's names.
#[cfg(test)]
#[allow(dead_code, unused_macros, reason = "only here to shadow names")]
mod shadowed {
    struct Uuid;
    type Result<T> = core::result::Result<T, ()>;
    mod uuid {}
    macro_rules! write {
        ($($t:tt)*) => {
            compile_error!("the macros must not use the caller's write!")
        };
    }

    crate::uid!(pub(crate) ShadowUid);
}
