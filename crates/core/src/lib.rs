//! Value types every uima service shares: validating newtype macros, PATCH-style updates, and the
//! port error.

mod count;
mod invalid;
mod number;
mod port_error;
mod text;
mod uid;

pub use count::CountError;
pub use invalid::{Invalid, Newtype};
pub use number::NumberError;
pub use port_error::{Failure, FailureKind, PortError, PortResult};
pub use text::{SecretError, TextError};

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};

    pub use crate::text::check as check_text;
}
