//! Test helpers.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::DefaultGuard;
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// Records every event logged on this thread from [`start`](Self::start) until it drops.
///
/// Only events on this thread are recorded. A `#[tokio::test]` runs its tasks here, but
/// `spawn_blocking` and a multi-thread runtime may run them elsewhere, so a check that something
/// isn't logged covers only this thread. Spans created before `start` aren't recorded, so
/// [`mentions`](Self::mentions) doesn't see their fields. Drop a capture started inside another
/// before the outer one, or the inner one stops recording.
///
/// ```
/// use tracing::Level;
/// use uima_log_tracing_subscriber::testing::LogCapture;
///
/// let logs = LogCapture::start();
/// tracing::warn!(user_uid = "u-1", "retrying later");
///
/// assert!(logs.has_message(Level::WARN, "retrying"));
/// assert_eq!(logs.events()[0].fields["user_uid"], "u-1");
/// assert!(!logs.mentions("secret"));
/// ```
pub struct LogCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    _guard: DefaultGuard,
}

impl LogCapture {
    /// Starts recording this thread's events, at every level and from every target.
    pub fn start() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorder = Recorder(Arc::clone(&events));
        let guard = tracing::subscriber::set_default(Registry::default().with(recorder));
        Self {
            events,
            _guard: guard,
        }
    }

    /// The events so far, oldest first.
    pub fn events(&self) -> Vec<CapturedEvent> {
        self.events
            .lock()
            .expect("a capture's lock isn't poisoned")
            .clone()
    }

    /// Whether an event at `level` has `text` in its message.
    pub fn has_message(&self, level: Level, text: &str) -> bool {
        self.count_messages(level, text) > 0
    }

    /// How many events at `level` have `text` in their message.
    pub fn count_messages(&self, level: Level, text: &str) -> usize {
        self.events()
            .iter()
            .filter(|event| event.level == level && event.message.contains(text))
            .count()
    }

    /// Whether `text` is in any event's message or field values, or the field values of the spans
    /// an event was logged in, at any level. Use it to check that something, such as a secret,
    /// isn't logged.
    pub fn mentions(&self, text: &str) -> bool {
        let in_fields =
            |fields: &BTreeMap<String, String>| fields.values().any(|v| v.contains(text));
        self.events().iter().any(|event| {
            event.message.contains(text)
                || in_fields(&event.fields)
                || event.spans.iter().any(|span| in_fields(&span.fields))
        })
    }
}

/// One logged event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedEvent {
    pub level: Level,
    pub target: String,
    pub message: String,
    /// The fields other than the message, each formatted as the event's own `%` or `?` asked, and
    /// an error as its message then its sources, joined by `: `.
    pub fields: BTreeMap<String, String>,
    /// The spans the event was logged in, outermost first.
    pub spans: Vec<CapturedSpan>,
}

/// A span an event was logged in, with its fields, the message included, as they were at the
/// event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSpan {
    pub name: String,
    pub fields: BTreeMap<String, String>,
}

/// The layer that turns each event into a [`CapturedEvent`].
struct Recorder(Arc<Mutex<Vec<CapturedEvent>>>);

impl<S> Layer<S> for Recorder
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        let span = ctx.span(id).expect("a new span is in the registry");
        span.extensions_mut().insert(fields);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("a recorded span is in the registry");
        if let Some(fields) = span.extensions_mut().get_mut::<Fields>() {
            values.record(fields);
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let spans = ctx
            .event_scope(event)
            .into_iter()
            .flat_map(|scope| scope.from_root())
            .map(|span| CapturedSpan {
                name: span.name().to_owned(),
                fields: span
                    .extensions()
                    .get::<Fields>()
                    .map(Fields::all)
                    .unwrap_or_default(),
            })
            .collect();
        let mut fields = Fields::default();
        event.record(&mut fields);
        let metadata = event.metadata();
        self.0
            .lock()
            .expect("a capture's lock isn't poisoned")
            .push(CapturedEvent {
                level: *metadata.level(),
                target: metadata.target().to_owned(),
                message: fields.message,
                fields: fields.others,
                spans,
            });
    }
}

/// An event's or a span's message and its other fields, as text.
#[derive(Default)]
struct Fields {
    message: String,
    others: BTreeMap<String, String>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        let mut text = value.to_string();
        let mut source = value.source();
        while let Some(error) = source {
            write!(text, ": {error}").expect("writing to a String can't fail");
            source = error.source();
        }
        self.record(field, text);
    }
}

