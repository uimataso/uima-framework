//! The error body of a JSON API, and its OpenAPI schema. Derive [`ApiError`](macro@ApiError) on an
//! app's error enum; its doc lists the attributes.
//!
//! # Examples
//!
//! ```
//! use uima_http_api_error::{ApiError, FrameworkError, ErrorResponse};
//!
//! #[derive(Debug, thiserror::Error, ApiError)]
//! enum AppError {
//!     #[error("Not found")]
//!     #[api(status = 404)]
//!     NotFound,
//!
//!     #[error("Payload too large")]
//!     #[api(status = 413)]
//!     PayloadTooLarge { max_size: usize, actual: usize },
//!
//!     #[error(transparent)]
//!     #[api(transparent)]
//!     Framework(#[from] FrameworkError),
//! }
//!
//! let error = AppError::PayloadTooLarge { max_size: 500, actual: 600 };
//! assert_eq!(error.status(), 413);
//! assert_eq!(
//!     serde_json::to_value(ErrorResponse::new(&error)).unwrap(),
//!     serde_json::json!({
//!         "error_code": "payload-too-large",
//!         "message": "Payload too large",
//!         "data": {"max_size": 500, "actual": 600},
//!     }),
//! );
//! ```
//!
//! # The body
//!
//! Every error answers `{"error_code", "message", "data"}`, all three always present.
//!
//! - `error_code` names the kind of error, in kebab case. A code always comes with the same status
//!   and the same shape of `data`.
//! - `message` is the error's `Display`, for a person to read.
//! - `data` is what a client needs to act on the error, or `null`.

mod framework_error;
mod schema;

#[doc(hidden)]
pub mod __private;

use http::StatusCode;
use serde::Serialize;
use serde_json::Value;

pub use framework_error::FrameworkError;
pub use schema::{CodeSchema, schema};
pub use uima_http_api_error_macro::ApiError;

/// An error a JSON API answers with an [`ErrorResponse`]; derive it with
/// [`ApiError`](macro@ApiError).
pub trait ApiError: std::fmt::Display {
    /// The response's status.
    fn status(&self) -> StatusCode;

    /// The body's `error_code`.
    fn error_code(&self) -> &'static str;

    /// The body's `data`; `None` answers `null`.
    fn data(&self) -> Option<Value>;

    /// Every error code the type can answer, for its schema.
    fn code_schemas() -> Vec<CodeSchema>
    where
        Self: Sized;
}

/// The body of an error's response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    error_code: &'static str,
    message: String,
    data: Option<Value>,
}

impl ErrorResponse {
    /// The body for `error`.
    pub fn new(error: &(impl ApiError + ?Sized)) -> Self {
        Self {
            error_code: error.error_code(),
            message: error.to_string(),
            data: error.data(),
        }
    }
}
