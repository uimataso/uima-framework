//! `#[derive(IntoResponse)]` from an app's side.

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use serde_json::{Value, json};
use uima_http_api::{ApiError, FrameworkError};
use uima_http_axum_api::IntoResponse;
use uima_testing::check;

#[derive(Debug, thiserror::Error, ApiError, IntoResponse)]
enum AppError {
    #[error("Name taken")]
    #[api(status = 409)]
    NameTaken { name: String },

    #[error(transparent)]
    #[api(transparent)]
    Framework(#[from] FrameworkError),
}

async fn body(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn an_app_error_answers_its_status_and_body() {
    let error = AppError::NameTaken {
        name: "root".into(),
    };

    let answer = body(error.into_response()).await;

    let expected =
        json!({"error_code": "name-taken", "message": "Name taken", "data": {"name": "root"}});
    check!(answer == (StatusCode::CONFLICT, expected));
}

#[tokio::test]
async fn a_framework_error_answers_through_the_app_error() {
    let (status, body) = body(AppError::from(FrameworkError::Timeout).into_response()).await;

    check!(status == StatusCode::SERVICE_UNAVAILABLE);
    check!(body["error_code"] == "timeout");
}

mod renamed {
    use uima_http_api as api;
    use uima_http_axum_api as axum_api;

    use super::*;

    #[derive(Debug, thiserror::Error, api::ApiError, axum_api::IntoResponse)]
    #[api(crate = api)]
    #[into_response(crate = axum_api)]
    enum Renamed {
        #[error("gone")]
        #[api(status = 410)]
        Gone,
    }

    #[test]
    fn crate_names_a_renamed_dependency() {
        check!(Renamed::Gone.into_response().status() == StatusCode::GONE);
    }
}

/// An error generic over its detail, with `ApiError` by hand, as the derive takes no generics.
#[derive(Debug, IntoResponse)]
struct Gone<T>(T);

impl<T> std::fmt::Display for Gone<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("gone")
    }
}

impl<T> ApiError for Gone<T> {
    fn status(&self) -> StatusCode {
        StatusCode::GONE
    }

    fn error_code(&self) -> &'static str {
        "gone"
    }

    fn data(&self) -> Option<Value> {
        None
    }

    fn code_schemas() -> Vec<uima_http_api::CodeSchema> {
        Vec::new()
    }
}

#[test]
fn a_generic_type_derives_it() {
    check!(Gone(1).into_response().status() == StatusCode::GONE);
}

#[test]
fn the_body_type_is_axums() {
    let _: Response<Body> = AppError::from(FrameworkError::NotFound).into_response();
}

mod shadowed {
    #![allow(dead_code, reason = "only here to shadow the crate's name")]

    /// A module with the crate's name, which a relative path in the derive would find first.
    mod uima_http_axum_api {}

    use super::*;

    #[derive(Debug, thiserror::Error, ApiError, ::uima_http_axum_api::IntoResponse)]
    enum Shadowed {
        #[error("gone")]
        #[api(status = 410)]
        Gone,
    }

    #[test]
    fn a_module_named_like_the_crate_doesnt_shadow_it() {
        check!(Shadowed::Gone.into_response().status() == StatusCode::GONE);
    }
}
