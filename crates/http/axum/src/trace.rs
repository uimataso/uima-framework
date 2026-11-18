//! [`Trace`], logging each request.

use std::fmt;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, Request, Response, Uri, header};
use tower::{Layer, Service};
use tower_http::classify::{ServerErrorsAsFailures, ServerErrorsFailureClass, SharedClassifier};
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestId, PropagateRequestIdLayer, RequestId, SetRequestId,
    SetRequestIdLayer,
};
use tower_http::sensitive_headers::{SetSensitiveHeaders, SetSensitiveHeadersLayer};
use tower_http::trace::{
    DefaultOnBodyChunk, DefaultOnEos, DefaultOnRequest, DefaultOnResponse, MakeSpan, OnFailure,
    OnResponse, TraceLayer,
};
use tracing::{Span, error, info_span};

use crate::OriginAuth;
use crate::paths::Paths;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
/// The headers that carry credentials.
const SENSITIVE: [HeaderName; 5] = [
    header::AUTHORIZATION,
    header::PROXY_AUTHORIZATION,
    header::COOKIE,
    header::SET_COOKIE,
    HeaderName::from_static(OriginAuth::HEADER),
];
/// The longest request id kept from a request.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Logs each request in a `request` span tagged with a request id, keeping credentials out of the
/// log.
///
/// What it logs, and the filter that keeps each:
///
/// - The `request` span, at `INFO` under the `uima_http_axum::trace` target, with the request's
///   `method`, `uri` and `request_id`. Every event logged while handling the request at `info` or
///   above, such as the handler's `error!`, carries them. `uima_http_axum=info` keeps it; `warn`
///   or `my_app=info` leaves it and its fields off.
/// - The start and finish events, at `DEBUG` under `tower_http` targets; `tower_http=debug` keeps
///   them.
/// - A 5xx response, once at `ERROR` under `uima_http_axum::trace` with the [`LoggedError`] it
///   carries, and an error the inner service returns in place of a response. `warn` keeps them,
///   without the span's fields; `tower_http=debug` or `my_app=info` alone doesn't.
/// - A response whose body fails after its status went out isn't logged.
///
/// The `uri` is the request's path and query, the path as the [crate docs](crate#paths) say this
/// layer sees it.
///
/// The request id is the request's `x-request-id` when that is 1 to 128 visible ASCII characters,
/// and a new UUID otherwise, so a client can't put a long or unprintable value on every line the
/// request logs, and the response carries it back. Headers that carry credentials, such as
/// `authorization`, `cookie`, `set-cookie` and the [`OriginAuth`] header, are marked sensitive, so
/// a log of them shows `Sensitive` instead of the value. A request's are marked before the layers
/// and the handler inside this one see them, and a response's as it leaves this layer.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    hidden_query_paths: Paths,
}

impl Trace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Logs requests to `paths` without their query string, for a query that carries a secret,
    /// such as a webhook's signature. A path matches as the [crate docs](crate#paths) say.
    ///
    /// # Panics
    ///
    /// If one of `paths` doesn't start with `/` or holds a `?` or `#`.
    pub fn hide_query_on<P: AsRef<str>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.hidden_query_paths.extend(paths);
        self
    }
}

impl<S> Layer<S> for Trace {
    type Service = TraceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        let inner = PropagateRequestIdLayer::new(REQUEST_ID).layer(inner);
        let inner = TraceLayer::new_for_http()
            .make_span_with(RequestSpan {
                hidden_query_paths: self.hidden_query_paths.clone(),
            })
            .on_response(ResponseLog)
            .on_failure(ResponseLog)
            .layer(inner);
        let inner = SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid).layer(inner);
        let inner = SetSensitiveHeadersLayer::new(SENSITIVE).layer(inner);
        TraceService { inner }
    }
}

/// The service [`Trace`] wraps around another.
#[derive(Clone)]
pub struct TraceService<S> {
    inner: Stack<S>,
}

impl<S> fmt::Debug for TraceService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceService").finish_non_exhaustive()
    }
}

