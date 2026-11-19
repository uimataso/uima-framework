//! A JSON API on axum, answering its errors with the body of `uima-http-api`.
//!
//! - An app's error derives [`IntoResponse`](macro@IntoResponse) beside `ApiError`, so a handler
//!   returns it.
//! - [`ApiJson`], [`ApiQuery`] and [`ApiPath`] take the place of axum's, so a request they refuse
//!   answers one of `FrameworkError`'s codes. Any other extractor refuses with axum's own text.
//! - [`with_fallbacks`] answers a path or method no route takes.
//!
//! # Examples
//!
//! ```
//! use axum::Router;
//! use axum::routing::post;
//! use uima_http_api::{ApiError, FrameworkError};
//! use uima_http_axum_api::{ApiJson, IntoResponse, with_fallbacks};
//!
//! #[derive(Debug, thiserror::Error, ApiError, IntoResponse)]
//! enum AppError {
//!     #[error("Name taken")]
//!     #[api(status = 409)]
//!     NameTaken,
//!
//!     #[error(transparent)]
//!     #[api(transparent)]
//!     Framework(#[from] FrameworkError),
//! }
//!
//! #[derive(serde::Deserialize, serde::Serialize)]
//! struct User {
//!     name: String,
//! }
//!
//! async fn create(ApiJson(user): ApiJson<User>) -> Result<ApiJson<User>, AppError> {
//!     if user.name == "root" {
//!         return Err(AppError::NameTaken);
//!     }
//!     Ok(ApiJson(user))
//! }
//!
//! let router: Router = with_fallbacks(Router::new().route("/users", post(create)));
//! ```

mod extract;

#[cfg(test)]
mod tests;

#[doc(hidden)]
pub mod __private {
    pub use axum;
    pub use uima_http_api::ApiError;
}

use std::fmt;

use axum::response::{IntoResponse as _, Response};
use axum::routing::Router;
use uima_http_api::{ApiError, ErrorResponse, FrameworkError};
use uima_http_axum::LoggedError;

pub use extract::{ApiJson, ApiPath, ApiQuery, ApiRejection};
pub use uima_http_axum_api_macro::IntoResponse;

/// The response to `error`: its status and its [`ErrorResponse`]. A 5xx response also carries
/// the error's `{:?}` as a [`LoggedError`], for `uima_http_axum::Trace` to log.
///
/// [`IntoResponse`](macro@IntoResponse) derives an `IntoResponse` that calls it.
pub fn error_response(error: &(impl ApiError + fmt::Debug + ?Sized)) -> Response {
    let status = error.status();
    let mut response = (status, axum::Json(ErrorResponse::new(error))).into_response();
    if status.is_server_error() {
        response.extensions_mut().insert(LoggedError::new(error));
    }
    response
}

/// `router` with a path it has no route for answering `not-found`, and a method its path doesn't
/// take answering `method-not-allowed`.
///
/// Call it on the outermost router, once every route is added. `method-not-allowed` covers only
/// the routes already there, so a route added later, or one of a router that this one is merged
/// into, answers axum's empty 405. A router that nests this one answers its own unknown paths with
/// axum's empty 404.
pub fn with_fallbacks<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .fallback(|| async { ApiRejection(FrameworkError::NotFound) })
        .method_not_allowed_fallback(|| async { ApiRejection(FrameworkError::MethodNotAllowed) })
}
