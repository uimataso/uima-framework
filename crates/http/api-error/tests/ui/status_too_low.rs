use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("odd")]
    #[api(status = 399)]
    Odd,
}

fn main() {}
