use uima_http_api_error::{ApiError, FrameworkError};

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error(transparent)]
    #[api(transparent, status = 404)]
    Framework(FrameworkError),
}

fn main() {}
