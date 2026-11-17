use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError<T: std::fmt::Debug> {
    #[error("bad")]
    Bad(T),
}

fn main() {}
