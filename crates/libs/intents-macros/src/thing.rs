use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{DeriveInput, Expr, Token, bracketed};

use crate::{named_fields, required};

const REFERENCE: &str = "reference";
const TITLE: &str = "title";
const FOREIGN: &str = "foreign";

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut id: Option<Expr> = None;
    let mut title: Option<Expr> = None;
    let mut view_of: Vec<Expr> = Vec::new();
    for attribute in &input.attrs {
        if attribute.path().is_ident("thing") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    id = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("title") {
                    title = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("view_of") {
                    let content;
                    let list = meta.value()?;
                    bracketed!(content in list);
                    view_of = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                } else {
                    return Err(meta.error("expected id, title or view_of"));
                }
                Ok(())
            })?;
        }
    }
    let id = required(id, &input.ident, "id")?;
    let label = required(title, &input.ident, "title")?;
    let fields = named_fields(&input)?;
    for needed in [REFERENCE, TITLE] {
        if !fields.iter().any(|(_, name, _)| name == needed) {
            return Err(syn::Error::new_spanned(
                &input.ident,
                format!("a thing has a `{needed}`"),
            ));
        }
    }
    let mut properties = Vec::new();
    let mut given = Vec::new();
    let also_foreign = match fields.iter().find(|(_, name, _)| name == FOREIGN) {
        Some((ident, ..)) => quote!(self.#ident.clone()),
        None => quote!(::std::vec::Vec::new()),
    };
    for (ident, name, field) in fields
        .iter()
        .filter(|(_, name, _)| name != REFERENCE && name != FOREIGN)
    {
        let (mut find, mut foreign, mut content) = (name == TITLE, false, false);
        for attribute in &field.attrs {
            if attribute.path().is_ident("thing") {
                attribute.parse_nested_meta(|meta| {
                    if meta.path.is_ident("find") {
                        find = true;
                    } else if meta.path.is_ident("foreign") {
                        foreign = true;
                    } else if meta.path.is_ident("content") {
                        content = true;
                    } else {
                        return Err(meta.error("expected find, foreign or content"));
                    }
                    Ok(())
                })?;
            }
        }
        let kind = &field.ty;
        properties.push(quote!((
            ::std::string::String::from(#name),
            ::bande_intents::Property::of::<#kind>(#find, #foreign, #content),
        )));
        given.push(quote!(::bande_intents::give(&mut properties, #name, self.#ident)?;));
    }
    let ident = &input.ident;
    let types = match view_of.is_empty() {
        true => quote!(#id),
        false => quote!(#(#view_of),*),
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::bande_intents::Referent for #ident {
            const TYPES: &'static [&'static str] = &[#types];
        }

        #[automatically_derived]
        impl ::bande_intents::Thing for #ident {
            const TYPE: &'static str = #id;

            fn definition() -> ::bande_intents::ThingDefinition {
                ::bande_intents::ThingDefinition {
                    title: (#label).into(),
                    view_of: ::std::vec![#(::std::string::String::from(#view_of)),*],
                    properties: ::std::collections::BTreeMap::from([#(#properties),*]),
                }
            }

            fn reference(&self) -> &str {
                self.reference.as_str()
            }

            fn also_foreign(&self) -> ::std::vec::Vec<&'static str> {
                #also_foreign
            }

            fn properties(
                self,
            ) -> ::core::result::Result<::bande_intents::Dict, ::bande_intents::Refusal> {
                let mut properties = ::bande_intents::Dict::new();
                #(#given)*
                ::core::result::Result::Ok(properties)
            }
        }
    })
}
