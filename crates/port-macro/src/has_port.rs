use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned as _;

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !args.is_empty() {
        return Err(syn::Error::new(
            args.span(),
            "`has_port` takes no arguments",
        ));
    }
    let item: syn::ItemTrait = syn::parse2(item)?;
    let port = &item.ident;
    let (has, getter) = crate::has_names(port)?;
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(
            item.generics.span(),
            "`has_port` doesn't support generic traits",
        ));
    }

    if let Some(clash) = item.items.iter().find_map(|i| match i {
        syn::TraitItem::Fn(f) if f.sig.ident == getter => Some(&f.sig.ident),
        _ => None,
    }) {
        return Err(syn::Error::new(
            clash.span(),
            format!("`has_port` adds a getter `{getter}`, which this method would clash with"),
        ));
    }

    let vis = &item.vis;
    let doc = format!("A context that provides a [`{port}`].");
    let getter_doc = format!("The [`{port}`] this context provides.");
    let message = format!("`{{Self}}` doesn't provide a `{port}`");
    let note = format!(
        "implement `{port}` for the adapter, or serve it from a field with `#[has_port({port})]`"
    );

    Ok(quote! {
        #item

        #[doc = #doc]
        #[diagnostic::on_unimplemented(message = #message, note = #note)]
        #vis trait #has {
            /// The adapter that serves the port.
            type Port: #port + ::core::marker::Send + ::core::marker::Sync + ?::core::marker::Sized;

            #[doc = #getter_doc]
            fn #getter(&self) -> &Self::Port;
        }

        impl<T: #port + ::core::marker::Send + ::core::marker::Sync + ?::core::marker::Sized> #has for T {
            type Port = Self;

            fn #getter(&self) -> &Self::Port {
                self
            }
        }

        impl<T: #has + ?::core::marker::Sized> #has for ::std::sync::Arc<T> {
            type Port = T::Port;

            fn #getter(&self) -> &Self::Port {
                T::#getter(self)
            }
        }
    })
}
