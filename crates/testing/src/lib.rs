//! Test helpers any crate can use: assertions, and [`run_case`] for a port's contract.
//!
//! ```
//! use uima_testing::{assert, check};
//!
//! let names = ["uima", "sowah"];
//! check!(names.len() == 2);
//! assert!(let [first, ..] = names);
//! check!(first == "uima");
//! ```

mod case;

pub use assert2::{assert, check};

pub use crate::case::{CaseOutcome, run_case};
