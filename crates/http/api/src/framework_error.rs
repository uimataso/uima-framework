//! [`FrameworkError`], the framework's error codes.

use std::fmt;

use uima_core::InvalidPage;

use crate::ApiError;

/// The errors the framework answers with, for an app's error enum to wrap in a `transparent`
/// variant.
#[derive(Debug, ApiError)]
#[api(crate = crate)]
#[non_exhaustive]
pub enum FrameworkError {
    /// The body isn't of a type the route takes, such as JSON without its `content-type`.
    #[api(status = 415, no_data)]
    UnsupportedMediaType { message: String },
    /// The body isn't valid JSON.
    #[api(status = 400, no_data)]
    InvalidJson { message: String },
    /// The body is JSON, but not what the route takes, such as with a field missing or of another
    /// type.
    #[api(status = 422, no_data)]
    InvalidBody { message: String },
    /// The body is over the route's limit.
    #[api(status = 413, no_data)]
    BodyTooLarge { message: String },
    /// The query string isn't what the route takes.
    #[api(status = 400, no_data)]
    InvalidQuery { message: String },
    /// A path parameter isn't what the route takes.
    #[api(status = 400, no_data)]
    InvalidPath { message: String },
    /// No such path, or nothing at it.
    #[api(status = 404)]
    NotFound,
    /// The path doesn't take the method.
    #[api(status = 405)]
    MethodNotAllowed,
    /// A page's limit is 0.
    #[api(status = 400)]
    InvalidLimit,
    /// A request names or asks for more items than the route takes, such as a page limit above
    /// the max.
    #[api(status = 400)]
    TooManyItems { limit: usize, actual: usize },
    /// The service couldn't answer in time, such as waiting for a database connection.
    #[api(status = 503)]
    Timeout,
    /// Anything else. Its error is never answered, but in `data` when this crate is built with
    /// `debug_assertions`.
    #[api(status = 500, debug_data)]
    InternalError(anyhow::Error),
}

impl fmt::Display for FrameworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMediaType { message }
            | Self::InvalidJson { message }
            | Self::InvalidBody { message }
            | Self::BodyTooLarge { message }
            | Self::InvalidQuery { message }
            | Self::InvalidPath { message } => f.write_str(message),
            Self::NotFound => f.write_str("not found"),
            Self::MethodNotAllowed => f.write_str("the path doesn't take this method"),
            Self::InvalidLimit => f.write_str("the limit must be at least 1"),
            Self::TooManyItems { limit, actual } => {
                write!(f, "{actual} items, over the limit of {limit}")
            }
            Self::Timeout => f.write_str("the request took too long"),
            Self::InternalError(_) => f.write_str("internal error"),
        }
    }
}

impl std::error::Error for FrameworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InternalError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for FrameworkError {
    fn from(e: anyhow::Error) -> Self {
        Self::InternalError(e)
    }
}

