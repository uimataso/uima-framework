//! Value types every uima service shares: validating newtype macros, PATCH-style updates, and the
//! port error.

mod count;
mod invalid;
mod number;
mod port_error;
mod uid;

pub use count::CountError;
pub use invalid::{Invalid, Newtype};
pub use number::NumberError;
pub use port_error::{Failure, FailureKind, PortError, PortResult};

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};
}
