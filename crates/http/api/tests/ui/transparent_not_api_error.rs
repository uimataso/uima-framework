use uima_http_api::ApiError;

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error(transparent)]
    #[api(transparent)]
    Io(std::io::Error),
}

fn main() {}
