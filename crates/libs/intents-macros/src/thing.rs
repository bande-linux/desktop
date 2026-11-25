use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Expr};

use crate::{named_fields, required};

const REFERENCE: &str = "reference";
const TITLE: &str = "title";

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut id: Option<Expr> = None;
    let mut title: Option<Expr> = None;
    for attribute in &input.attrs {
        if attribute.path().is_ident("thing") {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("id") {
                    id = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("title") {
                    title = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error("expected id or title"));
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
    for (ident, name, field) in fields.iter().filter(|(_, name, _)| name != REFERENCE) {
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
    Ok(quote! {
        #[automatically_derived]
        impl ::bande_intents::Referent for #ident {
            const TYPES: &'static [&'static str] = &[#id];
        }

        #[automatically_derived]
        impl ::bande_intents::Thing for #ident {
            const TYPE: &'static str = #id;

            fn definition() -> ::bande_intents::ThingDefinition {
                ::bande_intents::ThingDefinition {
                    title: (#label).into(),
                    properties: ::std::collections::BTreeMap::from([#(#properties),*]),
                }
            }

            fn reference(&self) -> &str {
                self.reference.as_str()
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
