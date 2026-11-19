use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("bad")]
    Bad(u32, u32),
}

fn main() {}
