use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(code = "a", code = "b")]
    Gone,
}

fn main() {}
