//! [`SameOrigin`], refusing writes sent from another site's page.

use std::fmt;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

/// Refuses a request with an unsafe method, such as `POST` or `DELETE`, that another site's page
/// sent, so that page can't act through the browser of someone who visits it (CSRF).
///
/// A service needs it when a browser's requests are trusted by what they carry on their own, such
/// as a cookie or the network they come from. A bearer token in `authorization` isn't one, since
/// another site's page can't send it.
///
/// A request passes when its `sec-fetch-site` is `same-origin` or `none`, or, from a browser that
/// doesn't send `sec-fetch-site`, when its `origin` names its own host. A request with neither
/// header, such as one from `curl`, passes too. A refused request gets a bare `403`, or what
/// [`refuse_with`](Self::refuse_with) returns.
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

    /// Wraps `router`'s routes and its fallback.
    pub fn layer<S: Clone + Send + Sync + 'static>(self, router: Router<S>) -> Router<S> {
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let refusal = (!request.method().is_safe() && !is_same_origin(&request))
                .then(|| (self.refusal)());
            async move {
                match refusal {
                    Some(refusal) => refusal,
                    None => next.run(request).await,
                }
            }
        }))
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

fn is_same_origin(request: &Request) -> bool {
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
    origin_host
        .zip(host)
        .is_some_and(|(origin_host, host)| origin_host.eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Method;
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
            (&[("origin", "https://T:8080"), ("host", "t:8080")], true),
            (&[("origin", "http://t:8080"), ("host", "t")], false),
            (&[("origin", "http://evil"), ("host", "t")], false),
            (&[("origin", "null"), ("host", "t")], false),
            (&[("origin", "http://t")], false),
            (&[], true),
        ];
        for (headers, passes) in cases {
            for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                let mut request = Request::builder().method(method.clone()).uri("/");
                for (name, value) in *headers {
                    request = request.header(*name, *value);
                }
                let status = send(SameOrigin::new(), request).await;
                check!(
                    (status == StatusCode::OK) == *passes,
                    "{method} {headers:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn safe_requests_pass_from_anywhere() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            let request = Request::builder()
                .method(method.clone())
                .uri("/")
                .header("sec-fetch-site", "cross-site");
            check!(
                send(SameOrigin::new(), request).await == StatusCode::OK,
                "{method}"
            );
        }
    }

    #[tokio::test]
    async fn the_host_can_come_from_the_uri() {
        let request = Request::post("http://t:8080/").header("origin", "http://t:8080");
        check!(send(SameOrigin::new(), request).await == StatusCode::OK);

        let request = Request::post("http://t:8080/").header("origin", "http://evil");
        check!(send(SameOrigin::new(), request).await == StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_refusal_can_be_the_services_own() {
        let same_origin =
            SameOrigin::new().refuse_with(|| (StatusCode::IM_A_TEAPOT, "go away").into_response());
        let request = Request::post("/").header("sec-fetch-site", "cross-site");
        check!(send(same_origin, request).await == StatusCode::IM_A_TEAPOT);
    }

    async fn send(same_origin: SameOrigin, request: axum::http::request::Builder) -> StatusCode {
        let router = same_origin.layer(Router::new().route("/", any(|| async { "ok" })));
        let request = request.body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap().status()
    }
}
