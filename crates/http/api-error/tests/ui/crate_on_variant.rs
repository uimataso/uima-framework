use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(crate = ::uima_http_api_error)]
    Gone,
}

fn main() {}
