use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    Gone,
    #[error("gone")]
    #[api(code = "gone")]
    AlsoGone,
}

fn main() {}
