//! Value types my services share.
//!
//! # Examples
//!
//! A validating newtype, read and written with serde:
//!
//! ```
//! use serde::{Deserialize, Serialize};
//!
//! uima_core::text!(
//!     /// A group's name.
//!     #[derive(Serialize, Deserialize)]
//!     #[serde(try_from = "String", into = "String")]
//!     pub GroupName, len = 1..=40, chars = |c| !c.is_control()
//! );
//!
//! let name: GroupName = serde_json::from_str(r#"" Friends ""#).unwrap();
//! assert_eq!(serde_json::to_string(&name).unwrap(), r#""Friends""#);
//! assert!(serde_json::from_str::<GroupName>(r#""""#).is_err());
//! ```
//!
//! # Newtypes
//!
//! [`uid!`], [`count!`], [`number!`], [`text!`] and [`secret!`] each define a newtype that holds
//! only checked values, with [`Invalid<Name>`](Invalid) as its error. Each macro's doc says what
//! the type implements. With serde, use `try_from` and `into` with the type the macro
//! converts from, as in the example.
//!
//! Don't derive `Default`, or `Deserialize` without `try_from`: both make a value the checks
//! never saw.
//!
//! # Lists
//!
//! A [`PageRule`] turns a request's limit and offset into an [`OffsetQuery`]; a read returns an
//! [`OffsetPage`] when it tells whether more rows follow, or a [`CountedPage`] when it counts them
//! all. A [`Sort`] pairs what a list sorts by with a [`SortOrder`].

mod count;
mod invalid;
mod number;
mod page;
mod port_error;
mod sort;
mod text;
mod uid;
mod update;

pub use count::CountErrorKind;
pub use invalid::{Invalid, Newtype};
pub use number::NumberErrorKind;
pub use page::{CountedPage, OffsetPage, OffsetQuery, PageErrorKind, PageRule};
pub use port_error::{Failure, FailureKind, PortError, PortResult};
pub use sort::{Sort, SortOrder};
pub use text::{SecretErrorKind, TextErrorKind};
pub use uid::UidErrorKind;
pub use update::{Changed, Update, UpdateOption};
pub use uuid::Uuid;

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};

    pub use crate::text::check as check_text;
}
