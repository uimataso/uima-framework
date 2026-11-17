use std::future::Ready;
use std::task::{Context, Poll, Waker};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::response::IntoResponse as _;
use axum::routing::post;
use tower::{Layer as _, Service, ServiceBuilder, ServiceExt as _};
use uima_log_tracing::testing::LogCapture;
use uima_testing::check;

use crate::{OriginAuth, OriginSecret, SameOrigin, Trace};

const SECRET: &str = "s3cret-s3cret-s3cret-s3cret-s3cret";

#[tokio::test]
async fn trace_first_logs_refused_requests_too() {
    let secret = OriginSecret::new(SECRET).unwrap();
    let router = Router::new().route("/", post(|| async { "ok" })).layer(
        ServiceBuilder::new()
            .layer(Trace::new())
            .layer(OriginAuth::new(Some(secret)))
            .layer(SameOrigin::new()),
    );
    let requests = [
        Request::post("/"),
        Request::post("/")
            .header("x-origin-auth", SECRET)
            .header("sec-fetch-site", "cross-site"),
    ];
    for request in requests {
        let logs = LogCapture::start();

        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();

        check!(response.status() == StatusCode::FORBIDDEN);
        check!(response.headers().contains_key("x-request-id"));
        let events = logs.events();
        check!(
            events
                .iter()
                .any(|event| event.spans.iter().any(|span| span.name == "request"))
        );
    }
}

#[tokio::test]
async fn each_layer_goes_on_a_method_router() {
    let secret = OriginSecret::new(SECRET).unwrap();
    let router: Router = Router::new()
        .route("/trace", post(|| async { "ok" }).layer(Trace::new()))
        .route(
            "/origin-auth",
            post(|| async { "ok" }).layer(OriginAuth::new(Some(secret))),
        )
        .route(
            "/same-origin",
            post(|| async { "ok" }).layer(SameOrigin::new()),
        );
    let cases = [
        ("/trace", StatusCode::OK),
        ("/origin-auth", StatusCode::FORBIDDEN),
        ("/same-origin", StatusCode::FORBIDDEN),
    ];
    for (path, status) in cases {
        let request = Request::post(path)
            .header("sec-fetch-site", "cross-site")
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        check!(response.status() == status, "{path}");
    }
}

#[test]
fn each_layer_passes_on_whether_the_service_inside_is_ready() {
    let secret = OriginSecret::new(SECRET).unwrap();
    for inner in [
        Poll::Pending,
        Poll::Ready(Ok(())),
        Poll::Ready(Err("broken")),
    ] {
        let mut trace = Trace::new().layer(Readiness(inner));
        check!(readiness(&mut trace) == inner);
        let mut auth = OriginAuth::new(Some(secret.clone())).layer(Readiness(inner));
        check!(readiness(&mut auth) == inner);
        let mut same_origin = SameOrigin::new().layer(Readiness(inner));
        check!(readiness(&mut same_origin) == inner);
    }
}

fn readiness<S>(service: &mut S) -> Poll<Result<(), &'static str>>
where
    S: Service<Request<Body>, Error = &'static str>,
{
    service.poll_ready(&mut Context::from_waker(Waker::noop()))
}

/// A service whose readiness is the one it is given.
#[derive(Debug, Clone)]
struct Readiness(Poll<Result<(), &'static str>>);

impl Service<Request<Body>> for Readiness {
    type Response = Response<Body>;
    type Error = &'static str;
    type Future = Ready<Result<Response<Body>, &'static str>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0
    }

    fn call(&mut self, _: Request<Body>) -> Self::Future {
        std::future::ready(Ok(Response::default()))
    }
}

#[tokio::test]
async fn origin_auth_outside_same_origin_refuses_a_bypass_with_no_body() {
    let secret = OriginSecret::new(SECRET).unwrap();
    let same_origin =
        SameOrigin::new().refuse_with(|| (StatusCode::IM_A_TEAPOT, "go away").into_response());
    let router: Router = Router::new().route("/", post(|| async { "ok" })).layer(
        ServiceBuilder::new()
            .layer(OriginAuth::new(Some(secret)))
            .layer(same_origin),
    );
    let request = Request::post("/")
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    check!(response.status() == StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(response.into_body(), 1024).await;
    check!(body.unwrap().is_empty());
}
