use heck::ToKebabCase as _;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let input: syn::DeriveInput = syn::parse2(input)?;
    let syn::Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "`ApiError` only supports enums",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "`ApiError` doesn't support generics",
        ));
    }
    if data.variants.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            "`ApiError` needs at least one variant",
        ));
    }
    let krate = crate_path(&input.attrs)?;
    let variants = data
        .variants
        .iter()
        .map(Variant::parse)
        .collect::<syn::Result<Vec<_>>>()?;
    let mut codes: Vec<(String, &syn::Ident)> = Vec::new();
    for variant in &variants {
        if let Kind::Own { .. } = variant.kind {
            let code = variant.code();
            if let Some((_, other)) = codes.iter().find(|(seen, _)| *seen == code) {
                return Err(syn::Error::new(
                    variant.ident.span(),
                    format!("`{other}` already has the error code `{code}`"),
                ));
            }
            codes.push((code, &variant.ident));
        }
    }

    let ident = &input.ident;
    let name = ident.unraw().to_string();
    let private = quote!(#krate::__private);
    let utoipa = quote!(#private::utoipa);
    let status = variants.iter().map(|v| v.status(&krate, &private));
    let error_code = variants.iter().map(|v| v.error_code(&krate));
    let data = variants.iter().map(|v| v.data(&krate, &private));
    let code_schemas = variants.iter().map(|v| v.code_schemas(&krate, &private));
    let referred = variants.iter().flat_map(Variant::types);
    let codes = local("codes");

    Ok(quote! {
        impl #krate::ApiError for #ident {
            fn status(&self) -> #private::http::StatusCode {
                match self { #(#status)* }
            }

            fn error_code(&self) -> &'static str {
                match self { #(#error_code)* }
            }

            fn data(&self) -> ::core::option::Option<#private::serde_json::Value> {
                match self { #(#data)* }
            }

            fn code_schemas() -> ::std::vec::Vec<#krate::CodeSchema> {
                let mut #codes = ::std::vec::Vec::new();
                #(#code_schemas)*
                #codes
            }
        }

        impl #utoipa::PartialSchema for #ident {
            fn schema() -> #utoipa::openapi::RefOr<#utoipa::openapi::schema::Schema> {
                #utoipa::openapi::RefOr::T(#krate::schema::<Self>())
            }
        }

        impl #utoipa::ToSchema for #ident {
            fn name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#name)
            }

            fn schemas(
                schemas: &mut ::std::vec::Vec<(
                    ::std::string::String,
                    #utoipa::openapi::RefOr<#utoipa::openapi::schema::Schema>,
                )>,
            ) {
                #(<#referred as #utoipa::ToSchema>::schemas(schemas);)*
            }
        }
    })
}

