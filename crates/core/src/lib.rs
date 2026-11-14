//! Value types every uima service shares: validating newtype macros, PATCH-style updates, and the
//! port error.

mod port_error;
mod uid;

pub use port_error::{Failure, FailureKind, PortError, PortResult};

#[doc(hidden)]
pub mod __private {
    pub use uuid::{self, Uuid};
}
