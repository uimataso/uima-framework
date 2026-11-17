//! Timestamps, lengths of time and a clock for every app, on jiff.
//!
//! # Examples
//!
//! ```
//! use std::time::Duration;
//!
//! use uima_time::TimestampExt as _;
//! use uima_time::jiff::Timestamp;
//!
//! let joined: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
//! let now = joined + Duration::from_secs(90);
//! assert_eq!(now.saturating_duration_since(joined), Duration::from_secs(90));
//! assert_eq!(joined.checked_duration_since(now), None);
//! ```
//!
//! # Timestamps and durations
//!
//! A point in time is a [`jiff::Timestamp`] and a length of time is a [`std::time::Duration`].
//! A timestamp plus or minus a `Duration` is jiff's own; [`TimestampExt`] gives the `Duration`
//! between two timestamps.
//!
//! # Features
//!
//! - `testing`: adds the `testing` module.
//! - `serde`: turns on jiff's, and adds the `serde` module.
//! - `tzdb-bundle-always`: turns on jiff's, for an image without `/usr/share/zoneinfo`.

mod clock;
mod duration;
#[cfg(feature = "serde")]
pub mod serde;
#[cfg(feature = "testing")]
pub mod testing;
mod timestamp;
mod zone;

pub use jiff;

pub use crate::clock::{Clock, HasClock, SystemClock};
pub use crate::duration::{InvalidDuration, parse_duration};
pub use crate::timestamp::TimestampExt;
pub use crate::zone::{InvalidTimeZone, NamedTimeZone};
