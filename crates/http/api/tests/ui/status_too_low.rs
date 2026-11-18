use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("odd")]
    #[api(status = 399)]
    Odd,
}

fn main() {}
