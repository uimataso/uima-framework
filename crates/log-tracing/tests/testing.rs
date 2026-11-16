//! `LogCapture` from outside the crate, as an app's tests use it.

#![cfg(feature = "testing")]

use tracing::Level;
use uima_log_tracing::testing::LogCapture;
use uima_testing::check;

#[test]
fn the_testing_feature_exposes_log_capture() {
    let logs = LogCapture::start();
    tracing::warn!("retrying later");

    check!(logs.contains(Level::WARN, "retrying"));
}
