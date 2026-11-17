use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use tower::{ServiceBuilder, ServiceExt as _};
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