impl<S, B> Service<Request<B>> for TraceService<S>
where
    Stack<S>: Service<Request<B>>,
{
    type Response = <Stack<S> as Service<Request<B>>>::Response;
    type Error = <Stack<S> as Service<Request<B>>>::Error;
    type Future = <Stack<S> as Service<Request<B>>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        // `SetRequestId` makes a new id only when the header is missing.
        if !has_valid_request_id(request.headers()) {
            request.headers_mut().remove(REQUEST_ID);
        }
        self.inner.call(request)
    }
}

type Stack<S> = SetSensitiveHeaders<
    SetRequestId<
        tower_http::trace::Trace<
            PropagateRequestId<S>,
            SharedClassifier<ServerErrorsAsFailures>,
            RequestSpan,
            DefaultOnRequest,
            ResponseLog,
            DefaultOnBodyChunk,
            DefaultOnEos,
            ResponseLog,
        >,
        MakeRequestUuid,
    >,
>;

/// `tower_http`'s default span at `INFO`, minus the query on `hidden_query_paths`, plus the
/// request id.
#[derive(Debug, Clone)]
pub struct RequestSpan {
    hidden_query_paths: Paths,
}

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let request_id = request
            .extensions()
            .get::<RequestId>()
            .and_then(|id| id.header_value().to_str().ok())
            .unwrap_or_default();
        info_span!(
            "request",
            method = %request.method(),
            uri = %LoggedUri {
                uri: request.uri(),
                hidden_query_paths: &self.hidden_query_paths,
            },
            version = ?request.version(),
            request_id,
        )
    }
}

/// An error a response carries in its extensions, for [`Trace`] to log with a 5xx response, such as
/// the chain of the error the response answers.
///
/// ```
/// use axum::http::StatusCode;
/// use axum::response::IntoResponse as _;
/// use uima_http_axum::LoggedError;
///
/// let error = anyhow::anyhow!("db down").context("load user");
/// let mut response = StatusCode::INTERNAL_SERVER_ERROR.into_response();
/// response.extensions_mut().insert(LoggedError::new(&error));
/// ```
#[derive(Debug, Clone)]
pub struct LoggedError(Arc<str>);

impl LoggedError {
    /// The error as its `{:?}`, which for an `anyhow::Error` is its chain.
    pub fn new(error: &(impl fmt::Debug + ?Sized)) -> Self {
        Self(format!("{error:?}").into())
    }
}

/// Logs a 5xx response once, with its [`LoggedError`], and an error the inner service returns in
/// place of a response; `tower_http`'s debug event for every response stays.
#[derive(Debug, Clone, Copy)]
pub struct ResponseLog;

impl<B> OnResponse<B> for ResponseLog {
    fn on_response(self, response: &Response<B>, latency: Duration, span: &Span) {
        DefaultOnResponse::new().on_response(response, latency, span);
        let status = response.status();
        if status.is_server_error() {
            let logged = response.extensions().get::<LoggedError>();
            let error = logged.map(|logged| &*logged.0);
            let latency = Millis(latency);
            error!(status = status.as_u16(), %latency, error, "response failed");
        }
    }
}

impl OnFailure<ServerErrorsFailureClass> for ResponseLog {
    fn on_failure(&mut self, class: ServerErrorsFailureClass, latency: Duration, _: &Span) {
        // A 5xx status is logged with its response.
        if let ServerErrorsFailureClass::Error(class) = class {
            let latency = Millis(latency);
            error!(error = class, %latency, "response failed");
        }
    }
}

/// A latency as `tower_http`'s events log it, in whole milliseconds.
struct Millis(Duration);

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ms", self.0.as_millis())
    }
}

/// A request's URI as the span logs it, formatted only when the span is enabled.
struct LoggedUri<'a> {
    uri: &'a Uri,
    hidden_query_paths: &'a Paths,
}

impl fmt::Display for LoggedUri<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.uri.path();
        match self.uri.query() {
            Some(query) if !self.hidden_query_paths.contains(path) => write!(f, "{path}?{query}"),
            _ => f.write_str(path),
        }
    }
}

