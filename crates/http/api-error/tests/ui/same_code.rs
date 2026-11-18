use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("http")]
    HTTPError,
    #[error("http")]
    HttpError,
}

fn main() {}
