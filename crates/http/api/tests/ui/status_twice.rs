use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(status = 404)]
    #[api(status = 410)]
    Gone,
}

fn main() {}
