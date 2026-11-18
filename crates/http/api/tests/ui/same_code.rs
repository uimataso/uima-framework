use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("http")]
    HTTPError,
    #[error("http")]
    HttpError,
}

fn main() {}
