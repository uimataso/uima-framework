//! Layers every axum service puts around its routes: request tracing, refusing cross-site form
//! posts, and refusing requests that bypass the proxy in front.
//!
//! # Examples
//!
//! ```
//! use axum::Router;
//! use axum::routing::get;
//! use uima_core::Secret;
//! use uima_http_axum::{OriginAuth, SameOrigin, Trace};
//!
//! let secret = Some(Secret::new("from-the-env".to_owned()));
//! let router: Router = Router::new().route("/", get(|| async { "hi" }));
//! let router = SameOrigin::new().layer(router);
//! let router = OriginAuth::new(secret).exempt(["/healthz"]).layer(router);
//! let router = Trace::new().layer(router);
//! ```
//!
//! Each `layer` wraps the routes the router has so far, so call it after adding them. The last
//! layer is the outermost and sees a request first: put [`Trace`] last, so every request is
//! logged, refused ones included.

mod origin_auth;
mod same_origin;
mod trace;

pub use origin_auth::OriginAuth;
pub use same_origin::SameOrigin;
pub use trace::Trace;
