//! The derive macro of `uima-http-axum-api`; use it from there.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/// Implements axum's `IntoResponse` for a type that implements `ApiError` and `Debug`, through
/// `uima_http_axum_api::error_response`.
///
/// `#[into_response(crate = path)]` on the type names `uima_http_axum_api`, for an app that renames
/// it.
#[proc_macro_derive(IntoResponse, attributes(into_response))]
pub fn derive_into_response(input: TokenStream) -> TokenStream {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: TokenStream2) -> syn::Result<TokenStream2> {
    let input: syn::DeriveInput = syn::parse2(input)?;
    let krate = crate_path(&input.attrs)?;
    let ident = &input.ident;
    let mut generics = input.generics.clone();
    // So a generic type's impl holds where the type is an `ApiError` and `Debug`.
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(Self: #krate::__private::ApiError + ::core::fmt::Debug));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let response = quote!(#krate::__private::axum::response);
    Ok(quote! {
        impl #impl_generics #response::IntoResponse for #ident #ty_generics #where_clause {
            fn into_response(self) -> #response::Response {
                #krate::error_response(&self)
            }
        }
    })
}

/// The path to `uima-http-axum-api`, from `#[into_response(crate = path)]`, or
/// `::uima_http_axum_api`.
fn crate_path(attrs: &[syn::Attribute]) -> syn::Result<syn::Path> {
    let mut krate = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("into_response")) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("crate") {
                return Err(meta.error("expected `crate`"));
            }
            if krate.is_some() {
                return Err(meta.error("`crate` is given twice"));
            }
            krate = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(krate.unwrap_or_else(|| syn::parse_quote!(::uima_http_axum_api)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_names_the_path_the_code_goes_through() {
        let input = quote!(
            #[into_response(crate = renamed)]
            struct A<T>(T);
        );

        let output = expand(input).unwrap().to_string();

        assert!(output.contains("renamed :: error_response"), "{output}");
        assert!(
            output.contains("renamed :: __private :: ApiError"),
            "{output}"
        );
        assert!(
            output.contains("renamed :: __private :: axum :: response"),
            "{output}"
        );
        assert!(!output.contains("uima_http_axum_api"), "{output}");
    }
}
