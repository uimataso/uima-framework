use uima_http_api_error::ApiError;

#[derive(Debug, serde::Serialize)]
struct Detail;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("bad")]
    Bad(Detail),
}

fn main() {}
