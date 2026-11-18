use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("odd")]
    #[api(status = 600)]
    Odd,
}

fn main() {}
