use uima_http_axum_api::IntoResponse;

#[derive(Debug, IntoResponse)]
#[into_response(status = 400)]
struct Unknown;

fn main() {}
