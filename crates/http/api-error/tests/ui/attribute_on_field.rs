use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("token")]
    Token(#[api(no_data)] String),
}

fn main() {}
