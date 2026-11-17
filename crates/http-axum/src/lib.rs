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
//! In a `ServiceBuilder` the first layer is the outermost and sees a request first. Put [`Trace`]
//! first to log every request, refused ones included, and [`OriginAuth`] before [`SameOrigin`], so
//! a request that bypasses the proxy gets a bare `403` and not the service's own refusal.
//! `Router::layer` wraps the routes the router has so far, so call it after adding them.
//! [`OriginAuth`] and [`SameOrigin`] go around a service that returns axum's `Response`, as
//! routes do.
//!
//! # Paths
//!
//! A path given to [`OriginAuth::exempt`] or [`Trace::hide_query_on`] matches a request only when
//! it is the same as the request's path as the layer sees it, the query left out. So `/healthz`
//! doesn't match `/healthz/` or `/HEALTHZ`, and under a `Router::nest` the path leaves out the
//! nest's prefix.

mod origin_auth;
mod paths;
mod same_origin;
mod trace;

#[cfg(test)]
mod tests;

pub use origin_auth::{OriginAuth, OriginSecret};
pub use same_origin::SameOrigin;
pub use trace::{Trace, TraceService};
