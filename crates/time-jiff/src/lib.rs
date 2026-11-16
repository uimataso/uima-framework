//! Time for every app: one [`jiff`] version for all of them, and the pieces jiff leaves to each
//! app.
//!
//! A point in time is a [`jiff::Timestamp`] and a length of time is a [`std::time::Duration`].
//! A timestamp plus or minus a `Duration` is jiff's own; [`TimestampExt`] gives the `Duration`
//! between two timestamps.
//!
//! ```
//! use std::time::Duration;
//!
//! use uima_time_jiff::TimestampExt as _;
//! use uima_time_jiff::jiff::Timestamp;
//!
//! let joined: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
//! let now = joined + Duration::from_secs(90);
//! assert_eq!(now.saturating_duration_since(joined), Duration::from_secs(90));
//! assert_eq!(joined.checked_duration_since(now), None);
//! ```
//!
//! Features, forwarded to jiff: `serde`, which also adds [`serde`](mod@serde) here, and
//! `tzdb-bundle-always`, for an image without `/usr/share/zoneinfo`.

mod duration;
#[cfg(feature = "serde")]
pub mod serde;
mod timestamp;
mod zone;

pub use jiff;

pub use crate::duration::{InvalidDuration, parse_duration};
pub use crate::timestamp::TimestampExt;
pub use crate::zone::{InvalidTimeZone, NamedTimeZone};
