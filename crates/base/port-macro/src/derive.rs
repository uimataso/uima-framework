use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let input: syn::DeriveInput = syn::parse2(input)?;
    let fields = match &input.data {
        syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Named(fields),
            ..
        }) => &fields.named,
        syn::Data::Struct(data) => {
            return Err(syn::Error::new(
                data.fields.span(),
                "`HasPort` needs named fields",
            ));
        }
        syn::Data::Enum(data) => {
            return Err(syn::Error::new(
                data.enum_token.span(),
                "`HasPort` only supports structs",
            ));
        }
        syn::Data::Union(data) => {
            return Err(syn::Error::new(
                data.union_token.span(),
                "`HasPort` only supports structs",
            ));
        }
    };

    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let mut seen: Vec<String> = Vec::new();
    let mut impls = Vec::new();

    for field in fields {
        let field_ident = &field.ident;
        let ty = &field.ty;
        for port in ports(&field.attrs)? {
            let key = quote!(#port).to_string();
            if seen.contains(&key) {
                return Err(syn::Error::new(
                    port.span(),
                    format!("another field already serves `{key}`"),
                ));
            }
            seen.push(key);

            let last = &port.segments.last().expect("a path has a segment").ident;
            let (has_ident, getter) = crate::has_names(last)?;
            let has = has_path(&port, has_ident);
            let mut where_clause = where_clause.cloned().unwrap_or_else(|| syn::WhereClause {
                where_token: Default::default(),
                predicates: Punctuated::new(),
            });
            // With the bound, a field that doesn't serve the port gets one error, with the
            // port's own message, at the field.
            where_clause.predicates.push(syn::parse_quote!(#ty: #has));

            impls.push(quote! {
                impl #impl_generics #has for #ident #ty_generics #where_clause {
                    type Port = <#ty as #has>::Port;

                    fn #getter(&self) -> &Self::Port {
                        <#ty as #has>::#getter(&self.#field_ident)
                    }
                }
            });
        }
    }

    Ok(quote!(#(#impls)*))
}

/// The ports listed in a field's `#[has_port(...)]` attributes.
fn ports(attrs: &[syn::Attribute]) -> syn::Result<Vec<syn::Path>> {
    let mut ports = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("has_port")) {
        let list =
            attr.parse_args_with(Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)?;
        if list.is_empty() {
            return Err(syn::Error::new(
                attr.meta.span(),
                "`has_port` needs at least one port",
            ));
        }
        for port in list {
            if let Some(seg) = port.segments.iter().find(|s| !s.arguments.is_none()) {
                return Err(syn::Error::new(
                    seg.arguments.span(),
                    "a port takes no generics",
                ));
            }
            ports.push(port);
        }
    }
    Ok(ports)
}

/// `Has<Port>` at the port's path: `ports::UserRepo` gives `ports::HasUserRepo`.
fn has_path(port: &syn::Path, has: syn::Ident) -> syn::Path {
    let mut path = port.clone();
    path.segments
        .last_mut()
        .expect("a path has a segment")
        .ident = has;
    path
}
