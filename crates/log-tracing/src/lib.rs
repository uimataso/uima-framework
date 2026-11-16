//! Installs the global `tracing` subscriber for a service.
//!
//! # Examples
//!
//! ```no_run
//! uima_log_tracing::init("info");
//! ```
//!
//! # Filter
//!
//! The filter comes from `RUST_LOG` (see the [directive syntax]). When it is unset, empty or
//! invalid, the default filter given here is used; an invalid one is warned about once the
//! subscriber is up.
//!
//! # Colors
//!
//! Logs go to stdout, colored when stdout is a terminal. The environment can override that:
//!
//! - `NO_COLOR` set: off.
//! - `FORCE_COLOR` set: on, even with `NO_COLOR` set.
//!
//! A variable counts as set when it isn't empty, whatever its value, so `FORCE_COLOR=0` turns
//! colors on too. `CLICOLOR` and `CLICOLOR_FORCE` are ignored.
//!
//! # Features
//!
//! - `testing`: adds the `testing` module, with `LogCapture` to check what a test logged.
//!
//! [directive syntax]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives

use std::io::IsTerminal as _;

use tracing::Subscriber;
use tracing_subscriber::filter::ParseError;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer, Registry};

#[cfg(feature = "testing")]
pub mod testing;

/// Installs the subscriber, with `default_filter` for when `RUST_LOG` is unset or invalid.
///
/// Same as `Builder::new(default_filter).init()`.
///
/// # Panics
///
/// If `default_filter` isn't a valid filter, or a global subscriber is already installed.
pub fn init(default_filter: impl Into<String>) {
    Builder::new(default_filter).init();
}

/// Builds the global subscriber, for when [`init`] isn't enough.
#[must_use = "call `.init()` to install the subscriber"]
pub struct Builder {
    default_filter: String,
    layers: Vec<BoxedLayer>,
}

impl Builder {
    pub fn new(default_filter: impl Into<String>) -> Self {
        Self {
            default_filter: default_filter.into(),
            layers: Vec::new(),
        }
    }

    /// Adds a layer next to the stdout one, behind the same filter.
    pub fn with_layer(mut self, layer: impl Layer<Registry> + Send + Sync + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Installs the subscriber as the global default.
    ///
    /// # Panics
    ///
    /// If the default filter isn't valid, or a global subscriber is already installed.
    pub fn init(self) {
        let (filter, invalid) = env_filter(
            std::env::var("RUST_LOG").ok().as_deref(),
            &self.default_filter,
        );

        let ansi = ColorChoice::from_env(|name| std::env::var(name).ok())
            .use_color(std::io::stdout().is_terminal());

        subscriber(filter, self.layers, std::io::stdout, ansi).init();

        // Only now is there a subscriber to report it to.
        if let Some(error) = invalid {
            tracing::warn!(%error, default = self.default_filter, "invalid RUST_LOG, using the default filter");
        }
    }
}

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// The subscriber [`Builder::init`] installs, with the stdout writer as a parameter so tests can
/// read what it prints.
fn subscriber<W>(
    filter: EnvFilter,
    layers: Vec<BoxedLayer>,
    writer: W,
    ansi: bool,
) -> impl Subscriber + Send + Sync + 'static
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_line_number(true)
        .with_ansi(ansi)
        .with_writer(writer);

    tracing_subscriber::registry()
        // An empty `Vec` layer hints `LevelFilter::OFF` and so silences every event; `None`
        // hints nothing.
        .with((!layers.is_empty()).then_some(layers))
        .with(fmt_layer)
        .with(filter)
}

/// Returns `rust_log` as the filter, or `default` when `rust_log` is unset, blank or invalid.
/// An invalid `rust_log` also returns its error, for the caller to log once the subscriber is up.
///
/// # Panics
///
/// If `default` doesn't parse, even when `rust_log` is used.
fn env_filter(rust_log: Option<&str>, default: &str) -> (EnvFilter, Option<ParseError>) {
    let default = EnvFilter::try_new(default)
        .unwrap_or_else(|error| panic!("invalid default filter {default:?}: {error}"));

    match rust_log
        .filter(|s| !s.trim().is_empty())
        .map(EnvFilter::try_new)
    {
        Some(Ok(filter)) => (filter, None),
        Some(Err(error)) => (default, Some(error)),
        None => (default, None),
    }
}

/// What the environment asks for colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorChoice {
    Never,
    Always,
    /// Color only on a terminal.
    Auto,
}