impl Fields {
    /// Every field, the message included.
    fn all(&self) -> BTreeMap<String, String> {
        let mut all = self.others.clone();
        if !self.message.is_empty() {
            all.insert("message".to_owned(), self.message.clone());
        }
        all
    }

    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.others.insert(field.name().to_owned(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn records_level_target_message_and_fields() {
        let logs = LogCapture::start();
        tracing::info!(target: "app::jobs", job = 7, name = "nightly", "job started");

        let events = logs.events();
        assert!(let [event] = events.as_slice());
        check!(event.level == Level::INFO);
        check!(event.target == "app::jobs");
        check!(event.message == "job started");
        check!(event.fields == fields(&[("job", "7"), ("name", "nightly")]));
        check!(event.spans.is_empty());
    }

    #[test]
    fn fields_follow_display_and_debug() {
        #[derive(Debug)]
        #[expect(dead_code, reason = "only read through Debug")]
        struct Retry {
            tries: u32,
        }

        let logs = LogCapture::start();
        let path = std::path::Path::new("/etc/app");
        tracing::warn!(path = %path.display(), reason = ?"bad toml", retry = ?Retry { tries: 2 }, "keeping last good snapshot");

        let event = &logs.events()[0];
        check!(event.fields["path"] == "/etc/app");
        check!(event.fields["reason"] == r#""bad toml""#);
        check!(event.fields["retry"] == "Retry { tries: 2 }");
    }

    #[test]
    fn an_error_field_is_its_message_then_its_sources() {
        #[derive(Debug)]
        struct Purge(std::io::Error);

        impl fmt::Display for Purge {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("purge")
            }
        }

        impl std::error::Error for Purge {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let logs = LogCapture::start();
        let error = Purge(std::io::Error::other("down"));
        tracing::warn!(error = &error as &dyn std::error::Error, "run failed");

        check!(logs.events()[0].fields["error"] == "purge: down");
    }

    #[test]
    fn records_the_spans_outermost_first_with_their_fields() {
        let logs = LogCapture::start();
        let request = tracing::info_span!("request", path = "/login", user = tracing::field::Empty);
        let _request = request.enter();
        request.record("user", "uima");
        let _query = tracing::debug_span!("query", table = "accounts").entered();
        tracing::info!("signed in");

        let events = logs.events();
        assert!(let [event] = events.as_slice());
        check!(
            event.spans
                == [
                    CapturedSpan {
                        name: "request".to_owned(),
                        fields: fields(&[("path", "/login"), ("user", "uima")]),
                    },
                    CapturedSpan {
                        name: "query".to_owned(),
                        fields: fields(&[("table", "accounts")]),
                    },
                ]
        );
    }

    #[test]
    fn records_every_level_in_order() {
        let logs = LogCapture::start();
        tracing::trace!("one");
        tracing::debug!("two");
        tracing::error!("three");

        let levels: Vec<Level> = logs.events().iter().map(|e| e.level).collect();
        check!(levels == [Level::TRACE, Level::DEBUG, Level::ERROR]);
    }

    #[test]
    fn has_message_and_count_messages_match_the_level_and_part_of_the_message() {
        let logs = LogCapture::start();
        tracing::info!("job 1 left queued");
        tracing::info!("job 2 left queued");
        tracing::warn!("job 3 left queued");

        check!(logs.has_message(Level::WARN, "left queued"));
        check!(!logs.has_message(Level::ERROR, "left queued"));
        check!(!logs.has_message(Level::INFO, "retrying"));
        check!(logs.count_messages(Level::INFO, "left queued") == 2);
        check!(logs.count_messages(Level::DEBUG, "left queued") == 0);
    }

    #[test]
    fn mentions_looks_in_messages_and_field_values() {
        let logs = LogCapture::start();
        tracing::debug!(token = "s3cr3t", "refreshing");
        tracing::info!("signed in as uima");

        check!(logs.mentions("s3cr3t"));
        check!(logs.mentions("uima"));
        check!(!logs.mentions("token"));
        check!(!logs.mentions("hunter2"));
    }

    #[test]
    fn mentions_looks_in_span_field_values() {
        let logs = LogCapture::start();
        let _login = tracing::info_span!("login", password = "s3cr3t").entered();
        tracing::info!("signed in");

        check!(logs.mentions("s3cr3t"));
        check!(!logs.mentions("password"));
        check!(!logs.mentions("login"));
    }

    #[test]
    fn span_fields_take_their_latest_value() {
        let logs = LogCapture::start();
        let span = tracing::info_span!("request", user = "a", message = "c");
        let _span = span.enter();
        span.record("user", "b");
        span.record("message", "d");
        tracing::info!("signed in");

        check!(logs.events()[0].spans[0].fields == fields(&[("message", "d"), ("user", "b")]));
    }

    #[test]
    fn misses_spans_created_before_start() {
        let login = tracing::info_span!("login", password = "s3cr3t");
        let logs = LogCapture::start();
        let _login = login.enter();
        tracing::info!("signed in");

        check!(logs.events()[0].spans.is_empty());
        check!(!logs.mentions("s3cr3t"));
    }

    #[test]
    fn an_inner_capture_stops_when_the_outer_drops_first() {
        let outer = LogCapture::start();
        let inner = LogCapture::start();
        drop(outer);
        tracing::info!("after");

        check!(inner.events().is_empty());
    }

    #[test]
    fn mentions_looks_in_a_span_message() {
        let logs = LogCapture::start();
        let _span = tracing::info_span!("login", message = "s3cr3t").entered();
        tracing::info!("signed in");

        check!(logs.mentions("s3cr3t"));
    }

    #[test]
    fn stops_recording_when_dropped() {
        let logs = LogCapture::start();
        let events = Arc::clone(&logs.events);
        drop(logs);

        tracing::info!("after");
        check!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn misses_events_from_other_threads() {
        let logs = LogCapture::start();
        std::thread::spawn(|| tracing::info!("elsewhere"))
            .join()
            .unwrap();

        check!(logs.events().is_empty());
    }

    #[tokio::test]
    async fn records_tasks_of_a_tokio_test() {
        let logs = LogCapture::start();
        tokio::spawn(async { tracing::info!("in a task") })
            .await
            .unwrap();

        check!(logs.has_message(Level::INFO, "in a task"));
    }
}
