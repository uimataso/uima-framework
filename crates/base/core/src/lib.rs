//! Types and ports my services share.
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
//! Don't derive `Default`, or `Deserialize` without `try_from`, since both make a value the
//! checks never saw.
//!
//! # Lists
//!
//! A [`PageRule`] turns a request's limit and offset into an [`OffsetQuery`]; a read returns an
//! [`OffsetPage`] when it tells whether more rows follow, or a [`CountedPage`] when it counts them
//! all. A [`Sort`] pairs what a list sorts by with a [`SortOrder`].
//!
//! # Features
//!
//! - `testing`: adds `unit_of_work::contract`, the cases a unit of work passes.

mod count;
mod invalid;
mod number;
mod page;
mod port_error;
mod secret;
mod sort;
mod text;
mod uid;
pub mod unit_of_work;
mod update;

pub use count::CountErrorKind;
pub use invalid::{Invalid, Newtype};
pub use number::NumberErrorKind;
pub use page::{CountedPage, InvalidPage, OffsetPage, OffsetQuery, PageRule};
pub use port_error::{Failure, FailureContext, FailureKind, PortError, PortResult};
pub use secret::Secret;
pub use sort::{Sort, SortOrder};
pub use text::{SecretErrorKind, TextErrorKind};
pub use uid::UidErrorKind;
pub use unit_of_work::{HasUnitOfWork, ScopedUow, UnitOfWork};
pub use update::{Changed, Update, UpdateOption};
pub use uuid::Uuid;

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};

    pub use crate::text::check as check_text;
}
