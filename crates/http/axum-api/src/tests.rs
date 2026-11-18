use axum::body::{Body, Bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, Request, StatusCode};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use tracing::Level;
use uima_log_tracing_subscriber::testing::LogCapture;
use uima_testing::{assert, check};

use super::*;

#[derive(Debug, Deserialize, Serialize)]
struct Item {
    n: u32,
}

fn router() -> Router {
    let router = Router::new()
        .route(
            "/json",
            post(|ApiJson(item): ApiJson<Item>| async move { ApiJson(item) }),
        )
        .route(
            "/query",
            get(|ApiQuery(item): ApiQuery<Item>| async move { item.n.to_string() }),
        )
        .route(
            "/path/{n}",
            get(|ApiPath(n): ApiPath<u32>| async move { n.to_string() }),
        )
        // A route whose path has fewer parameters than its `ApiPath` takes.
        .route(
            "/short/{n}",
            get(|ApiPath((n, _)): ApiPath<(u32, u32)>| async move { n.to_string() }),
        );
    with_fallbacks(router)
}

async fn send(method: Method, uri: &str, ty: Option<&str>, body: Body) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(ty) = ty {
        request = request.header(CONTENT_TYPE, ty);
    }
    let response = router().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn post_json(ty: Option<&str>, body: impl Into<Body>) -> (StatusCode, Value) {
    send(Method::POST, "/json", ty, body.into()).await
}

async fn get_uri(uri: &str) -> (StatusCode, Value) {
    send(Method::GET, uri, None, Body::empty()).await
}

/// The error body: `error_code`, a `message` saying what went wrong, and no `data`.
fn check_error(body: &Value, code: &str) {
    check!(body["error_code"] == code);
    check!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    check!(body["data"] == Value::Null);
}

