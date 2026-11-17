//! [`SameOrigin`], refusing writes sent from another site's page.

use std::fmt;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};

use crate::BoxFuture;

/// Refuses a request with an unsafe method, any but `GET`, `HEAD`, `OPTIONS` and `TRACE`, that
/// another site's page sent, so that page can't act through the browser of someone who visits it
/// (CSRF).
///
/// A service needs it when a browser's requests are trusted by what they carry on their own, such
/// as a cookie or the network they come from. A bearer token in `authorization` isn't one, since
/// the browser never adds it on its own. It checks where a request comes from, not that the host
/// is the service's own, so a page that points its own domain at the service (DNS rebinding)
/// passes.
///
/// A request passes when its `sec-fetch-site` is `same-origin` or `none`, or, from a browser that
/// doesn't send `sec-fetch-site`, when its `origin` names its own host exactly. A request with
/// neither header, such as one from `curl`, passes too. A refused request gets a bare `403`, or
/// what [`refuse_with`](Self::refuse_with) returns.
#[derive(Clone)]
pub struct SameOrigin {
    refusal: Arc<dyn Fn() -> Response + Send + Sync>,
}

impl SameOrigin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers a refused request with what `refusal` returns, such as the service's error page.
    pub fn refuse_with(self, refusal: impl Fn() -> Response + Send + Sync + 'static) -> Self {
        Self {
            refusal: Arc::new(refusal),
        }
    }
}

impl Default for SameOrigin {
    fn default() -> Self {
        Self {
            refusal: Arc::new(|| StatusCode::FORBIDDEN.into_response()),
        }
    }
}

impl fmt::Debug for SameOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SameOrigin").finish_non_exhaustive()
    }
}

impl<S> Layer<S> for SameOrigin {
    type Service = SameOriginService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SameOriginService {
            inner,
            refusal: self.refusal.clone(),
        }
    }
}

/// The service [`SameOrigin`] wraps around another.
#[derive(Clone)]
pub struct SameOriginService<S> {
    inner: S,
    refusal: Arc<dyn Fn() -> Response + Send + Sync>,
}

impl<S: fmt::Debug> fmt::Debug for SameOriginService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SameOriginService")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<S, B> Service<Request<B>> for SameOriginService<S>
where
    S: Service<Request<B>>,
    S::Response: IntoResponse,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<Result<Response, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        if !request.method().is_safe() && !is_same_origin(&request) {
            let refusal = (self.refusal)();
            return Box::pin(async { Ok(refusal) });
        }
        let response = self.inner.call(request);
        Box::pin(async { response.await.map(IntoResponse::into_response) })
    }
}

fn is_same_origin<B>(request: &Request<B>) -> bool {
    let header = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    if let Some(site) = header("sec-fetch-site") {
        return matches!(site, "same-origin" | "none");
    }
    let Some(origin) = header("origin") else {
        return true;
    };
    // HTTP/2 carries the host in the URI instead of a `host` header.
    let host = header("host").or_else(|| request.uri().authority().map(|a| a.as_str()));
    let origin_host = origin.split_once("://").map(|(_, host)| host);
    origin_host.is_some_and(|origin_host| Some(origin_host) == host)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Version};
    use axum::routing::any;
    use tower::ServiceExt as _;
    use uima_testing::check;

    use super::*;

    #[tokio::test]
    async fn unsafe_requests_need_the_same_origin() {
        let cases: &[(&[(&str, &str)], bool)] = &[
            (&[("sec-fetch-site", "same-origin")], true),
            (&[("sec-fetch-site", "none")], true),
            (&[("sec-fetch-site", "same-site")], false),
            (&[("sec-fetch-site", "cross-site")], false),
            // `sec-fetch-site` wins over a matching `origin`
            (
                &[
                    ("sec-fetch-site", "cross-site"),
                    ("origin", "http://t"),
                    ("host", "t"),
                ],
                false,
            ),
            (&[("origin", "http://t:8080"), ("host", "t:8080")], true),
            (&[("origin", "https://T:8080"), ("host", "t:8080")], false),
            (&[("origin", "http://t:8080"), ("host", "t")], false),
            (&[("origin", "http://evil"), ("host", "t")], false),
            (&[("origin", "null"), ("host", "t")], false),
            (&[("origin", "http://t")], false),
            (&[], true),
        ];
        let methods = ["POST", "PUT", "PATCH", "DELETE", "CONNECT", "PROPFIND"];
        for (headers, passes) in cases {
            for method in methods {
                let mut request = Request::builder().method(method).uri("/");
                for (name, value) in *headers {
                    request = request.header(*name, *value);
                }
                let status = send(SameOrigin::new(), request).await.status();
                check!(
                    (status == StatusCode::OK) == *passes,
                    "{method} {headers:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn safe_requests_pass_from_anywhere() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
            let request = Request::builder()
                .method(method.clone())
                .uri("/")
                .header("sec-fetch-site", "cross-site");
            check!(
                send(SameOrigin::new(), request).await.status() == StatusCode::OK,
                "{method}"
            );
        }
    }

    #[tokio::test]
    async fn the_host_comes_from_the_uri_over_http2() {
        // What hyper hands axum for an HTTP/2 request: the authority in the URI, no `host`.
        let request = |origin| {
            Request::post("http://t:8080/")
                .version(Version::HTTP_2)
                .header("origin", origin)
        };
        let status = send(SameOrigin::new(), request("http://t:8080"))
            .await
            .status();
        check!(status == StatusCode::OK);
        let status = send(SameOrigin::new(), request("http://evil"))
            .await
            .status();
        check!(status == StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_refusal_is_a_bare_403_by_default() {
        for uri in ["/", "/missing"] {
            let request = Request::post(uri).header("sec-fetch-site", "cross-site");

            let response = send(SameOrigin::new(), request).await;

            check!(response.status() == StatusCode::FORBIDDEN, "{uri}");
            let body = axum::body::to_bytes(response.into_body(), 1024).await;
            check!(body.unwrap().is_empty(), "{uri}");
        }
    }

    #[tokio::test]
    async fn the_refusal_can_be_the_services_own() {
        let same_origin =
            SameOrigin::new().refuse_with(|| (StatusCode::IM_A_TEAPOT, "go away").into_response());
        let request = Request::post("/").header("sec-fetch-site", "cross-site");
        check!(send(same_origin, request).await.status() == StatusCode::IM_A_TEAPOT);
    }

    #[test]
    fn debug_names_the_type() {
        check!(format!("{:?}", SameOrigin::new()) == "SameOrigin { .. }");
    }

    async fn send(same_origin: SameOrigin, request: axum::http::request::Builder) -> Response {
        let router = Router::new()
            .route("/", any(|| async { "ok" }))
            .layer(same_origin);
        let request = request.body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap()
    }
}
