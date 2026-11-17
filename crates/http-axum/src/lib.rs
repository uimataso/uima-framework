//! The layers an axum service puts around its routes.
//!
//! # Examples
//!
//! ```
//! use axum::Router;
//! use axum::routing::get;
//! use tower::ServiceBuilder;
//! use uima_http_axum::{OriginAuth, OriginSecret, SameOrigin, Trace};
//!
//! let secret = OriginSecret::new("from-the-env-at-least-32-characters").unwrap();
//! let router: Router = Router::new().route("/", get(|| async { "hi" })).layer(
//!     ServiceBuilder::new()
//!         .layer(Trace::new())
//!         .layer(OriginAuth::new(Some(secret)).exempt(["/healthz"]))
//!         .layer(SameOrigin::new()),
//! );
//! ```
//!
//! In a `ServiceBuilder` the first layer is the outermost and sees a request first, so put
//! [`Trace`] first to log every request, refused ones included. `Router::layer` wraps the routes
//! the router has so far, so call it after adding them.

use std::pin::Pin;

mod origin_auth;
mod same_origin;
mod trace;

#[cfg(test)]
mod tests;

pub use origin_auth::{OriginAuth, OriginAuthService, OriginSecret};
pub use same_origin::{SameOrigin, SameOriginService};
pub use trace::{Trace, TraceService};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
