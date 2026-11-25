use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

fn kebab(name: &str) -> String {
    let mut word = String::new();
    for (index, character) in name.char_indices() {
        if character.is_uppercase() && index > 0 {
            word.push('-');
        }
        word.extend(character.to_lowercase());
    }
    word
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(ident, "only an enum"));
    };
    let mut variants = Vec::new();
    let mut words = Vec::new();
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(variant, "a word has no fields"));
        }
        words.push(kebab(&variant.ident.to_string()));
        variants.push(&variant.ident);
    }
    Ok(quote! {
        #[automatically_derived]
        impl ::bande_intents::Words for #ident {
            const WORDS: &'static [&'static str] = &[#(#words),*];

            fn word(&self) -> &'static str {
                match self {
                    #(Self::#variants => #words),*
                }
            }

            fn from_word(word: &str) -> ::core::option::Option<Self> {
                match word {
                    #(#words => ::core::option::Option::Some(Self::#variants),)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}
