//! [`OriginAuth`], refusing requests that bypass the proxy in front.

use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use tracing::warn;
use uima_core::Secret;

/// Refuses a request that doesn't carry the secret the proxy in front, such as Cloudflare, adds in
/// the [`HEADER`](Self::HEADER) header, so a request sent straight to the host gets a bare `403`.
///
/// With no secret the check is off and every request passes, for running without the proxy.
#[derive(Debug, Clone)]
pub struct OriginAuth {
    secret: Option<Arc<Secret<String>>>,
    exempt_paths: Vec<String>,
}

impl OriginAuth {
    /// The header the proxy puts the secret in.
    pub const HEADER: &str = "x-origin-auth";

    /// Checks for `secret`. With `None` or an empty secret the check is off, and this logs a
    /// warning saying so.
    pub fn new(secret: Option<Secret<String>>) -> Self {
        let secret = secret.filter(|secret| !secret.expose().is_empty());
        if secret.is_none() {
            warn!(
                "no origin auth secret, accepting requests that bypass the proxy in front of the \
                 service"
            );
        }
        Self {
            secret: secret.map(Arc::new),
            exempt_paths: Vec::new(),
        }
    }

    /// Lets requests to `paths` through without the secret, such as the health check the host
    /// sends itself.
    pub fn exempt<P: Into<String>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.exempt_paths.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Whether a request to `path` carrying `given` in the [`HEADER`](Self::HEADER) header may
    /// pass, for a server that isn't on axum.
    pub fn allows(&self, path: &str, given: Option<&[u8]>) -> bool {
        let Some(secret) = &self.secret else {
            return true;
        };
        self.exempt_paths.iter().any(|exempt| exempt == path)
            || given.is_some_and(|given| constant_time_eq(given, secret.expose().as_bytes()))
    }

    /// Wraps `router`'s routes and its fallback.
    pub fn layer<S: Clone + Send + Sync + 'static>(self, router: Router<S>) -> Router<S> {
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let given = request
                .headers()
                .get(Self::HEADER)
                .map(|value| value.as_bytes());
            let allowed = self.allows(request.uri().path(), given);
            async move {
                if allowed {
                    next.run(request).await
                } else {
                    StatusCode::FORBIDDEN.into_response()
                }
            }
        }))
    }
}

/// Compares in a time that doesn't depend on how much of `a` matches `b`, so the response time
/// doesn't leak the secret.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt as _;
    use tracing::Level;
    use uima_log_tracing::testing::LogCapture;
    use uima_testing::check;

    use super::*;

    #[tokio::test]
    async fn only_the_secret_passes() {
        let auth = OriginAuth::new(Some(Secret::new("s3cret".to_owned())));
        check!(send(auth.clone(), "/", Some("s3cret")).await == StatusCode::OK);
        check!(send(auth.clone(), "/", Some("s3cre")).await == StatusCode::FORBIDDEN);
        check!(send(auth.clone(), "/", Some("s3creT")).await == StatusCode::FORBIDDEN);
        check!(send(auth.clone(), "/", Some("")).await == StatusCode::FORBIDDEN);
        check!(send(auth, "/", None).await == StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn exempt_paths_pass_without_the_secret() {
        let auth = OriginAuth::new(Some(Secret::new("s3cret".to_owned()))).exempt(["/healthz"]);
        check!(send(auth.clone(), "/healthz", None).await == StatusCode::OK);
        check!(send(auth, "/", None).await == StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn no_secret_turns_the_check_off_with_a_warning() {
        for secret in [None, Some(Secret::new(String::new()))] {
            let logs = LogCapture::start();
            let auth = OriginAuth::new(secret);
            check!(logs.has_message(Level::WARN, "no origin auth secret"));
            check!(send(auth.clone(), "/", None).await == StatusCode::OK);
            check!(send(auth, "/", Some("")).await == StatusCode::OK);
        }
    }

    #[test]
    fn the_secret_is_kept_out_of_debug() {
        let auth = OriginAuth::new(Some(Secret::new("s3cret".to_owned())));
        check!(!format!("{auth:?}").contains("s3cret"));
    }

    async fn send(auth: OriginAuth, path: &str, given: Option<&str>) -> StatusCode {
        let router = auth.layer(
            Router::new()
                .route("/", get(|| async { "ok" }))
                .route("/healthz", get(|| async { "ok" })),
        );
        let mut request = Request::get(path);
        if let Some(given) = given {
            request = request.header(OriginAuth::HEADER, given);
        }
        let request = request.body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap().status()
    }
}
