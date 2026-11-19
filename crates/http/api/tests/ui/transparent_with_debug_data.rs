use uima_http_api::{ApiError, FrameworkError};

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error(transparent)]
    #[api(transparent, debug_data)]
    Framework(FrameworkError),
}

fn main() {}
