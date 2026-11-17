//! Dependency injection for ports.
//!
//! # Examples
//!
//! A context type says which adapter serves each port, and code generic over the context asks for
//! the port by name. [`has_port`] on a port trait `UserRepo` adds the trait `HasUserRepo`, whose
//! `user_repo()` gives the adapter. [`HasPort`] implements it for a context, from the field that
//! serves the port.
//!
//! ```
//! use std::future::Future;
//! use std::sync::Arc;
//!
//! use uima_port_macro::{HasPort, has_port};
//!
//! #[has_port]
//! pub trait UserRepo {
//!     fn name(&self, id: u32) -> impl Future<Output = String> + Send;
//! }
//!
//! struct Postgres;
//!
//! impl UserRepo for Postgres {
//!     async fn name(&self, id: u32) -> String {
//!         format!("user {id}")
//!     }
//! }
//!
//! #[derive(HasPort)]
//! struct Ctx {
//!     #[has_port(UserRepo)]
//!     postgres: Arc<Postgres>,
//! }
//!
//! async fn greet<C: HasUserRepo>(ctx: &C) -> String {
//!     format!("hello, {}", ctx.user_repo().name(1).await)
//! }
//! # let ctx = Ctx { postgres: Arc::new(Postgres) };
//! # let _ = greet(&ctx);
//! ```

mod derive;
mod has_port;

use proc_macro::TokenStream;

/// Adds the trait `Has<Port>` for a port trait `<Port>`, with the getter `<port>()` (snake case)
/// and the trait's visibility.
///
/// `Has<Port>` is implemented for:
///
/// - every `Send + Sync` type that implements the port, trait objects included, giving itself;
/// - `Arc<T>` of a type `T` that implements `Has<Port>`, giving what `T` gives.
///
/// So an `Arc` gets the port through what it holds: an `impl Port for Arc<P>` of your own
/// conflicts with these impls when `P` provides the port too.
///
/// The port trait can't be generic, can't have a method named like the getter, and its getter
/// can't be a keyword. A supertrait's method named like the getter isn't caught here; calls to it
/// are then ambiguous. The attribute takes no arguments.
#[proc_macro_attribute]
pub fn has_port(args: TokenStream, item: TokenStream) -> TokenStream {
    has_port::expand(args.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Implements `Has<Port>` for a struct with named fields, from the field marked
/// `#[has_port(<Port>, ...)]`.
///
/// The field's type must implement `Has<Port>` itself: an adapter, an `Arc` of one, or another
/// context, owned. A borrowed field such as `&'a Adapter` doesn't serve a port, and is reported
/// only where the context is used.
///
/// A port can be served by only one field. The check compares how the ports are written, so the
/// same port written two ways, such as `UserRepo` and `crate::UserRepo`, is reported by rustc as
/// conflicting impls instead. A port named by a path, such as `ports::UserRepo`, takes
/// `Has<Port>` from the same path.
#[proc_macro_derive(HasPort, attributes(has_port))]
pub fn derive_has_port(input: TokenStream) -> TokenStream {
    derive::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// `Has<Port>` and the getter's name for the port named `port`.
///
/// Fails when the getter would be a keyword, such as `type` for a port `Type`.
fn has_names(port: &syn::Ident) -> syn::Result<(syn::Ident, syn::Ident)> {
    use heck::ToSnakeCase as _;
    use syn::ext::IdentExt as _;

    let name = port.unraw().to_string();
    let getter = name.to_snake_case();
    let getter = syn::parse_str::<syn::Ident>(&getter).map_err(|_| {
        syn::Error::new(
            port.span(),
            format!("the getter `{getter}` for this port would be a keyword; rename the port"),
        )
    })?;
    Ok((
        syn::Ident::new(&format!("Has{name}"), port.span()),
        syn::Ident::new(&getter.to_string(), port.span()),
    ))
}