/// The path to `uima-http-api`, from `#[api(crate = path)]` on the enum, or
/// `::uima_http_api`.
fn crate_path(attrs: &[syn::Attribute]) -> syn::Result<syn::Path> {
    let mut krate = None;
    for attr in attrs.iter().filter(|a| a.path().is_ident("api")) {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("crate") {
                return Err(meta.error("expected `crate` on the enum"));
            }
            if krate.is_some() {
                return Err(meta.error("`crate` is given twice"));
            }
            krate = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(krate.unwrap_or_else(|| syn::parse_quote!(::uima_http_api)))
}

struct Variant {
    ident: syn::Ident,
    /// `#[api(code = "..")]`, if given.
    code: Option<String>,
    kind: Kind,
}

enum Kind {
    /// The inner error, of this type, answers for it.
    Transparent(Box<syn::Type>),
    Own {
        status: u16,
        data: Data,
    },
}

enum Data {
    /// `null`.
    Null,
    /// The named fields, as an object.
    Fields(Vec<(syn::Ident, syn::Type)>),
    /// The one field of a tuple variant.
    Field(Box<syn::Type>),
    /// The one field's `{:?}` in a debug build, `null` otherwise.
    Debug,
}

impl Variant {
    fn parse(variant: &syn::Variant) -> syn::Result<Self> {
        for field in &variant.fields {
            if let Some(attr) = field.attrs.iter().find(|a| a.path().is_ident("api")) {
                return Err(syn::Error::new(
                    attr.span(),
                    "`api` goes on the variant, not on a field",
                ));
            }
        }
        let mut status = None;
        let mut code = None;
        let (mut transparent, mut no_data, mut debug_data) = (false, false, false);
        for attr in variant.attrs.iter().filter(|a| a.path().is_ident("api")) {
            attr.parse_nested_meta(|meta| {
                let flag = if meta.path.is_ident("status") {
                    if status.is_some() {
                        return Err(meta.error("`status` is given twice"));
                    }
                    let lit: syn::LitInt = meta.value()?.parse()?;
                    if !lit.suffix().is_empty() {
                        return Err(syn::Error::new(lit.span(), "a status has no suffix"));
                    }
                    let code: u16 = lit.base10_parse()?;
                    if !(400..=599).contains(&code) {
                        return Err(syn::Error::new(
                            lit.span(),
                            "an error's status is 400 to 599",
                        ));
                    }
                    status = Some(code);
                    return Ok(());
                } else if meta.path.is_ident("code") {
                    if code.is_some() {
                        return Err(meta.error("`code` is given twice"));
                    }
                    let lit: syn::LitStr = meta.value()?.parse()?;
                    let value = lit.value();
                    if !is_kebab(&value) {
                        return Err(syn::Error::new(
                            lit.span(),
                            "a code is in kebab case: lowercase letters and digits, joined by `-`",
                        ));
                    }
                    code = Some(value);
                    return Ok(());
                } else if meta.path.is_ident("transparent") {
                    &mut transparent
                } else if meta.path.is_ident("no_data") {
                    &mut no_data
                } else if meta.path.is_ident("debug_data") {
                    &mut debug_data
                } else {
                    return Err(meta.error(
                        "expected `status`, `code`, `transparent`, `no_data` or `debug_data`",
                    ));
                };
                if *flag {
                    let name = meta.path.get_ident().expect("checked above");
                    return Err(meta.error(format!("`{name}` is given twice")));
                }
                *flag = true;
                Ok(())
            })?;
        }

        let one_field = match &variant.fields {
            syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                Some(Box::new(fields.unnamed[0].ty.clone()))
            }
            _ => None,
        };
        let span = variant.ident.span();
        let kind = if transparent {
            if status.is_some() || code.is_some() || no_data || debug_data {
                return Err(syn::Error::new(
                    span,
                    "`transparent` takes no other `api` attribute",
                ));
            }
            let Some(ty) = one_field else {
                return Err(syn::Error::new(
                    span,
                    "`transparent` needs one unnamed field",
                ));
            };
            Kind::Transparent(ty)
        } else {
            let data = match (no_data, debug_data, &variant.fields) {
                (true, true, _) => {
                    return Err(syn::Error::new(
                        span,
                        "`no_data` and `debug_data` don't go together",
                    ));
                }
                (true, false, _) | (false, false, syn::Fields::Unit) => Data::Null,
                (false, true, _) => {
                    if one_field.is_none() {
                        return Err(syn::Error::new(
                            span,
                            "`debug_data` needs one unnamed field",
                        ));
                    }
                    Data::Debug
                }
                (false, false, syn::Fields::Named(fields)) => Data::Fields(
                    fields
                        .named
                        .iter()
                        .map(|f| (f.ident.clone().expect("a named field"), f.ty.clone()))
                        .collect(),
                ),
                (false, false, syn::Fields::Unnamed(_)) => match one_field {
                    Some(ty) => Data::Field(ty),
                    None => {
                        return Err(syn::Error::new(
                            variant.fields.span(),
                            "a variant's data is one unnamed field or named fields; \
                             add `no_data` to leave them out",
                        ));
                    }
                },
            };
            Kind::Own {
                status: status.unwrap_or(400),
                data,
            }
        };

        let this = Self {
            ident: variant.ident.clone(),
            code,
            kind,
        };
        if let Kind::Own { .. } = this.kind
            && !is_kebab(&this.code())
        {
            return Err(syn::Error::new(
                span,
                format!(
                    "the code `{}` from this name isn't in kebab case; give one with `code`",
                    this.code()
                ),
            ));
        }
        Ok(this)
    }

    fn code(&self) -> String {
        self.code
            .clone()
            .unwrap_or_else(|| self.ident.unraw().to_string().to_kebab_case())
    }

    /// The types whose `ToSchema::schemas` the enum's takes in.
    fn types(&self) -> Vec<&syn::Type> {
        match &self.kind {
            Kind::Transparent(ty)
            | Kind::Own {
                data: Data::Field(ty),
                ..
            } => vec![ty],
            Kind::Own {
                data: Data::Fields(fields),
                ..
            } => fields.iter().map(|(_, ty)| ty).collect(),
            Kind::Own { .. } => Vec::new(),
        }
    }

    /// `inner.method()` through the trait for a transparent variant, so an inherent method of
    /// the same name doesn't answer instead; `own` otherwise.
    fn arm(&self, krate: &syn::Path, method: &str, own: TokenStream) -> TokenStream {
        let ident = &self.ident;
        match &self.kind {
            Kind::Transparent(ty) => {
                let method = syn::Ident::new(method, Span::call_site());
                let inner = local("inner");
                quote!(Self::#ident(#inner) => <#ty as #krate::ApiError>::#method(#inner),)
            }
            Kind::Own { .. } => quote!(Self::#ident { .. } => #own,),
        }
    }

    fn error_code(&self, krate: &syn::Path) -> TokenStream {
        let code = self.code();
        self.arm(krate, "error_code", quote!(#code))
    }

    fn status(&self, krate: &syn::Path, private: &TokenStream) -> TokenStream {
        let own = match &self.kind {
            Kind::Own { status, .. } => status_value(*status, private),
            Kind::Transparent(_) => TokenStream::new(),
        };
        self.arm(krate, "status", own)
    }

    fn data(&self, krate: &syn::Path, private: &TokenStream) -> TokenStream {
        let ident = &self.ident;
        let some = quote!(::core::option::Option::Some);
        let json = quote!(#private::serde_json);
        let code = self.code();
        let (field, value, e) = (local("field"), local("value"), local("e"));
        let to_value = |input: TokenStream| {
            quote! {
                match #json::to_value(#input) {
                    ::core::result::Result::Ok(#value) => #value,
                    ::core::result::Result::Err(#e) => {
                        if ::core::cfg!(debug_assertions) {
                            ::core::panic!("the data of `{}` doesn't serialize: {}", #code, #e);
                        }
                        #json::Value::Null
                    }
                }
            }
        };
        let Kind::Own { data, .. } = &self.kind else {
            return self.arm(krate, "data", TokenStream::new());
        };
        match data {
            Data::Null => quote!(Self::#ident { .. } => ::core::option::Option::None,),
            Data::Field(_) => {
                let value = to_value(quote!(#field));
                quote!(Self::#ident(#field) => #some(#value),)
            }
            Data::Debug => quote! {
                Self::#ident(#field) => {
                    if ::core::cfg!(debug_assertions) {
                        #some(#json::Value::String(::std::format!("{:?}", #field)))
                    } else {
                        ::core::option::Option::None
                    }
                }
            },
            Data::Fields(fields) => {
                let idents: Vec<_> = fields.iter().map(|(ident, _)| ident).collect();
                let names = idents.iter().map(|ident| ident.unraw().to_string());
                // Bound to locals, since a field binds as `{ n }` only when no constant `n` is
                // in scope.
                let locals: Vec<_> = (0..idents.len())
                    .map(|i| local(&format!("field_{i}")))
                    .collect();
                let values = locals.iter().map(|local| to_value(quote!(#local)));
                quote! {
                    Self::#ident { #(#idents: #locals),* } => #some(#json::Value::Object(
                        ::core::iter::Iterator::collect(::core::iter::IntoIterator::into_iter([
                            #((::std::string::String::from(#names), #values)),*
                        ]))
                    )),
                }
            }
        }
    }

    /// Pushes the variant's `CodeSchema`s onto `codes`.
    fn code_schemas(&self, krate: &syn::Path, private: &TokenStream) -> TokenStream {
        let utoipa = quote!(#private::utoipa);
        let schema = quote!(#utoipa::openapi::schema);
        let (status, data) = match &self.kind {
            Kind::Transparent(ty) => {
                let codes = local("codes");
                return quote!(#codes.extend(<#ty as #krate::ApiError>::code_schemas()););
            }
            Kind::Own { status, data } => (status, data),
        };
        let code = self.code();
        let codes = local("codes");
        let data = match data {
            Data::Null => quote!(#private::null_schema()),
            Data::Debug => quote!(#private::debug_schema()),
            Data::Field(ty) => quote!(<#ty as #utoipa::PartialSchema>::schema()),
            Data::Fields(fields) => {
                let names: Vec<_> = fields
                    .iter()
                    .map(|(ident, _)| ident.unraw().to_string())
                    .collect();
                let types: Vec<_> = fields.iter().map(|(_, ty)| ty).collect();
                quote! {
                    #utoipa::openapi::RefOr::T(#schema::Schema::Object(
                        #schema::ObjectBuilder::new()
                            .schema_type(#schema::Type::Object)
                            #(
                                .property(#names, <#types as #utoipa::PartialSchema>::schema())
                                .required(#names)
                            )*
                            .build()
                    ))
                }
            }
        };
        let status = status_value(*status, private);
        quote! {
            #codes.push(#krate::CodeSchema::new(#code, #status, #data));
        }
    }
}

fn status_value(status: u16, private: &TokenStream) -> TokenStream {
    quote!(#private::http::StatusCode::from_u16(#status).expect("the derive checked the status"))
}

/// Whether `code` is lowercase letters and digits, joined by single `-`.
fn is_kebab(code: &str) -> bool {
    code.split('-').all(|word| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

/// A local of the generated code, prefixed, as serde does, so that no item of the app, such as a
/// constant, has its name.
fn local(name: &str) -> syn::Ident {
    syn::Ident::new(&format!("__api_error_{name}"), Span::call_site())
}
