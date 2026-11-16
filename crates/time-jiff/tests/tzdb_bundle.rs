//! `tzdb-bundle-always` builds the tz database into the binary, for an image without
//! `/usr/share/zoneinfo`.
#![cfg(feature = "tzdb-bundle-always")]

use assert2::check;
use uima_time_jiff::jiff::tz::TimeZoneDatabase;

#[test]
fn the_bundled_database_has_the_zones() {
    check!(TimeZoneDatabase::bundled().get("Asia/Taipei").is_ok());
}