/// `invalid-limit` for a limit of 0, and `too-many-items` for one above the rule's max.
///
/// `?` converts only once, so an app error that wraps [`FrameworkError`] converts the refusal
/// through it:
///
/// ```
/// use uima_core::{InvalidPage, OffsetQuery, PageRule};
/// use uima_http_api::{ApiError, FrameworkError};
///
/// #[derive(Debug, thiserror::Error, ApiError)]
/// enum AppError {
///     #[error(transparent)]
///     #[api(transparent)]
///     Framework(#[from] FrameworkError),
/// }
///
/// impl From<InvalidPage> for AppError {
///     fn from(e: InvalidPage) -> Self {
///         FrameworkError::from(e).into()
///     }
/// }
///
/// const RULE: PageRule = PageRule::new(20, 100);
///
/// fn page(limit: Option<u32>, offset: Option<u32>) -> Result<OffsetQuery, AppError> {
///     Ok(RULE.offset_query(limit, offset)?)
/// }
///
/// assert_eq!(page(Some(0), None).unwrap_err().error_code(), "invalid-limit");
/// ```
impl From<InvalidPage> for FrameworkError {
    fn from(e: InvalidPage) -> Self {
        match e {
            InvalidPage::ZeroLimit => Self::InvalidLimit,
            InvalidPage::AboveMax { received, max } => Self::TooManyItems {
                limit: usize::try_from(max).expect("a u32 fits in a usize"),
                actual: usize::try_from(received).expect("a u32 fits in a usize"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use serde_json::json;
    use uima_core::PageRule;
    use uima_testing::check;

    use super::*;
    use crate::ErrorResponse;

    #[test]
    fn a_zero_limit_is_an_invalid_limit_and_one_above_the_max_too_many_items() {
        const RULE: PageRule = PageRule::new(20, 100);
        let zero = RULE.offset_query(Some(0), None).unwrap_err();
        let above = RULE.offset_query(Some(250), None).unwrap_err();

        check!(let FrameworkError::InvalidLimit = zero.into());
        check!(let FrameworkError::TooManyItems { limit: 100, actual: 250 } = above.into());
    }

    #[test]
    fn each_code_has_one_status() {
        let message = || "m".to_owned();
        for (error, code, status) in [
            (
                FrameworkError::UnsupportedMediaType { message: message() },
                "unsupported-media-type",
                415,
            ),
            (
                FrameworkError::InvalidJson { message: message() },
                "invalid-json",
                400,
            ),
            (
                FrameworkError::InvalidBody { message: message() },
                "invalid-body",
                422,
            ),
            (
                FrameworkError::BodyTooLarge { message: message() },
                "body-too-large",
                413,
            ),
            (
                FrameworkError::InvalidQuery { message: message() },
                "invalid-query",
                400,
            ),
            (
                FrameworkError::InvalidPath { message: message() },
                "invalid-path",
                400,
            ),
            (FrameworkError::NotFound, "not-found", 404),
            (FrameworkError::MethodNotAllowed, "method-not-allowed", 405),
            (FrameworkError::InvalidLimit, "invalid-limit", 400),
            (
                FrameworkError::TooManyItems {
                    limit: 1,
                    actual: 2,
                },
                "too-many-items",
                400,
            ),
            (FrameworkError::Timeout, "timeout", 503),
            (
                FrameworkError::InternalError(anyhow::anyhow!("x")),
                "internal-error",
                500,
            ),
        ] {
            check!(error.error_code() == code);
            let has_data = matches!(error, FrameworkError::TooManyItems { .. })
                || matches!(error, FrameworkError::InternalError(_)) && cfg!(debug_assertions);
            check!(error.data().is_some() == has_data, "{code}");
            check!(
                error.status() == StatusCode::from_u16(status).unwrap(),
                "{code}"
            );
        }
    }

    #[test]
    fn a_fixed_message_says_what_went_wrong() {
        check!(FrameworkError::NotFound.to_string() == "not found");
        check!(FrameworkError::MethodNotAllowed.to_string() == "the path doesn't take this method");
        check!(FrameworkError::InvalidLimit.to_string() == "the limit must be at least 1");
        check!(FrameworkError::Timeout.to_string() == "the request took too long");
    }

    #[test]
    fn an_internal_error_keeps_its_source() {
        let error = FrameworkError::from(anyhow::anyhow!("db down"));
        let source = std::error::Error::source(&error).map(ToString::to_string);
        check!(source.as_deref() == Some("db down"));
    }

    #[test]
    fn a_rejection_answers_its_message_and_no_data() {
        let error = FrameworkError::InvalidBody {
            message: "missing field `name`".into(),
        };
        check!(
            serde_json::to_value(ErrorResponse::new(&error)).unwrap()
                == json!({
                    "error_code": "invalid-body",
                    "message": "missing field `name`",
                    "data": null,
                })
        );
    }

    #[test]
    fn an_internal_error_answers_its_chain_only_in_a_debug_build() {
        let error = FrameworkError::from(anyhow::anyhow!("db down").context("load user"));
        let body = serde_json::to_value(ErrorResponse::new(&error)).unwrap();
        check!(body["message"] == "internal error");
        if cfg!(debug_assertions) {
            let data = body["data"].as_str().unwrap();
            // With `RUST_BACKTRACE`, a backtrace follows.
            check!(data.starts_with("load user\n\nCaused by:\n    db down"));
        } else {
            check!(body["data"].is_null());
        }
    }
}
