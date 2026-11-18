use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(code = "Gone_Now")]
    Gone,
}

fn main() {}
