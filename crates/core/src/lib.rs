//! Value types every uima service shares: validating newtype macros and the port error.
//!
//! # Newtypes
//!
//! [`uid!`], [`count!`], [`number!`], [`text!`] and [`secret!`] each define a newtype that holds
//! only checked values. A type's errors are [`Invalid<Name>`](Invalid), and the attributes before
//! its name, such as doc comments and derives, go on the struct. With serde, deserializing goes
//! through the checks with `try_from`, and serializing writes the inner value with `into`:
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
//! `uid!` types do the same with `"String"`, and `count!` and `number!` types with `"i64"`. A
//! `secret!` type derives only `Deserialize`, with `try_from = "String"`: serializing it would
//! write the secret out. Don't derive `Default`, or `Deserialize` without `try_from`: both make a
//! value the checks never saw.

mod count;
mod invalid;
mod number;
mod port_error;
mod text;
mod uid;

pub use count::CountErrorKind;
pub use invalid::{Invalid, Newtype};
pub use number::NumberErrorKind;
pub use port_error::{Failure, FailureKind, PortError, PortResult};
pub use text::{SecretErrorKind, TextErrorKind};
pub use uid::UidErrorKind;
pub use uuid::Uuid;

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};

    pub use crate::text::check as check_text;
}
