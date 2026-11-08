use bande_l10n_source::syntax::{Invocation, Kind};
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;

#[proc_macro]
pub fn tr(input: TokenStream) -> TokenStream {
    expand(Kind::Translate, input)
}

#[proc_macro]
pub fn trn(input: TokenStream) -> TokenStream {
    expand(Kind::Plural, input)
}

#[proc_macro]
pub fn text(input: TokenStream) -> TokenStream {
    expand(Kind::Text, input)
}

fn expand(kind: Kind, input: TokenStream) -> TokenStream {
    let invocation = match Invocation::parse(kind, input.into()) {
        Ok(invocation) => invocation,
        Err(error) => return quote!(::core::compile_error!(#error)).into(),
    };
    let context = match &invocation.context {
        Some(context) => quote!(::core::option::Option::Some(#context)),
        None => quote!(::core::option::Option::None),
    };
    let english = &invocation.english;
    let counted = Ident::new("count", Span::mixed_site());
    let names = invocation.args.iter().map(|(name, _)| name);
    let values = invocation
        .args
        .iter()
        .enumerate()
        .map(|(index, (_, value))| {
            if kind == Kind::Plural && index == 0 {
                quote!(#counted)
            } else {
                quote!(&(#value))
            }
        });
    let args = quote!(&[#((#names, #values as &dyn ::core::fmt::Display)),*]);
    match (kind, &invocation.plural, invocation.args.first()) {
        (Kind::Plural, Some(plural), Some((_, count))) => quote!({
            let #counted = &(#count);
            ::bande_l10n::translate_plural(#context, #english, #plural, ::bande_l10n::Count::count(#counted), #args)
        }),
        (Kind::Text, _, _) => quote!(::bande_l10n::Text::new(#context, #english)),
        _ => quote!(::bande_l10n::translate(#context, #english, #args)),
    }
    .into()
}