fn has_valid_request_id(headers: &HeaderMap) -> bool {
    let mut ids = headers.get_all(REQUEST_ID).iter();
    let (Some(id), None) = (ids.next(), ids.next()) else {
        return false;
    };
    (1..=MAX_REQUEST_ID_LEN).contains(&id.len()) && id.as_bytes().iter().all(u8::is_ascii_graphic)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::{HeaderValue, StatusCode, Version};
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use tower::ServiceExt as _;
    use tracing::{Level, error};
    use uima_log_tracing_subscriber::testing::{CapturedEvent, LogCapture};
    use uima_testing::{assert, check};

    use super::*;

    #[tokio::test]
    async fn a_valid_request_id_is_kept() {
        let longest = "a".repeat(128);
        let ids = [
            "abc-123",
            "4a826a43-6628-41d7-b903-7da981d22075",
            "!",
            "~",
            "a.b_c:~/=+",
            &longest,
        ];
        for id in ids {
            let logs = LogCapture::start();

            let response = send(Trace::new(), Request::get("/").header("x-request-id", id)).await;

            check!(response.headers()["x-request-id"] == id);
            check!(span_fields(&logs)["request_id"] == id);
        }
    }

    #[tokio::test]
    async fn a_missing_or_invalid_request_id_is_replaced() {
        let too_long = "a".repeat(129);
        let cases: &[&[&[u8]]] = &[
            &[],
            &[b""],
            &[too_long.as_bytes()],
            &[b"a b"],
            &[b"a\tb"],
            &[b"caf\xc3\xa9"],
            &[b"id-\xff"],
            &[b"one", b"two"],
        ];
        for ids in cases {
            let logs = LogCapture::start();
            let mut request = Request::get("/");
            for id in *ids {
                request = request.header("x-request-id", HeaderValue::from_bytes(id).unwrap());
            }

            let response = send(Trace::new(), request).await;

            let made = response.headers()["x-request-id"].to_str().unwrap();
            check!(made.len() == 36, "a UUID for {ids:?}: {made}");
            check!(span_fields(&logs)["request_id"] == made);
        }
    }

    #[tokio::test]
    async fn the_span_is_an_info_span_with_the_request() {
        let logs = LogCapture::start();

        let response = send(
            Trace::new(),
            Request::get("/span?page=2").header("x-request-id", "abc-123"),
        )
        .await;

        let body = axum::body::to_bytes(response.into_body(), 1024).await;
        check!(body.unwrap() == "uima_http_axum::trace request INFO");
        let fields = span_fields(&logs);
        check!(fields["method"] == "GET");
        check!(fields["uri"] == "/span?page=2");
        check!(fields["version"] == "HTTP/1.1");
        check!(fields["request_id"] == "abc-123");
    }

    #[tokio::test]
    async fn the_query_is_hidden_only_on_the_given_paths() {
        let trace = Trace::new().hide_query_on(["/hook"]);

        let logs = LogCapture::start();
        send(trace.clone(), Request::get("/hook?sign=s3cret")).await;
        check!(!logs.mentions("s3cret"));
        check!(span_fields(&logs)["uri"] == "/hook");

        for uri in ["/hooks?sign=s3cret", "/HOOK?sign=s3cret", "/?sign=s3cret"] {
            let logs = LogCapture::start();
            send(trace.clone(), Request::get(uri)).await;
            check!(span_fields(&logs)["uri"] == uri);
        }
    }

    #[tokio::test]
    async fn the_uri_is_the_path_and_query_also_over_http2() {
        let logs = LogCapture::start();

        let request = Request::get("http://user:pw@t/span?page=2").version(Version::HTTP_2);
        send(Trace::new(), request).await;

        check!(span_fields(&logs)["uri"] == "/span?page=2");
    }

    #[tokio::test]
    async fn hidden_paths_add_up() {
        let trace = Trace::new().hide_query_on(["/hook"]).hide_query_on(["/"]);
        for path in ["/hook", "/"] {
            let logs = LogCapture::start();
            send(trace.clone(), Request::get(format!("{path}?sign=s3cret"))).await;
            check!(span_fields(&logs)["uri"] == path);
        }
    }

    #[tokio::test]
    async fn under_a_nest_the_path_leaves_out_its_prefix() {
        let inner = Router::new()
            .route("/hook", get(|| async {}))
            .layer(Trace::new().hide_query_on(["/hook"]));
        let router = Router::new().nest("/api", inner);
        let logs = LogCapture::start();

        let request = Request::get("/api/hook?sign=s3cret").body(Body::empty());
        router.oneshot(request.unwrap()).await.unwrap();

        check!(!logs.mentions("s3cret"));
        check!(span_fields(&logs)["uri"] == "/hook");
    }

    #[tokio::test]
    async fn start_and_finish_are_debug_events_under_tower_http() {
        let logs = LogCapture::start();

        send(Trace::new(), Request::get("/")).await;

        let events = logs.events();
        for target in [
            "tower_http::trace::on_request",
            "tower_http::trace::on_response",
        ] {
            let event = events.iter().find(|event| event.target == target);
            assert!(let Some(event) = event, "{target}");
            check!(event.level == Level::DEBUG, "{target}");
        }
    }

    #[tokio::test]
    async fn credentials_are_sensitive_inside_the_trace() {
        let response = send(
            Trace::new(),
            Request::get("/credentials")
                .header("authorization", "Bearer t")
                .header("proxy-authorization", "Basic p")
                .header("cookie", "session=s")
                .header("x-origin-auth", "o"),
        )
        .await;

        check!(
            response.status() == StatusCode::OK,
            "the handler saw them sensitive"
        );
        check!(response.headers()["set-cookie"].is_sensitive());
    }

    #[tokio::test]
    async fn events_in_the_request_carry_the_span() {
        let logs = LogCapture::start();

        send(
            Trace::new(),
            Request::get("/fail").header("x-request-id", "abc-123"),
        )
        .await;

        let events = logs.events();
        let in_span = |event: &CapturedEvent| event.spans[0].fields["request_id"] == "abc-123";
        let app_error = events
            .iter()
            .find(|event| event.message == "the handler failed");
        assert!(let Some(app_error) = app_error);
        check!(in_span(app_error));
        let failures: Vec<_> = events
            .iter()
            .filter(|event| event.message == "response failed")
            .collect();
        assert!(let [failure] = &failures[..]);
        check!(failure.level == Level::ERROR);
        check!(failure.target == "uima_http_axum::trace");
        check!(failure.fields["status"] == "500");
        check!(!failure.fields.contains_key("error"));
        check!(in_span(failure));
    }

    #[tokio::test]
    async fn a_5xx_logs_the_error_it_carries_once() {
        let logs = LogCapture::start();

        send(Trace::new(), Request::get("/fail-with-error")).await;

        let failures: Vec<_> = logs
            .events()
            .into_iter()
            .filter(|event| event.level == Level::ERROR)
            .collect();
        assert!(let [failure] = &failures[..]);
        check!(failure.fields["error"] == "load user\n\nCaused by:\n    db down");
        let latency = failure.fields["latency"].strip_suffix(" ms").unwrap();
        check!(latency.parse::<u64>().unwrap() >= 20);
    }

    #[tokio::test]
    async fn an_error_in_place_of_a_response_is_logged() {
        let service = tower::service_fn(|_: Request| async {
            Err::<Response, _>(std::io::Error::other("gone"))
        });
        let logs = LogCapture::start();

        let request = Request::get("/").body(Body::empty()).unwrap();
        let result = Trace::new().layer(service).oneshot(request).await;

        check!(result.is_err());
        let failures: Vec<_> = logs
            .events()
            .into_iter()
            .filter(|event| event.level == Level::ERROR)
            .collect();
        assert!(let [failure] = &failures[..]);
        check!(failure.fields["error"].contains("gone"));
        check!(failure.target == "uima_http_axum::trace");
        check!(failure.fields["latency"].ends_with(" ms"));
    }

    #[test]
    fn a_latency_is_in_whole_milliseconds() {
        check!(Millis(Duration::from_micros(1_500_900)).to_string() == "1500 ms");
    }

    #[tokio::test]
    async fn a_newline_in_the_error_stays_on_its_line() {
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || Writer(writer.clone()))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        send(Trace::new(), Request::get("/fail-with-error")).await;

        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        let lines: Vec<_> = output
            .lines()
            .filter(|line| line.contains("ERROR"))
            .collect();
        assert!(let [line] = &lines[..]);
        check!(
            line.contains(r"load user\n\nCaused by:\n    db down"),
            "{line}"
        );
    }

    /// A `fmt` writer into a shared buffer.
    struct Writer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_4xx_logs_no_error_even_with_one() {
        let logs = LogCapture::start();

        let response = send(Trace::new(), Request::get("/refuse-with-error")).await;

        check!(response.status() == StatusCode::BAD_REQUEST);
        check!(
            !logs
                .events()
                .iter()
                .any(|event| event.level == Level::ERROR)
        );
    }

    #[tokio::test]
    async fn the_fallback_is_traced_too() {
        let logs = LogCapture::start();

        let response = send(Trace::new(), Request::get("/missing")).await;

        check!(response.status() == StatusCode::NOT_FOUND);
        check!(response.headers().contains_key("x-request-id"));
        check!(span_fields(&logs)["uri"] == "/missing");
    }

    #[tokio::test]
    async fn wrapping_a_whole_router_sees_the_full_path() {
        // Such a service returns tower-http's body, so `axum::serve` needs it mapped to axum's
        // `Response` first.
        let nested = Router::new().nest("/api", Router::new().route("/hook", get(|| async {})));
        let service = Trace::new().hide_query_on(["/api/hook"]).layer(nested);
        let logs = LogCapture::start();

        let request = Request::get("/api/hook?sign=s3cret").body(Body::empty());
        let response = service.oneshot(request.unwrap()).await.unwrap();

        check!(response.status() == StatusCode::OK);
        check!(!logs.mentions("s3cret"));
    }

    #[tokio::test]
    async fn the_handler_tells_whether_credentials_are_sensitive() {
        let mut headers = HeaderMap::new();
        for name in [
            "authorization",
            "proxy-authorization",
            "cookie",
            "x-origin-auth",
        ] {
            headers.insert(name, HeaderValue::from_static("v"));
        }
        let response = credentials(headers).await.into_response();
        check!(response.status() == StatusCode::IM_A_TEAPOT);
    }

    #[test]
    fn debug_names_the_service() {
        check!(format!("{:?}", Trace::new().layer(())).starts_with("TraceService"));
    }

    async fn send(trace: Trace, request: axum::http::request::Builder) -> Response {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/hook", get(|| async { "ok" }))
            .route("/span", get(current_span))
            .route("/credentials", get(credentials))
            .route("/fail", get(fail))
            .route("/fail-with-error", get(fail_with_error))
            .route("/refuse-with-error", get(refuse_with_error))
            .layer(trace);
        router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn current_span() -> String {
        let span = Span::current();
        let metadata = span.metadata().unwrap();
        format!(
            "{} {} {}",
            metadata.target(),
            metadata.name(),
            metadata.level()
        )
    }

    async fn credentials(headers: HeaderMap) -> impl IntoResponse {
        let names = [
            "authorization",
            "proxy-authorization",
            "cookie",
            "x-origin-auth",
        ];
        let status = if names.iter().all(|name| headers[*name].is_sensitive()) {
            StatusCode::OK
        } else {
            StatusCode::IM_A_TEAPOT
        };
        (status, [("set-cookie", "session=s")])
    }

    async fn fail() -> StatusCode {
        error!("the handler failed");
        StatusCode::INTERNAL_SERVER_ERROR
    }

    async fn fail_with_error() -> Response {
        tokio::time::sleep(Duration::from_millis(20)).await;
        with_error(StatusCode::INTERNAL_SERVER_ERROR)
    }

    async fn refuse_with_error() -> Response {
        with_error(StatusCode::BAD_REQUEST)
    }

    fn with_error(status: StatusCode) -> Response {
        let error = std::io::Error::other("db down");
        let error = format!("load user\n\nCaused by:\n    {error}");
        let mut response = status.into_response();
        response
            .extensions_mut()
            .insert(LoggedError::new(&Text(error)));
        response
    }

    /// Debugs as its text, as an `anyhow::Error` debugs as its chain.
    struct Text(String);

    impl fmt::Debug for Text {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    /// The fields of the span the first event was logged in.
    fn span_fields(logs: &LogCapture) -> std::collections::BTreeMap<String, String> {
        let events = logs.events();
        let event = events.first().expect("an event in the request");
        event.spans[0].fields.clone()
    }
}
