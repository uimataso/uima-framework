use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(crate = ::uima_http_api)]
    Gone,
}

fn main() {}