#[tokio::test]
async fn a_good_json_body_goes_through_and_back() {
    let (status, body) = post_json(Some("application/json"), r#"{"n":1}"#).await;

    check!(status == StatusCode::OK);
    check!(body == json!({"n": 1}));
}

#[tokio::test]
async fn a_body_without_a_json_content_type_is_a_415() {
    for ty in [None, Some("text/plain")] {
        let (status, body) = post_json(ty, r#"{"n":1}"#).await;

        check!(status == StatusCode::UNSUPPORTED_MEDIA_TYPE);
        check_error(&body, "unsupported-media-type");
    }
}

#[tokio::test]
async fn a_json_body_that_doesnt_parse_is_a_400() {
    let (status, body) = post_json(Some("application/json"), "{").await;

    check!(status == StatusCode::BAD_REQUEST);
    check_error(&body, "invalid-json");
}

#[tokio::test]
async fn a_missing_or_mistyped_field_is_a_422_naming_it() {
    for bad in [r#"{}"#, r#"{"n":"one"}"#] {
        let (status, body) = post_json(Some("application/json"), bad).await;

        check!(status == StatusCode::UNPROCESSABLE_ENTITY);
        check_error(&body, "invalid-body");
        check!(body["message"].as_str().unwrap().contains('n'));
    }
}

#[tokio::test]
async fn a_json_body_over_2_mb_is_a_413() {
    let pad = |len| format!(r#"{{"n":1,"pad":"{}"}}"#, "x".repeat(len));

    let (status, body) = post_json(Some("application/json"), pad(2 * 1024 * 1024)).await;
    check!(status == StatusCode::PAYLOAD_TOO_LARGE);
    check_error(&body, "body-too-large");

    let under = pad(2 * 1024 * 1024 - 20);
    check!(post_json(Some("application/json"), under).await.0 == StatusCode::OK);
}

#[tokio::test]
async fn a_bad_or_missing_query_is_a_400() {
    for uri in ["/query?n=one", "/query"] {
        let (status, body) = get_uri(uri).await;

        check!(status == StatusCode::BAD_REQUEST);
        check_error(&body, "invalid-query");
    }
    check!(get_uri("/query?n=2").await == (StatusCode::OK, json!(2)));
}

#[tokio::test]
async fn a_bad_path_parameter_is_a_400() {
    let (status, body) = get_uri("/path/one").await;

    check!(status == StatusCode::BAD_REQUEST);
    check_error(&body, "invalid-path");
    check!(body["message"] == "Invalid URL: Cannot parse `one` to a `u32`");
    check!(get_uri("/path/2").await == (StatusCode::OK, json!(2)));
}

#[tokio::test]
async fn a_path_the_route_cant_give_is_an_internal_error() {
    let request = Request::get("/short/1").body(Body::empty()).unwrap();
    let response = router().oneshot(request).await.unwrap();

    check!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(let Some(logged) = response.extensions().get::<LoggedError>());
    check!(format!("{logged:?}").contains("Wrong number of path arguments"));
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    check!(body["error_code"] == "internal-error");
    check!(body["message"] == "internal error");
}

#[tokio::test]
async fn a_merged_router_keeps_the_fallback_but_not_the_405() {
    let router = Router::new()
        .route("/outer", get(|| async {}))
        .merge(router());

    let unknown = Request::get("/nothing").body(Body::empty()).unwrap();
    let response = router.clone().oneshot(unknown).await.unwrap();
    check!(response.status() == StatusCode::NOT_FOUND);
    check!(response.headers().contains_key(CONTENT_TYPE));

    let method = Request::delete("/outer").body(Body::empty()).unwrap();
    let response = router.oneshot(method).await.unwrap();
    check!(response.status() == StatusCode::METHOD_NOT_ALLOWED);
    check!(!response.headers().contains_key(CONTENT_TYPE));
}

#[tokio::test]
async fn an_unknown_path_is_a_404() {
    let (status, body) = get_uri("/nothing").await;

    check!(status == StatusCode::NOT_FOUND);
    check_error(&body, "not-found");
}

#[tokio::test]
async fn a_method_the_path_doesnt_take_is_a_405() {
    let (status, body) = send(Method::DELETE, "/json", None, Body::empty()).await;

    check!(status == StatusCode::METHOD_NOT_ALLOWED);
    check_error(&body, "method-not-allowed");
}

#[test]
fn a_5xx_carries_the_error_to_log_and_a_4xx_doesnt() {
    let internal = FrameworkError::InternalError(anyhow::anyhow!("db down").context("load user"));
    let response = error_response(&internal);
    assert!(let Some(logged) = response.extensions().get::<LoggedError>());
    check!(format!("{logged:?}").contains("db down"));

    let response = error_response(&FrameworkError::NotFound);
    check!(response.extensions().get::<LoggedError>().is_none());
}

#[tokio::test]
async fn a_body_that_fails_to_arrive_is_invalid_body() {
    let chunks = [Ok(Bytes::from("{")), Err(std::io::Error::other("reset"))];
    let body = Body::from_stream(futures_util::stream::iter(chunks));

    let (status, body) = post_json(Some("application/json"), body).await;

    check!(status == StatusCode::UNPROCESSABLE_ENTITY);
    check_error(&body, "invalid-body");
}

#[tokio::test]
async fn the_message_is_axums_text() {
    let (_, body) = get_uri("/query?n=one").await;

    let message = "Failed to deserialize query string: n: invalid digit found in string";
    check!(body["message"] == message);
}

#[tokio::test]
async fn under_trace_a_5xx_logs_its_chain_once() {
    let fail = || async {
        let error = anyhow::anyhow!("db down").context("load user");
        ApiRejection(FrameworkError::InternalError(error))
    };
    let router = Router::new()
        .route("/", get(fail))
        .layer(uima_http_axum::Trace::new());
    let logs = LogCapture::start();

    let request = Request::get("/").body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();

    check!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
    let errors: Vec<_> = logs
        .events()
        .into_iter()
        .filter(|event| event.level == Level::ERROR)
        .collect();
    assert!(let [error] = &errors[..]);
    check!(error.fields["error"].contains("load user\n\nCaused by:\n    db down"));
}
