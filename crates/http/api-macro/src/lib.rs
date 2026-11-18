//! The derive macro of `uima-http-api`; use it from there.

mod derive;

use proc_macro::TokenStream;

/// Implements `ApiError` for an enum, each of whose variants is one error code.
///
/// A variant's code is its name in kebab case (`PayloadTooLarge` is `payload-too-large`), unless
/// `code` gives one. Its `data` comes from its fields:
///
/// - none give `null`;
/// - named fields give an object of them, by field name;
/// - one unnamed field gives that field.
///
/// The type of each field in `data` implements `Serialize` and utoipa's `ToSchema`. A field that
/// fails to serialize panics in a build with `debug_assertions`, and is `null` in `data` otherwise.
///
/// The attributes, all in `#[api(..)]`:
///
/// - `status = 404` on a variant sets its status, 400 to 599; a variant without one is a `400`.
/// - `code = "oauth-failed"` on a variant sets its error code, in kebab case, for a name the
///   default splits wrongly (`OAuthFailed` is `o-auth-failed`).
/// - `no_data` on a variant makes `data` `null`, so fields such as an `anyhow::Error` stay out of
///   it. `message` is still the `Display`, so keep what a client mustn't see out of it too.
/// - `debug_data` on a variant with one unnamed field makes `data` the field's `{:?}`, such as an
///   `anyhow::Error`'s chain, when the app is built with `debug_assertions`, and `null` otherwise.
///   As with `no_data`, `message` is still the `Display`.
/// - `transparent` on a variant with one unnamed field that implements `ApiError` and `ToSchema`
///   lets the field answer for it, and adds its codes to the schema. Wrap
///   `uima_http_api::FrameworkError` this way to take the framework's codes.
/// - `crate = path` on the enum names `uima_http_api`, for an app that renames it.
///
/// The derive also implements utoipa's `ToSchema`, from `uima_http_api::schema`.
#[proc_macro_derive(ApiError, attributes(api))]
pub fn derive_api_error(input: TokenStream) -> TokenStream {
    derive::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
