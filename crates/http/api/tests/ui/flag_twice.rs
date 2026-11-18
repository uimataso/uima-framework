use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("gone")]
    #[api(no_data, no_data)]
    Gone(String),
}

fn main() {}
