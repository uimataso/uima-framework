use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
#[api(status = 404)]
enum AppError {
    #[error("gone")]
    Gone,
}

fn main() {}
