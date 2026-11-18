use uima_http_api_error::{ApiError, FrameworkError};

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error(transparent)]
    #[api(transparent, code = "framework")]
    Framework(FrameworkError),
}

fn main() {}
