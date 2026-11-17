//! [`OriginAuth`], refusing requests that bypass the proxy in front.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::response::IntoResponse as _;
use subtle::ConstantTimeEq as _;
use tower::Layer;
use tower_http::validate_request::{ValidateRequest, ValidateRequestHeader};
use tracing::warn;

use crate::paths::Paths;

uima_core::secret!(
    /// The secret the proxy in front adds to every request, 32 to 256 visible ASCII characters.
    pub OriginSecret,
    len = 32..=256,
    chars = |c| c.is_ascii_graphic(),
);

/// Refuses a request that doesn't carry the secret the proxy in front, such as Cloudflare, adds in
/// the [`HEADER`](Self::HEADER) header, so a request sent straight to the host gets a bare `403`.
///
/// A request that passes goes on without the header, so the secret doesn't reach the layers and
/// the handler inside this one.
#[derive(Debug, Clone)]
pub struct OriginAuth {
    // Shared, since axum clones the layer's service, this included, for every request.
    secret: Option<Arc<OriginSecret>>,
    exempt_paths: Paths,
}

impl OriginAuth {
    /// The header the proxy puts the secret in.
    pub const HEADER: &str = "x-origin-auth";

    /// Checks for `secret`. With `None` the check is off, for running without the proxy, and this
    /// logs a warning saying so.
    pub fn new(secret: Option<OriginSecret>) -> Self {
        if secret.is_none() {
            warn!(
                "no origin auth secret, accepting requests that bypass the proxy in front of the \
                 service"
            );
        }

        Self {
            secret: secret.map(Arc::new),
            exempt_paths: Paths::default(),
        }
    }

    /// Lets requests to `paths` through without the secret, such as the health check the host
    /// sends itself. A path matches as the [crate docs](crate#paths) say.
    ///
    /// # Panics
    ///
    /// If one of `paths` doesn't start with `/`.
    pub fn exempt<P: AsRef<str>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.exempt_paths = self.exempt_paths.with(paths);
        self
    }

    fn allows(&self, path: &str, given: Option<&[u8]>) -> bool {
        let Some(secret) = &self.secret else {
            return true;
        };

        // Constant-time for a given length, so the response time shows at most the length.
        self.exempt_paths.contains(path)
            || given.is_some_and(|given| given.ct_eq(secret.expose().as_bytes()).into())
    }
}

impl<S> Layer<S> for OriginAuth {
    type Service = ValidateRequestHeader<S, Self>;

    fn layer(&self, inner: S) -> Self::Service {
        ValidateRequestHeader::custom(inner, self.clone())
    }
}

impl<B> ValidateRequest<B> for OriginAuth {
    type ResponseBody = Body;

