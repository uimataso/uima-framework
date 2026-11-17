//! [`Trace`], logging each request.

use std::fmt;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::{HeaderMap, HeaderName, Request, Uri, header};
use tower::{Layer, Service};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestId, PropagateRequestIdLayer, RequestId, SetRequestId,
    SetRequestIdLayer,
};
use tower_http::sensitive_headers::{
    SetSensitiveRequestHeaders, SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeaders,
    SetSensitiveResponseHeadersLayer,
};
use tower_http::trace::{MakeSpan, TraceLayer};
use tracing::{Span, info_span};

use crate::OriginAuth;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
/// The longest request id kept from a request.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Logs each request in a `request` span tagged with a request id, keeping credentials out of the
/// log.
///
/// The span is at `INFO`, so every event logged while handling the request at `info` or above,
/// such as the handler's `error!`, carries the request's `method`, `uri` and `request_id`. Its
/// start and finish events are at `DEBUG` under `tower_http` targets, so a filter such as
/// `tower_http=debug` turns them on, and a 5xx response is logged at `ERROR`.
///
/// The request id is the request's `x-request-id` when that is 1 to 128 visible ASCII characters,
/// and a new UUID otherwise, and the response carries it back. Headers that carry credentials,
/// such as `authorization`, `cookie`, `set-cookie` and the [`OriginAuth`] header, are marked
/// sensitive, so a log of them shows `Sensitive` instead of the value. A request's are marked
/// before any layer inside this one sees them, and a response's before this one logs it.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    hidden_query_paths: Arc<[String]>,
}

impl Trace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Logs requests to `paths` without their query string, for a query that carries a secret,
    /// such as a webhook's signature.
    ///
    /// A path matches only when it is the same as the request's path as this layer sees it, so
    /// under a `Router::nest` it is the path with the nest's prefix taken off.
    pub fn hide_query_on<P: Into<String>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.hidden_query_paths = self
            .hidden_query_paths
            .iter()
            .cloned()
            .chain(paths.into_iter().map(Into::into))
            .collect();
        self
    }
}

impl<S> Layer<S> for Trace {
    type Service = TraceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        let sensitive = || {
            [
                header::AUTHORIZATION,
                header::PROXY_AUTHORIZATION,
                header::COOKIE,
                header::SET_COOKIE,
                HeaderName::from_static(OriginAuth::HEADER),
            ]
        };
        // The request headers are marked outside the trace and the response headers inside it,
        // so the trace sees both marked.
        let inner = PropagateRequestIdLayer::new(REQUEST_ID).layer(inner);
        let inner = SetSensitiveResponseHeadersLayer::new(sensitive()).layer(inner);
        let inner = TraceLayer::new_for_http()
            .make_span_with(RequestSpan {
                hidden_query_paths: self.hidden_query_paths.clone(),
            })
            .layer(inner);
        let inner = SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid).layer(inner);
        let inner = SetSensitiveRequestHeadersLayer::new(sensitive()).layer(inner);
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

type Stack<S> = SetSensitiveRequestHeaders<
    SetRequestId<
        tower_http::trace::Trace<
            SetSensitiveResponseHeaders<PropagateRequestId<S>>,
            SharedClassifier<ServerErrorsAsFailures>,
            RequestSpan,
        >,
        MakeRequestUuid,
    >,
>;

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

fn has_valid_request_id(headers: &HeaderMap) -> bool {
    let mut ids = headers.get_all(REQUEST_ID).iter();
    let (Some(id), None) = (ids.next(), ids.next()) else {
        return false;
    };
    (1..=MAX_REQUEST_ID_LEN).contains(&id.len()) && id.as_bytes().iter().all(u8::is_ascii_graphic)
}

/// `tower_http`'s default span at `INFO`, minus the query on `hidden_query_paths`, plus the
/// request id.
#[derive(Debug, Clone)]
pub struct RequestSpan {
    hidden_query_paths: Arc<[String]>,
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

/// A request's URI as the span logs it, formatted only when the span is enabled.
struct LoggedUri<'a> {
    uri: &'a Uri,
    hidden_query_paths: &'a [String],
}

impl fmt::Display for LoggedUri<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.uri.path();
        if self.hidden_query_paths.iter().any(|hidden| hidden == path) {
            f.write_str(path)
        } else {
            fmt::Display::fmt(self.uri, f)
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use tower::ServiceExt as _;
    use tracing::Level;
    use uima_log_tracing::testing::{CapturedEvent, LogCapture};
    use uima_testing::{assert, check};

    use super::*;

    #[tokio::test]
    async fn a_valid_request_id_is_kept() {
        let longest = "a".repeat(128);
        for id in ["abc-123", "4a826a43-6628-41d7-b903-7da981d22075", &longest] {
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

        for uri in ["/hooks?sign=s3cret", "/?sign=s3cret"] {
            let logs = LogCapture::start();
            send(trace.clone(), Request::get(uri)).await;
            check!(span_fields(&logs)["uri"] == uri);
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
        let failure = events
            .iter()
            .find(|event| event.target == "tower_http::trace::on_failure");
        assert!(let Some(failure) = failure);
        check!(failure.level == Level::ERROR);
        check!(in_span(failure));
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
        let nested = Router::new().nest("/api", Router::new().route("/hook", get(|| async {})));
        let service = Trace::new().hide_query_on(["/api/hook"]).layer(nested);
        let logs = LogCapture::start();

        let request = Request::get("/api/hook?sign=s3cret").body(Body::empty());
        let response = service.oneshot(request.unwrap()).await.unwrap();

        check!(response.status() == StatusCode::OK);
        check!(!logs.mentions("s3cret"));
    }

    async fn send(trace: Trace, request: axum::http::request::Builder) -> Response {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/hook", get(|| async { "ok" }))
            .route("/hooks", get(|| async { "ok" }))
            .route("/span", get(current_span))
            .route("/credentials", get(credentials))
            .route("/fail", get(fail))
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
        tracing::error!("the handler failed");
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// The fields of the span the first event was logged in.
    fn span_fields(logs: &LogCapture) -> std::collections::BTreeMap<String, String> {
        let events = logs.events();
        let event = events.first().expect("an event in the request");
        event.spans[0].fields.clone()
    }
}
