//! Time for every app, on jiff.
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
//! The app reads the current time from a [`Clock`] in its context, so a test can set it with a
//! `testing::SimulatedClock`.
//!
//! ```
//! use uima_time_jiff::jiff::Timestamp;
//! use uima_time_jiff::{Clock as _, HasClock, SystemClock};
//!
//! #[derive(uima_port_macro::HasPort)]
//! struct Ctx {
//!     #[has_port(Clock)]
//!     clock: SystemClock,
//! }
//!
//! fn stamp(ctx: &impl HasClock) -> Timestamp {
//!     ctx.clock().now()
//! }
//!
//! assert!(stamp(&Ctx { clock: SystemClock }) <= Timestamp::now());
//! ```
//!
//! Features: `testing` adds the `testing` module. `serde` and `tzdb-bundle-always` turn on jiff's;
//! `serde` also adds the `serde` module here, and `tzdb-bundle-always` is for an image without
//! `/usr/share/zoneinfo`.

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
