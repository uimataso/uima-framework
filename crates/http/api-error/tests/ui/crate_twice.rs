use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
#[api(crate = nonexistent, crate = ::uima_http_api_error)]
enum AppError {
    #[error("gone")]
    Gone,
}

fn main() {}
