use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("moved")]
    #[api(status = 301)]
    Moved,
}

fn main() {}
