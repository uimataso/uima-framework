use uima_http_api_error::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("internal")]
    #[api(status = 500, debug_data)]
    Internal { error: String },
}

fn main() {}
