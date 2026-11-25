use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::named_fields;

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let fields = named_fields(&input)?;
    let kinds = fields.iter().map(|(_, name, field)| {
        let kind = &field.ty;
        quote!((
            ::std::string::String::from(#name),
            <#kind as ::bande_intents::Value>::kind(),
        ))
    });
    let given = fields
        .iter()
        .map(|(ident, name, _)| quote!(::bande_intents::give(&mut values, #name, self.#ident)?;));
    let ident = &input.ident;
    Ok(quote! {
        #[automatically_derived]
        impl ::bande_intents::Record for #ident {
            fn kinds() -> ::std::collections::BTreeMap<::std::string::String, ::bande_intents::Kind> {
                ::std::collections::BTreeMap::from([#(#kinds),*])
            }

            fn into_values(
                self,
            ) -> ::core::result::Result<::bande_intents::Dict, ::bande_intents::Refusal> {
                let mut values = ::bande_intents::Dict::new();
                #(#given)*
                ::core::result::Result::Ok(values)
            }
        }
    })
}
