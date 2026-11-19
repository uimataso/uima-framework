use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError<T: std::fmt::Debug> {
    #[error("bad")]
    Bad(T),
}

fn main() {}
