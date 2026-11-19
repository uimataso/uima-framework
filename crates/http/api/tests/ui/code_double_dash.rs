use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(code = "a--b")]
    Gone,
}

fn main() {}
