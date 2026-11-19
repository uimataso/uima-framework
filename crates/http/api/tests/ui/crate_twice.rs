use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
#[api(crate = nonexistent, crate = ::uima_http_api)]
enum AppError {
    #[error("gone")]
    Gone,
}

fn main() {}
