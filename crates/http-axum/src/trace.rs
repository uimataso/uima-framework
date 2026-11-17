//! [`Trace`], logging each request.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderName, Request, header};
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::trace::TraceLayer;
use tracing::{Span, debug_span};

use crate::OriginAuth;

const REQUEST_ID: &str = "x-request-id";

/// Logs each request in a `request` span tagged with a request id, keeping credentials out of the
/// log.
///
/// The request id is the request's `x-request-id`, or a new UUID when it has none, and the
/// response carries it back. The span and its events are at `DEBUG` under `tower_http` targets,
/// so a filter such as `tower_http=debug` turns them on; a 5xx response is logged at `ERROR`.
/// Headers that carry credentials, such as `authorization`, `cookie` and the [`OriginAuth`]
/// header, are marked sensitive, so a log of them shows `Sensitive` instead of the value.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    hidden_query_paths: Vec<String>,
}

impl Trace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Logs requests to `paths` without their query string, for a query that carries a secret,
    /// such as a webhook's signature.
    pub fn hide_query_on<P: Into<String>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.hidden_query_paths
            .extend(paths.into_iter().map(Into::into));
        self
    }

    /// Wraps `router`'s routes and its fallback.
    pub fn layer<S: Clone + Send + Sync + 'static>(self, router: Router<S>) -> Router<S> {
        let hidden_query_paths: Arc<[String]> = self.hidden_query_paths.into();
        let make_span = move |request: &Request<Body>| span(request, &hidden_query_paths);
        router.layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(
                    HeaderName::from_static(REQUEST_ID),
                    MakeRequestUuid,
                ))
                .layer(SetSensitiveHeadersLayer::new([
                    header::AUTHORIZATION,
                    header::PROXY_AUTHORIZATION,
                    header::COOKIE,
                    header::SET_COOKIE,
                    HeaderName::from_static(OriginAuth::HEADER),
                ]))
                .layer(TraceLayer::new_for_http().make_span_with(make_span))
                .layer(PropagateRequestIdLayer::new(HeaderName::from_static(
                    REQUEST_ID,
                ))),
        )
    }
}

/// `tower_http`'s default span, minus the query on `hidden_query_paths`, plus the request id.
fn span(request: &Request<Body>, hidden_query_paths: &[String]) -> Span {
    let uri = request.uri();
    let uri = if hidden_query_paths.iter().any(|path| path == uri.path()) {
        uri.path().to_owned()
    } else {
        uri.to_string()
    };
    let request_id = request
        .headers()
        .get(REQUEST_ID)
        .and_then(|id| id.to_str().ok())
        .unwrap_or_default();
    // The default span's target, so the same filter turns it on.
    debug_span!(
        target: "tower_http::trace::make_span",
        "request",
        method = %request.method(),
        uri,
        version = ?request.version(),
        request_id,
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use tower::ServiceExt as _;
    use uima_log_tracing::testing::LogCapture;
    use uima_testing::{assert, check};

    use super::*;

    #[tokio::test]
    async fn the_response_carries_the_request_id() {
        let router = Trace::new().layer(Router::new().route("/", get(|| async { "ok" })));

        let made = send(router.clone(), Request::get("/")).await;
        assert!(let Some(id) = made.headers().get(REQUEST_ID));
        check!(id.len() == 36, "a UUID: {id:?}");

        let given = send(router, Request::get("/").header(REQUEST_ID, "abc-123")).await;
        check!(given.headers()[REQUEST_ID] == "abc-123");
    }

    #[tokio::test]
    async fn the_span_has_the_uri_and_request_id() {
        let router = Trace::new().layer(Router::new().route("/a", get(|| async { "ok" })));
        let logs = LogCapture::start();

        send(
            router,
            Request::get("/a?page=2").header(REQUEST_ID, "abc-123"),
        )
        .await;

        let events = logs.events();
        assert!(let Some(event) = events.first());
        assert!(let [span] = event.spans.as_slice());
        check!(span.name == "request");
        check!(span.fields["uri"] == "/a?page=2");
        check!(span.fields["request_id"] == "abc-123");
    }

    #[tokio::test]
    async fn the_query_is_hidden_on_the_given_paths() {
        let router = Trace::new().hide_query_on(["/hook"]).layer(
            Router::new()
                .route("/hook", get(|| async { "ok" }))
                .route("/other", get(|| async { "ok" })),
        );
        let logs = LogCapture::start();

        send(router.clone(), Request::get("/hook?sign=s3cret")).await;
        check!(!logs.mentions("s3cret"));
        check!(logs.events()[0].spans[0].fields["uri"] == "/hook");

        send(router, Request::get("/other?sign=s3cret")).await;
        check!(logs.mentions("s3cret"));
    }

    #[tokio::test]
    async fn credentials_are_marked_sensitive() {
        let router = Trace::new().layer(Router::new().route(
            "/",
            get(|headers: HeaderMap| async move {
                let sensitive = ["authorization", "cookie", OriginAuth::HEADER]
                    .iter()
                    .all(|name| headers[*name].is_sensitive());
                if sensitive {
                    StatusCode::OK
                } else {
                    StatusCode::IM_A_TEAPOT
                }
            }),
        ));

        let response = send(
            router,
            Request::get("/")
                .header(header::AUTHORIZATION, "Bearer t")
                .header(header::COOKIE, "session=s")
                .header(OriginAuth::HEADER, "o"),
        )
        .await;

        check!(response.status() == StatusCode::OK);
    }

    #[tokio::test]
    async fn the_fallback_is_traced_too() {
        let router = Trace::new().layer(Router::new());
        let logs = LogCapture::start();

        let response = send(router, Request::get("/missing")).await;

        check!(response.status() == StatusCode::NOT_FOUND);
        check!(response.headers().contains_key(REQUEST_ID));
        check!(logs.events()[0].spans[0].fields["uri"] == "/missing");
    }

    async fn send(router: Router, request: axum::http::request::Builder) -> Response {
        router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
}
