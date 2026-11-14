//! Value types every uima service shares: validating newtype macros, PATCH-style updates, and the
//! port error.

mod port_error;

pub use port_error::{Failure, FailureKind, PortError, PortResult};
