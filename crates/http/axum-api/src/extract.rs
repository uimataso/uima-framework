//! [`ApiJson`], [`ApiQuery`] and [`ApiPath`], axum's extractors refusing a request with the
//! framework's error codes.

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uima_http_api_error::FrameworkError;

/// [`axum::Json`], refusing a body with `unsupported-media-type` without a JSON content type,
/// `invalid-json` when it doesn't parse, `invalid-body` when it isn't what `T` takes, and
/// `body-too-large` over axum's limit. As a response it is `T` as JSON, or `internal-error` when
/// `T` fails to serialize.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

/// [`axum::extract::Query`], refusing a query that isn't what `T` takes with `invalid-query`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiQuery<T>(pub T);

/// [`axum::extract::Path`], refusing path parameters that aren't what `T` takes with
/// `invalid-path`.
#[derive(Debug)]
pub struct ApiPath<T>(pub T);

/// A request [`ApiJson`], [`ApiQuery`], [`ApiPath`] or [`with_fallbacks`](crate::with_fallbacks)
/// refused, answering its error.
#[derive(Debug)]
pub struct ApiRejection(pub(crate) FrameworkError);

impl IntoResponse for ApiRejection {
    fn into_response(self) -> Response {
        crate::error_response(&self.0)
    }
}

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::from_request(request, state)
            .await
            .map_err(|rejection| ApiRejection(json_error(rejection)))?;
        Ok(Self(value))
    }
}

impl<T: DeserializeOwned> ApiJson<T> {
    /// [`axum::Json::from_bytes`], for a body read as bytes first, refusing it as the extractor
    /// does.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FrameworkError> {
        let axum::Json(value) = axum::Json::from_bytes(bytes).map_err(json_error)?;
        Ok(Self(value))
    }
}

impl<T: Serialize> IntoResponse for ApiJson<T> {
    fn into_response(self) -> Response {
        match serde_json::to_vec(&self.0) {
            Ok(body) => {
                let json = HeaderValue::from_static("application/json");
                ([(CONTENT_TYPE, json)], body).into_response()
            }
            Err(e) => {
                let e = anyhow::Error::new(e).context("serialize the response body");
                crate::error_response(&FrameworkError::InternalError(e))
            }
        }
    }
}

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Query(value) = axum::extract::Query::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiRejection(query_error(rejection)))?;
        Ok(Self(value))
    }
}

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(value) = axum::extract::Path::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiRejection(path_error(rejection)))?;
        Ok(Self(value))
    }
}

fn json_error(rejection: JsonRejection) -> FrameworkError {
    let message = rejection.body_text();
    match rejection {
        JsonRejection::MissingJsonContentType(_) => {
            FrameworkError::UnsupportedMediaType { message }
        }
        JsonRejection::JsonSyntaxError(_) => FrameworkError::InvalidJson { message },
        JsonRejection::JsonDataError(_) => FrameworkError::InvalidBody { message },
        // The body failed to arrive, over the limit or with a broken stream, both 4xx.
        other if other.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            FrameworkError::BodyTooLarge { message }
        }
        _ => FrameworkError::InvalidBody { message },
    }
}

fn query_error(rejection: QueryRejection) -> FrameworkError {
    FrameworkError::InvalidQuery {
        message: rejection.body_text(),
    }
}

/// `invalid-path`, or an internal error when axum answers the rejection with a 5xx, such as for a
/// route whose path has fewer parameters than its `ApiPath` takes.
fn path_error(rejection: PathRejection) -> FrameworkError {
    let message = rejection.body_text();
    if rejection.status().is_server_error() {
        return FrameworkError::InternalError(anyhow::anyhow!(message));
    }
    FrameworkError::InvalidPath { message }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde::Deserialize;
    use serde_json::Value;
    use uima_http_axum::LoggedError;
    use uima_testing::{assert, check};

    use super::*;

    #[derive(Debug, Deserialize, Serialize)]
    struct Item {
        n: u32,
    }

    #[test]
    fn json_from_bytes_refuses_as_the_extractor_does() {
        check!(let Ok(ApiJson(Item { n: 1 })) = ApiJson::<Item>::from_bytes(br#"{"n":1}"#));
        check!(let Err(FrameworkError::InvalidJson { .. }) = ApiJson::<Item>::from_bytes(b"{"));
        let mistyped = ApiJson::<Item>::from_bytes(br#"{"n":"one"}"#);
        check!(let Err(FrameworkError::InvalidBody { .. }) = mistyped);
    }

    #[tokio::test]
    async fn a_json_response_that_doesnt_serialize_is_an_internal_error() {
        let map = std::collections::HashMap::from([((1, 2), 3)]);

        let response = ApiJson(map).into_response();

        check!(response.status() == StatusCode::INTERNAL_SERVER_ERROR);
        assert!(let Some(logged) = response.extensions().get::<LoggedError>());
        check!(format!("{logged:?}").contains("serialize the response body"));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        check!(body["error_code"] == "internal-error");
    }

    #[tokio::test]
    async fn a_json_response_is_json() {
        let response = ApiJson(Item { n: 1 }).into_response();

        check!(response.headers()[CONTENT_TYPE] == "application/json");
    }
}
