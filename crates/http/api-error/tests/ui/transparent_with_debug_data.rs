use uima_http_api_error::{ApiError, Error};

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error(transparent)]
    #[api(transparent, debug_data)]
    Framework(Error),
}

fn main() {}
