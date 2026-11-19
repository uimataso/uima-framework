use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    Gone,
    #[error("gone")]
    #[api(code = "gone")]
    AlsoGone,
}

fn main() {}