    fn validate(&mut self, request: &mut Request<B>) -> Result<(), Response<Body>> {
        let given = request
            .headers()
            .get(Self::HEADER)
            .map(|value| value.as_bytes());
        if !self.allows(request.uri().path(), given) {
            return Err(StatusCode::FORBIDDEN.into_response());
        }

        request.headers_mut().remove(Self::HEADER);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use proptest::prelude::*;
    use tower::ServiceExt as _;
    use tracing::Level;
    use uima_log_tracing::testing::LogCapture;
    use uima_testing::{assert, check};

    use super::*;

    const SECRET: &str = "s3cret-s3cret-s3cret-s3cret-s3cret";

    #[tokio::test]
    async fn only_the_secret_passes() {
        let auth = OriginAuth::new(Some(secret(SECRET)));
        check!(send(auth.clone(), "/", Some(SECRET)).await == StatusCode::OK);

        // the bytes differ by the same bit, so they'd cancel out in a careless compare
        let flipped = SECRET.replacen("s3", "r2", 1);
        let longer = format!("{SECRET}x");
        let upper = SECRET.to_uppercase();
        for given in [&flipped, &SECRET[1..], &longer, &upper, ""] {
            check!(
                send(auth.clone(), "/", Some(given)).await == StatusCode::FORBIDDEN,
                "{given}"
            );
        }
        check!(send(auth, "/", None).await == StatusCode::FORBIDDEN);
    }

    proptest! {
        #[test]
        fn only_the_exact_secret_is_allowed(
            secret in "[!-~]{32,64}",
            flips in prop::collection::vec((any::<prop::sample::Index>(), 1..=u8::MAX), 0..4),
            other in prop::collection::vec(any::<u8>(), 0..80),
        ) {
            let auth = OriginAuth::new(Some(OriginSecret::new(&secret).unwrap()));
            let mut near = secret.clone().into_bytes();
            for (index, mask) in &flips {
                near[index.index(secret.len())] ^= mask;
            }

            for given in [&near, &other] {
                check!(auth.allows("/", Some(given)) == (given == secret.as_bytes()));
            }
        }
    }

    #[tokio::test]
    async fn the_secret_doesnt_reach_the_handler() {
        let auth = OriginAuth::new(Some(secret(SECRET))).exempt(["/healthz"]);
        check!(send(auth.clone(), "/header", Some(SECRET)).await == StatusCode::OK);
        check!(send(auth, "/healthz", Some("anything")).await == StatusCode::OK);
    }

    #[tokio::test]
    async fn a_refusal_is_a_bare_403_also_on_the_fallback() {
        let auth = OriginAuth::new(Some(secret(SECRET)));
        for path in ["/", "/missing"] {
            let response = request(auth.clone(), path, None).await;
            check!(response.status() == StatusCode::FORBIDDEN, "{path}");
            let body = axum::body::to_bytes(response.into_body(), 1024).await;
            check!(body.unwrap().is_empty(), "{path}");
        }
        check!(send(auth, "/missing", Some(SECRET)).await == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn exempt_paths_match_exactly_and_add_up() {
        let auth = OriginAuth::new(Some(secret(SECRET)))
            .exempt(["/healthz"])
            .exempt(["/"]);
        check!(send(auth.clone(), "/healthz", None).await == StatusCode::OK);
        check!(send(auth.clone(), "/healthz?probe=1", None).await == StatusCode::OK);
        check!(send(auth.clone(), "/", None).await == StatusCode::OK);
        for path in ["/healthz/x", "/healthzz", "/HEALTHZ", "/header"] {
            check!(
                send(auth.clone(), path, None).await == StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn under_a_nest_exempt_paths_leave_out_its_prefix() {
        let nested = |exempt| {
            let auth = OriginAuth::new(Some(secret(SECRET))).exempt([exempt]);
            let inner = Router::new()
                .route("/healthz", get(|| async { "ok" }))
                .layer(auth);
            Router::new().nest("/api", inner)
        };
        let request = || Request::get("/api/healthz").body(Body::empty()).unwrap();

        let response = nested("/healthz").oneshot(request()).await.unwrap();
        check!(response.status() == StatusCode::OK);
        let response = nested("/api/healthz").oneshot(request()).await.unwrap();
        check!(response.status() == StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn no_secret_turns_the_check_off_with_a_warning() {
        let logs = LogCapture::start();
        let auth = OriginAuth::new(None);
        check!(logs.has_message(Level::WARN, "no origin auth secret"));
        check!(send(auth.clone(), "/", None).await == StatusCode::OK);
        check!(send(auth, "/", Some("anything")).await == StatusCode::OK);
    }

    #[test]
    fn a_secret_logs_no_warning() {
        let logs = LogCapture::start();
        OriginAuth::new(Some(secret(SECRET)));
        check!(!logs.has_message(Level::WARN, "no origin auth secret"));
    }

    #[test]
    fn a_secret_is_32_to_256_visible_ascii_characters() {
        let padded = format!("  {SECRET}\n");
        assert!(let Ok(trimmed) = OriginSecret::new(&padded));
        check!(trimmed.expose() == SECRET);
        check!(OriginSecret::new("a".repeat(32)).is_ok());
        check!(OriginSecret::new("a".repeat(256)).is_ok());
        check!(OriginSecret::new(format!("{SECRET}!~")).is_ok());

        let invalid = [
            "a".repeat(31),
            "a".repeat(257),
            format!("{SECRET} x"),
            format!("{SECRET}\tx"),
            format!("{SECRET}\u{1}"),
            format!("{SECRET}é"),
        ];
        for invalid in invalid {
            check!(OriginSecret::new(&invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn the_secret_is_kept_out_of_debug() {
        let auth = OriginAuth::new(Some(secret(SECRET)));
        check!(!format!("{auth:?}").contains("s3cret"));
    }

    fn secret(text: &str) -> OriginSecret {
        OriginSecret::new(text).unwrap()
    }

    async fn send(auth: OriginAuth, path: &str, given: Option<&str>) -> StatusCode {
        request(auth, path, given).await.status()
    }

    async fn request(auth: OriginAuth, path: &str, given: Option<&str>) -> Response<Body> {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/healthz", get(header_is_gone))
            .route("/header", get(header_is_gone))
            .layer(auth);
        let mut request = Request::get(path);
        if let Some(given) = given {
            request = request.header("x-origin-auth", given);
        }

        let request = request.body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap()
    }

    async fn header_is_gone(headers: HeaderMap) -> StatusCode {
        if headers.contains_key("x-origin-auth") {
            StatusCode::IM_A_TEAPOT
        } else {
            StatusCode::OK
        }
    }
}