impl ColorChoice {
    /// Reads the color variables through `var`; see the crate docs.
    fn from_env(var: impl Fn(&str) -> Option<String>) -> Self {
        let is_set = |name| var(name).is_some_and(|value| !value.is_empty());
        if is_set("FORCE_COLOR") {
            Self::Always
        } else if is_set("NO_COLOR") {
            Self::Never
        } else {
            Self::Auto
        }
    }

    fn use_color(self, is_terminal: bool) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Auto => is_terminal,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use uima_testing::check;

    use super::*;

    // Installs the global subscriber, so it relies on nextest running each test in its own
    // process.
    #[test]
    fn init_installs_the_global_subscriber() {
        init("info");
        check!(tracing::dispatcher::has_been_set());
    }

    /// A writer that keeps what was written, for reading back after logging.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Captured {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }

        fn layer(&self) -> BoxedLayer {
            let writer = self.clone();
            Box::new(tracing_subscriber::fmt::layer().with_writer(move || writer.clone()))
        }
    }

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Logs one event at info and one at debug through `subscriber`.
    fn log_two_events(subscriber: impl Subscriber + Send + Sync + 'static) {
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("shown");
            tracing::debug!("filtered");
        });
    }

    #[test]
    #[should_panic(expected = "invalid default filter")]
    fn invalid_default_panics_even_with_a_valid_rust_log() {
        env_filter(Some("info"), "info,=[");
    }

    fn color_choice(vars: &[(&str, &str)]) -> ColorChoice {
        ColorChoice::from_env(|name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn color_follows_the_terminal_by_default() {
        check!(color_choice(&[]) == ColorChoice::Auto);
        check!(ColorChoice::Auto.use_color(true));
        check!(!ColorChoice::Auto.use_color(false));
    }

    #[test]
    fn no_color_turns_colors_off_whatever_its_value() {
        for value in ["1", "0", "false"] {
            check!(color_choice(&[("NO_COLOR", value)]) == ColorChoice::Never);
        }
        check!(!ColorChoice::Never.use_color(true));
    }

    #[test]
    fn force_color_turns_colors_on_whatever_its_value() {
        for value in ["1", "0", "false"] {
            check!(color_choice(&[("FORCE_COLOR", value)]) == ColorChoice::Always);
        }
        check!(ColorChoice::Always.use_color(false));
    }

    #[test]
    fn force_color_wins_over_no_color() {
        check!(color_choice(&[("NO_COLOR", "1"), ("FORCE_COLOR", "1")]) == ColorChoice::Always);
    }

    #[test]
    fn empty_counts_as_unset() {
        for name in ["NO_COLOR", "FORCE_COLOR"] {
            check!(color_choice(&[(name, "")]) == ColorChoice::Auto);
        }
    }

    #[test]
    fn clicolor_variables_are_ignored() {
        check!(color_choice(&[("CLICOLOR_FORCE", "1"), ("CLICOLOR", "0")]) == ColorChoice::Auto);
    }

    #[test]
    fn logs_without_extra_layers() {
        let stdout = Captured::default();
        let writer = stdout.clone();
        log_two_events(subscriber(
            EnvFilter::new("info"),
            Vec::new(),
            move || writer.clone(),
            false,
        ));

        check!(stdout.text().contains("shown"));
        check!(!stdout.text().contains("filtered"));
    }

    #[test]
    fn every_extra_layer_gets_the_filtered_events() {
        let (stdout, first, second) = (
            Captured::default(),
            Captured::default(),
            Captured::default(),
        );
        let writer = stdout.clone();
        let layers = vec![first.layer(), second.layer()];
        log_two_events(subscriber(
            EnvFilter::new("info"),
            layers,
            move || writer.clone(),
            false,
        ));

        for captured in [stdout, first, second] {
            check!(captured.text().contains("shown"));
            check!(!captured.text().contains("filtered"));
        }
    }

    #[test]
    fn rust_log_wins_when_valid() {
        let (filter, invalid) = env_filter(Some("warn"), "info");
        check!(filter.to_string() == "warn");
        check!(invalid.is_none());
    }

    #[test]
    fn unset_rust_log_uses_the_default_quietly() {
        let (filter, invalid) = env_filter(None, "info");
        check!(filter.to_string() == "info");
        check!(invalid.is_none());
    }

    #[test]
    fn blank_rust_log_counts_as_unset() {
        let (filter, invalid) = env_filter(Some(" "), "info");
        check!(filter.to_string() == "info");
        check!(invalid.is_none());
    }

    #[test]
    fn invalid_rust_log_uses_the_default_and_reports_it() {
        let (filter, invalid) = env_filter(Some("info,=["), "info");
        check!(filter.to_string() == "info");
        check!(invalid.is_some());
    }
}
