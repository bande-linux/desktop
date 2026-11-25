use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{DeriveInput, Expr, Ident, Token, bracketed, parenthesized};

use crate::{description, named_fields, required};

struct Phrase {
    words: Expr,
    arguments: Vec<(String, Expr)>,
}

#[derive(Default)]
struct Attributes {
    id: Option<Expr>,
    title: Option<Expr>,
    effects: Option<Punctuated<Ident, Token![,]>>,
    phrases: Vec<Phrase>,
    undo: bool,
}

impl syn::parse::Parse for Phrase {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        if name != "phrase" {
            return Err(syn::Error::new_spanned(name, "expected `phrase(…)`"));
        }
        let content;
        parenthesized!(content in input);
        let words = content.parse()?;
        let mut arguments = Vec::new();
        while content.parse::<Option<Token![,]>>()?.is_some() && !content.is_empty() {
            let argument: Ident = content.parse()?;
            content.parse::<Token![=]>()?;
            arguments.push((argument.to_string(), content.parse()?));
        }
        Ok(Self { words, arguments })
    }
}

fn attributes(input: &DeriveInput) -> syn::Result<Attributes> {
    let mut found = Attributes::default();
    for attribute in &input.attrs {
        if !attribute.path().is_ident("intent") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            let name = meta.path.get_ident().map(ToString::to_string);
            match name.as_deref() {
                Some("id") => found.id = Some(meta.value()?.parse()?),
                Some("title") => found.title = Some(meta.value()?.parse()?),
                Some("effects") => {
                    let content;
                    let list = meta.value()?;
                    bracketed!(content in list);
                    found.effects = Some(Punctuated::parse_terminated(&content)?);
                }
                Some("phrases") => {
                    let content;
                    let list = meta.value()?;
                    bracketed!(content in list);
                    found.phrases = Punctuated::<Phrase, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                }
                Some("undo") => found.undo = true,
                _ => return Err(meta.error("expected id, title, effects, undo or phrases")),
            }
            Ok(())
        })?;
    }
    Ok(found)
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let found = attributes(&input)?;
    let id = required(found.id, &input.ident, "id")?;
    let title = required(found.title, &input.ident, "title")?;
    let effects = required(found.effects, &input.ident, "effects")?;
    let effects = effects.iter();
    let undo = found.undo;
    let phrases = found.phrases.iter().map(|phrase| {
        let words = &phrase.words;
        let arguments = phrase
            .arguments
            .iter()
            .map(|(name, value)| quote!((#name, #value)));
        quote!(::bande_intents::Phrase::new(#words, &[#(#arguments),*]))
    });
    let fields = named_fields(&input)?;
    let parameters = fields.iter().map(|(_, name, field)| {
        let described = description(&field.attrs);
        let kind = &field.ty;
        quote!(::bande_intents::Parameter::of::<#kind>(#name, #described))
    });
    let taken = fields
        .iter()
        .map(|(ident, name, _)| quote!(#ident: ::bande_intents::take(&mut arguments, #name)?));
    let given = fields.iter().map(
        |(ident, name, _)| quote!(::bande_intents::give(&mut arguments, #name, self.#ident)?;),
    );
    let names = fields.iter().map(|(_, name, _)| name);
    let described = description(&input.attrs);
    let ident = &input.ident;
    Ok(quote! {
        #[automatically_derived]
        impl ::bande_intents::Intent for #ident {
            const ID: &'static str = #id;

            fn definition() -> ::bande_intents::Definition {
                ::bande_intents::Definition {
                    title: (#title).into(),
                    description: ::std::string::String::from(#described),
                    effects: ::std::vec![#(::bande_intents::Effect::#effects),*],
                    undo: #undo,
                    phrases: ::std::vec![#(#phrases),*],
                    parameters: ::std::vec![#(#parameters),*],
                    returns: ::core::option::Option::None,
                }
            }

            fn decode(
                mut arguments: ::bande_intents::Dict,
            ) -> ::core::result::Result<Self, ::bande_intents::Refusal> {
                ::bande_intents::reject_unknown(Self::ID, &arguments, &[#(#names),*])?;
                ::core::result::Result::Ok(Self { #(#taken),* })
            }

            fn encode(
                self,
            ) -> ::core::result::Result<::bande_intents::Dict, ::bande_intents::Refusal> {
                #[allow(unused_mut)]
                let mut arguments = ::bande_intents::Dict::new();
                #(#given)*
                ::core::result::Result::Ok(arguments)
            }
        }
    })
}
